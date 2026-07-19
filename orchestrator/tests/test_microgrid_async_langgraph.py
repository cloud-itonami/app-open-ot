"""Async LangGraph (astream) tests (#2).

Validates the production-shape async runtime path: same loop graph,
driven via `app.astream(...)` instead of `app.invoke(...)`. Outputs must
match the sync invoke-based version byte-for-byte (same WASM cells +
same params + same LangGraph graph → same numerical results).
"""

from __future__ import annotations

import asyncio

import pytest

from open_ot_orchestrator.microgrid_async_langgraph import (
    run_concurrent_loops,
    run_schedule,
    step_async,
)
from open_ot_orchestrator.microgrid_langgraph import (
    build_freq_droop_graph,
    step as sync_step,
)
from open_ot_orchestrator.microgrid_pregel import DROOP_WASM_PATH


requires_wasm = pytest.mark.skipif(
    not DROOP_WASM_PATH.exists(),
    reason=(
        f"droop_p_f.wasm not at {DROOP_WASM_PATH}. Build with:\n"
        "  cd ../cells && cargo build --release --no-default-features "
        "--target wasm32-unknown-unknown -p droop-p-f"
    ),
)


@requires_wasm
def test_async_step_returns_consolidated_final_state():
    bess = [("a", 1000.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    cfg = {"configurable": {"thread_id": "t-async-1"}}

    s = asyncio.run(
        step_async(app, cfg, grid_freq_hz=50.5, current_p_kw={"a": 800.0})
    )
    assert "cohort_total_delta_kw" in s
    assert s["cohort_total_delta_kw"] < 0  # over-frequency → power down
    assert "cell_outputs" in s
    assert "a" in s["cell_outputs"]


@requires_wasm
def test_async_matches_sync_byte_for_byte():
    """Same schedule / same params → async and sync must agree numerically."""
    bess = [("a", 1000.0)]
    app_async = build_freq_droop_graph(bess, cycle_period_ms=100)
    app_sync = build_freq_droop_graph(bess, cycle_period_ms=100)
    schedule = [50.000, 50.300, 50.500, 50.300, 50.050, 50.000]

    sync_outputs = []
    cfg_sync = {"configurable": {"thread_id": "t-sync"}}
    for f in schedule:
        s = sync_step(app_sync, cfg_sync, f, {"a": 800.0})
        sync_outputs.append(s["cell_outputs"]["a"])

    async_states = asyncio.run(
        run_schedule(app_async, "t-async", schedule, {"a": 800.0})
    )
    async_outputs = [s["cell_outputs"]["a"] for s in async_states]

    assert async_outputs == sync_outputs


@requires_wasm
def test_concurrent_loops_complete_without_crash():
    """`asyncio.gather` across two threads completes without crashing.

    DEMO LIMITATION (intentional, documented): the demo orchestrator binds
    ONE `CellLoader` per graph node via closure (see `_make_bess_node` in
    `microgrid_langgraph.py`). LangGraph runs sync node functions on its
    thread pool under `astream`, so two concurrent thread_ids can race
    on the shared Wasmtime instance — the cell's WASM-linear-memory
    `DataIn` slot is overwritten by whichever OS thread wrote last
    before the cell tick reads it.

    Production fix: one Wasmtime instance per (cell-instance × loop
    instance), allocated by the orchestrator at thread spawn. That's a
    separate architectural deliverable outside this demo's scope. This
    test asserts only what holds in the current shared-loader setup: the
    runtime doesn't crash under concurrent gather, and each thread
    receives its own per-step state stream.
    """
    bess = [("a", 1000.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    results = asyncio.run(
        run_concurrent_loops(
            app,
            loops=[
                ("iso-A", [50.5, 50.5, 50.5], {"a": 800.0}),
                ("iso-B", [49.5, 49.5, 49.5], {"a": 800.0}),
            ],
        )
    )
    # Both threads completed the full schedule.
    assert len(results["iso-A"]) == 3
    assert len(results["iso-B"]) == 3
    # Each thread's per-step state stream contains the expected fields.
    for tid in ("iso-A", "iso-B"):
        for step in results[tid]:
            assert "cohort_total_delta_kw" in step
            assert "cell_outputs" in step
            assert "a" in step["cell_outputs"]


@requires_wasm
def test_astream_yields_node_updates():
    """Verify we actually consume per-node update chunks, not just the final state."""
    bess = [("a", 1000.0), ("b", 500.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    cfg = {"configurable": {"thread_id": "t-stream"}}

    async def collect_chunks():
        chunks = []
        async for chunk in app.astream(
            {"grid_freq_hz": 50.5, "freq_nominal_hz": 50.0, "current_p_kw": {"a": 800.0, "b": 400.0}},
            config=cfg,
            stream_mode="updates",
        ):
            chunks.append(chunk)
        return chunks

    chunks = asyncio.run(collect_chunks())
    # We should see updates from each BESS node + the aggregator (3 chunks
    # in this graph, in some scheduler-dependent order).
    assert len(chunks) >= 2
