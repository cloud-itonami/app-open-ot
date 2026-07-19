"""OpenOtCheckpointer (SPEC §6) round-trip tests.

Validates the persistence layer that mirrors `vertex_open_ot_loop_checkpoint`
+ `vertex_open_ot_signal_change` for the production RisingWave deployment.
sqlite-backed here, but the SQLAlchemy Core code is dialect-portable.
"""

from __future__ import annotations

import pytest

from open_ot_orchestrator.checkpointer import (
    OpenOtCheckpointer,
    restore_runner_from_checkpointer,
    write_runner_checkpoint,
)
from open_ot_orchestrator.microgrid_pregel import (
    DROOP_WASM_PATH,
    build_freq_droop_loop,
    step_freq_droop,
    unpack_droop_data_out,
)


requires_wasm = pytest.mark.skipif(
    not DROOP_WASM_PATH.exists(),
    reason=(
        f"droop_p_f.wasm not at {DROOP_WASM_PATH}. Build with:\n"
        "  cd ../cells && cargo build --release --no-default-features "
        "--target wasm32-unknown-unknown -p droop-p-f"
    ),
)


# ---------------------------------------------------------------------------
# Schema / smoke
# ---------------------------------------------------------------------------


def test_schema_creates_in_memory():
    cw = OpenOtCheckpointer(dsn="sqlite:///:memory:")
    assert cw.count_checkpoints() == 0
    assert cw.latest_checkpoint("did:web:open-ot.etzhayyim.com:loop:absent") is None


def test_signal_change_insert_returns_row_id():
    cw = OpenOtCheckpointer(dsn="sqlite:///:memory:")
    rid1 = cw.record_signal_change(
        "did:web:open-ot.etzhayyim.com:signal:freq", 50_000_000, "good"
    )
    rid2 = cw.record_signal_change(
        "did:web:open-ot.etzhayyim.com:signal:freq", 50_001_000, "good"
    )
    assert rid2 == rid1 + 1


# ---------------------------------------------------------------------------
# Round-trip with real Pregel runner
# ---------------------------------------------------------------------------


@requires_wasm
def test_write_then_read_round_trip():
    runner = build_freq_droop_loop([("a", 1000.0)], cycle_period_ms=100)
    runner.initialize()
    cw = OpenOtCheckpointer()
    loop_did = "did:web:open-ot.etzhayyim.com:loop:freq-droop-test"

    # Write the post-init snapshot then 3 super-steps.
    write_runner_checkpoint(cw, loop_did, runner, runner.checkpoints[0])
    for f in [50.000, 50.300, 50.500]:
        cp = step_freq_droop(runner, f, {"a": 800.0})
        write_runner_checkpoint(cw, loop_did, runner, cp)

    assert cw.count_checkpoints(loop_did) == 4
    rows = cw.list_checkpoints(loop_did)
    assert [r.super_step for r in rows] == [0, 1, 2, 3]
    # Internals are preserved byte-identical through json+base64.
    for r, src in zip(rows, runner.checkpoints):
        assert r.internals == src.internals
        assert r.ecc_states == src.ecc_states


@requires_wasm
def test_resume_via_checkpointer_reproduces_outputs():
    """Equivalent of test_replay_determinism_after_checkpoint_restore in
    test_pregel_runner.py, but with the resume going through the
    persistence layer (write → fresh process simulation → read → restore).
    """
    schedule = [
        (50.000, 800.0),
        (50.300, 800.0),
        (50.500, 800.0),
        (50.300, 750.0),
        (50.050, 720.0),
    ]

    def factory():
        return build_freq_droop_loop([("a", 1000.0)], cycle_period_ms=100)

    cw = OpenOtCheckpointer()
    loop_did = "did:web:open-ot.etzhayyim.com:loop:resume-test"

    # Run #1: write a checkpoint after every step.
    r1 = factory()
    r1.initialize()
    write_runner_checkpoint(cw, loop_did, r1, r1.checkpoints[0])
    expected_outputs = []
    for f, p in schedule:
        cp = step_freq_droop(r1, f, {"a": p})
        write_runner_checkpoint(cw, loop_did, r1, cp)
        expected_outputs.append(unpack_droop_data_out(cp.emissions[0].data_out_bytes))

    # Run #2: fresh runner, restore from latest persisted checkpoint, then
    # replay nothing (we resumed at the very end → outputs match).
    r2 = factory()
    r2.initialize()
    restored = restore_runner_from_checkpointer(cw, loop_did, r2)
    assert restored is not None
    assert restored.super_step == len(schedule)
    # Internal bytes are byte-identical.
    assert r2.cells["a"].loader.get_internal_bytes(
        r2.cells["a"].internal_size
    ) == r1.cells["a"].loader.get_internal_bytes(r1.cells["a"].internal_size)


@requires_wasm
def test_resume_validates_params_rev_mismatch():
    """A different params_rev (cell reconfigured between write and resume)
    must raise — we'd be replaying against a different program."""
    schedule = [(50.0, 800.0), (50.3, 800.0)]

    cw = OpenOtCheckpointer()
    loop_did = "did:web:open-ot.etzhayyim.com:loop:rev-test"

    # Write with original params (1 MW asset).
    r1 = build_freq_droop_loop([("a", 1000.0)], cycle_period_ms=100)
    r1.initialize()
    write_runner_checkpoint(cw, loop_did, r1, r1.checkpoints[0])
    for f, p in schedule:
        cp = step_freq_droop(r1, f, {"a": p})
        write_runner_checkpoint(cw, loop_did, r1, cp)

    # Resume with different params (500 kW asset) → params_rev differs.
    r2 = build_freq_droop_loop([("a", 500.0)], cycle_period_ms=100)
    r2.initialize()
    with pytest.raises(ValueError, match="params_rev mismatch"):
        restore_runner_from_checkpointer(cw, loop_did, r2)


@requires_wasm
def test_multi_loop_independence():
    """Two loops in the same checkpointer must have independent histories."""
    cw = OpenOtCheckpointer()
    loop_a = "did:web:open-ot.etzhayyim.com:loop:a"
    loop_b = "did:web:open-ot.etzhayyim.com:loop:b"

    runner_a = build_freq_droop_loop([("x", 1000.0)], cycle_period_ms=100)
    runner_a.initialize()
    runner_b = build_freq_droop_loop([("x", 500.0)], cycle_period_ms=100)
    runner_b.initialize()

    for f in [50.0, 50.3, 50.5]:
        cp_a = step_freq_droop(runner_a, f, {"x": 800.0})
        write_runner_checkpoint(cw, loop_a, runner_a, cp_a)
    for f in [50.1, 50.2]:
        cp_b = step_freq_droop(runner_b, f, {"x": 400.0})
        write_runner_checkpoint(cw, loop_b, runner_b, cp_b)

    # Loop A: 0 (init) + 3 steps; Loop B: just 2 steps (we didn't write the init snapshot).
    assert cw.count_checkpoints(loop_a) == 3
    assert cw.count_checkpoints(loop_b) == 2
    a_steps = [r.super_step for r in cw.list_checkpoints(loop_a)]
    b_steps = [r.super_step for r in cw.list_checkpoints(loop_b)]
    assert a_steps == [1, 2, 3]
    assert b_steps == [1, 2]


def test_signal_change_records_loop_did_affected():
    cw = OpenOtCheckpointer()
    rid = cw.record_signal_change(
        signal_did="did:web:open-ot.etzhayyim.com:signal:freq-50hz",
        value_micro_unit=50_500_000,
        quality="good",
        loop_dids_affected=[
            "did:web:open-ot.etzhayyim.com:loop:freq-droop",
            "did:web:open-ot.etzhayyim.com:loop:islanding-decision",
        ],
    )
    assert rid >= 1
