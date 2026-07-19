"""Tests for `microgrid_volt_var_langgraph` — multi-cell-type loop."""

from __future__ import annotations

import pytest

from open_ot_orchestrator.microgrid_volt_var_langgraph import (
    build_volt_var_graph,
    step,
)


@pytest.fixture
def inverter_dids() -> list[str]:
    return [
        "did:web:open-ot.etzhayyim.com:cell:inv-1",
        "did:web:open-ot.etzhayyim.com:cell:inv-2",
    ]


@pytest.fixture
def q_max(inverter_dids):
    return {did: 100_000.0 for did in inverter_dids}


def test_nominal_voltage_is_in_dead_band(inverter_dids, q_max):
    app = build_volt_var_graph(inverter_dids, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-nominal"}}
    voltages = {did: 1.000 for did in inverter_dids}  # exact nominal
    state = step(app, config, voltages, q_max)
    for did in inverter_dids:
        assert state["inverter_q_setpoint_var"][did] == 0.0
        assert state["inverter_states"][did] == "InDeadBand"


def test_over_voltage_absorbs_reactive(inverter_dids, q_max):
    app = build_volt_var_graph(inverter_dids, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-over"}}
    voltages = {did: 1.045 for did in inverter_dids}  # mid absorb ramp
    state = step(app, config, voltages, q_max)
    for did in inverter_dids:
        assert state["inverter_q_setpoint_var"][did] < 0.0
        assert state["inverter_states"][did] == "Absorbing"


def test_under_voltage_injects_reactive(inverter_dids, q_max):
    app = build_volt_var_graph(inverter_dids, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-under"}}
    voltages = {did: 0.935 for did in inverter_dids}  # mid inject ramp
    state = step(app, config, voltages, q_max)
    for did in inverter_dids:
        assert state["inverter_q_setpoint_var"][did] > 0.0
        assert state["inverter_states"][did] == "Injecting"


def test_ltc_lowers_on_high_bus_voltage(inverter_dids, q_max):
    # Long dwell so the LTC issues commands rather than waiting.
    app = build_volt_var_graph(
        inverter_dids,
        cycle_period_ms=1_000,
        dwell_ms=0,
    )
    config = {"configurable": {"thread_id": "test-ltc-high"}}
    voltages = {did: 1.10 for did in inverter_dids}  # 12.1 kV (far above target)
    state = step(app, config, voltages, q_max, bus_voltage_target_v=11_000.0)
    assert state["ltc_command"] == "Lower"
    assert state["ltc_state"] == "Lowering"


def test_ltc_raises_on_low_bus_voltage(inverter_dids, q_max):
    app = build_volt_var_graph(
        inverter_dids,
        cycle_period_ms=1_000,
        dwell_ms=0,
    )
    config = {"configurable": {"thread_id": "test-ltc-low"}}
    voltages = {did: 0.90 for did in inverter_dids}  # 9.9 kV
    state = step(app, config, voltages, q_max, bus_voltage_target_v=11_000.0)
    assert state["ltc_command"] == "Raise"
    assert state["ltc_state"] == "Raising"


def test_resumes_via_memory_saver(inverter_dids, q_max):
    """Same loop instance across two invocations must retain LTC dwell."""
    app = build_volt_var_graph(
        inverter_dids,
        cycle_period_ms=1_000,
        dwell_ms=5_000,
    )
    config = {"configurable": {"thread_id": "test-resume"}}
    voltages = {did: 1.10 for did in inverter_dids}
    state1 = step(app, config, voltages, q_max, bus_voltage_target_v=11_000.0)
    assert state1["ltc_command"] == "Lower"
    state2 = step(app, config, voltages, q_max, bus_voltage_target_v=11_000.0)
    assert state2["ltc_command"] == "Hold"
    assert state2["ltc_state"] == "Holding"


def test_history_has_checkpoints(inverter_dids, q_max):
    app = build_volt_var_graph(inverter_dids, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-history"}}
    voltages = {did: 1.000 for did in inverter_dids}
    step(app, config, voltages, q_max)
    step(app, config, voltages, q_max)
    history = list(app.get_state_history(config))
    assert len(history) >= 2
