#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `VV_CURVE` BFB — Volt-VAR piecewise-linear curve.
//!
//! Used by `:loop:volt-var` per PROTOTYPE-MICROGRID.md §13.2. Maps measured
//! voltage (per-unit) to reactive-power setpoint (Q in µVAR) via a 5-point
//! piecewise linear curve. The IEEE 1547 / IEC 62116 default curve maps:
//!
//! ```text
//!   1.06 pu → -Q_max (absorb reactive, fully)
//!   1.03 pu →   0
//!   0.97-1.03 pu →   0  (dead band)
//!   0.94 pu →   0
//!   0.90 pu → +Q_max (inject reactive, fully)
//! ```
//!
//! Curve breakpoints are configurable via `Params` to support utility-
//! specific tuning. All math is fixed-point integer (per cells/CLAUDE.md
//! no-float rule).

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct VvCurve;

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
    /// Measured terminal voltage in micro-per-unit (1_000_000 == 1.0 pu).
    pub voltage_micro_pu: i32,
    /// Asset rated reactive-power magnitude in µVAR (positive).
    pub q_max_micro_var: i32,
    pub voltage_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    /// Commanded reactive-power setpoint in µVAR.
    pub q_setpoint_micro_var: i32,
    /// `voltage_micro_pu - 1_000_000` (signed deviation, helpful for telemetry).
    pub voltage_deviation_micro_pu: i32,
    pub in_dead_band: bool,
    pub saturated: bool,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Curve breakpoint: voltage above this → start absorbing Q linearly.
    /// IEEE 1547 default 1_030_000 (= 1.03 pu).
    pub v_dead_high_micro_pu: i32,
    /// Curve breakpoint: voltage above this → fully absorbing (-Q_max).
    /// IEEE 1547 default 1_060_000 (= 1.06 pu).
    pub v_full_high_micro_pu: i32,
    /// Curve breakpoint: voltage below this → start injecting Q linearly.
    /// IEEE 1547 default 970_000 (= 0.97 pu).
    pub v_dead_low_micro_pu: i32,
    /// Curve breakpoint: voltage below this → fully injecting (+Q_max).
    /// IEEE 1547 default 900_000 (= 0.90 pu).
    pub v_full_low_micro_pu: i32,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    pub last_setpoint_micro_var: i32,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    InDeadBand = 1,
    Absorbing = 2,
    Injecting = 3,
    Saturated = 4,
    Alarm = 5,
}

impl BasicFunctionBlock for VvCurve {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "VV_CURVE";

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
        // Quality / mode gates.
        if matches!(
            data_in.voltage_quality,
            SignalQuality::Bad | SignalQuality::Stale
        ) {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }
        if !data_in.enable {
            internal.initialized = false;
            return TickResult::new(EccState::Idle);
        }

        // Param sanity: curve must be monotonic, q_max > 0.
        let p = params;
        if data_in.q_max_micro_var <= 0
            || p.v_full_low_micro_pu >= p.v_dead_low_micro_pu
            || p.v_dead_low_micro_pu >= p.v_dead_high_micro_pu
            || p.v_dead_high_micro_pu >= p.v_full_high_micro_pu
        {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        let v = data_in.voltage_micro_pu;
        let q_max = data_in.q_max_micro_var;
        let nominal_pu: i32 = 1_000_000;

        // Compute curve output. i64 intermediate to keep slope math exact.
        let (q_setpoint, state, in_dead, saturated) = if v >= p.v_full_high_micro_pu {
            (-q_max, EccState::Saturated, false, true)
        } else if v >= p.v_dead_high_micro_pu {
            // Linear from 0 at v_dead_high → -q_max at v_full_high.
            let span = (p.v_full_high_micro_pu - p.v_dead_high_micro_pu) as i64;
            let off = (v - p.v_dead_high_micro_pu) as i64;
            let q = -((q_max as i64).saturating_mul(off) / span) as i32;
            (q, EccState::Absorbing, false, false)
        } else if v > p.v_dead_low_micro_pu {
            (0, EccState::InDeadBand, true, false)
        } else if v > p.v_full_low_micro_pu {
            // Linear from 0 at v_dead_low → +q_max at v_full_low.
            let span = (p.v_dead_low_micro_pu - p.v_full_low_micro_pu) as i64;
            let off = (p.v_dead_low_micro_pu - v) as i64;
            let q = ((q_max as i64).saturating_mul(off) / span) as i32;
            (q, EccState::Injecting, false, false)
        } else {
            (q_max, EccState::Saturated, false, true)
        };

        internal.last_setpoint_micro_var = q_setpoint;
        internal.initialized = true;

        let mut r = TickResult::new(state);
        let _ = r.emitted.push((
            EventOut::Cnf,
            DataOut {
                q_setpoint_micro_var: q_setpoint,
                voltage_deviation_micro_pu: v.saturating_sub(nominal_pu),
                in_dead_band: in_dead,
                saturated,
            },
        ));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI wrappers
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn vv_curve_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = VvCurve::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn vv_curve_tick(
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
        1 => EccState::InDeadBand,
        2 => EccState::Absorbing,
        3 => EccState::Injecting,
        4 => EccState::Saturated,
        5 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = VvCurve::tick(event_in, data_in, ecc_state, internal, params, super_step);
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
// kani harnesses
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
        match kani::any::<u8>() % 6 {
            0 => EccState::Idle,
            1 => EccState::InDeadBand,
            2 => EccState::Absorbing,
            3 => EccState::Injecting,
            4 => EccState::Saturated,
            _ => EccState::Alarm,
        }
    }

    #[kani::proof]
    fn tick_never_panics() {
        let data_in = DataIn {
            voltage_micro_pu: kani::any(),
            q_max_micro_var: kani::any(),
            voltage_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        };
        let mut internal = Internal {
            last_setpoint_micro_var: kani::any(),
            initialized: kani::any(),
        };
        let params = Params {
            v_dead_high_micro_pu: kani::any(),
            v_full_high_micro_pu: kani::any(),
            v_dead_low_micro_pu: kani::any(),
            v_full_low_micro_pu: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = VvCurve::tick(
            EventIn::Req,
            &data_in,
            arbitrary_ecc_state(),
            &mut internal,
            &params,
            kani::any(),
        );
    }

    #[kani::proof]
    fn init_never_panics() {
        let params = Params {
            v_dead_high_micro_pu: kani::any(),
            v_full_high_micro_pu: kani::any(),
            v_dead_low_micro_pu: kani::any(),
            v_full_low_micro_pu: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = VvCurve::init(&params);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ieee_1547_params() -> Params {
        Params {
            v_dead_high_micro_pu: 1_030_000, // 1.03 pu
            v_full_high_micro_pu: 1_060_000, // 1.06 pu
            v_dead_low_micro_pu: 970_000,    // 0.97 pu
            v_full_low_micro_pu: 900_000,    // 0.90 pu
            cycle_period_ms: 100,
        }
    }

    fn data_at(v_micro_pu: i32) -> DataIn {
        DataIn {
            voltage_micro_pu: v_micro_pu,
            q_max_micro_var: 100_000_000, // 100 kVAR
            voltage_quality: SignalQuality::Good,
            enable: true,
        }
    }

    fn run_one(v_micro_pu: i32) -> (EccState, DataOut) {
        let p = ieee_1547_params();
        let mut internal = Internal::default();
        let r = VvCurve::tick(
            EventIn::Req,
            &data_at(v_micro_pu),
            EccState::Idle,
            &mut internal,
            &p,
            0,
        );
        let (_, data) = *r.emitted.first().expect("emitted at least one event");
        (r.next_state, data)
    }

    #[test]
    fn nominal_voltage_is_in_dead_band() {
        let (state, data) = run_one(1_000_000); // 1.0 pu
        assert_eq!(state, EccState::InDeadBand);
        assert_eq!(data.q_setpoint_micro_var, 0);
        assert!(data.in_dead_band);
    }

    #[test]
    fn over_voltage_absorbs() {
        let (state, data) = run_one(1_045_000); // half-way through absorb ramp
        assert_eq!(state, EccState::Absorbing);
        assert!(data.q_setpoint_micro_var < 0);
        assert!(!data.saturated);
    }

    #[test]
    fn over_voltage_saturates() {
        let (state, data) = run_one(1_100_000); // above v_full_high
        assert_eq!(state, EccState::Saturated);
        assert_eq!(data.q_setpoint_micro_var, -100_000_000);
        assert!(data.saturated);
    }

    #[test]
    fn under_voltage_injects() {
        let (state, data) = run_one(935_000); // half-way through inject ramp
        assert_eq!(state, EccState::Injecting);
        assert!(data.q_setpoint_micro_var > 0);
        assert!(!data.saturated);
    }

    #[test]
    fn under_voltage_saturates() {
        let (state, data) = run_one(800_000); // below v_full_low
        assert_eq!(state, EccState::Saturated);
        assert_eq!(data.q_setpoint_micro_var, 100_000_000);
        assert!(data.saturated);
    }

    #[test]
    fn bad_quality_alarms() {
        let mut d = data_at(1_000_000);
        d.voltage_quality = SignalQuality::Bad;
        let p = ieee_1547_params();
        let mut internal = Internal::default();
        let r = VvCurve::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &p, 0);
        assert_eq!(r.next_state, EccState::Alarm);
    }

    #[test]
    fn disabled_goes_idle() {
        let mut d = data_at(1_100_000);
        d.enable = false;
        let p = ieee_1547_params();
        let mut internal = Internal::default();
        let r = VvCurve::tick(EventIn::Req, &d, EccState::Saturated, &mut internal, &p, 0);
        assert_eq!(r.next_state, EccState::Idle);
    }

    #[test]
    fn non_monotonic_curve_alarms() {
        // v_dead_high < v_dead_low → invalid configuration
        let p = Params {
            v_dead_high_micro_pu: 900_000,
            v_full_high_micro_pu: 1_060_000,
            v_dead_low_micro_pu: 970_000,
            v_full_low_micro_pu: 900_000,
            cycle_period_ms: 100,
        };
        let mut internal = Internal::default();
        let r = VvCurve::tick(
            EventIn::Req,
            &data_at(1_000_000),
            EccState::Idle,
            &mut internal,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Alarm);
    }
}
