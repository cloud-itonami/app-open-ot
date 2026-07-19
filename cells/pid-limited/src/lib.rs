#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `PID_LIMITED` BFB — saturating PI controller with anti-windup.
//!
//! Reference cell for ADR-2605151200 §R4 Risk-1 Gate A. Math is fixed-point
//! `i32` in micro-units (1e-6) per signal `unitCode`; intermediates are `i64`.
//! No floats anywhere in the tick path so WCET is bounded by integer ALU
//! latency on Cortex-M7.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct PidLimited;

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
#[repr(u8)]
pub enum SignalQuality {
    Good = 0,
    Uncertain = 1,
    Bad = 2,
    Stale = 3,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataIn {
    /// Process variable in micro-units of the bound signal's `unitCode`.
    pub pv_micro: i32,
    /// Setpoint in micro-units.
    pub sp_micro: i32,
    pub pv_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    pub cv_micro: i32,
    pub error_micro: i32,
    pub saturated: bool,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Proportional gain in micro-units (kp_micro = kp * 1e6).
    pub kp_micro: i32,
    /// Integral gain in micro-units per second (ki_micro = ki * 1e6).
    pub ki_micro: i32,
    /// Output lower clamp in micro-units of CV unitCode.
    pub out_min_micro: i32,
    /// Output upper clamp in micro-units of CV unitCode.
    pub out_max_micro: i32,
    /// Cycle period in milliseconds.
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    /// Accumulated integral term in micro-units (i64 to avoid overflow).
    pub integral_micro: i64,
    pub last_pv_micro: i32,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Running = 1,
    Saturated = 2,
    Alarm = 3,
}

impl BasicFunctionBlock for PidLimited {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "PID_LIMITED";

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
        // Quality gate: bad / stale → ALM, integral preserved.
        if matches!(data_in.pv_quality, SignalQuality::Bad | SignalQuality::Stale) {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Mode gate: disabled → Idle, integral cleared.
        if !data_in.enable {
            internal.integral_micro = 0;
            internal.initialized = false;
            return TickResult::new(EccState::Idle);
        }

        // Bumpless start: prime last_pv on first run after enable.
        if !internal.initialized {
            internal.last_pv_micro = data_in.pv_micro;
            internal.integral_micro = 0;
            internal.initialized = true;
        }

        let error_micro: i32 = data_in.sp_micro.saturating_sub(data_in.pv_micro);

        // P term in micro-units: (kp_micro * error_micro) / 1e6.
        let p_term_micro: i64 =
            (params.kp_micro as i64).saturating_mul(error_micro as i64) / 1_000_000;

        // I increment in CV-micro units over one cycle.
        // ki_micro = ki * 1e6 [CV/(EU*s)]; error_micro = error * 1e6 [EU];
        // dt_ms in ms. CV-micro increment = ki * error * dt[s] * 1e6
        //   = (ki_micro * error_micro * dt_ms) / 1e9.
        let dt_ms = params.cycle_period_ms as i64;
        let i_increment_micro: i64 = (params.ki_micro as i64)
            .saturating_mul(error_micro as i64)
            .saturating_mul(dt_ms)
            / 1_000_000_000;

        let new_integral: i64 = internal.integral_micro.saturating_add(i_increment_micro);

        let raw_cv: i64 = p_term_micro.saturating_add(new_integral);

        // Saturate to [out_min, out_max].
        let (cv_micro, saturated) = if raw_cv > params.out_max_micro as i64 {
            (params.out_max_micro, true)
        } else if raw_cv < params.out_min_micro as i64 {
            (params.out_min_micro, true)
        } else {
            (raw_cv as i32, false)
        };

        // Anti-windup: only commit integral if not pushing further into saturation.
        let pushing_further = saturated
            && ((raw_cv > 0 && i_increment_micro > 0)
                || (raw_cv < 0 && i_increment_micro < 0));
        if !pushing_further {
            internal.integral_micro = new_integral;
        }
        internal.last_pv_micro = data_in.pv_micro;

        let next = if saturated {
            EccState::Saturated
        } else {
            EccState::Running
        };

        let mut r = TickResult::new(next);
        let _ = r.emitted.push((
            EventOut::Cnf,
            DataOut {
                cv_micro,
                error_micro,
                saturated,
            },
        ));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI surface — what WAMR / Wasmtime call directly.
// All pointers reference memory owned by the host; the cell never allocates.
// ---------------------------------------------------------------------------

/// Initialise cell state. Returns 0 on success.
///
/// # Safety
/// `params_ptr` must point to a readable `Params`, `internal_ptr` to a writable
/// `Internal`. Both pointers must be aligned and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn pid_limited_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    let params = &*params_ptr;
    *internal_ptr = PidLimited::init(params);
    0
}

/// One IEC 61499 tick == one Pregel super-step contribution.
///
/// Returns next ECC state code (`EccState as u8`). Output `DataOut` is written
/// to `data_out_ptr`; `out_event_ptr` receives 0 = none, 1 = CNF, 2 = ALM.
///
/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn pid_limited_tick(
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
        return EccState::Alarm as u8;
    }

    let event_in = match event_in_code {
        0 => EventIn::Req,
        _ => return EccState::Alarm as u8,
    };
    let ecc_state = match ecc_state_code {
        0 => EccState::Idle,
        1 => EccState::Running,
        2 => EccState::Saturated,
        3 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);

    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;

    let result = PidLimited::tick(event_in, data_in, ecc_state, internal, params, super_step);

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
// kani harnesses (Gate C §2.3 follow-up — symbolic verification of the
// tick path's panic / UB freedom under arbitrary inputs).
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod proofs {
    use super::*;

    fn arbitrary_signal_quality() -> SignalQuality {
        match kani::any::<u8>() & 0b11 {
            0 => SignalQuality::Good,
            1 => SignalQuality::Uncertain,
            2 => SignalQuality::Bad,
            _ => SignalQuality::Stale,
        }
    }

    fn arbitrary_ecc_state() -> EccState {
        match kani::any::<u8>() & 0b11 {
            0 => EccState::Idle,
            1 => EccState::Running,
            2 => EccState::Saturated,
            _ => EccState::Alarm,
        }
    }

    fn arbitrary_data_in() -> DataIn {
        DataIn {
            pv_micro: kani::any(),
            sp_micro: kani::any(),
            pv_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        }
    }

    fn arbitrary_internal() -> Internal {
        Internal {
            integral_micro: kani::any(),
            last_pv_micro: kani::any(),
            initialized: kani::any(),
        }
    }

    fn arbitrary_params() -> Params {
        Params {
            kp_micro: kani::any(),
            ki_micro: kani::any(),
            out_min_micro: kani::any(),
            out_max_micro: kani::any(),
            cycle_period_ms: kani::any(),
        }
    }

    /// Verifies `tick` never panics under arbitrary input. Per SPEC §3 and
    /// docs/openot-bfb-rs-memory-safety.md, the tick path is total: every
    /// branch must terminate without `unreachable!` / `panic!` / arithmetic
    /// overflow. The cell uses saturating arithmetic throughout, so this
    /// proof should succeed without `kani::assume`-style preconditions.
    #[kani::proof]
    fn tick_never_panics() {
        let data_in = arbitrary_data_in();
        let mut internal = arbitrary_internal();
        let params = arbitrary_params();
        let super_step: u64 = kani::any();
        let _ = PidLimited::tick(
            EventIn::Req,
            &data_in,
            arbitrary_ecc_state(),
            &mut internal,
            &params,
            super_step,
        );
    }

    /// Verifies `init` is a total function.
    #[kani::proof]
    fn init_never_panics() {
        let _ = PidLimited::init(&arbitrary_params());
    }
}

// ---------------------------------------------------------------------------
// Tests (host-side, std).
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> Params {
        Params {
            kp_micro: 1_200_000, // kp = 1.2
            ki_micro: 50_000,    // ki = 0.05
            out_min_micro: 0,
            out_max_micro: 100_000_000, // 100.0
            cycle_period_ms: 100,
        }
    }

    #[test]
    fn quality_bad_emits_alarm() {
        let params = default_params();
        let mut internal = PidLimited::init(&params);
        let data_in = DataIn {
            pv_micro: 50_000_000,
            sp_micro: 60_000_000,
            pv_quality: SignalQuality::Bad,
            enable: true,
        };
        let r = PidLimited::tick(EventIn::Req, &data_in, EccState::Running, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Alarm);
        assert!(matches!(r.emitted.first().unwrap().0, EventOut::Alm));
    }

    #[test]
    fn disabled_returns_idle_clears_integral() {
        let params = default_params();
        let mut internal = PidLimited::init(&params);
        internal.integral_micro = 12_345;
        let data_in = DataIn {
            pv_micro: 50_000_000,
            sp_micro: 60_000_000,
            pv_quality: SignalQuality::Good,
            enable: false,
        };
        let r = PidLimited::tick(EventIn::Req, &data_in, EccState::Running, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Idle);
        assert_eq!(internal.integral_micro, 0);
        assert!(!internal.initialized);
    }

    #[test]
    fn positive_error_drives_cv_up() {
        let params = default_params();
        let mut internal = PidLimited::init(&params);
        let data_in = DataIn {
            pv_micro: 50_000_000,
            sp_micro: 60_000_000, // error = +10.0
            pv_quality: SignalQuality::Good,
            enable: true,
        };
        let r = PidLimited::tick(EventIn::Req, &data_in, EccState::Idle, &mut internal, &params, 1);
        let (event, out) = *r.emitted.first().unwrap();
        assert!(matches!(event, EventOut::Cnf));
        assert!(out.cv_micro > 0);
        assert!(!out.saturated);
        assert_eq!(r.next_state, EccState::Running);
    }

    #[test]
    fn saturation_clamps_and_anti_windup_holds_integral() {
        let mut params = default_params();
        params.out_max_micro = 1_000_000; // 1.0 — easy to saturate
        let mut internal = PidLimited::init(&params);
        let data_in = DataIn {
            pv_micro: 0,
            sp_micro: 100_000_000, // huge positive error
            pv_quality: SignalQuality::Good,
            enable: true,
        };
        // First tick saturates.
        let r1 = PidLimited::tick(EventIn::Req, &data_in, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r1.next_state, EccState::Saturated);
        let saturated_integral = internal.integral_micro;
        // Second tick same input — anti-windup should hold integral steady.
        let _ = PidLimited::tick(EventIn::Req, &data_in, r1.next_state, &mut internal, &params, 2);
        assert_eq!(internal.integral_micro, saturated_integral);
    }

    #[test]
    fn replay_determinism() {
        // Determinism contract per SPEC §4.2.
        let params = default_params();
        let inputs = [
            (50_000_000i32, 55_000_000i32),
            (52_000_000, 55_000_000),
            (54_500_000, 55_000_000),
            (54_900_000, 55_000_000),
        ];
        let mut internal_a = PidLimited::init(&params);
        let mut internal_b = PidLimited::init(&params);
        let mut state_a = EccState::Idle;
        let mut state_b = EccState::Idle;
        for (i, (pv, sp)) in inputs.iter().enumerate() {
            let data_in = DataIn {
                pv_micro: *pv,
                sp_micro: *sp,
                pv_quality: SignalQuality::Good,
                enable: true,
            };
            let ra = PidLimited::tick(EventIn::Req, &data_in, state_a, &mut internal_a, &params, i as u64);
            let rb = PidLimited::tick(EventIn::Req, &data_in, state_b, &mut internal_b, &params, i as u64);
            assert_eq!(ra.next_state, rb.next_state);
            assert_eq!(internal_a.integral_micro, internal_b.integral_micro);
            assert_eq!(internal_a.last_pv_micro, internal_b.last_pv_micro);
            let (_, oa) = *ra.emitted.first().unwrap();
            let (_, ob) = *rb.emitted.first().unwrap();
            assert_eq!(oa.cv_micro, ob.cv_micro);
            state_a = ra.next_state;
            state_b = rb.next_state;
        }
    }
}
