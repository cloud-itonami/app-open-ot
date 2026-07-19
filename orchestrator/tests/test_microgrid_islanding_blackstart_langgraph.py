"""Tests for `microgrid_islanding_blackstart_langgraph` — chained FSM."""

from __future__ import annotations

import pytest

from open_ot_orchestrator.microgrid_islanding_blackstart_langgraph import (
    build_islanding_blackstart_graph,
    step,
)


def test_nominal_grid_holds_both_fsms_idle():
    app = build_islanding_blackstart_graph(cycle_period_ms=1_000)
    config = {"configurable": {"thread_id": "test-isl-nominal"}}
    s = step(app, config, grid_freq_hz=50.0, grid_voltage_v=230.0)
    assert s["protection_state"] == "Monitoring"
    assert s["protection_trip"] is False
    assert s["blackstart_state"] == "Idle"


def test_unauthorised_outage_stays_idle():
    app = build_islanding_blackstart_graph(cycle_period_ms=1_000)
    config = {"configurable": {"thread_id": "test-isl-unauth"}}
    for _ in range(5):
        s = step(
            app, config,
            grid_freq_hz=49.0, grid_voltage_v=230.0,
            authorised=False,
        )
    # Even with trip, !authorised → BS stays Idle.
    assert s["blackstart_state"] == "Idle"


def test_severe_under_freq_trips_and_starts_detection():
    """Sustained 49 Hz (below 49.5 min) for 3 ticks → trip → BS enters Detecting."""
    app = build_islanding_blackstart_graph(cycle_period_ms=1_000)
    config = {"configurable": {"thread_id": "test-isl-trip"}}
    for _ in range(5):
        s = step(app, config, grid_freq_hz=49.0, grid_voltage_v=230.0)
    assert s["protection_state"] == "Tripped"
    assert s["protection_trip"] is True
    assert s["blackstart_state"] == "Detecting"


def test_blackstart_advances_through_stages():
    """Walk the FSM: trip → detect dwell → StartGen → EnergizeBus → Syncing."""
    app = build_islanding_blackstart_graph(cycle_period_ms=1_000)
    config = {"configurable": {"thread_id": "test-isl-walk"}}
    # Confirm trip (3 ticks below freq band).
    for _ in range(4):
        step(app, config, grid_freq_hz=49.0, grid_voltage_v=230.0)
    # Confirmed-trip state: BS is in Detecting with a 5-step dwell.
    s = step(app, config, grid_freq_hz=49.0, grid_voltage_v=230.0)
    assert s["blackstart_state"] == "Detecting"
    # Walk dwell to 0.
    for _ in range(5):
        s = step(app, config, grid_freq_hz=49.0, grid_voltage_v=230.0)
    assert s["blackstart_state"] == "StartingGen"
    assert s["blackstart_command"] == "StartGen"
    # Generator ready → advance.
    s = step(
        app, config,
        grid_freq_hz=49.0, grid_voltage_v=230.0, gen_ready=True,
    )
    assert s["blackstart_state"] == "EnergizingBus"
    # Bus stable → Syncing.
    s = step(
        app, config,
        grid_freq_hz=49.0, grid_voltage_v=230.0,
        gen_ready=True, bus_voltage_stable=True,
    )
    assert s["blackstart_state"] == "Syncing"


def test_history_has_checkpoints():
    app = build_islanding_blackstart_graph(cycle_period_ms=1_000)
    config = {"configurable": {"thread_id": "test-isl-history"}}
    for _ in range(3):
        step(app, config, grid_freq_hz=50.0, grid_voltage_v=230.0)
    history = list(app.get_state_history(config))
    assert len(history) >= 3
