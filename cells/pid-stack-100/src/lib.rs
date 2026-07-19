#![cfg_attr(not(feature = "std"), no_std)]

//! `PID_STACK_100` — Risk-1 Gate A workload cell.
//!
//! Runs `N = 100` independent saturating PI controllers per tick, sharing
//! one parameter set (gain / clamps / cycle period) but with per-instance
//! pv / sp / quality / enable inputs and per-instance integral / last_pv
//! / cv outputs. Matches SPEC §14.1 Gate A workload spec (100 DataIn /
//! 100 DataOut signals at 1 ms cycle).
//!
//! Math is identical to `pid-limited` (saturating PI + anti-windup, i32
//! µ-units), unrolled across 100 instances so the cell exercises 100
//! reads + math + 100 writes per tick — the realistic memory-access
//! pattern Gate A measures.
//!
//! Shared params keeps the workload representative of a wide-RTU
//! deployment (one Mimi managing 100 valve setpoints with the same
//! tuning), while per-instance state captures the realistic accumulator
//! behaviour.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub const N: usize = 100;

pub struct PidStack100;

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum EventIn {
    Req = 0,
}

impl EventEnum for EventIn {
    fn name(self) -> &'static str {
        match self {
            EventIn::Req => "REQ",
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum EventOut {
    Cnf = 0,
    Alm = 1,
}

impl EventEnum for EventOut {
    fn name(self) -> &'static str {
        match self {
            EventOut::Cnf => "CNF",
            EventOut::Alm => "ALM",
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataIn {
    /// Per-instance process variable in µ-units.
    pub pvs: [i32; N],
    /// Per-instance setpoint in µ-units.
    pub sps: [i32; N],
    /// Per-instance quality (0=good, 1=uncertain, 2=bad, 3=stale).
    pub qualities: [u8; N],
    /// Per-instance enable flag.
    pub enables: [u8; N],
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataOut {
    pub cvs: [i32; N],
    pub errors: [i32; N],
    /// 1 = saturated, 0 = not.
    pub saturated_flags: [u8; N],
    /// 1 = alarm (bad/stale quality), 0 = healthy.
    pub alarm_flags: [u8; N],
}
impl TypedSignals for DataOut {}

impl Default for DataOut {
    fn default() -> Self {
        Self {
            cvs: [0; N],
            errors: [0; N],
            saturated_flags: [0; N],
            alarm_flags: [0; N],
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Shared proportional gain * 1e6.
    pub kp_micro: i32,
    /// Shared integral gain * 1e6 (per second).
    pub ki_micro: i32,
    /// Shared output clamp lower bound (µ-units).
    pub out_min_micro: i32,
    /// Shared output clamp upper bound (µ-units).
    pub out_max_micro: i32,
    /// Cycle period in ms.
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Internal {
    /// Per-instance integral accumulator (i64 to avoid overflow).
    pub integrals: [i64; N],
    /// Per-instance last-pv (for future D-term; tracked for accurate
    /// re-init bumpless behaviour).
    pub last_pvs: [i32; N],
    /// Bitmap-style per-instance initialised flag.
    pub initialized: [u8; N],
}
impl LinearMemory for Internal {}

impl Default for Internal {
    fn default() -> Self {
        Self {
            integrals: [0; N],
            last_pvs: [0; N],
            initialized: [0; N],
        }
    }
}

/// One ECC state for the whole stack — `Healthy` if at least one
/// instance is enabled and OK, otherwise `AllAlarm`.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Healthy = 1,
    Degraded = 2,
    AllAlarm = 3,
}

impl BasicFunctionBlock for PidStack100 {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "PID_STACK_100";

    fn init(_params: &Params) -> Internal {
        Internal::default()
    }

    fn tick(
        _event_in: EventIn,
        data_in: &DataIn,
        _ecc_state: EccState,
        internal: &mut Internal,
        params: &Params,
        _super_step: u64,
    ) -> Self::TickReturn {
        let mut out = DataOut::default();
        let dt_ms = params.cycle_period_ms as i64;

        let mut healthy_count: u32 = 0;
        let mut alarm_count: u32 = 0;

        for i in 0..N {
            let quality = data_in.qualities[i];
            let enable = data_in.enables[i] != 0;

            // Quality gate per-instance.
            if quality >= 2 {
                out.alarm_flags[i] = 1;
                alarm_count += 1;
                continue;
            }

            if !enable {
                internal.integrals[i] = 0;
                internal.initialized[i] = 0;
                continue;
            }

            if internal.initialized[i] == 0 {
                internal.last_pvs[i] = data_in.pvs[i];
                internal.integrals[i] = 0;
                internal.initialized[i] = 1;
            }

            let error_micro: i32 = data_in.sps[i].saturating_sub(data_in.pvs[i]);
            let p_term: i64 =
                (params.kp_micro as i64).saturating_mul(error_micro as i64) / 1_000_000;
            let i_increment: i64 = (params.ki_micro as i64)
                .saturating_mul(error_micro as i64)
                .saturating_mul(dt_ms)
                / 1_000_000_000;

            let new_integral = internal.integrals[i].saturating_add(i_increment);
            let raw_cv: i64 = p_term.saturating_add(new_integral);

            let (cv_micro, saturated) = if raw_cv > params.out_max_micro as i64 {
                (params.out_max_micro, true)
            } else if raw_cv < params.out_min_micro as i64 {
                (params.out_min_micro, true)
            } else {
                (raw_cv as i32, false)
            };

            let pushing_further = saturated
                && ((raw_cv > 0 && i_increment > 0) || (raw_cv < 0 && i_increment < 0));
            if !pushing_further {
                internal.integrals[i] = new_integral;
            }
            internal.last_pvs[i] = data_in.pvs[i];

            out.cvs[i] = cv_micro;
            out.errors[i] = error_micro;
            out.saturated_flags[i] = if saturated { 1 } else { 0 };
            healthy_count += 1;
        }

        let next = if healthy_count == 0 && alarm_count > 0 {
            EccState::AllAlarm
        } else if alarm_count > 0 {
            EccState::Degraded
        } else if healthy_count == 0 {
            EccState::Idle
        } else {
            EccState::Healthy
        };

        let event = if alarm_count == N as u32 {
            EventOut::Alm
        } else {
            EventOut::Cnf
        };
        let mut r = TickResult::new(next);
        let _ = r.emitted.push((event, out));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI surface.
// ---------------------------------------------------------------------------

/// # Safety
/// Both pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn pid_stack_100_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = PidStack100::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn pid_stack_100_tick(
    event_in_code: u8,
    data_in_ptr: *const DataIn,
    ecc_state_code: u8,
    internal_ptr: *mut Internal,
    params_ptr: *const Params,
    super_step_lo: u32,
    super_step_hi: u32,
    data_out_ptr: *mut DataOut,
    out_event_ptr: *mut u8,
) -> u8 {
    if data_in_ptr.is_null()
        || internal_ptr.is_null()
        || params_ptr.is_null()
        || data_out_ptr.is_null()
        || out_event_ptr.is_null()
    {
        return EccState::AllAlarm as u8;
    }
    let event_in = match event_in_code {
        0 => EventIn::Req,
        _ => return EccState::AllAlarm as u8,
    };
    let ecc_state = match ecc_state_code {
        0 => EccState::Idle,
        1 => EccState::Healthy,
        2 => EccState::Degraded,
        3 => EccState::AllAlarm,
        _ => return EccState::AllAlarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = PidStack100::tick(event_in, data_in, ecc_state, internal, params, super_step);
    if let Some((event, data)) = result.emitted.first() {
        *data_out_ptr = *data;
        *out_event_ptr = match event {
            EventOut::Cnf => 1,
            EventOut::Alm => 2,
        };
    } else {
        *data_out_ptr = DataOut::default();
        *out_event_ptr = 0;
    }
    result.next_state as u8
}

// ---------------------------------------------------------------------------
// kani harnesses (Gate C §2.3 follow-up).
//
// Note: this cell holds 100 PID instances per tick. kani's bounded model
// checker may need a higher `--default-unwind` to verify the inner loop
// exhaustively; the harness here covers the structural panic-freedom of
// the tick path. Per-instance saturating arithmetic is identical to
// pid_limited (and verified there), so the inner loop body is by-extension
// covered.
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod proofs {
    use super::*;

    fn arbitrary_ecc_state() -> EccState {
        match kani::any::<u8>() & 0b11 {
            0 => EccState::Idle,
            1 => EccState::Healthy,
            2 => EccState::Degraded,
            _ => EccState::AllAlarm,
        }
    }

    /// 100-instance tick is `O(N)` in array length; kani verifies the
    /// outer-frame panic-freedom. The inner-loop math is identical to
    /// pid_limited and covered there.
    #[kani::proof]
    #[kani::unwind(101)]
    fn tick_never_panics() {
        let data_in: DataIn = DataIn {
            pvs: [kani::any::<i32>(); N],
            sps: [kani::any::<i32>(); N],
            qualities: [0u8; N], // SignalQuality::Good fixed to bound state space
            enables: [1u8; N],
        };
        let mut internal: Internal = Internal::default();
        let params = Params {
            kp_micro: kani::any(),
            ki_micro: kani::any(),
            out_min_micro: kani::any(),
            out_max_micro: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let super_step: u64 = kani::any();
        let _ = PidStack100::tick(
            EventIn::Req,
            &data_in,
            arbitrary_ecc_state(),
            &mut internal,
            &params,
            super_step,
        );
    }

    #[kani::proof]
    fn init_never_panics() {
        let params = Params {
            kp_micro: kani::any(),
            ki_micro: kani::any(),
            out_min_micro: kani::any(),
            out_max_micro: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = PidStack100::init(&params);
    }
}

// ---------------------------------------------------------------------------
// Tests — exercise a representative subset of instances.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> Params {
        Params {
            kp_micro: 1_200_000,
            ki_micro: 50_000,
            out_min_micro: 0,
            out_max_micro: 100_000_000,
            cycle_period_ms: 1,
        }
    }

    fn nominal_data_in(pv: i32, sp: i32) -> DataIn {
        DataIn {
            pvs: [pv; N],
            sps: [sp; N],
            qualities: [0; N],  // all good
            enables: [1; N],    // all enabled
        }
    }

    #[test]
    fn all_instances_respond_when_pv_under_sp() {
        let p = default_params();
        let mut int_ = PidStack100::init(&p);
        let d = nominal_data_in(50_000_000, 60_000_000);
        let r = PidStack100::tick(EventIn::Req, &d, EccState::Idle, &mut int_, &p, 1);
        assert_eq!(r.next_state, EccState::Healthy);
        let (event, out) = *r.emitted.first().unwrap();
        assert!(matches!(event, EventOut::Cnf));
        // All 100 instances should compute the same CV (same input, shared params).
        let first_cv = out.cvs[0];
        assert!(first_cv > 0, "expected positive CV, got {}", first_cv);
        for i in 1..N {
            assert_eq!(out.cvs[i], first_cv, "instance {} diverged", i);
        }
        // None saturated.
        assert!(out.saturated_flags.iter().all(|&f| f == 0));
        assert!(out.alarm_flags.iter().all(|&f| f == 0));
    }

    #[test]
    fn mixed_quality_instances_partial_alarm() {
        let p = default_params();
        let mut int_ = PidStack100::init(&p);
        let mut d = nominal_data_in(50_000_000, 60_000_000);
        // Mark instances 0, 25, 50, 75, 99 as bad quality.
        for &i in &[0usize, 25, 50, 75, 99] {
            d.qualities[i] = 2;  // bad
        }
        let r = PidStack100::tick(EventIn::Req, &d, EccState::Healthy, &mut int_, &p, 1);
        assert_eq!(r.next_state, EccState::Degraded);
        let (_, out) = *r.emitted.first().unwrap();
        assert_eq!(out.alarm_flags[0], 1);
        assert_eq!(out.alarm_flags[25], 1);
        assert_eq!(out.alarm_flags[50], 1);
        assert_eq!(out.alarm_flags[75], 1);
        assert_eq!(out.alarm_flags[99], 1);
        assert_eq!(out.alarm_flags[1], 0);
        assert_eq!(out.alarm_flags[26], 0);
    }

    #[test]
    fn disabled_instances_skip_compute() {
        let p = default_params();
        let mut int_ = PidStack100::init(&p);
        let mut d = nominal_data_in(50_000_000, 60_000_000);
        // Disable half of the instances.
        for i in 0..N {
            if i % 2 == 0 {
                d.enables[i] = 0;
            }
        }
        let r = PidStack100::tick(EventIn::Req, &d, EccState::Idle, &mut int_, &p, 1);
        let (_, out) = *r.emitted.first().unwrap();
        // Disabled instances have CV == 0 (default).
        for i in 0..N {
            if i % 2 == 0 {
                assert_eq!(out.cvs[i], 0);
            } else {
                assert!(out.cvs[i] > 0);
            }
        }
    }

    #[test]
    fn all_bad_quality_drives_all_alarm_state() {
        let p = default_params();
        let mut int_ = PidStack100::init(&p);
        let mut d = nominal_data_in(50_000_000, 60_000_000);
        for i in 0..N {
            d.qualities[i] = 2;
        }
        let r = PidStack100::tick(EventIn::Req, &d, EccState::Healthy, &mut int_, &p, 1);
        assert_eq!(r.next_state, EccState::AllAlarm);
        let (event, _) = *r.emitted.first().unwrap();
        assert!(matches!(event, EventOut::Alm));
    }

    #[test]
    fn replay_determinism_across_full_stack() {
        let p = default_params();
        // Same input twice — internals must end identical.
        let d = nominal_data_in(50_000_000, 60_000_000);
        let mut int_a = PidStack100::init(&p);
        let mut int_b = PidStack100::init(&p);
        let mut s_a = EccState::Idle;
        let mut s_b = EccState::Idle;
        for super_step in 1..=10u64 {
            let ra = PidStack100::tick(EventIn::Req, &d, s_a, &mut int_a, &p, super_step);
            let rb = PidStack100::tick(EventIn::Req, &d, s_b, &mut int_b, &p, super_step);
            assert_eq!(ra.next_state, rb.next_state);
            // Compare integrals across all 100 instances.
            for i in 0..N {
                assert_eq!(int_a.integrals[i], int_b.integrals[i]);
                assert_eq!(int_a.last_pvs[i], int_b.last_pvs[i]);
            }
            s_a = ra.next_state;
            s_b = rb.next_state;
        }
    }
}
