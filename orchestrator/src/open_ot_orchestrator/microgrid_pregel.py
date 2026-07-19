"""Microgrid prototype loop, executed by the minimal Pregel runner.

Two BESS assets each running a `DROOP_P_F` cell respond in parallel to the
same grid-frequency signal. The orchestrator's job per super-step:

  1. Build a `StepInput` for each BESS cell from the current grid frequency
     reading (single-task / row-driven trigger — one signal-change row per
     super-step).
  2. Call `LoopRunner.run_step(...)`.
  3. Sum the two cells' `delta_p_micro_kw` to compute the cohort's total
     response. Log it (and let the checkpoint stream carry the audit
     evidence per cell).

This mirrors the `:loop:freq-droop` shape sketched in
`PROTOTYPE-MICROGRID.md` §2.3, restricted to two field BESS cells (no
orchestrator-side aggregator cell yet).

Run: `uv run python -m open_ot_orchestrator.microgrid_pregel`
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from ._generated import droop_p_f as gen_droop
from .cell_loader import CellLoader
from .pregel_runner import CellSpec, Checkpoint, LoopRunner, StepInput

# ---------------------------------------------------------------------------
# DROOP_P_F struct layouts come from the generated wrapper (codegen-cell-
# types.py). The constants below are re-exported for backwards compatibility
# with code/tests that previously imported them from this module.
# ---------------------------------------------------------------------------

DROOP_PARAMS_FMT = gen_droop.PARAMS_FMT
DROOP_PARAMS_SIZE = gen_droop.PARAMS_SIZE
DROOP_INTERNAL_FMT = gen_droop.INTERNAL_FMT
DROOP_INTERNAL_SIZE = gen_droop.INTERNAL_SIZE
DROOP_DATA_IN_FMT = gen_droop.DATA_IN_FMT
DROOP_DATA_IN_SIZE = gen_droop.DATA_IN_SIZE
DROOP_DATA_OUT_FMT = gen_droop.DATA_OUT_FMT
DROOP_DATA_OUT_SIZE = gen_droop.DATA_OUT_SIZE


def pack_droop_params(
    p_rated_kw: float,
    p_min_kw: float,
    p_max_kw: float,
    droop_pct: float,
    dead_band_hz: float,
    cycle_period_ms: int,
) -> bytes:
    """User-facing engineering units → packed Params bytes (µ-units inside)."""
    return gen_droop.pack_params(
        gen_droop.Params(
            p_rated_micro_kw=int(p_rated_kw * 1_000_000),
            p_min_micro_kw=int(p_min_kw * 1_000_000),
            p_max_micro_kw=int(p_max_kw * 1_000_000),
            droop_permille=int(droop_pct * 10),
            dead_band_micro_hz=int(dead_band_hz * 1_000_000),
            cycle_period_ms=cycle_period_ms,
        )
    )


def pack_droop_data_in(
    grid_freq_hz: float,
    freq_nominal_hz: float,
    current_p_kw: float,
    enable: bool = True,
    freq_quality: int = 0,
    voltage_quality: int = 0,  # accepted for API symmetry; droop has no voltage
) -> bytes:
    """Pack one DROOP_P_F DataIn from human-readable engineering units."""
    return gen_droop.pack_data_in(
        gen_droop.DataIn(
            grid_freq=int(grid_freq_hz * 1_000_000),
            freq_nominal=int(freq_nominal_hz * 1_000_000),
            current_p=int(current_p_kw * 1_000_000),
            freq_quality=freq_quality,
            enable=enable,
        )
    )


@dataclass
class DroopOutput:
    p_setpoint_kw: float
    delta_p_kw: float
    freq_error_hz: float
    dead_band_active: bool
    saturated: bool


def unpack_droop_data_out(buf: bytes) -> DroopOutput:
    raw = gen_droop.unpack_data_out(buf)
    return DroopOutput(
        p_setpoint_kw=raw.p_setpoint / 1_000_000,
        delta_p_kw=raw.delta_p / 1_000_000,
        freq_error_hz=raw.freq_error / 1_000_000,
        dead_band_active=raw.dead_band_active,
        saturated=raw.saturated,
    )


# ---------------------------------------------------------------------------
# Microgrid loop builder
# ---------------------------------------------------------------------------

DROOP_WASM_PATH = Path(__file__).resolve().parent.parent.parent.parent / (
    "cells/target/wasm32-unknown-unknown/release/droop_p_f.wasm"
)


def build_freq_droop_loop(
    bess_assets: list[tuple[str, float]],
    cycle_period_ms: int = 100,
) -> LoopRunner:
    """Build a `:loop:freq-droop` runner.

    `bess_assets` = list of `(asset_did, p_rated_kw)`.
    """
    cells = []
    for did, p_rated in bess_assets:
        loader = CellLoader(DROOP_WASM_PATH, "droop_p_f")
        params = pack_droop_params(
            p_rated_kw=p_rated,
            p_min_kw=0,
            p_max_kw=p_rated,
            droop_pct=5.0,
            dead_band_hz=0.2,
            cycle_period_ms=cycle_period_ms,
        )
        cells.append(
            CellSpec(
                did=did,
                loader=loader,
                params_bytes=params,
                internal_size=DROOP_INTERNAL_SIZE,
                data_in_size=DROOP_DATA_IN_SIZE,
                data_out_size=DROOP_DATA_OUT_SIZE,
                initial_ecc=0,  # Idle
            )
        )
    return LoopRunner(cells)


def step_freq_droop(
    runner: LoopRunner,
    grid_freq_hz: float,
    current_p_per_asset_kw: dict[str, float],
    freq_nominal_hz: float = 50.0,
) -> Checkpoint:
    """Build per-cell inputs from a single grid-frequency reading and run."""
    inputs = {}
    for did in runner.cells:
        cur_p = current_p_per_asset_kw.get(did, 0.0)
        inputs[did] = StepInput(
            event_in_code=0,  # REQ
            data_in_bytes=pack_droop_data_in(
                grid_freq_hz=grid_freq_hz,
                freq_nominal_hz=freq_nominal_hz,
                current_p_kw=cur_p,
            ),
        )
    return runner.run_step(inputs)


def cohort_total_delta_kw(checkpoint: Checkpoint) -> float:
    total = 0.0
    for em in checkpoint.emissions:
        out = unpack_droop_data_out(em.data_out_bytes)
        total += out.delta_p_kw
    return total


# ---------------------------------------------------------------------------
# Demo entry point
# ---------------------------------------------------------------------------


def demo() -> None:
    print("[microgrid-pregel] building :loop:freq-droop with 2 BESS assets")
    runner = build_freq_droop_loop(
        bess_assets=[
            ("did:web:open-ot.etzhayyim.com:cell:droop-bess-1", 1000.0),  # 1 MW
            ("did:web:open-ot.etzhayyim.com:cell:droop-bess-2", 500.0),   #  500 kW
        ],
        cycle_period_ms=100,
    )
    runner.initialize()

    # Synthetic grid-frequency excursion: nominal → over-frequency → recover.
    schedule = [
        50.000,  # super_step 1 — initialize / nominal
        50.050,  # 2 — within deadband
        50.300,  # 3 — outside deadband, over-frequency, droop responds down
        50.500,  # 4 — bigger excursion
        50.300,  # 5 — recovering
        50.050,  # 6 — back in deadband
        50.000,  # 7 — fully recovered
    ]
    current_p = {
        "did:web:open-ot.etzhayyim.com:cell:droop-bess-1": 800.0,  # 800 kW current output
        "did:web:open-ot.etzhayyim.com:cell:droop-bess-2": 400.0,  # 400 kW current output
    }
    for grid_freq in schedule:
        cp = step_freq_droop(runner, grid_freq, current_p)
        total = cohort_total_delta_kw(cp)
        print(
            f"[step {cp.super_step:>2}] f={grid_freq:.3f} Hz  "
            f"cohort ΔP = {total:+8.2f} kW"
        )
    print()
    print(f"[microgrid-pregel] {len(runner.checkpoints)} checkpoints written")


if __name__ == "__main__":
    demo()
