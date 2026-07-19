#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `MPPT_PERTURB_OBSERVE` BFB — Perturb & Observe MPPT.
//!
//! Used by `:loop:pv-array-mppt` per PROTOTYPE-MICROGRID.md §13.2. 100 Hz
//! field-tier cell. Measures (V, I), computes P = V·I, perturbs the
//! voltage setpoint by ±delta. If new P > last P, keep perturbing in same
//! direction; if P decreased, flip direction.
//!
//! All math is fixed-point integer; intermediate uses i64 to keep V·I
//! product within range for typical PV ratings (V up to ~600 V, I up to
//! ~50 A → P up to ~30 kW per string, well inside i64).

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct MpptPerturbObserve;

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

#[derive(Copy, Clone, Debug, Default)]
#[repr(u8)]
pub enum PerturbDir {
    #[default]
    Up = 0,
    Down = 1,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataIn {
    /// Measured PV terminal voltage in µV.
    pub pv_voltage_micro_v: i32,
    /// Measured PV current in µA.
    pub pv_current_micro_a: i32,
    pub voltage_quality: SignalQuality,
    pub current_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    /// New commanded voltage setpoint in µV.
    pub voltage_setpoint_micro_v: i32,
    /// Computed P = V × I in pW (V in µV × I in µA = pW).
    pub power_pw: i64,
    /// Direction of the current perturbation step.
    pub direction: PerturbDir,
    /// Tracking quality flag — `true` if power converged within tolerance.
    pub mpp_reached: bool,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Step size for each perturbation, in µV. Typical 100_000 (= 0.1 V).
    pub perturb_step_micro_v: i32,
    /// Lower bound on voltage setpoint (V_min of the PV string), in µV.
    pub v_min_micro_v: i32,
    /// Upper bound on voltage setpoint (V_oc of the PV string), in µV.
    pub v_max_micro_v: i32,
    /// Power difference threshold below which we consider MPP reached, in pW.
    /// Typical 1_000_000 (= 1 mW) — small relative to ~kW PV ratings.
    pub mpp_tolerance_pw: i64,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    pub last_voltage_setpoint_micro_v: i32,
    pub last_power_pw: i64,
    pub direction: PerturbDir,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Searching = 1,
    AtMpp = 2,
    Alarm = 3,
}

impl BasicFunctionBlock for MpptPerturbObserve {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "MPPT_PERTURB_OBSERVE";

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
        // Gates.
        if matches!(
            data_in.voltage_quality,
            SignalQuality::Bad | SignalQuality::Stale
        ) || matches!(
            data_in.current_quality,
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
        if params.v_min_micro_v >= params.v_max_micro_v
            || params.perturb_step_micro_v <= 0
            || params.mpp_tolerance_pw <= 0
        {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Compute power = V × I. i64 intermediate handles realistic PV scales.
        let power_pw: i64 = (data_in.pv_voltage_micro_v as i64)
            .saturating_mul(data_in.pv_current_micro_a as i64);

        // First-tick bootstrap.
        if !internal.initialized {
            internal.last_voltage_setpoint_micro_v = data_in.pv_voltage_micro_v;
            internal.last_power_pw = power_pw;
            internal.direction = PerturbDir::Up;
            internal.initialized = true;
            let new_v = data_in
                .pv_voltage_micro_v
                .saturating_add(params.perturb_step_micro_v)
                .clamp(params.v_min_micro_v, params.v_max_micro_v);
            let mut r = TickResult::new(EccState::Searching);
            let _ = r.emitted.push((
                EventOut::Cnf,
                DataOut {
                    voltage_setpoint_micro_v: new_v,
                    power_pw,
                    direction: PerturbDir::Up,
                    mpp_reached: false,
                },
            ));
            internal.last_voltage_setpoint_micro_v = new_v;
            return r;
        }

        // P&O: compare current power vs last; if delta within tolerance → MPP.
        // If power increased → keep direction; if decreased → flip.
        let delta = power_pw.saturating_sub(internal.last_power_pw);
        let abs_delta = delta.saturating_abs();
        let mpp_reached = abs_delta <= params.mpp_tolerance_pw;

        let new_direction = if mpp_reached {
            internal.direction
        } else if delta > 0 {
            internal.direction
        } else {
            match internal.direction {
                PerturbDir::Up => PerturbDir::Down,
                PerturbDir::Down => PerturbDir::Up,
            }
        };

        let step_signed = match new_direction {
            PerturbDir::Up => params.perturb_step_micro_v,
            PerturbDir::Down => -params.perturb_step_micro_v,
        };
        let new_v = internal
            .last_voltage_setpoint_micro_v
            .saturating_add(step_signed)
            .clamp(params.v_min_micro_v, params.v_max_micro_v);

        internal.last_voltage_setpoint_micro_v = new_v;
        internal.last_power_pw = power_pw;
        internal.direction = new_direction;

        let state = if mpp_reached {
            EccState::AtMpp
        } else {
            EccState::Searching
        };

        let mut r = TickResult::new(state);
        let _ = r.emitted.push((
            EventOut::Cnf,
            DataOut {
                voltage_setpoint_micro_v: new_v,
                power_pw,
                direction: new_direction,
                mpp_reached,
            },
        ));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn mppt_perturb_observe_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = MpptPerturbObserve::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn mppt_perturb_observe_tick(
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
        1 => EccState::Searching,
        2 => EccState::AtMpp,
        3 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = MpptPerturbObserve::tick(event_in, data_in, ecc_state, internal, params, super_step);
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

    fn arbitrary_direction() -> PerturbDir {
        match kani::any::<u8>() & 1 {
            0 => PerturbDir::Up,
            _ => PerturbDir::Down,
        }
    }

    fn arbitrary_ecc_state() -> EccState {
        match kani::any::<u8>() & 0b11 {
            0 => EccState::Idle,
            1 => EccState::Searching,
            2 => EccState::AtMpp,
            _ => EccState::Alarm,
        }
    }

    #[kani::proof]
    fn tick_never_panics() {
        let data_in = DataIn {
            pv_voltage_micro_v: kani::any(),
            pv_current_micro_a: kani::any(),
            voltage_quality: arbitrary_signal_quality(),
            current_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        };
        let mut internal = Internal {
            last_voltage_setpoint_micro_v: kani::any(),
            last_power_pw: kani::any(),
            direction: arbitrary_direction(),
            initialized: kani::any(),
        };
        let params = Params {
            perturb_step_micro_v: kani::any(),
            v_min_micro_v: kani::any(),
            v_max_micro_v: kani::any(),
            mpp_tolerance_pw: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = MpptPerturbObserve::tick(
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
            perturb_step_micro_v: kani::any(),
            v_min_micro_v: kani::any(),
            v_max_micro_v: kani::any(),
            mpp_tolerance_pw: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = MpptPerturbObserve::init(&params);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> Params {
        Params {
            perturb_step_micro_v: 100_000, // 0.1 V step
            v_min_micro_v: 200_000_000,    // 200 V
            v_max_micro_v: 600_000_000,    // 600 V
            mpp_tolerance_pw: 5_000_000_000_000, // 5 W tolerance (in pW); realistic for kW-scale PV
            cycle_period_ms: 10,
        }
    }

    // Helper: compute V (µV) × I (µA) as pW to keep test expectations grounded.
    // 400 V × 25 A → V=400_000_000 µV × I=25_000_000 µA → P = 1.0e16 pW (= 10 kW).
    fn pw(v_micro: i32, i_micro: i32) -> i64 {
        (v_micro as i64) * (i_micro as i64)
    }

    fn data(v_micro: i32, i_micro: i32) -> DataIn {
        DataIn {
            pv_voltage_micro_v: v_micro,
            pv_current_micro_a: i_micro,
            voltage_quality: SignalQuality::Good,
            current_quality: SignalQuality::Good,
            enable: true,
        }
    }

    #[test]
    fn first_tick_initializes() {
        let p = default_params();
        let mut internal = Internal::default();
        // 400 V, 25 A = 10 kW
        let r = MpptPerturbObserve::tick(
            EventIn::Req,
            &data(400_000_000, 25_000_000),
            EccState::Idle,
            &mut internal,
            &p,
            0,
        );
        assert!(internal.initialized);
        // Direction starts Up.
        let (_, data) = *r.emitted.first().unwrap();
        assert!(matches!(data.direction, PerturbDir::Up));
        // Setpoint = measured + step.
        assert_eq!(data.voltage_setpoint_micro_v, 400_100_000);
    }

    #[test]
    fn increasing_power_keeps_direction() {
        let p = default_params();
        // last = 7.5 kW (300 V × 25 A scenario).
        let mut internal = Internal {
            last_voltage_setpoint_micro_v: 400_000_000,
            last_power_pw: pw(300_000_000, 25_000_000),
            direction: PerturbDir::Up,
            initialized: true,
        };
        // Now 10 kW (400 V × 25 A) → power increased → keep Up.
        let r = MpptPerturbObserve::tick(
            EventIn::Req,
            &data(400_000_000, 25_000_000),
            EccState::Searching,
            &mut internal,
            &p,
            1,
        );
        let (_, out) = *r.emitted.first().unwrap();
        assert!(matches!(out.direction, PerturbDir::Up));
        // New setpoint should move Up.
        assert!(out.voltage_setpoint_micro_v > 400_000_000);
    }

    #[test]
    fn decreasing_power_flips_direction() {
        let p = default_params();
        // last = 12 kW (400 V × 30 A).
        let mut internal = Internal {
            last_voltage_setpoint_micro_v: 400_000_000,
            last_power_pw: pw(400_000_000, 30_000_000),
            direction: PerturbDir::Up,
            initialized: true,
        };
        // Now 8 kW (400 V × 20 A) → flip to Down.
        let r = MpptPerturbObserve::tick(
            EventIn::Req,
            &data(400_000_000, 20_000_000),
            EccState::Searching,
            &mut internal,
            &p,
            1,
        );
        let (_, out) = *r.emitted.first().unwrap();
        assert!(matches!(out.direction, PerturbDir::Down));
        assert!(out.voltage_setpoint_micro_v < 400_000_000);
    }

    #[test]
    fn mpp_reached_when_within_tolerance() {
        let p = default_params(); // tolerance = 5 W = 5e12 pW
        // last = 10 kW (400 V × 25 A).
        let mut internal = Internal {
            last_voltage_setpoint_micro_v: 400_000_000,
            last_power_pw: pw(400_000_000, 25_000_000),
            direction: PerturbDir::Up,
            initialized: true,
        };
        // Now 10.0004 kW (400 V × 25.001 A) → delta = 0.4 W << 5 W tolerance.
        let r = MpptPerturbObserve::tick(
            EventIn::Req,
            &data(400_000_000, 25_001_000),
            EccState::Searching,
            &mut internal,
            &p,
            1,
        );
        assert_eq!(r.next_state, EccState::AtMpp);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.mpp_reached);
    }

    #[test]
    fn clamps_at_v_max() {
        let p = default_params();
        let mut internal = Internal {
            last_voltage_setpoint_micro_v: p.v_max_micro_v,
            last_power_pw: pw(p.v_max_micro_v, 25_000_000),
            direction: PerturbDir::Up,
            initialized: true,
        };
        let r = MpptPerturbObserve::tick(
            EventIn::Req,
            &data(p.v_max_micro_v, 25_000_000),
            EccState::Searching,
            &mut internal,
            &p,
            1,
        );
        let (_, out) = *r.emitted.first().unwrap();
        assert_eq!(out.voltage_setpoint_micro_v, p.v_max_micro_v);
    }

    #[test]
    fn bad_quality_alarms() {
        let p = default_params();
        let mut internal = Internal::default();
        let mut d = data(400_000_000, 25_000_000);
        d.voltage_quality = SignalQuality::Bad;
        let r = MpptPerturbObserve::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &p, 0);
        assert_eq!(r.next_state, EccState::Alarm);
    }

    #[test]
    fn disabled_returns_idle() {
        let p = default_params();
        let mut internal = Internal {
            initialized: true,
            ..Default::default()
        };
        let mut d = data(400_000_000, 25_000_000);
        d.enable = false;
        let r = MpptPerturbObserve::tick(EventIn::Req, &d, EccState::AtMpp, &mut internal, &p, 0);
        assert_eq!(r.next_state, EccState::Idle);
        assert!(!internal.initialized);
    }
}
