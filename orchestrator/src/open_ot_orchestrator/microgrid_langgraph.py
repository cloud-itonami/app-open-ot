"""Microgrid `:loop:freq-droop` as a real LangGraph `StateGraph` (#3b).

Same loop as `microgrid_pregel`, but built on the actual LangGraph SDK
(`langgraph>=0.2`, matches `40-engine/kotoba/crates/kotoba-kotodama/py` convention). Cells are
loaded via the shared `cell_loader.CellLoader`; LangGraph supplies the
graph runtime, parallel fan-out / fan-in, and the checkpointer.

Mapping to ADR-2605151200 §LangGraph + Pregel binding:

  - One LangGraph node      = one Pregel node           = one cell
  - One graph invocation    = one Pregel super-step     = one IEC 61499 tick
  - LangGraph thread        = one loop instance
  - LangGraph checkpointer  = `vertex_open_ot_loop_checkpoint` SSoT
                              (here: in-memory `MemorySaver`)

Run: `uv run python -m open_ot_orchestrator.microgrid_langgraph`
"""

from __future__ import annotations

from operator import or_
from typing import Annotated, Any, TypedDict

from langgraph.checkpoint.memory import MemorySaver
from langgraph.graph import END, START, StateGraph

from .cell_loader import CellLoader
from .microgrid_pregel import (
    DROOP_DATA_IN_SIZE,
    DROOP_DATA_OUT_SIZE,
    DROOP_INTERNAL_SIZE,
    DROOP_PARAMS_SIZE,
    DROOP_WASM_PATH,
    pack_droop_data_in,
    pack_droop_params,
    unpack_droop_data_out,
)


class MicrogridState(TypedDict, total=False):
    """Shared graph state — every node reads and writes a subset.

    `Annotated[..., or_]` makes parallel writes from BESS nodes merge
    instead of clobber (Python `dict | dict`).
    """

    grid_freq_hz: float
    freq_nominal_hz: float
    current_p_kw: dict[str, float]
    # cell outputs as plain dicts so MemorySaver can serialize them.
    cell_outputs: Annotated[dict[str, dict[str, Any]], or_]
    # Per-cell internal bytes — the SSoT for resume. base64-able, MemorySaver-safe.
    cell_internals: Annotated[dict[str, bytes], or_]
    # Per-cell ECC state (int).
    cell_ecc_states: Annotated[dict[str, int], or_]
    cohort_total_delta_kw: float


def _make_bess_node(asset_did: str, p_rated_kw: float, cycle_period_ms: int):
    """One BFB cell wrapped as a LangGraph node.

    The node owns its own `CellLoader` (one Wasmtime instance per asset).
    Each invocation:

      1. Restore the cell's `internal` bytes from `state['cell_internals']`
         if present; otherwise call `init()` with the cell's params.
      2. Pack DataIn from current grid frequency / current power.
      3. Tick.
      4. Return state delta with the new internal bytes + ECC + parsed output.

    This is the BSP super-step contract (per ADR §4.1) — checkpoint
    survives across graph invocations via the `MemorySaver` thread.
    """
    loader = CellLoader(DROOP_WASM_PATH, "droop_p_f")
    params_bytes = pack_droop_params(
        p_rated_kw=p_rated_kw,
        p_min_kw=0,
        p_max_kw=p_rated_kw,
        droop_pct=5.0,
        dead_band_hz=0.2,
        cycle_period_ms=cycle_period_ms,
    )
    initialized = {"done": False}

    def node(state: MicrogridState) -> dict[str, Any]:
        # Resume from prior checkpoint if it exists; else init once.
        prior_internals = state.get("cell_internals") or {}
        if asset_did in prior_internals:
            if not initialized["done"]:
                # Cell needs init even before set_internal — this allocates
                # the params slot. Subsequent restores skip init.
                loader.init(params_bytes, DROOP_INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior_internals[asset_did])
        else:
            loader.init(params_bytes, DROOP_INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(asset_did, 0)
        cur_p = (state.get("current_p_kw") or {}).get(asset_did, 0.0)
        data_in = pack_droop_data_in(
            grid_freq_hz=state["grid_freq_hz"],
            freq_nominal_hz=state["freq_nominal_hz"],
            current_p_kw=cur_p,
        )
        # super_step is implicit in MemorySaver's thread state — pass 0
        # for now (replay determinism is checked via internal bytes only;
        # super_step in pid-limited / droop are ignored by the math).
        result = loader.tick(
            event_in=0,  # REQ
            data_in_bytes=data_in,
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=DROOP_DATA_OUT_SIZE,
        )
        parsed = unpack_droop_data_out(result.data_out_bytes)
        return {
            "cell_outputs": {asset_did: parsed.__dict__},
            "cell_internals": {asset_did: loader.get_internal_bytes(DROOP_INTERNAL_SIZE)},
            "cell_ecc_states": {asset_did: result.next_ecc_state},
        }

    return node


def _aggregator_node(state: MicrogridState) -> dict[str, Any]:
    outputs = state.get("cell_outputs") or {}
    total = sum(o["delta_p_kw"] for o in outputs.values())
    return {"cohort_total_delta_kw": total}


def _safe_node_id(did: str) -> str:
    """LangGraph 0.6+ rejects `:` `/` `.` in node names. The DID stays the
    canonical state key; only the graph node label is sanitized."""
    return did.replace(":", "_").replace("/", "_").replace(".", "_")


def build_freq_droop_graph(
    bess_assets: list[tuple[str, float]],
    cycle_period_ms: int = 100,
):
    """Build and compile the LangGraph `StateGraph`."""
    graph = StateGraph(MicrogridState)
    for did, p_rated in bess_assets:
        node_id = _safe_node_id(did)
        graph.add_node(node_id, _make_bess_node(did, p_rated, cycle_period_ms))
    graph.add_node("aggregator", _aggregator_node)
    for did, _ in bess_assets:
        node_id = _safe_node_id(did)
        graph.add_edge(START, node_id)
        graph.add_edge(node_id, "aggregator")
    graph.add_edge("aggregator", END)
    return graph.compile(checkpointer=MemorySaver())


def step(
    app,
    config: dict[str, Any],
    grid_freq_hz: float,
    current_p_kw: dict[str, float],
    freq_nominal_hz: float = 50.0,
) -> MicrogridState:
    """Run one super-step (one full graph invocation)."""
    return app.invoke(
        {
            "grid_freq_hz": grid_freq_hz,
            "freq_nominal_hz": freq_nominal_hz,
            "current_p_kw": current_p_kw,
        },
        config=config,
    )


# ---------------------------------------------------------------------------
# Demo
# ---------------------------------------------------------------------------


def demo() -> None:
    print("[microgrid-langgraph] building :loop:freq-droop with 2 BESS assets")
    bess = [
        ("did:web:open-ot.etzhayyim.com:cell:droop-bess-1", 1000.0),
        ("did:web:open-ot.etzhayyim.com:cell:droop-bess-2", 500.0),
    ]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "microgrid-demo-1"}}

    schedule = [50.000, 50.050, 50.300, 50.500, 50.300, 50.050, 50.000]
    current_p = {bess[0][0]: 800.0, bess[1][0]: 400.0}
    for i, f in enumerate(schedule, start=1):
        state = step(app, config, f, current_p)
        print(
            f"[step {i:>2}] f={f:.3f} Hz  "
            f"cohort ΔP = {state['cohort_total_delta_kw']:+8.2f} kW"
        )
    history = list(app.get_state_history(config))
    print(f"\n[microgrid-langgraph] {len(history)} checkpoint snapshots in MemorySaver")


if __name__ == "__main__":
    demo()
