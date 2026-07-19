"""Anti-islanding LangGraph integration tests (#2).

Validates orchestrator handling of: latched Tripped state, RESET event
semantics, and multi-event-output (CNF + TRIP packed in one u16).
"""

from __future__ import annotations

import pytest

from open_ot_orchestrator.microgrid_islanding_langgraph import (
    ECC_IDLE,
    ECC_MONITORING,
    ECC_TRIPPED,
    EVENT_IN_RESET,
    EVENT_OUT_CNF,
    EVENT_OUT_TRIP,
    ISL_WASM_PATH,
    build_islanding_graph,
    default_demo_params,
    reset_loader_registry,
    step,
)


requires_wasm = pytest.mark.skipif(
    not ISL_WASM_PATH.exists(),
    reason=(
        f"anti_islanding_rocof.wasm not at {ISL_WASM_PATH}. Build with:\n"
        "  cd ../cells && cargo build --release --no-default-features "
        "--target wasm32-unknown-unknown -p anti-islanding-rocof"
    ),
)


@pytest.fixture(autouse=True)
def _isolate_loader_cache():
    reset_loader_registry()
    yield
    reset_loader_registry()


@requires_wasm
def test_normal_grid_stays_monitoring():
    app = build_islanding_graph("ai", **default_demo_params())
    cfg = {"configurable": {"thread_id": "t1"}}
    # First tick initializes; second tick should be Monitoring.
    step(app, cfg, grid_freq_hz=50.0, grid_voltage_v=230.0)
    s = step(app, cfg, grid_freq_hz=50.0, grid_voltage_v=230.0)
    assert s["cell_ecc_state"] == ECC_MONITORING
    assert s["last_output"]["trip"] is False
    assert s["last_emitted"] == [EVENT_OUT_CNF]


@requires_wasm
def test_rocof_window_trips_and_emits_cnf_plus_trip():
    """3 consecutive ROCOF spikes → Tripped + emit (CNF, TRIP) on the same tick."""
    app = build_islanding_graph("ai", **default_demo_params())
    cfg = {"configurable": {"thread_id": "t-trip"}}
    # Init at 50.0
    step(app, cfg, 50.0, 230.0)
    # 3 consecutive ROCOF spikes (each +0.1 Hz over 100 ms = +1 Hz/s, >= 0.5 Hz/s threshold)
    s = None
    for f in (50.1, 50.2, 50.3):
        s = step(app, cfg, f, 230.0)
    assert s is not None
    assert s["cell_ecc_state"] == ECC_TRIPPED
    assert s["last_output"]["trip"] is True
    assert s["last_output"]["trip_reason"] == "Rocof"
    # Multi-event-output: both CNF and TRIP emitted on the same tick.
    assert sorted(s["last_emitted"]) == sorted([EVENT_OUT_CNF, EVENT_OUT_TRIP])


@requires_wasm
def test_tripped_state_latched_across_invocations():
    app = build_islanding_graph("ai", **default_demo_params())
    cfg = {"configurable": {"thread_id": "t-latch"}}
    step(app, cfg, 50.0, 230.0)
    for f in (50.1, 50.2, 50.3):
        step(app, cfg, f, 230.0)
    # Now feed perfectly nominal samples for several invocations — should
    # remain TRIPPED until RESET arrives.
    for _ in range(4):
        s = step(app, cfg, 50.0, 230.0)
        assert s["cell_ecc_state"] == ECC_TRIPPED
        assert s["last_output"]["trip"] is True


@requires_wasm
def test_reset_event_clears_latch():
    app = build_islanding_graph("ai", **default_demo_params())
    cfg = {"configurable": {"thread_id": "t-reset"}}
    step(app, cfg, 50.0, 230.0)
    for f in (50.1, 50.2, 50.3):
        step(app, cfg, f, 230.0)
    # latched
    assert step(app, cfg, 50.0, 230.0)["cell_ecc_state"] == ECC_TRIPPED
    # RESET
    s = step(app, cfg, 50.0, 230.0, event_in=EVENT_IN_RESET)
    assert s["cell_ecc_state"] == ECC_MONITORING
    # Subsequent normal REQ stays Monitoring.
    s2 = step(app, cfg, 50.0, 230.0)
    assert s2["cell_ecc_state"] == ECC_MONITORING
    assert s2["last_output"]["trip"] is False


@requires_wasm
def test_overvoltage_trip_reason_correct():
    app = build_islanding_graph("ai", **default_demo_params())
    cfg = {"configurable": {"thread_id": "t-ov"}}
    step(app, cfg, 50.0, 230.0)  # init
    # 5 consecutive overvoltage samples (260 V > 253 V max).
    s = None
    for _ in range(5):
        s = step(app, cfg, 50.0, 260.0)
    assert s is not None
    assert s["cell_ecc_state"] == ECC_TRIPPED
    assert s["last_output"]["trip_reason"] == "Overvoltage"


@requires_wasm
def test_disabled_returns_idle():
    app = build_islanding_graph("ai", **default_demo_params())
    cfg = {"configurable": {"thread_id": "t-disabled"}}
    s = step(app, cfg, 50.0, 230.0, enable=False)
    assert s["cell_ecc_state"] == ECC_IDLE
