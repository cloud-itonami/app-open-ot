"""Tests for `microgrid_bess_langgraph` — SOC_KALMAN → DROOP_P_F chain."""

from __future__ import annotations

import pytest

from open_ot_orchestrator.microgrid_bess_langgraph import (
    build_bess_graph,
    step,
)


BESS_DID = "did:web:open-ot.etzhayyim.com:cell:bess-test"


@pytest.fixture
def assets():
    return [(BESS_DID, 100.0, 100.0)]  # 100 kW, 100 Ah


def test_nominal_frequency_holds_setpoint(assets):
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-nominal"}}
    state = step(
        app,
        config,
        grid_freq_hz=50.0,
        asset_voltage_v={BESS_DID: 48.8},
        asset_current_a={BESS_DID: 10.0},  # ~488 W discharge
    )
    sp = state.get("asset_p_setpoint_kw", {}).get(BESS_DID, 0.0)
    # Nominal frequency → DROOP_P_F enters deadband → setpoint == current_p.
    assert abs(sp - 0.488) < 0.1


def test_under_frequency_discharges(assets):
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-under"}}
    state = step(
        app,
        config,
        grid_freq_hz=49.5,  # 0.5 Hz under
        asset_voltage_v={BESS_DID: 48.8},
        asset_current_a={BESS_DID: 0.0},
    )
    sp = state.get("asset_p_setpoint_kw", {}).get(BESS_DID, 0.0)
    # Under-freq → discharge (positive P).
    assert sp > 0.0
    assert state["asset_droop_state"][BESS_DID] == "Responding"


def test_over_frequency_charges(assets):
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-over"}}
    state = step(
        app,
        config,
        grid_freq_hz=50.5,  # 0.5 Hz over
        asset_voltage_v={BESS_DID: 48.8},
        asset_current_a={BESS_DID: 0.0},
    )
    sp = state.get("asset_p_setpoint_kw", {}).get(BESS_DID, 0.0)
    # Over-freq → charge (negative P).
    assert sp < 0.0


def test_high_soc_clamps_charge(assets):
    """At V near the OCV-100% (~57.6 V), SOC bootstrap puts us > 90 %, so
    p_min should clamp to 0 — no further charging even under over-freq."""
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-high-soc"}}
    # Bootstrap at OCV close to 100 % SOC: 57 V on a 40–57.6 V pack.
    state = step(
        app,
        config,
        grid_freq_hz=50.5,
        asset_voltage_v={BESS_DID: 57.0},
        asset_current_a={BESS_DID: 0.0},
    )
    soc = state.get("asset_soc_pct", {}).get(BESS_DID, 0.0)
    sp = state.get("asset_p_setpoint_kw", {}).get(BESS_DID, 0.0)
    # High SOC → p_min clamped to 0 → no charge despite over-freq.
    assert soc > 90.0
    assert sp >= 0.0


def test_low_soc_clamps_discharge(assets):
    """SOC near 0 % → discharge clamp triggers."""
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-low-soc"}}
    state = step(
        app,
        config,
        grid_freq_hz=49.5,
        asset_voltage_v={BESS_DID: 41.0},  # OCV near 0 % SOC
        asset_current_a={BESS_DID: 0.0},
    )
    soc = state.get("asset_soc_pct", {}).get(BESS_DID, 0.0)
    sp = state.get("asset_p_setpoint_kw", {}).get(BESS_DID, 0.0)
    assert soc < 10.0
    # Low SOC → p_max clamped → no discharge.
    assert sp <= 0.0


def test_history_has_checkpoints(assets):
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-history"}}
    for _ in range(3):
        step(
            app,
            config,
            grid_freq_hz=50.0,
            asset_voltage_v={BESS_DID: 48.8},
            asset_current_a={BESS_DID: 10.0},
        )
    history = list(app.get_state_history(config))
    assert len(history) >= 3


def test_multi_asset_aggregator():
    """Two BESS assets — aggregator sums per-asset Δp."""
    a, b = "did:test:bess-a", "did:test:bess-b"
    assets = [(a, 50.0, 50.0), (b, 100.0, 100.0)]
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "test-bess-multi"}}
    state = step(
        app,
        config,
        grid_freq_hz=49.5,
        asset_voltage_v={a: 48.8, b: 48.8},
        asset_current_a={a: 0.0, b: 0.0},
    )
    sp_a = state["asset_p_setpoint_kw"][a]
    sp_b = state["asset_p_setpoint_kw"][b]
    assert sp_a > 0.0 and sp_b > 0.0
    # Larger asset discharges more under same droop.
    assert sp_b > sp_a
