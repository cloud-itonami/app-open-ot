"""LangGraph integration tests (#3b).

Same shape as #3a: super-step semantics, cohort sum, checkpoint resume,
determinism. Demonstrates that the binding works under the real LangGraph
SDK (not just our minimal Pregel runner).
"""

from __future__ import annotations

import pytest

from open_ot_orchestrator.microgrid_langgraph import (
    build_freq_droop_graph,
    step,
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
def test_single_super_step_within_deadband():
    bess = [("a", 1000.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "t-deadband"}}
    s = step(app, config, grid_freq_hz=50.0, current_p_kw={"a": 500.0})
    out = s["cell_outputs"]["a"]
    assert out["dead_band_active"] is True
    assert out["delta_p_kw"] == 0.0
    assert s["cohort_total_delta_kw"] == 0.0


@requires_wasm
def test_two_cells_cohort_sum():
    bess = [("a", 1000.0), ("b", 500.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "t-cohort"}}
    s = step(app, config, grid_freq_hz=50.5, current_p_kw={"a": 800.0, "b": 400.0})
    # 5 % droop, 1 % freq dev → 20 % rated power. A:1MW→200kW, B:500→100. Total -300.
    assert abs(s["cohort_total_delta_kw"] - (-300.0)) < 1.0


@requires_wasm
def test_checkpoint_history_grows_per_invocation():
    """LangGraph writes a checkpoint per invocation; history accumulates per thread."""
    bess = [("a", 1000.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "t-history"}}
    for f in [50.0, 50.3, 50.5, 50.3, 50.0]:
        step(app, config, f, {"a": 800.0})
    history = list(app.get_state_history(config))
    # MemorySaver writes >= 1 snapshot per invoke; exact count depends on
    # graph topology (one per node + final). Just assert "many".
    assert len(history) >= 5


@requires_wasm
def test_replay_determinism_via_thread_isolation():
    """Two independent threads run the same schedule → identical outputs.

    This is the LangGraph-equivalent of the #3a replay-determinism test:
    we don't rewind one thread; instead we prove that two fresh threads
    fed the same inputs produce byte-identical outputs.
    """
    bess = [("a", 1000.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    schedule = [
        (50.000, 800.0),
        (50.300, 800.0),
        (50.500, 800.0),
        (50.300, 750.0),
        (50.050, 720.0),
    ]
    out_a = []
    out_b = []
    for thread_id, sink in [("t-replay-a", out_a), ("t-replay-b", out_b)]:
        config = {"configurable": {"thread_id": thread_id}}
        for f, p in schedule:
            s = step(app, config, f, {"a": p})
            sink.append(s["cell_outputs"]["a"])
    assert out_a == out_b


@requires_wasm
def test_thread_isolation_keeps_internals_separate():
    """Two threads running the same graph must NOT share cell internal state."""
    bess = [("a", 1000.0)]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    cfg1 = {"configurable": {"thread_id": "iso-1"}}
    cfg2 = {"configurable": {"thread_id": "iso-2"}}
    # Thread 1 sees over-frequency, Thread 2 sees under-frequency.
    s1 = step(app, cfg1, 50.5, {"a": 800.0})
    s2 = step(app, cfg2, 49.5, {"a": 800.0})
    assert s1["cell_outputs"]["a"]["delta_p_kw"] < 0  # responding down
    assert s2["cell_outputs"]["a"]["delta_p_kw"] > 0  # responding up
    # Internals must differ.
    assert s1["cell_internals"]["a"] != s2["cell_internals"]["a"]
