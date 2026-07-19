"""Anti-islanding loop on LangGraph (#2).

Wraps the `ANTI_ISLANDING_ROCOF` cell as a LangGraph node — the third
reference cell, the most elaborate so far. Validates orchestrator handling
of:

  - Multi-event-input  : `REQ` (normal) and `RESET` (operator clears trip)
  - Multi-event-output : the cell can emit `CNF` + `TRIP` on the same tick;
                         our orchestrator surfaces both via the cell's
                         `u16` packed `out_event` slot.
  - Latched ECC state  : `Tripped` survives across LangGraph invocations
                         (carried in `cell_internals` + `cell_ecc_states`).
  - RESET semantics    : a single LangGraph invocation with `event_in =
                         RESET (1)` clears counters, transitions to
                         `Monitoring`.

Mapping to ADR-2605151200 §4.1 unchanged from `microgrid_langgraph.py` —
the cell shape doesn't change the binding, just stresses it.

Run: `uv run python -m open_ot_orchestrator.microgrid_islanding_langgraph`
"""

from __future__ import annotations

import struct
from dataclasses import dataclass
from pathlib import Path
from typing import Annotated, Any, TypedDict

from langgraph.checkpoint.memory import MemorySaver
from langgraph.graph import END, START, StateGraph
from operator import or_

from ._generated import anti_islanding_rocof as gen_islanding
from .cell_loader import CellLoader, OUT_EVENT_WIDTH_PACKED_U16

# ---------------------------------------------------------------------------
# Struct layouts come from the generated wrapper. Re-exported here for
# backwards compat with code/tests that import the `ISL_*` names from this
# module.
# ---------------------------------------------------------------------------

ISL_PARAMS_FMT = gen_islanding.PARAMS_FMT
ISL_PARAMS_SIZE = gen_islanding.PARAMS_SIZE
ISL_INTERNAL_FMT = gen_islanding.INTERNAL_FMT
ISL_INTERNAL_SIZE = gen_islanding.INTERNAL_SIZE
ISL_DATA_IN_FMT = gen_islanding.DATA_IN_FMT
ISL_DATA_IN_SIZE = gen_islanding.DATA_IN_SIZE
ISL_DATA_OUT_FMT = gen_islanding.DATA_OUT_FMT
ISL_DATA_OUT_SIZE = gen_islanding.DATA_OUT_SIZE

assert ISL_PARAMS_SIZE == 64, ISL_PARAMS_SIZE
assert ISL_INTERNAL_SIZE == 24, ISL_INTERNAL_SIZE
assert ISL_DATA_IN_SIZE == 40, ISL_DATA_IN_SIZE
assert ISL_DATA_OUT_SIZE == 40, ISL_DATA_OUT_SIZE


# Event codes mirror the Rust EventIn / EventOut enum reprs.
EVENT_IN_REQ = 0
EVENT_IN_RESET = 1
EVENT_OUT_NONE = 0
EVENT_OUT_CNF = 1
EVENT_OUT_TRIP = 2
EVENT_OUT_ALM = 3

ECC_IDLE = 0
ECC_MONITORING = 1
ECC_WARNING = 2
ECC_TRIPPED = 3
ECC_ALARM = 4

TRIP_REASON_NAMES = {
    0: "None",
    1: "Rocof",
    2: "Overvoltage",
    3: "Undervoltage",
    4: "Overfrequency",
    5: "Underfrequency",
}


def pack_islanding_params(
    rocof_threshold_hz_per_s: float,
    rocof_window: int,
    voltage_min_v: float,
    voltage_max_v: float,
    voltage_window: int,
    freq_min_hz: float,
    freq_max_hz: float,
    freq_window: int,
    cycle_period_ms: int,
) -> bytes:
    return gen_islanding.pack_params(
        gen_islanding.Params(
            rocof_threshold_micro_hz_per_s=int(rocof_threshold_hz_per_s * 1_000_000),
            rocof_window_samples=rocof_window,
            voltage_min_micro_v=int(voltage_min_v * 1_000_000),
            voltage_max_micro_v=int(voltage_max_v * 1_000_000),
            voltage_window_samples=voltage_window,
            freq_min_micro_hz=int(freq_min_hz * 1_000_000),
            freq_max_micro_hz=int(freq_max_hz * 1_000_000),
            freq_window_samples=freq_window,
            cycle_period_ms=cycle_period_ms,
        )
    )


def pack_islanding_data_in(
    grid_freq_hz: float,
    grid_voltage_v: float,
    enable: bool = True,
    freq_nominal_hz: float = 50.0,
    voltage_nominal_v: float = 230.0,
    freq_quality: int = 0,
    voltage_quality: int = 0,
) -> bytes:
    return gen_islanding.pack_data_in(
        gen_islanding.DataIn(
            grid_freq=int(grid_freq_hz * 1_000_000),
            freq_nominal=int(freq_nominal_hz * 1_000_000),
            grid_voltage=int(grid_voltage_v * 1_000_000),
            voltage_nominal=int(voltage_nominal_v * 1_000_000),
            freq_quality=freq_quality,
            voltage_quality=voltage_quality,
            enable=enable,
        )
    )


@dataclass
class IslandingOutput:
    trip: bool
    trip_reason: str
    rocof_hz_per_s: float
    voltage_dev_pct: float
    freq_dev_hz: float
    rocof_count: int
    voltage_count: int
    freq_count: int


def unpack_islanding_data_out(buf: bytes) -> IslandingOutput:
    trip, reason_code, rocof, vdev_milli_pct, fdev, rcnt, vcnt, fcnt = struct.unpack(
        ISL_DATA_OUT_FMT, buf
    )
    return IslandingOutput(
        trip=bool(trip),
        trip_reason=TRIP_REASON_NAMES.get(reason_code, f"Unknown({reason_code})"),
        rocof_hz_per_s=rocof / 1_000_000,
        voltage_dev_pct=vdev_milli_pct / 1000.0,
        freq_dev_hz=fdev / 1_000_000,
        rocof_count=rcnt,
        voltage_count=vcnt,
        freq_count=fcnt,
    )


def emitted_events_from_packed(packed: int) -> list[int]:
    """Anti-islanding packs (low | high) into a u16. Decode to event codes."""
    low = packed & 0xFF
    high = (packed >> 8) & 0xFF
    out: list[int] = []
    if low != EVENT_OUT_NONE:
        out.append(low)
    if high != EVENT_OUT_NONE:
        out.append(high)
    return out


# ---------------------------------------------------------------------------
# LangGraph wiring
# ---------------------------------------------------------------------------


ISL_WASM_PATH = Path(__file__).resolve().parent.parent.parent.parent / (
    "cells/target/wasm32-unknown-unknown/release/anti_islanding_rocof.wasm"
)


class IslandingState(TypedDict, total=False):
    grid_freq_hz: float
    grid_voltage_v: float
    enable: bool
    event_in: int  # EVENT_IN_REQ or EVENT_IN_RESET
    cell_internal: bytes
    cell_ecc_state: int
    last_output: dict[str, Any]
    last_emitted: list[int]
    # accumulated history per thread (the LangGraph version of the
    # checkpoint stream).
    history: Annotated[list[dict[str, Any]], lambda a, b: list(a) + list(b)]


_LOADER_REGISTRY: dict[str, CellLoader] = {}
_PARAMS_REGISTRY: dict[str, bytes] = {}


def _get_or_init_loader(node_id: str, params_bytes: bytes) -> CellLoader:
    """One persistent CellLoader per LangGraph node (one Wasmtime instance).

    Reusing the same loader across invocations preserves Tripped latch
    state in the WASM linear memory between super-steps. The state-graph
    ALSO carries `cell_internal` bytes so checkpoint + resume work — the
    in-memory Wasmtime state is just a perf cache.
    """
    if node_id not in _LOADER_REGISTRY:
        loader = CellLoader(
            ISL_WASM_PATH,
            "anti_islanding_rocof",
            out_event_width=OUT_EVENT_WIDTH_PACKED_U16,
        )
        loader.init(params_bytes, ISL_INTERNAL_SIZE)
        _LOADER_REGISTRY[node_id] = loader
        _PARAMS_REGISTRY[node_id] = params_bytes
    return _LOADER_REGISTRY[node_id]


def _make_islanding_node(node_id: str, params_bytes: bytes):
    def node(state: IslandingState) -> dict[str, Any]:
        loader = _get_or_init_loader(node_id, params_bytes)
        if state.get("cell_internal"):
            loader.set_internal_bytes(state["cell_internal"])
        ecc_in = state.get("cell_ecc_state", ECC_IDLE)
        data_in = pack_islanding_data_in(
            grid_freq_hz=state["grid_freq_hz"],
            grid_voltage_v=state["grid_voltage_v"],
            enable=state.get("enable", True),
        )
        result = loader.tick(
            event_in=state.get("event_in", EVENT_IN_REQ),
            data_in_bytes=data_in,
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=ISL_DATA_OUT_SIZE,
        )
        emitted = emitted_events_from_packed(result.out_event_raw)
        parsed = unpack_islanding_data_out(result.data_out_bytes)
        return {
            "cell_internal": loader.get_internal_bytes(ISL_INTERNAL_SIZE),
            "cell_ecc_state": result.next_ecc_state,
            "last_output": parsed.__dict__,
            "last_emitted": emitted,
            "history": [
                {
                    "ecc": result.next_ecc_state,
                    "emitted": emitted,
                    "trip": parsed.trip,
                    "trip_reason": parsed.trip_reason,
                }
            ],
        }

    return node


def build_islanding_graph(node_id: str = "ai-rocof", **params_kwargs):
    params_bytes = pack_islanding_params(**params_kwargs)
    graph = StateGraph(IslandingState)
    graph.add_node(node_id, _make_islanding_node(node_id, params_bytes))
    graph.add_edge(START, node_id)
    graph.add_edge(node_id, END)
    return graph.compile(checkpointer=MemorySaver())


def reset_loader_registry() -> None:
    """Test helper — clear the per-node Wasmtime instance cache."""
    _LOADER_REGISTRY.clear()
    _PARAMS_REGISTRY.clear()


def step(
    app,
    config: dict[str, Any],
    grid_freq_hz: float,
    grid_voltage_v: float,
    event_in: int = EVENT_IN_REQ,
    enable: bool = True,
):
    return app.invoke(
        {
            "grid_freq_hz": grid_freq_hz,
            "grid_voltage_v": grid_voltage_v,
            "event_in": event_in,
            "enable": enable,
        },
        config=config,
    )


# ---------------------------------------------------------------------------
# Demo
# ---------------------------------------------------------------------------


def default_demo_params() -> dict[str, Any]:
    return dict(
        rocof_threshold_hz_per_s=0.5,
        rocof_window=3,
        voltage_min_v=207.0,
        voltage_max_v=253.0,
        voltage_window=5,
        freq_min_hz=49.5,
        freq_max_hz=50.5,
        freq_window=5,
        cycle_period_ms=100,
    )


def demo() -> None:
    print("[microgrid-islanding] building :loop:islanding-decision (1 cell)")
    reset_loader_registry()
    app = build_islanding_graph(**default_demo_params())
    cfg = {"configurable": {"thread_id": "islanding-demo"}}

    # Schedule: 2 normal samples (init + nominal), then 3 ROCOF spikes
    # (each tick freq jumps +0.1 Hz over 100 ms = +1 Hz/s ROCOF, well over
    # the 0.5 Hz/s threshold). After 3 consecutive: TRIP. Then attempt
    # a normal REQ on the latched cell. Then RESET. Then resume normal.
    schedule = [
        # (freq_hz, voltage_v, event_in_label)
        (50.000, 230.0, "REQ"),  # init
        (50.000, 230.0, "REQ"),  # nominal
        (50.100, 230.0, "REQ"),  # ROCOF +1 Hz/s, count=1
        (50.200, 230.0, "REQ"),  # ROCOF +1 Hz/s, count=2
        (50.300, 230.0, "REQ"),  # ROCOF +1 Hz/s, count=3 → TRIP
        (50.000, 230.0, "REQ"),  # latched — still tripped
        (50.000, 230.0, "RESET"),  # operator clears
        (50.000, 230.0, "REQ"),  # back to monitoring
    ]
    for i, (f, v, ev) in enumerate(schedule, start=1):
        ev_code = EVENT_IN_RESET if ev == "RESET" else EVENT_IN_REQ
        s = step(app, cfg, grid_freq_hz=f, grid_voltage_v=v, event_in=ev_code)
        out = s["last_output"]
        emitted = s["last_emitted"]
        emitted_names = [
            {1: "CNF", 2: "TRIP", 3: "ALM"}.get(e, "?") for e in emitted
        ]
        ecc_name = {0: "Idle", 1: "Mon", 2: "Warn", 3: "TRIP", 4: "Alm"}[
            s["cell_ecc_state"]
        ]
        print(
            f"[step {i}] f={f:.3f} v={v:.0f} ev={ev:<5}  "
            f"ecc={ecc_name:<5} emit={emitted_names}  "
            f"trip={out['trip']!s:<5} reason={out['trip_reason']:<14} "
            f"counts(R/V/F)={out['rocof_count']}/{out['voltage_count']}/{out['freq_count']}"
        )


if __name__ == "__main__":
    demo()
