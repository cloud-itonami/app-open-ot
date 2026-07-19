//! Risk-1 Gate A simulator (cell-agnostic).
//!
//! Loads a BFB cell as `wasm32-unknown-unknown`, lays out the cell's
//! Params / Internal / DataIn / DataOut in linear memory, runs N ticks
//! at synthetic input, and emits a latency report.
//!
//! Per-cell layouts + synth functions live in the `cells::*` module
//! below — adding a new cell means: declare its `CellLayout`, write a
//! `synthesize_*` function, register both in `select_cell`. The rest is
//! cell-agnostic.
//!
//! Host latency on commodity x86_64 / Apple Silicon is **not** what
//! Gate A measures — the real measurement is the same harness on
//! Cortex-M7 (Giemon Mimi) via WAMR AOT (per SPEC §14.1). This rig
//! validates:
//!
//! 1. The cell's `wasm32` artefact loads cleanly.
//! 2. The `#[no_mangle] extern "C"` ABI surface is callable end-to-end.
//! 3. Struct layouts match between the cell's Rust source and the rig.
//! 4. Determinism: same input → same output.
//! 5. The histogram / report pipeline works at the cell's data scale.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::time::Instant;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

const PAGE_SIZE: u32 = 65_536;

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "gate-a-rig", version, about = "Risk-1 Gate A simulator")]
struct Cli {
    /// Path to the cell `.wasm` artefact (wasm32-unknown-unknown).
    #[arg(long)]
    wasm_path: Option<PathBuf>,

    /// Cell name. One of `pid_limited` (default), `pid_stack_100`, `droop_p_f`, `anti_islanding_rocof`.
    #[arg(long, default_value = "pid_limited")]
    cell: String,

    /// Number of tick iterations to run.
    #[arg(long, default_value_t = 100_000)]
    iterations: u64,

    /// Cycle period in milliseconds (Params.cycle_period_ms).
    #[arg(long, default_value_t = 1u32)]
    cycle_period_ms: u32,

    /// Markdown report output path. Default depends on `--cell`.
    #[arg(long)]
    report: Option<PathBuf>,

    /// Per-tick deadline in nanoseconds. If > 0, each tick that takes longer
    /// is counted as a deadline miss and the rig exits non-zero when the
    /// run finishes with > 0 misses. Default 0 disables enforcement so host
    /// runs only report latency. The real Gate A budget on Mimi is 200_000
    /// (200 µs) at p99.9 per SPEC §14.1.
    #[arg(long, default_value_t = 0u64)]
    deadline_ns: u64,
}

// ---------------------------------------------------------------------------
// Cell layout registry
// ---------------------------------------------------------------------------

struct CellLayout {
    /// `<cell>_init` / `<cell>_tick` symbol prefix.
    symbol: &'static str,
    params_size: u32,
    internal_size: u32,
    data_in_size: u32,
    data_out_size: u32,
    /// out_event size in bytes (1 for u8 packing, 2 for u16 packing).
    out_event_size: u32,
    /// Required scratch base — past data + bss + stack of the cdylib.
    /// Defaults to 1 MiB; raise for cells with >256 KB scratch.
    scratch_base: u32,
    /// Pages of WASM linear memory we need (each page is 64 KB).
    required_pages: u64,
    /// Default report file basename.
    default_report: &'static str,
    /// Build the cell's Params bytes.
    build_params: fn(cycle_period_ms: u32) -> Vec<u8>,
    /// Synthesise one tick of DataIn bytes (deterministic; tick is the seed).
    synthesize_data_in: fn(tick: u64) -> Vec<u8>,
}

fn select_cell(name: &str) -> Result<&'static CellLayout> {
    match name {
        "pid_limited" => Ok(&PID_LIMITED),
        "pid_stack_100" => Ok(&PID_STACK_100),
        "droop_p_f" => Ok(&DROOP_P_F),
        "anti_islanding_rocof" => Ok(&ANTI_ISLANDING_ROCOF),
        "vv_curve" => Ok(&VV_CURVE),
        "ltc_tap_fsm" => Ok(&LTC_TAP_FSM),
        "mppt_perturb_observe" => Ok(&MPPT_PERTURB_OBSERVE),
        "black_start_seq" => Ok(&BLACK_START_SEQ),
        "soc_kalman" => Ok(&SOC_KALMAN),
        other => bail!(
            "unknown --cell {:?} (expected one of: pid_limited, pid_stack_100, droop_p_f, anti_islanding_rocof, vv_curve, ltc_tap_fsm, mppt_perturb_observe, black_start_seq, soc_kalman)",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// pid_limited layout
// ---------------------------------------------------------------------------

static PID_LIMITED: CellLayout = CellLayout {
    symbol: "pid_limited",
    params_size: 20,
    internal_size: 16,
    data_in_size: 12,
    data_out_size: 12,
    out_event_size: 1,
    scratch_base: 0x10_0000, // 1 MiB
    required_pages: 32,       // 2 MiB
    default_report: "gate-a-report.md",
    build_params: build_params_pid_limited,
    synthesize_data_in: synthesize_data_in_pid_limited,
};

fn build_params_pid_limited(cycle_period_ms: u32) -> Vec<u8> {
    // Mirror of cells/pid-limited Params: i32 × 4 + u32. align 4, no tail pad.
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&1_200_000_i32.to_le_bytes()); // kp
    buf.extend_from_slice(&50_000_i32.to_le_bytes()); // ki
    buf.extend_from_slice(&0_i32.to_le_bytes()); // out_min
    buf.extend_from_slice(&100_000_000_i32.to_le_bytes()); // out_max
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes());
    buf
}

fn synthesize_data_in_pid_limited(tick: u64) -> Vec<u8> {
    // Mirror of pid-limited DataIn: i32 pv + i32 sp + u8 quality + bool enable
    // + 2 pad → 12 bytes.
    let drift = ((tick as i64 * 257) % 1_000_000) as i32 - 500_000;
    let pv: i32 = 50_000_000 + drift;
    let sp: i32 = 60_000_000;
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&pv.to_le_bytes());
    buf.extend_from_slice(&sp.to_le_bytes());
    buf.push(0); // quality good
    buf.push(1); // enable
    buf.extend_from_slice(&[0u8; 2]); // pad
    buf
}

// ---------------------------------------------------------------------------
// pid_stack_100 layout
// ---------------------------------------------------------------------------

static PID_STACK_100: CellLayout = CellLayout {
    symbol: "pid_stack_100",
    params_size: 20,    // same shape as pid_limited Params
    internal_size: 1300,
    data_in_size: 1000,
    data_out_size: 1000,
    out_event_size: 1,
    // Same 1 MiB base; the per-buffer offsets are larger though, so make
    // sure we have enough memory pages.
    scratch_base: 0x10_0000,
    required_pages: 64, // 4 MiB to comfortably fit 1300 + 1000 + 1000 + slack
    default_report: "gate-a-stack100-report.md",
    build_params: build_params_pid_stack_100,
    synthesize_data_in: synthesize_data_in_pid_stack_100,
};

fn build_params_pid_stack_100(cycle_period_ms: u32) -> Vec<u8> {
    // pid-stack-100 shares the Params shape with pid-limited (single shared
    // PI tuning).
    build_params_pid_limited(cycle_period_ms)
}

fn synthesize_data_in_pid_stack_100(tick: u64) -> Vec<u8> {
    // 100 instances: pvs[100] + sps[100] + qualities[100] + enables[100].
    // Each instance gets a deterministic drift seeded by (tick, instance_idx)
    // so the integral term stays active across the whole stack.
    const N: usize = 100;
    let mut buf = Vec::with_capacity(1000);
    // pvs
    for i in 0..N {
        let seed = tick.wrapping_mul(257).wrapping_add(i as u64 * 7);
        let drift = ((seed as i64) % 1_000_000) as i32 - 500_000;
        let pv: i32 = 50_000_000 + drift;
        buf.extend_from_slice(&pv.to_le_bytes());
    }
    // sps — fixed setpoint per instance, slightly varied so error sign
    // varies across instances.
    for i in 0..N {
        let sp: i32 = 60_000_000 + (i as i32 * 1_000);
        buf.extend_from_slice(&sp.to_le_bytes());
    }
    // qualities — all good
    buf.extend_from_slice(&[0u8; N]);
    // enables — all enabled
    buf.extend_from_slice(&[1u8; N]);
    buf
}

// ---------------------------------------------------------------------------
// droop_p_f layout (per cells/droop-p-f/src/lib.rs)
// ---------------------------------------------------------------------------

static DROOP_P_F: CellLayout = CellLayout {
    symbol: "droop_p_f",
    // Params: 5 × i32 + u32 = 24 bytes.
    params_size: 24,
    // Internal: i32 + bool, align 4 → 8 bytes.
    internal_size: 8,
    // DataIn: i64 + i64 + i32 + u8 + bool, align 8, tail-pad → 24 bytes.
    data_in_size: 24,
    // DataOut: i32 + i32 + i64 + bool + bool, align 8, tail-pad → 24 bytes.
    data_out_size: 24,
    // *mut u8.
    out_event_size: 1,
    scratch_base: 0x10_0000, // 1 MiB
    required_pages: 32,       // 2 MiB
    default_report: "gate-a-droop-report.md",
    build_params: build_params_droop_p_f,
    synthesize_data_in: synthesize_data_in_droop_p_f,
};

fn build_params_droop_p_f(cycle_period_ms: u32) -> Vec<u8> {
    // 100 kW asset, ±100 kW clamp, 5 % droop, 0.2 Hz deadband.
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&100_000_000_i32.to_le_bytes()); // p_rated_micro_kw (100 kW)
    buf.extend_from_slice(&(-100_000_000_i32).to_le_bytes()); // p_min_micro_kw
    buf.extend_from_slice(&100_000_000_i32.to_le_bytes()); // p_max_micro_kw
    buf.extend_from_slice(&50_i32.to_le_bytes()); // droop_permille = 5 %
    buf.extend_from_slice(&200_000_i32.to_le_bytes()); // dead_band_micro_hz (0.2 Hz)
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes());
    buf
}

fn synthesize_data_in_droop_p_f(tick: u64) -> Vec<u8> {
    // Sinusoidal-ish drift in grid frequency across the deadband boundary so
    // the cell exits the trivial deadband branch on a meaningful share of
    // ticks. Nominal 50 Hz, drift up to ±0.5 Hz (= ±500_000 µHz).
    let drift_micro_hz = ((tick as i64).wrapping_mul(257) % 1_000_001) - 500_000;
    let grid_freq_micro_hz: i64 = 50_000_000_i64.saturating_add(drift_micro_hz);
    let freq_nominal_micro_hz: i64 = 50_000_000;
    let current_p_micro_kw: i32 = 50_000_000; // 50 kW currently
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&grid_freq_micro_hz.to_le_bytes()); // 0..8
    buf.extend_from_slice(&freq_nominal_micro_hz.to_le_bytes()); // 8..16
    buf.extend_from_slice(&current_p_micro_kw.to_le_bytes()); // 16..20
    buf.push(0); // freq_quality = Good (20)
    buf.push(1); // enable (21)
    buf.extend_from_slice(&[0u8; 2]); // tail-pad → 24
    buf
}

// ---------------------------------------------------------------------------
// anti_islanding_rocof layout (per cells/anti-islanding-rocof/src/lib.rs)
// ---------------------------------------------------------------------------

static ANTI_ISLANDING_ROCOF: CellLayout = CellLayout {
    symbol: "anti_islanding_rocof",
    // Params: i64 + u32 + (pad 4) + i64 + i64 + u32 + (pad 4) + i64 + i64
    //          + u32 + u32 = 64 bytes.
    params_size: 64,
    // Internal: i64 + u32 + u32 + u32 + u8 + bool, align 8, tail-pad → 24.
    internal_size: 24,
    // DataIn: 4 × i64 + 2 × u8 + bool, align 8, tail-pad → 40.
    data_in_size: 40,
    // DataOut: bool + u8 + pad6 + i64 + i32 + pad4 + i64 + 3 × u8, tail-pad → 40.
    data_out_size: 40,
    // *mut u16 — multi-event packed.
    out_event_size: 2,
    scratch_base: 0x10_0000,
    required_pages: 32,
    default_report: "gate-a-anti-islanding-report.md",
    build_params: build_params_anti_islanding_rocof,
    synthesize_data_in: synthesize_data_in_anti_islanding_rocof,
};

fn build_params_anti_islanding_rocof(cycle_period_ms: u32) -> Vec<u8> {
    // ENTSO-E-ish defaults: 0.5 Hz/s ROCOF, ±10 % voltage envelope, ±1 % freq.
    let mut buf = Vec::with_capacity(64);
    buf.extend_from_slice(&500_000_i64.to_le_bytes()); // rocof_threshold_micro_hz_per_s (0..8)
    buf.extend_from_slice(&3_u32.to_le_bytes()); // rocof_window_samples (8..12)
    buf.extend_from_slice(&[0u8; 4]); // pad (12..16)
    buf.extend_from_slice(&207_000_000_i64.to_le_bytes()); // voltage_min_micro_v (207 V) (16..24)
    buf.extend_from_slice(&253_000_000_i64.to_le_bytes()); // voltage_max_micro_v (253 V) (24..32)
    buf.extend_from_slice(&3_u32.to_le_bytes()); // voltage_window_samples (32..36)
    buf.extend_from_slice(&[0u8; 4]); // pad (36..40)
    buf.extend_from_slice(&49_500_000_i64.to_le_bytes()); // freq_min_micro_hz (49.5 Hz) (40..48)
    buf.extend_from_slice(&50_500_000_i64.to_le_bytes()); // freq_max_micro_hz (50.5 Hz) (48..56)
    buf.extend_from_slice(&3_u32.to_le_bytes()); // freq_window_samples (56..60)
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes()); // (60..64)
    buf
}

fn synthesize_data_in_anti_islanding_rocof(tick: u64) -> Vec<u8> {
    // Nominal 50 Hz / 230 V grid with sub-threshold drift so the cell stays
    // in Monitoring and emits CNF every tick. Real Gate A criterion is
    // p99.9 ≤ 200 µs in steady-state monitoring — fault transients are
    // covered by replay tests on the cell, not here.
    let drift_micro_hz = ((tick as i64).wrapping_mul(311) % 400_001) - 200_000;
    let drift_micro_v = ((tick as i64).wrapping_mul(541) % 20_000_001) - 10_000_000;
    let grid_freq_micro_hz: i64 = 50_000_000_i64.saturating_add(drift_micro_hz);
    let freq_nominal_micro_hz: i64 = 50_000_000;
    let grid_voltage_micro_v: i64 = 230_000_000_i64.saturating_add(drift_micro_v);
    let voltage_nominal_micro_v: i64 = 230_000_000;
    let mut buf = Vec::with_capacity(40);
    buf.extend_from_slice(&grid_freq_micro_hz.to_le_bytes()); // 0..8
    buf.extend_from_slice(&freq_nominal_micro_hz.to_le_bytes()); // 8..16
    buf.extend_from_slice(&grid_voltage_micro_v.to_le_bytes()); // 16..24
    buf.extend_from_slice(&voltage_nominal_micro_v.to_le_bytes()); // 24..32
    buf.push(0); // freq_quality = Good (32)
    buf.push(0); // voltage_quality = Good (33)
    buf.push(1); // enable (34)
    buf.extend_from_slice(&[0u8; 5]); // tail-pad → 40
    buf
}

// ---------------------------------------------------------------------------
// vv_curve layout (per cells/vv-curve/src/lib.rs)
// ---------------------------------------------------------------------------

static VV_CURVE: CellLayout = CellLayout {
    symbol: "vv_curve",
    params_size: 20,    // i32 × 4 + u32 = 20
    internal_size: 8,   // i32 + bool, align 4 → 8
    data_in_size: 12,   // i32 + i32 + u8 + bool, align 4 → 12
    data_out_size: 12,  // i32 + i32 + bool + bool, align 4 → 12
    out_event_size: 1,
    scratch_base: 0x10_0000,
    required_pages: 32,
    default_report: "gate-a-vv-curve-report.md",
    build_params: build_params_vv_curve,
    synthesize_data_in: synthesize_data_in_vv_curve,
};

fn build_params_vv_curve(cycle_period_ms: u32) -> Vec<u8> {
    // IEEE 1547 default curve breakpoints.
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&1_030_000_i32.to_le_bytes()); // v_dead_high_micro_pu
    buf.extend_from_slice(&1_060_000_i32.to_le_bytes()); // v_full_high_micro_pu
    buf.extend_from_slice(&970_000_i32.to_le_bytes());   // v_dead_low_micro_pu
    buf.extend_from_slice(&900_000_i32.to_le_bytes());   // v_full_low_micro_pu
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes());
    buf
}

fn synthesize_data_in_vv_curve(tick: u64) -> Vec<u8> {
    // Voltage sweeps slowly across the curve so all branches get exercised.
    let off = ((tick as i64).wrapping_mul(257) % 200_001) - 100_000; // ±0.1 pu
    let v_micro_pu: i32 = (1_000_000_i64.saturating_add(off)) as i32;
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&v_micro_pu.to_le_bytes());
    buf.extend_from_slice(&100_000_000_i32.to_le_bytes()); // q_max = 100 kVAR
    buf.push(0); // voltage_quality = Good
    buf.push(1); // enable
    buf.extend_from_slice(&[0u8; 2]); // tail-pad → 12
    buf
}

// ---------------------------------------------------------------------------
// ltc_tap_fsm layout (per cells/ltc-tap-fsm/src/lib.rs)
// ---------------------------------------------------------------------------

static LTC_TAP_FSM: CellLayout = CellLayout {
    symbol: "ltc_tap_fsm",
    params_size: 24,    // i64 + u32 + i16 + i16 + u32 + 4 pad = 24 (align 8)
    internal_size: 8,   // u32 + TapCommand u8 + bool + 2 pad = 8 (align 4)
    data_in_size: 24,   // i64 + i64 + i16 + u8 + bool + 4 pad = 24 (align 8)
    data_out_size: 24,  // TapCommand u8 + 7 pad + i64 + u32 + bool + 3 pad = 24 (align 8)
    out_event_size: 1,
    scratch_base: 0x10_0000,
    required_pages: 32,
    default_report: "gate-a-ltc-tap-report.md",
    build_params: build_params_ltc_tap_fsm,
    synthesize_data_in: synthesize_data_in_ltc_tap_fsm,
};

fn build_params_ltc_tap_fsm(cycle_period_ms: u32) -> Vec<u8> {
    // 11 kV bus, ±150 V deadband, 30 s dwell, ±8 tap range.
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&150_000_000_i64.to_le_bytes()); // dead_band_micro_v (= 0.15 V at µV; for 11 kV scale tweak in real deployments)
    buf.extend_from_slice(&30_000_u32.to_le_bytes());      // dwell_ms (8)
    buf.extend_from_slice(&(-8_i16).to_le_bytes());        // tap_min (12)
    buf.extend_from_slice(&8_i16.to_le_bytes());           // tap_max (14)
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes()); // (16)
    buf.extend_from_slice(&[0u8; 4]);                       // tail-pad → 24
    buf
}

fn synthesize_data_in_ltc_tap_fsm(tick: u64) -> Vec<u8> {
    // Bus voltage oscillates slowly above and below target. Tap stays at 0
    // for the host run; real deployments would track this in the loop.
    let target_v: i64 = 11_000_000_000;
    let off = ((tick as i64).wrapping_mul(311) % 1_000_001) - 500_000_000;
    let meas_v: i64 = target_v.saturating_add(off);
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&meas_v.to_le_bytes());   // voltage_meas (0)
    buf.extend_from_slice(&target_v.to_le_bytes()); // voltage_target (8)
    buf.extend_from_slice(&0_i16.to_le_bytes());    // tap_position (16)
    buf.push(0);                                     // voltage_quality (18)
    buf.push(1);                                     // enable (19)
    buf.extend_from_slice(&[0u8; 4]);                // tail-pad → 24
    buf
}

// ---------------------------------------------------------------------------
// mppt_perturb_observe layout (per cells/mppt-perturb-observe/src/lib.rs)
// ---------------------------------------------------------------------------

static MPPT_PERTURB_OBSERVE: CellLayout = CellLayout {
    symbol: "mppt_perturb_observe",
    params_size: 32,    // i32 × 3 + 4 pad + i64 + u32 + 4 pad = 32 (align 8)
    internal_size: 24,  // i32 + 4 pad + i64 + PerturbDir u8 + bool + 6 pad = 24 (align 8)
    data_in_size: 12,   // i32 + i32 + u8 + u8 + bool + 1 pad = 12 (align 4)
    data_out_size: 24,  // i32 + 4 pad + i64 + PerturbDir u8 + bool + 6 pad = 24 (align 8)
    out_event_size: 1,
    scratch_base: 0x10_0000,
    required_pages: 32,
    default_report: "gate-a-mppt-report.md",
    build_params: build_params_mppt_perturb_observe,
    synthesize_data_in: synthesize_data_in_mppt_perturb_observe,
};

fn build_params_mppt_perturb_observe(cycle_period_ms: u32) -> Vec<u8> {
    // 200–600 V PV string, 0.1 V step, 5 W MPP tolerance.
    let mut buf = Vec::with_capacity(32);
    buf.extend_from_slice(&100_000_i32.to_le_bytes());     // perturb_step_micro_v (0)
    buf.extend_from_slice(&200_000_000_i32.to_le_bytes()); // v_min (4)
    buf.extend_from_slice(&600_000_000_i32.to_le_bytes()); // v_max (8)
    buf.extend_from_slice(&[0u8; 4]);                       // pad to 16
    buf.extend_from_slice(&5_000_000_000_000_i64.to_le_bytes()); // mpp_tolerance_pw (16) = 5 W
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes()); // (24)
    buf.extend_from_slice(&[0u8; 4]);                       // tail-pad → 32
    buf
}

fn synthesize_data_in_mppt_perturb_observe(tick: u64) -> Vec<u8> {
    // 400 V nominal PV with small irradiance-driven drift in current.
    let i_drift = ((tick as i64).wrapping_mul(257) % 5_000_001) - 2_500_000;
    let i_micro_a: i32 = (25_000_000_i64.saturating_add(i_drift)) as i32;
    let mut buf = Vec::with_capacity(12);
    buf.extend_from_slice(&400_000_000_i32.to_le_bytes()); // pv_voltage (0)
    buf.extend_from_slice(&i_micro_a.to_le_bytes());        // pv_current (4)
    buf.push(0); // voltage_quality (8)
    buf.push(0); // current_quality (9)
    buf.push(1); // enable (10)
    buf.extend_from_slice(&[0u8; 1]); // tail-pad → 12
    buf
}

// ---------------------------------------------------------------------------
// black_start_seq layout (per cells/black-start-seq/src/lib.rs)
// ---------------------------------------------------------------------------

static BLACK_START_SEQ: CellLayout = CellLayout {
    symbol: "black_start_seq",
    params_size: 20,    // 5 × u32 = 20 (align 4)
    internal_size: 8,   // u32 + u8 + bool + 2 pad = 8 (align 4)
    data_in_size: 5,    // 5 bools = 5 (align 1)
    data_out_size: 12,  // u8 + Command u8 + 2 pad + u32 + bool + 3 pad = 12 (align 4)
    out_event_size: 1,
    scratch_base: 0x10_0000,
    required_pages: 32,
    default_report: "gate-a-black-start-report.md",
    build_params: build_params_black_start_seq,
    synthesize_data_in: synthesize_data_in_black_start_seq,
};

fn build_params_black_start_seq(cycle_period_ms: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(20);
    buf.extend_from_slice(&5_000_u32.to_le_bytes());     // detect_dwell_ms
    buf.extend_from_slice(&60_000_u32.to_le_bytes());    // gen_timeout_ms
    buf.extend_from_slice(&30_000_u32.to_le_bytes());    // bus_timeout_ms
    buf.extend_from_slice(&120_000_u32.to_le_bytes());   // sync_timeout_ms
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes());
    buf
}

fn synthesize_data_in_black_start_seq(_tick: u64) -> Vec<u8> {
    // Stable healthy-grid scenario: stays in Idle every tick. The state
    // machine is well-tested in cell-side unit tests; the rig measures
    // the steady-state tick budget.
    let mut buf = Vec::with_capacity(5);
    buf.push(1); // grid_present
    buf.push(0); // gen_ready
    buf.push(0); // bus_voltage_stable
    buf.push(0); // voltage_synced
    buf.push(1); // authorised
    buf
}

// ---------------------------------------------------------------------------
// soc_kalman layout (per cells/soc-kalman/src/lib.rs)
// ---------------------------------------------------------------------------

static SOC_KALMAN: CellLayout = CellLayout {
    symbol: "soc_kalman",
    params_size: 40,    // 4 × i64 + u16 + 2 pad + u32 + 4 pad = 40 (align 8)
    internal_size: 24,  // i32 + 4 pad + i64 + u16 + bool + 5 pad = 24 (align 8)
    data_in_size: 24,   // i64 + i64 + i32 + u8 + u8 + bool + 5 pad = 24 (align 8)
    data_out_size: 32,  // i32 + 4 pad + i64 + i64 + u16 + 6 pad = 32 (align 8)
    out_event_size: 1,
    scratch_base: 0x10_0000,
    required_pages: 32,
    default_report: "gate-a-soc-kalman-report.md",
    build_params: build_params_soc_kalman,
    synthesize_data_in: synthesize_data_in_soc_kalman,
};

fn build_params_soc_kalman(cycle_period_ms: u32) -> Vec<u8> {
    // 100 Ah LFP pack, 1 mΩ IR, 40–57.6 V OCV span, 10 % gain blend.
    let mut buf = Vec::with_capacity(40);
    buf.extend_from_slice(&360_000_000_000_i64.to_le_bytes()); // capacity_micro_c (0)
    buf.extend_from_slice(&1_000_i64.to_le_bytes());           // internal_resistance (8)
    buf.extend_from_slice(&40_000_000_i64.to_le_bytes());      // ocv_at_0_pct (16)
    buf.extend_from_slice(&57_600_000_i64.to_le_bytes());      // ocv_at_100_pct (24)
    buf.extend_from_slice(&100_u16.to_le_bytes());             // correction_gain_milli (32)
    buf.extend_from_slice(&[0u8; 2]);                           // pad (34)
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes());     // (36)
    buf.extend_from_slice(&[0u8; 4]);                           // tail-pad → 40
    buf
}

fn synthesize_data_in_soc_kalman(tick: u64) -> Vec<u8> {
    // Steady 48 V terminal with mild discharge current.
    let i_drift = ((tick as i64).wrapping_mul(257) % 20_000_001) - 10_000_000;
    let i_micro_a: i64 = 50_000_000_i64.saturating_add(i_drift); // ~50 A
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&48_800_000_i64.to_le_bytes()); // voltage_micro_v (0)
    buf.extend_from_slice(&i_micro_a.to_le_bytes());       // current_micro_a (8)
    buf.extend_from_slice(&25_000_i32.to_le_bytes());      // temp_milli_c (16)
    buf.push(0); // voltage_quality (20)
    buf.push(0); // current_quality (21)
    buf.push(1); // enable (22)
    buf.extend_from_slice(&[0u8; 1]); // tail-pad → 24
    buf
}

// ---------------------------------------------------------------------------
// Memory helpers (cell-agnostic)
// ---------------------------------------------------------------------------

struct MemMap {
    params: u32,
    internal: u32,
    data_in: u32,
    data_out: u32,
    out_event: u32,
}

fn lay_out(layout: &CellLayout) -> MemMap {
    // Pad each region up to the next 16-byte boundary so neighbouring
    // writes can't collide with i64 alignment requirements.
    fn pad16(o: u32) -> u32 {
        (o + 15) & !15
    }
    let params = layout.scratch_base;
    let internal = pad16(params + layout.params_size);
    let data_in = pad16(internal + layout.internal_size);
    let data_out = pad16(data_in + layout.data_in_size);
    let out_event = pad16(data_out + layout.data_out_size);
    MemMap {
        params,
        internal,
        data_in,
        data_out,
        out_event,
    }
}

fn write_bytes(memory: &Memory, store: &mut Store<()>, offset: u32, data: &[u8]) -> Result<()> {
    memory
        .write(store, offset as usize, data)
        .with_context(|| format!("write {} bytes at 0x{:x}", data.len(), offset))?;
    Ok(())
}

fn read_u8(memory: &Memory, store: &mut Store<()>, offset: u32) -> Result<u8> {
    let mut buf = [0u8; 1];
    memory.read(store, offset as usize, &mut buf)?;
    Ok(buf[0])
}

// ---------------------------------------------------------------------------
// Histogram (unchanged)
// ---------------------------------------------------------------------------

struct Histogram {
    samples_ns: Vec<u64>,
}

impl Histogram {
    fn new(cap: usize) -> Self {
        Self {
            samples_ns: Vec::with_capacity(cap),
        }
    }
    fn record(&mut self, ns: u64) {
        self.samples_ns.push(ns);
    }
    fn summary(mut self) -> HistogramSummary {
        self.samples_ns.sort_unstable();
        let n = self.samples_ns.len();
        let pct = |p: f64| -> u64 {
            if n == 0 {
                return 0;
            }
            let idx = (((n - 1) as f64) * p / 100.0).round() as usize;
            self.samples_ns[idx]
        };
        let sum: u128 = self.samples_ns.iter().map(|&x| x as u128).sum();
        let mean = if n > 0 { (sum / n as u128) as u64 } else { 0 };
        HistogramSummary {
            n: n as u64,
            min: *self.samples_ns.first().unwrap_or(&0),
            max: *self.samples_ns.last().unwrap_or(&0),
            mean_ns: mean,
            p50_ns: pct(50.0),
            p90_ns: pct(90.0),
            p99_ns: pct(99.0),
            p99_9_ns: pct(99.9),
            p99_99_ns: pct(99.99),
        }
    }
}

struct HistogramSummary {
    n: u64,
    min: u64,
    max: u64,
    mean_ns: u64,
    p50_ns: u64,
    p90_ns: u64,
    p99_ns: u64,
    p99_9_ns: u64,
    p99_99_ns: u64,
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn default_wasm_path(cell: &str) -> PathBuf {
    PathBuf::from(format!(
        "../../cells/target/wasm32-unknown-unknown/release/{}.wasm",
        cell
    ))
}

fn default_report_path(cell_layout: &CellLayout) -> PathBuf {
    PathBuf::from("..").join(cell_layout.default_report)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let layout = select_cell(&cli.cell)?;
    let wasm_path = cli
        .wasm_path
        .clone()
        .unwrap_or_else(|| default_wasm_path(&cli.cell));
    let report_path = cli
        .report
        .clone()
        .unwrap_or_else(|| default_report_path(layout));

    eprintln!("[gate-a-rig] cell={}", cli.cell);
    eprintln!("[gate-a-rig] wasm={}", wasm_path.display());
    eprintln!("[gate-a-rig] iterations={}", cli.iterations);
    eprintln!("[gate-a-rig] cycle_period_ms={}", cli.cycle_period_ms);
    eprintln!(
        "[gate-a-rig] layout: params={} internal={} data_in={} data_out={}",
        layout.params_size, layout.internal_size, layout.data_in_size, layout.data_out_size
    );

    if !wasm_path.exists() {
        bail!(
            "wasm artefact not found at {}\n\
             Build it first:\n  cd ../../cells && cargo build --release \\\n    --no-default-features --target wasm32-unknown-unknown -p {}",
            wasm_path.display(),
            cli.cell.replace('_', "-")
        );
    }

    let engine = Engine::default();
    let module = Module::from_file(&engine, &wasm_path)
        .with_context(|| format!("load {}", wasm_path.display()))?;
    let mut store: Store<()> = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .context("Instance::new — no imports expected for wasm32-unknown-unknown cdylib")?;

    let memory: Memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("module did not export `memory` — check linker / cdylib config"))?;

    let cur_pages = memory.size(&mut store);
    eprintln!(
        "[gate-a-rig] initial memory: {} pages = {} bytes",
        cur_pages,
        cur_pages * PAGE_SIZE as u64
    );
    if cur_pages < layout.required_pages {
        let delta = layout.required_pages - cur_pages;
        memory
            .grow(&mut store, delta)
            .with_context(|| format!("memory.grow by {} pages", delta))?;
        eprintln!(
            "[gate-a-rig] grew memory to {} pages = {} bytes",
            memory.size(&mut store),
            memory.size(&mut store) * PAGE_SIZE as u64
        );
    }

    let init_name = format!("{}_init", layout.symbol);
    let tick_name = format!("{}_tick", layout.symbol);
    let init_fn: TypedFunc<(i32, i32), i32> = instance
        .get_typed_func(&mut store, &init_name)
        .with_context(|| format!("get exported function `{}`", init_name))?;
    let tick_fn: TypedFunc<(i32, i32, i32, i32, i32, i32, i32, i32, i32), i32> = instance
        .get_typed_func(&mut store, &tick_name)
        .with_context(|| format!("get exported function `{}`", tick_name))?;

    let mem = lay_out(layout);
    eprintln!(
        "[gate-a-rig] memory map: params=0x{:x} internal=0x{:x} data_in=0x{:x} data_out=0x{:x} out_event=0x{:x}",
        mem.params, mem.internal, mem.data_in, mem.data_out, mem.out_event
    );

    let params_bytes = (layout.build_params)(cli.cycle_period_ms);
    write_bytes(&memory, &mut store, mem.params, &params_bytes)?;
    write_bytes(&memory, &mut store, mem.internal, &vec![0u8; layout.internal_size as usize])?;

    let rc = init_fn
        .call(&mut store, (mem.params as i32, mem.internal as i32))
        .context("call <cell>_init")?;
    if rc != 0 {
        bail!("<cell>_init returned {}", rc);
    }

    // Tick loop.
    let mut hist = Histogram::new(cli.iterations as usize);
    let mut last_ecc: u8 = 0;
    let mut alarm_count: u64 = 0;
    let mut error_count: u64 = 0;
    let mut deadline_miss_count: u64 = 0;
    let zero_out_event = vec![0u8; layout.out_event_size as usize];

    let run_start = Instant::now();
    for tick in 0..cli.iterations {
        let data_in = (layout.synthesize_data_in)(tick);
        write_bytes(&memory, &mut store, mem.data_in, &data_in)?;
        write_bytes(&memory, &mut store, mem.out_event, &zero_out_event)?;

        let t0 = Instant::now();
        let next_ecc = tick_fn
            .call(
                &mut store,
                (
                    0i32, // event_in: REQ
                    mem.data_in as i32,
                    last_ecc as i32,
                    mem.internal as i32,
                    mem.params as i32,
                    (tick & 0xFFFF_FFFF) as i32,
                    (tick >> 32) as i32,
                    mem.data_out as i32,
                    mem.out_event as i32,
                ),
            )
            .context("call <cell>_tick")?;
        let elapsed_ns = t0.elapsed().as_nanos() as u64;
        hist.record(elapsed_ns);
        if cli.deadline_ns > 0 && elapsed_ns > cli.deadline_ns {
            deadline_miss_count += 1;
        }

        let out_event = read_u8(&memory, &mut store, mem.out_event)?;
        last_ecc = next_ecc as u8;
        // ECC code for "alarm" varies by cell; use simple "is the cell ALM
        // emitting?" check via out_event == 2 (or 3 for u16 packing low byte).
        if out_event == 2 || out_event == 3 {
            alarm_count += 1;
        }
        if out_event != 1 && out_event != 2 && out_event != 3 {
            error_count += 1;
        }
    }
    let total_elapsed = run_start.elapsed();
    let final_pages = memory.size(&mut store);
    let heap_delta_pages = final_pages.saturating_sub(layout.required_pages.max(cur_pages));

    let summary = hist.summary();

    let mut out = String::new();
    out.push_str("# Risk-1 Gate A — Wasmtime harness report\n\n");
    out.push_str(&format!("**Cell**: `{}`\n\n", layout.symbol));
    out.push_str(&format!("**Wasm artefact**: `{}`\n\n", wasm_path.display()));
    out.push_str(&format!("**Iterations**: {}\n\n", cli.iterations));
    out.push_str(&format!("**Cycle period**: {} ms\n\n", cli.cycle_period_ms));
    out.push_str(&format!(
        "**Layout**: params={} B  internal={} B  data_in={} B  data_out={} B\n\n",
        layout.params_size, layout.internal_size, layout.data_in_size, layout.data_out_size
    ));
    out.push_str(&format!("**Total wall-clock**: {:.3} s\n\n", total_elapsed.as_secs_f64()));
    out.push_str("## Tick latency (host, x86_64 / aarch64 — not embedded)\n\n");
    out.push_str("| Stat | Value (ns) |\n|---|---|\n");
    out.push_str(&format!("| n        | {} |\n", summary.n));
    out.push_str(&format!("| min      | {} |\n", summary.min));
    out.push_str(&format!("| mean     | {} |\n", summary.mean_ns));
    out.push_str(&format!("| p50      | {} |\n", summary.p50_ns));
    out.push_str(&format!("| p90      | {} |\n", summary.p90_ns));
    out.push_str(&format!("| p99      | {} |\n", summary.p99_ns));
    out.push_str(&format!("| p99.9    | {} |\n", summary.p99_9_ns));
    out.push_str(&format!("| p99.99   | {} |\n", summary.p99_99_ns));
    out.push_str(&format!("| max      | {} |\n\n", summary.max));
    out.push_str("## Counters\n\n");
    out.push_str(&format!("- ALM-emitting ticks: {}\n", alarm_count));
    out.push_str(&format!("- Unexpected `out_event`: {}\n", error_count));
    if cli.deadline_ns > 0 {
        out.push_str(&format!(
            "- Deadline ({} ns) misses: {}\n",
            cli.deadline_ns, deadline_miss_count
        ));
    } else {
        out.push_str("- Deadline enforcement: disabled (--deadline-ns 0)\n");
    }
    out.push_str(&format!(
        "- Memory pages: initial={} final={} delta={} (1 page = 64 KB)\n\n",
        cur_pages, final_pages, heap_delta_pages
    ));
    out.push_str("## Notes\n\n");
    out.push_str("- This is **host harness validation**, not Mimi WCET measurement.\n");
    out.push_str("- Real Gate A criteria (per SPEC §14.1): STM32H753 @ 480 MHz, Zephyr LTS, WAMR AOT, 1 ms cycle, 10 h continuous. **PASS**: p99.9 tick latency ≤ 200 µs, zero deadline misses, observed heap delta = 0 bytes.\n");
    out.push_str("- Host run validates: artefact load, ABI surface, struct layouts (cell ⇄ rig), determinism, report pipeline.\n");
    out.push_str("- `--deadline-ns 200_000` approximates the SPEC §14.1 budget on the host. PASS on host is necessary but not sufficient: a host p99.9 already above budget would mean the cell can't possibly meet it on Mimi.\n");

    std::fs::write(&report_path, out).with_context(|| format!("write {}", report_path.display()))?;
    eprintln!("[gate-a-rig] report written: {}", report_path.display());
    eprintln!(
        "[gate-a-rig] p50={} ns  p99={} ns  p99.9={} ns  max={} ns",
        summary.p50_ns, summary.p99_ns, summary.p99_9_ns, summary.max
    );

    if error_count > 0 {
        bail!("{} unexpected out_event values — possible ABI mismatch", error_count);
    }
    if cli.deadline_ns > 0 && deadline_miss_count > 0 {
        bail!(
            "{} of {} ticks missed the {} ns deadline",
            deadline_miss_count,
            cli.iterations,
            cli.deadline_ns
        );
    }
    Ok(())
}
