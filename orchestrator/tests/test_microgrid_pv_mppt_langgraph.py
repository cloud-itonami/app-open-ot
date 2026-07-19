"""Tests for `microgrid_pv_mppt_langgraph` — N × MPPT_PERTURB_OBSERVE."""

from __future__ import annotations

import pytest

from open_ot_orchestrator.microgrid_pv_mppt_langgraph import (
    build_pv_mppt_graph,
    step,
)


@pytest.fixture
def strings() -> list[tuple[str, float, float]]:
    return [
        ("did:test:pv-1", 200.0, 600.0),
        ("did:test:pv-2", 200.0, 600.0),
    ]


@pytest.fixture
def voltages(strings):
    return {did: 400.0 for did, _, _ in strings}


def test_first_tick_searches_upward(strings, voltages):
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-first"}}
    currents = {did: 25.0 for did, _, _ in strings}
    state = step(app, config, voltages, currents)
    for did, _, _ in strings:
        assert state["string_direction"][did] == "Up"
        # First tick: setpoint = measured + step (0.1 V).
        assert state["string_voltage_setpoint_v"][did] == pytest.approx(400.1, abs=1e-3)


def test_increasing_power_keeps_direction(strings, voltages):
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-inc"}}
    # Step 1: bootstrap.
    step(app, config, voltages, {did: 20.0 for did, _, _ in strings})
    # Step 2: more current → more power → keep going Up.
    state = step(app, config, voltages, {did: 25.0 for did, _, _ in strings})
    for did, _, _ in strings:
        assert state["string_direction"][did] == "Up"


def test_decreasing_power_flips_direction(strings, voltages):
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-dec"}}
    step(app, config, voltages, {did: 25.0 for did, _, _ in strings})
    # Lower current → lower power → flip to Down.
    state = step(app, config, voltages, {did: 15.0 for did, _, _ in strings})
    for did, _, _ in strings:
        assert state["string_direction"][did] == "Down"


def test_constant_power_reaches_mpp(strings, voltages):
    """Holding current constant across two ticks should converge to AtMpp."""
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-mpp"}}
    currents = {did: 25.0 for did, _, _ in strings}
    step(app, config, voltages, currents)
    # Same exact current → power_pw matches last_power_pw → AtMpp.
    state = step(app, config, voltages, currents)
    for did, _, _ in strings:
        assert state["string_state"][did] == "AtMpp"
    assert state["arrays_at_mpp"] == len(strings)


def test_aggregator_sums_power(strings, voltages):
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-agg"}}
    currents = {did: 25.0 for did, _, _ in strings}
    state = step(app, config, voltages, currents)
    total = state["total_pv_power_w"]
    expected = 2 * 400.0 * 25.0  # 2 strings × 10 kW each
    assert abs(total - expected) < 1.0


def test_history_has_checkpoints(strings, voltages):
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-history"}}
    for _ in range(3):
        step(app, config, voltages, {did: 25.0 for did, _, _ in strings})
    history = list(app.get_state_history(config))
    assert len(history) >= 3


def test_v_clamps_at_v_max():
    """When measured V == V_max, setpoint should clamp at V_max."""
    strings = [("did:test:pv-clamp", 200.0, 400.0)]
    app = build_pv_mppt_graph(strings, cycle_period_ms=10)
    config = {"configurable": {"thread_id": "test-pv-clamp"}}
    # Bootstrap at V_max with direction Up → setpoint should NOT exceed V_max.
    state = step(
        app,
        config,
        {strings[0][0]: 400.0},
        {strings[0][0]: 25.0},
    )
    # First tick sets setpoint = V + step, then clamps to V_max.
    assert state["string_voltage_setpoint_v"][strings[0][0]] <= 400.0
