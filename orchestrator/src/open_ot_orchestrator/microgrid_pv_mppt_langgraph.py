"""Microgrid `:loop:pv-mppt` — N × MPPT_PERTURB_OBSERVE strings + aggregator.

Per PROTOTYPE-MICROGRID.md §13.2. Each PV string runs its own MPPT cell;
the aggregator computes total PV power for downstream consumers (BESS
charge controller, economic dispatch).

```text
   START ─┬─→ mppt_string1 ─┐
          ├─→ mppt_string2 ─┤
          │       ...        ├─→ pv_aggregator ─→ END
          └─→ mppt_stringN ─┘
```

Run: `uv run python -m open_ot_orchestrator.microgrid_pv_mppt_langgraph`
"""

from __future__ import annotations

from operator import or_
from pathlib import Path
from typing import Annotated, Any, TypedDict

from langgraph.checkpoint.memory import MemorySaver
from langgraph.graph import END, START, StateGraph

from ._generated import mppt_perturb_observe as mppt_gen
from .cell_loader import CellLoader

REPO_ROOT = Path(__file__).resolve().parents[5]
CELLS_TARGET = (
    REPO_ROOT
    / "60-apps/etzhayyim-project-open-ot/cells/target/wasm32-unknown-unknown/release"
)
MPPT_WASM_PATH = CELLS_TARGET / "mppt_perturb_observe.wasm"


class PvMpptState(TypedDict, total=False):
    """Per-super-step PV array state."""

    string_voltage_v: dict[str, float]
    string_current_a: dict[str, float]
    string_voltage_setpoint_v: Annotated[dict[str, float], or_]
    string_power_w: Annotated[dict[str, float], or_]
    string_direction: Annotated[dict[str, str], or_]
    string_state: Annotated[dict[str, str], or_]
    cell_internals: Annotated[dict[str, bytes], or_]
    cell_ecc_states: Annotated[dict[str, int], or_]
    total_pv_power_w: float
    arrays_at_mpp: int


_MPPT_ECC = {i: n for i, n in enumerate(mppt_gen.ECC_STATES)}
_DIR_NAMES = {0: "Up", 1: "Down"}


def _mppt_node(string_did: str, v_min_v: float, v_max_v: float, cycle_period_ms: int):
    """One MPPT cell per PV string."""
    loader = CellLoader(MPPT_WASM_PATH, mppt_gen.CELL_SYMBOL)
    params = mppt_gen.Params(
        perturb_step_micro_v=100_000,  # 0.1 V step
        v_min_micro_v=int(round(v_min_v * 1_000_000)),
        v_max_micro_v=int(round(v_max_v * 1_000_000)),
        mpp_tolerance_pw=5_000_000_000_000,  # 5 W tolerance
        cycle_period_ms=cycle_period_ms,
    )
    params_bytes = mppt_gen.pack_params(params)
    initialized = {"done": False}

    def node(state: PvMpptState) -> dict[str, Any]:
        prior = state.get("cell_internals") or {}
        if string_did in prior:
            if not initialized["done"]:
                loader.init(params_bytes, mppt_gen.INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior[string_did])
        else:
            loader.init(params_bytes, mppt_gen.INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(string_did, 0)
        v = state.get("string_voltage_v", {}).get(string_did, 0.0)
        i = state.get("string_current_a", {}).get(string_did, 0.0)

        data_in = mppt_gen.DataIn(
            pv_voltage_micro_v=int(round(v * 1_000_000)),
            pv_current_micro_a=int(round(i * 1_000_000)),
            voltage_quality=0,
            current_quality=0,
            enable=True,
        )
        result = loader.tick(
            event_in=0,
            data_in_bytes=mppt_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=mppt_gen.DATA_OUT_SIZE,
        )
        out = mppt_gen.unpack_data_out(result.data_out_bytes)
        # power_pw → W: divide by 1e12.
        power_w = out.power_pw / 1_000_000_000_000.0
        return {
            "string_voltage_setpoint_v": {
                string_did: out.voltage_setpoint_micro_v / 1_000_000.0
            },
            "string_power_w": {string_did: power_w},
            "string_direction": {string_did: _DIR_NAMES.get(out.direction, "?")},
            "string_state": {
                string_did: _MPPT_ECC.get(result.next_ecc_state, "?")
            },
            "cell_internals": {
                string_did: loader.get_internal_bytes(mppt_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {string_did: result.next_ecc_state},
        }

    return node


def _pv_aggregator(state: PvMpptState) -> dict[str, Any]:
    powers = state.get("string_power_w") or {}
    states = state.get("string_state") or {}
    total = sum(powers.values())
    at_mpp = sum(1 for s in states.values() if s == "AtMpp")
    return {
        "total_pv_power_w": total,
        "arrays_at_mpp": at_mpp,
    }


def _safe_node_id(did: str) -> str:
    return did.replace(":", "_").replace("/", "_").replace(".", "_")


def build_pv_mppt_graph(
    pv_strings: list[tuple[str, float, float]],  # (did, v_min_v, v_max_v)
    cycle_period_ms: int = 10,
):
    graph = StateGraph(PvMpptState)
    for did, vmin, vmax in pv_strings:
        graph.add_node(_safe_node_id(did), _mppt_node(did, vmin, vmax, cycle_period_ms))
    graph.add_node("pv_aggregator", _pv_aggregator)
    for did, _, _ in pv_strings:
        graph.add_edge(START, _safe_node_id(did))
        graph.add_edge(_safe_node_id(did), "pv_aggregator")
    graph.add_edge("pv_aggregator", END)
    return graph.compile(checkpointer=MemorySaver())


def step(
    app,
    config: dict[str, Any],
    string_voltage_v: dict[str, float],
    string_current_a: dict[str, float],
) -> PvMpptState:
    return app.invoke(
        {
            "string_voltage_v": string_voltage_v,
            "string_current_a": string_current_a,
        },
        config=config,
    )


def demo() -> None:
    strings = [
        ("did:web:open-ot.etzhayyim.com:cell:pv-string-1", 200.0, 600.0),
        ("did:web:open-ot.etzhayyim.com:cell:pv-string-2", 200.0, 600.0),
        ("did:web:open-ot.etzhayyim.com:cell:pv-string-3", 200.0, 600.0),
    ]
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "pv-demo"}}

    # Simulate irradiance ramp + clamping near MPP. Currents = power / voltage.
    schedule = [
        # (voltage, current_per_string)
        (400.0, 20.0),
        (400.0, 22.0),
        (400.0, 25.0),
        (400.0, 25.0),
        (400.0, 24.5),
        (400.0, 24.5),
        (400.0, 24.5),
    ]
    for i, (v, cur) in enumerate(schedule, 1):
        voltages = {did: v for did, _, _ in strings}
        currents = {did: cur for did, _, _ in strings}
        state = step(app, config, voltages, currents)
        total = state.get("total_pv_power_w", 0.0)
        n_mpp = state.get("arrays_at_mpp", 0)
        print(
            f"[step {i:>2}] V={v:5.1f}V I={cur:5.1f}A/string  "
            f"Σ P={total/1000:6.2f} kW  arrays_at_mpp={n_mpp}/{len(strings)}"
        )


if __name__ == "__main__":
    demo()
