"""Pregel runner unit tests (#3a).

Validates super-step semantics, checkpoint shape, and the determinism
contract (replay from checkpoint == original run from same point).
"""

from __future__ import annotations

import struct
from pathlib import Path

import pytest

from open_ot_orchestrator.cell_loader import CellLoader
from open_ot_orchestrator.microgrid_pregel import (
    DROOP_DATA_IN_SIZE,
    DROOP_DATA_OUT_SIZE,
    DROOP_INTERNAL_SIZE,
    DROOP_WASM_PATH,
    build_freq_droop_loop,
    cohort_total_delta_kw,
    pack_droop_data_in,
    pack_droop_params,
    step_freq_droop,
    unpack_droop_data_out,
)
from open_ot_orchestrator.pregel_runner import CellSpec, LoopRunner, StepInput


# ---------------------------------------------------------------------------
# Skip when wasm artefact missing — gives a useful error instead of a
# cryptic FileNotFoundError mid-test.
# ---------------------------------------------------------------------------

requires_wasm = pytest.mark.skipif(
    not DROOP_WASM_PATH.exists(),
    reason=(
        f"droop_p_f.wasm not at {DROOP_WASM_PATH}. Build with:\n"
        "  cd ../cells && cargo build --release --no-default-features "
        "--target wasm32-unknown-unknown -p droop-p-f"
    ),
)


# ---------------------------------------------------------------------------
# Single-cell smoke
# ---------------------------------------------------------------------------


@requires_wasm
def test_single_cell_step():
    runner = build_freq_droop_loop(
        bess_assets=[("did:web:open-ot.etzhayyim.com:cell:droop-test", 1000.0)],
        cycle_period_ms=100,
    )
    runner.initialize()
    cp = step_freq_droop(runner, grid_freq_hz=50.0, current_p_per_asset_kw={
        "did:web:open-ot.etzhayyim.com:cell:droop-test": 500.0,
    })
    assert cp.super_step == 1
    assert len(cp.emissions) == 1
    out = unpack_droop_data_out(cp.emissions[0].data_out_bytes)
    # Nominal frequency, deadband ±0.2 Hz → no response.
    assert out.dead_band_active
    assert out.delta_p_kw == 0.0


# ---------------------------------------------------------------------------
# Multi-cell + cohort sum
# ---------------------------------------------------------------------------


@requires_wasm
def test_two_cells_cohort_response():
    runner = build_freq_droop_loop(
        bess_assets=[
            ("a", 1000.0),  # 1 MW
            ("b", 500.0),   # 500 kW
        ],
        cycle_period_ms=100,
    )
    runner.initialize()
    # 50.5 Hz over-frequency, outside 0.2 Hz deadband → both respond DOWN.
    cp = step_freq_droop(
        runner,
        grid_freq_hz=50.5,
        current_p_per_asset_kw={"a": 800.0, "b": 400.0},
    )
    total = cohort_total_delta_kw(cp)
    assert total < 0.0, f"expected negative cohort response, got {total}"
    # Each emission should be a CNF (out_event_raw == 1).
    for em in cp.emissions:
        assert em.out_event_raw == 1


# ---------------------------------------------------------------------------
# Single-task / row-driven: cells without StepInput skip this step
# ---------------------------------------------------------------------------


@requires_wasm
def test_single_task_skips_unaffected_cells():
    runner = build_freq_droop_loop(
        bess_assets=[("a", 1000.0), ("b", 500.0)],
        cycle_period_ms=100,
    )
    runner.initialize()
    # Only feed cell `a`. `b` should be skipped (no emission, ECC unchanged).
    inputs = {
        "a": StepInput(
            event_in_code=0,
            data_in_bytes=pack_droop_data_in(50.5, 50.0, 800.0),
        )
    }
    cp = runner.run_step(inputs)
    assert cp.super_step == 1
    cell_dids_emitted = {em.cell_did for em in cp.emissions}
    assert cell_dids_emitted == {"a"}
    # Both cells still have an entry in ecc_states / internals.
    assert set(cp.ecc_states.keys()) == {"a", "b"}
    assert set(cp.internals.keys()) == {"a", "b"}


# ---------------------------------------------------------------------------
# Checkpoint stream is monotonic and per-step complete
# ---------------------------------------------------------------------------


@requires_wasm
def test_checkpoint_stream_monotonic():
    runner = build_freq_droop_loop(
        bess_assets=[("a", 1000.0)],
        cycle_period_ms=100,
    )
    runner.initialize()
    for f in [50.0, 50.1, 50.3, 50.5, 50.3, 50.0]:
        step_freq_droop(runner, f, {"a": 800.0})
    # initialize() snapshot + 6 steps = 7 checkpoints.
    assert len(runner.checkpoints) == 7
    for i, cp in enumerate(runner.checkpoints):
        assert cp.super_step == i


# ---------------------------------------------------------------------------
# Determinism: resume from checkpoint reproduces subsequent outputs
# ---------------------------------------------------------------------------


@requires_wasm
def test_replay_determinism_after_checkpoint_restore():
    schedule = [
        (50.000, {"a": 800.0}),
        (50.300, {"a": 800.0}),
        (50.500, {"a": 800.0}),
        (50.300, {"a": 750.0}),
        (50.050, {"a": 720.0}),
    ]

    def factory():
        return build_freq_droop_loop(
            bess_assets=[("a", 1000.0)], cycle_period_ms=100
        )

    # Run #1: full sequence.
    r1 = factory()
    r1.initialize()
    for f, p in schedule:
        step_freq_droop(r1, f, p)
    full_outputs = [
        unpack_droop_data_out(cp.emissions[0].data_out_bytes)
        for cp in r1.checkpoints[1:]
    ]
    full_internals = [
        cp.internals["a"] for cp in r1.checkpoints[1:]
    ]

    # Run #2: take checkpoint at step 2, restore, replay rest.
    pivot = r1.checkpoints[2]  # after running 2 of the 5 inputs
    r2 = factory()
    r2.initialize()
    r2.restore_from_checkpoint(pivot)
    for f, p in schedule[2:]:
        step_freq_droop(r2, f, p)
    resumed_outputs = [
        unpack_droop_data_out(cp.emissions[0].data_out_bytes)
        for cp in r2.checkpoints[1:]  # first cp is the restored pivot
    ]
    resumed_internals = [cp.internals["a"] for cp in r2.checkpoints[1:]]

    # Step 3, 4, 5 outputs/internals must match between the two runs.
    expected_outputs = full_outputs[2:]
    expected_internals = full_internals[2:]
    assert len(resumed_outputs) == len(expected_outputs)
    for i, (got, want) in enumerate(zip(resumed_outputs, expected_outputs)):
        assert got == want, f"output mismatch at resumed step {i}: {got} vs {want}"
    for i, (got, want) in enumerate(zip(resumed_internals, expected_internals)):
        assert got == want, f"internal mismatch at resumed step {i}"


# ---------------------------------------------------------------------------
# Restore guards
# ---------------------------------------------------------------------------


@requires_wasm
def test_restore_rejects_mismatched_cell_set():
    runner_a = build_freq_droop_loop(
        bess_assets=[("a", 1000.0)], cycle_period_ms=100
    )
    runner_a.initialize()
    step_freq_droop(runner_a, 50.0, {"a": 500.0})
    cp = runner_a.checkpoints[-1]

    runner_b = build_freq_droop_loop(
        bess_assets=[("b", 500.0)], cycle_period_ms=100
    )
    runner_b.initialize()
    with pytest.raises(ValueError):
        runner_b.restore_from_checkpoint(cp)


def test_loop_runner_rejects_duplicate_dids():
    # No wasm needed — this is a pure-Python validation.
    fake_loader = object()  # never used; init() is never called
    cell = CellSpec(
        did="x",
        loader=fake_loader,  # type: ignore[arg-type]
        params_bytes=b"",
        internal_size=0,
        data_in_size=0,
        data_out_size=0,
    )
    with pytest.raises(ValueError, match="duplicate cell DID"):
        LoopRunner([cell, cell])
