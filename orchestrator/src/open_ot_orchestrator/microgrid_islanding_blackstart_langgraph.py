"""Microgrid `:loop:islanding-decision` extended — ANTI_ISLANDING_ROCOF + BLACK_START_SEQ coordination.

Per PROTOTYPE-MICROGRID.md §13.2. Two-cell FSM coordination:

  1. **ANTI_ISLANDING_ROCOF** monitors grid frequency / voltage envelopes
     + ROCOF. On trip → `grid_present = False`.
  2. **BLACK_START_SEQ** observes `grid_present` and walks the 5-stage
     restart sequence (Detecting → StartingGen → EnergizingBus → Syncing
     → Connected) if authorised.

```text
   START → anti_islanding_rocof ─→ status_router ─→ black_start_seq → END
```

The `status_router` translates the protection cell's output (trip bool,
trip reason) into the black-start cell's inputs (grid_present,
authorised). This is the **first chained-FSM** orchestrator demo —
shows how protection and restart logic compose without sharing cell-side
state.

Run: `uv run python -m open_ot_orchestrator.microgrid_islanding_blackstart_langgraph`
"""

from __future__ import annotations

from operator import or_
from pathlib import Path
from typing import Annotated, Any, TypedDict

from langgraph.checkpoint.memory import MemorySaver
from langgraph.graph import END, START, StateGraph

from ._generated import anti_islanding_rocof as anti_gen
from ._generated import black_start_seq as bs_gen
from .cell_loader import CellLoader, OUT_EVENT_WIDTH_PACKED_U16

REPO_ROOT = Path(__file__).resolve().parents[5]
CELLS_TARGET = (
    REPO_ROOT
    / "60-apps/etzhayyim-project-open-ot/cells/target/wasm32-unknown-unknown/release"
)
ANTI_WASM_PATH = CELLS_TARGET / "anti_islanding_rocof.wasm"
BS_WASM_PATH = CELLS_TARGET / "black_start_seq.wasm"


class IslandingState(TypedDict, total=False):
    """Per-super-step state for the chained islanding+blackstart loop."""

    # Inputs to the protection cell.
    grid_freq_hz: float
    freq_nominal_hz: float
    grid_voltage_v: float
    voltage_nominal_v: float
    # Inputs to the blackstart cell (gen / bus / sync sensors).
    gen_ready: bool
    bus_voltage_stable: bool
    voltage_synced: bool
    authorised: bool
    # Outputs.
    protection_state: str  # ANTI_ISLANDING_ROCOF ECC name
    protection_trip: bool
    protection_trip_reason: int
    blackstart_state: str
    blackstart_stage: int
    blackstart_command: str
    blackstart_connected: bool
    # Per-cell checkpoint.
    cell_internals: Annotated[dict[str, bytes], or_]
    cell_ecc_states: Annotated[dict[str, int], or_]


_ANTI_ECC = {i: n for i, n in enumerate(anti_gen.ECC_STATES)}
_BS_ECC = {i: n for i, n in enumerate(bs_gen.ECC_STATES)}
_BS_COMMANDS = {
    0: "None",
    1: "StartGen",
    2: "EnergizeBus",
    3: "WaitSync",
    4: "CloseTieBreaker",
    5: "HoldConnected",
}

# Keys for cell_internals storage.
ANTI_KEY = "protection_anti_islanding"
BS_KEY = "blackstart_seq"


def _anti_islanding_node(cycle_period_ms: int):
    """ANTI_ISLANDING_ROCOF node."""
    loader = CellLoader(
        ANTI_WASM_PATH,
        anti_gen.CELL_SYMBOL,
        out_event_width=OUT_EVENT_WIDTH_PACKED_U16,
    )
    # ENTSO-E defaults.
    params = anti_gen.Params(
        rocof_threshold_micro_hz_per_s=500_000,  # 0.5 Hz/s
        rocof_window_samples=3,
        voltage_min_micro_v=207_000_000,
        voltage_max_micro_v=253_000_000,
        voltage_window_samples=3,
        freq_min_micro_hz=49_500_000,
        freq_max_micro_hz=50_500_000,
        freq_window_samples=3,
        cycle_period_ms=cycle_period_ms,
    )
    params_bytes = anti_gen.pack_params(params)
    initialized = {"done": False}

    def node(state: IslandingState) -> dict[str, Any]:
        prior = state.get("cell_internals") or {}
        if ANTI_KEY in prior:
            if not initialized["done"]:
                loader.init(params_bytes, anti_gen.INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior[ANTI_KEY])
        else:
            loader.init(params_bytes, anti_gen.INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(ANTI_KEY, 0)
        f = state.get("grid_freq_hz", 50.0)
        fn = state.get("freq_nominal_hz", 50.0)
        v = state.get("grid_voltage_v", 230.0)
        vn = state.get("voltage_nominal_v", 230.0)

        data_in = anti_gen.DataIn(
            grid_freq=int(round(f * 1_000_000)),
            freq_nominal=int(round(fn * 1_000_000)),
            grid_voltage=int(round(v * 1_000_000)),
            voltage_nominal=int(round(vn * 1_000_000)),
            freq_quality=0,
            voltage_quality=0,
            enable=True,
        )
        result = loader.tick(
            event_in=0,  # REQ
            data_in_bytes=anti_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=anti_gen.DATA_OUT_SIZE,
        )
        out = anti_gen.unpack_data_out(result.data_out_bytes)
        return {
            "protection_state": _ANTI_ECC.get(result.next_ecc_state, "?"),
            "protection_trip": bool(out.trip),
            "protection_trip_reason": int(out.trip_reason),
            "cell_internals": {
                ANTI_KEY: loader.get_internal_bytes(anti_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {ANTI_KEY: result.next_ecc_state},
        }

    return node


def _status_router(state: IslandingState) -> dict[str, Any]:
    """Translate protection cell output → blackstart cell input.

    `grid_present = !protection_trip`. The protection cell trips on
    islanding-style anomalies; the blackstart cell uses `grid_present` to
    advance through stages.
    """
    trip = state.get("protection_trip", False)
    # We don't write the grid_present key here — the demo / step()
    # supplies it explicitly. This router is reserved for future
    # cross-stage conditioning (e.g., latched override).
    return {}


def _blackstart_node(cycle_period_ms: int):
    """BLACK_START_SEQ node."""
    loader = CellLoader(BS_WASM_PATH, bs_gen.CELL_SYMBOL)
    params = bs_gen.Params(
        detect_dwell_ms=5_000,
        gen_timeout_ms=60_000,
        bus_timeout_ms=30_000,
        sync_timeout_ms=120_000,
        cycle_period_ms=cycle_period_ms,
    )
    params_bytes = bs_gen.pack_params(params)
    initialized = {"done": False}

    def node(state: IslandingState) -> dict[str, Any]:
        prior = state.get("cell_internals") or {}
        if BS_KEY in prior:
            if not initialized["done"]:
                loader.init(params_bytes, bs_gen.INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior[BS_KEY])
        else:
            loader.init(params_bytes, bs_gen.INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(BS_KEY, 0)
        # grid_present is the inverse of protection_trip.
        grid_present = not state.get("protection_trip", False)

        data_in = bs_gen.DataIn(
            grid_present=grid_present,
            gen_ready=state.get("gen_ready", False),
            bus_voltage_stable=state.get("bus_voltage_stable", False),
            voltage_synced=state.get("voltage_synced", False),
            authorised=state.get("authorised", True),
        )
        result = loader.tick(
            event_in=0,  # REQ
            data_in_bytes=bs_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=bs_gen.DATA_OUT_SIZE,
        )
        out = bs_gen.unpack_data_out(result.data_out_bytes)
        return {
            "blackstart_state": _BS_ECC.get(result.next_ecc_state, "?"),
            "blackstart_stage": int(out.stage),
            "blackstart_command": _BS_COMMANDS.get(out.command, "?"),
            "blackstart_connected": bool(out.connected),
            "cell_internals": {
                BS_KEY: loader.get_internal_bytes(bs_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {BS_KEY: result.next_ecc_state},
        }

    return node


def build_islanding_blackstart_graph(cycle_period_ms: int = 1_000):
    """Build the chained protection → blackstart graph."""
    graph = StateGraph(IslandingState)
    graph.add_node("anti_islanding_rocof", _anti_islanding_node(cycle_period_ms))
    graph.add_node("status_router", _status_router)
    graph.add_node("black_start_seq", _blackstart_node(cycle_period_ms))
    graph.add_edge(START, "anti_islanding_rocof")
    graph.add_edge("anti_islanding_rocof", "status_router")
    graph.add_edge("status_router", "black_start_seq")
    graph.add_edge("black_start_seq", END)
    return graph.compile(checkpointer=MemorySaver())


def step(
    app,
    config: dict[str, Any],
    *,
    grid_freq_hz: float,
    grid_voltage_v: float,
    gen_ready: bool = False,
    bus_voltage_stable: bool = False,
    voltage_synced: bool = False,
    authorised: bool = True,
) -> IslandingState:
    return app.invoke(
        {
            "grid_freq_hz": grid_freq_hz,
            "freq_nominal_hz": 50.0,
            "grid_voltage_v": grid_voltage_v,
            "voltage_nominal_v": 230.0,
            "gen_ready": gen_ready,
            "bus_voltage_stable": bus_voltage_stable,
            "voltage_synced": voltage_synced,
            "authorised": authorised,
        },
        config=config,
    )


def demo() -> None:
    app = build_islanding_blackstart_graph(cycle_period_ms=1_000)
    config = {"configurable": {"thread_id": "islanding-blackstart-demo"}}

    print("[islanding+blackstart] simulating outage → detect → restart sequence")
    # Schedule: nominal → severe under-freq (trip) → grid stays gone → gen
    # comes ready → bus stabilises → sync achieves → connected.
    schedule = [
        # (label,            f,     v,     gen, bus,  sync)
        ("nominal",          50.0,  230.0, False, False, False),
        ("nominal",          50.0,  230.0, False, False, False),
        ("under-freq trip",  49.0,  230.0, False, False, False),
        ("under-freq trip",  49.0,  230.0, False, False, False),
        ("under-freq trip",  49.0,  230.0, False, False, False),
        ("under-freq trip",  49.0,  230.0, False, False, False),  # outage confirmed
        ("dwell",            49.0,  230.0, False, False, False),  # detect dwell
        ("dwell",            49.0,  230.0, False, False, False),
        ("dwell",            49.0,  230.0, False, False, False),
        ("dwell",            49.0,  230.0, False, False, False),
        ("dwell",            49.0,  230.0, False, False, False),
        ("gen ready",        49.0,  230.0, True,  False, False),
        ("bus up",           49.0,  230.0, True,  True,  False),
        ("grid back+sync",   50.0,  230.0, True,  True,  True),
    ]
    for i, (label, f, v, gen, bus, sync) in enumerate(schedule, 1):
        s = step(
            app, config,
            grid_freq_hz=f, grid_voltage_v=v,
            gen_ready=gen, bus_voltage_stable=bus, voltage_synced=sync,
        )
        print(
            f"[step {i:>2} {label:>18}] "
            f"prot={s.get('protection_state'):>10} (trip={s.get('protection_trip')})  "
            f"bs={s.get('blackstart_state'):>14} stage={s.get('blackstart_stage')}  "
            f"cmd={s.get('blackstart_command')}"
        )


if __name__ == "__main__":
    demo()
