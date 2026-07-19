#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `SOC_KALMAN` BFB — battery State-of-Charge estimator.
//!
//! Used by `:loop:bess-charge-discharge` per PROTOTYPE-MICROGRID.md §13.2.
//! Implements a **simplified Kalman-style estimator**:
//!
//!   1. **Predict**: Coulomb-counter integrates `i` over `dt` to update SOC.
//!   2. **Correct**: lookup expected OCV (open-circuit voltage) for the
//!      current SOC estimate via a piecewise-linear OCV-SOC curve, compare
//!      with measured voltage adjusted for IR drop, and update the SOC
//!      estimate by a weighted blend (the Kalman gain analogue).
//!
//! A full extended Kalman filter on Cortex-M7 requires matrix inversion,
//! which is impractical without floats. The simplified blend approximates
//! the same effect: the `correction_gain_milli` parameter is the static
//! "Kalman gain" (0..1000 representing 0..1 weight on the OCV correction).
//! For most BESS deployments static gain is adequate; adaptive gain is
//! deferred to MVP+1.
//!
//! All math is fixed-point integer.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct SocKalman;

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
    /// Battery terminal voltage in µV (positive).
    pub voltage_micro_v: i64,
    /// Battery current in µA. Positive = discharging, negative = charging.
    pub current_micro_a: i64,
    /// Pack temperature in milli-°C (25_000 == 25.0 °C).
    pub temp_milli_c: i32,
    pub voltage_quality: SignalQuality,
    pub current_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    /// State of charge in milli-percent (50_000 == 50.000 %).
    pub soc_milli_pct: i32,
    /// Estimated OCV for the SOC, in µV.
    pub ocv_estimated_micro_v: i64,
    /// Coulomb-counter delta this tick, in micro-coulombs (µAs = i64).
    pub coulomb_delta_micro_c: i64,
    /// Confidence score 0..1000 (1000 = high confidence).
    pub confidence_milli: u16,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Battery capacity in micro-coulombs (`Ah × 3600 × 1e6`). For a 100 Ah
    /// pack: 360_000_000_000 µC.
    pub capacity_micro_c: i64,
    /// Internal resistance in micro-ohms (1_000_000 = 1.0 mΩ).
    pub internal_resistance_micro_ohm: i64,
    /// OCV at 0 % SOC, in µV.
    pub ocv_at_0_pct_micro_v: i64,
    /// OCV at 100 % SOC, in µV.
    pub ocv_at_100_pct_micro_v: i64,
    /// Kalman-gain analogue, 0..1000 (= 0 .. 1.0 weight on OCV correction).
    /// Higher values trust the OCV measurement more; lower values trust the
    /// Coulomb-counter prediction more. Typical 100 (= 0.1).
    pub correction_gain_milli: u16,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    /// Current SOC estimate in milli-percent.
    pub soc_milli_pct: i32,
    /// Accumulated coulombs since last reset, in µC.
    pub coulomb_accumulator_micro_c: i64,
    /// Confidence accumulator: increases over time, drops on Alarm.
    pub confidence_milli: u16,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Tracking = 1,
    LowConfidence = 2,
    Saturated = 3,
    Alarm = 4,
}

/// Piecewise-linear OCV-SOC: at SOC `soc_milli_pct`, returns expected OCV
/// in µV via linear interpolation from `(0%, ocv_at_0)` to `(100%, ocv_at_100)`.
fn ocv_for_soc(soc_milli_pct: i32, p: &Params) -> i64 {
    // Clamp SOC into [0, 100_000].
    let soc = soc_milli_pct.clamp(0, 100_000) as i64;
    let span = p.ocv_at_100_pct_micro_v.saturating_sub(p.ocv_at_0_pct_micro_v);
    let off = span.saturating_mul(soc) / 100_000;
    p.ocv_at_0_pct_micro_v.saturating_add(off)
}

/// Inverse: given a measured OCV, return the SOC in milli-percent.
fn soc_for_ocv(ocv_micro_v: i64, p: &Params) -> i32 {
    let span = p.ocv_at_100_pct_micro_v.saturating_sub(p.ocv_at_0_pct_micro_v);
    if span <= 0 {
        return 0;
    }
    let off = ocv_micro_v.saturating_sub(p.ocv_at_0_pct_micro_v);
    let scaled = off.saturating_mul(100_000) / span;
    scaled.clamp(0, 100_000) as i32
}

impl BasicFunctionBlock for SocKalman {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "SOC_KALMAN";

    fn init(_params: &Params) -> Internal {
        Internal {
            soc_milli_pct: 50_000, // bootstrap at 50 % until first OCV reading
            confidence_milli: 0,
            ..Default::default()
        }
    }

    fn tick(
        _event_in: EventIn,
        data_in: &DataIn,
        _ecc_state: EccState,
        internal: &mut Internal,
        params: &Params,
        _super_step: u64,
    ) -> Self::TickReturn {
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
            internal.confidence_milli = 0;
            return TickResult::new(EccState::Idle);
        }
        if params.capacity_micro_c <= 0
            || params.ocv_at_0_pct_micro_v >= params.ocv_at_100_pct_micro_v
            || params.correction_gain_milli > 1_000
            || params.cycle_period_ms == 0
        {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        if !internal.initialized {
            // Cold-start: trust OCV completely on first tick.
            let v_ocv = data_in
                .voltage_micro_v
                .saturating_sub(data_in.current_micro_a
                    .saturating_mul(params.internal_resistance_micro_ohm) / 1_000_000);
            internal.soc_milli_pct = soc_for_ocv(v_ocv, params);
            internal.confidence_milli = 500;
            internal.coulomb_accumulator_micro_c = 0;
            internal.initialized = true;
        }

        // --- Predict step: Coulomb counter ---
        //
        // Δq (µC) = i (µA) × dt (s) = current_micro_a × cycle_period_ms / 1000
        let dt_ms = params.cycle_period_ms as i64;
        // Positive current = discharging → SOC decreases.
        let coulomb_delta_micro_c: i64 =
            data_in.current_micro_a.saturating_mul(dt_ms) / 1000;
        // Δsoc_milli_pct = -Δq × 100_000 / capacity (milli-percent).
        let soc_change_milli: i32 = (-coulomb_delta_micro_c)
            .saturating_mul(100_000)
            .saturating_div(params.capacity_micro_c)
            .clamp(i32::MIN as i64, i32::MAX as i64) as i32;
        let predicted_soc = internal.soc_milli_pct.saturating_add(soc_change_milli);
        internal.coulomb_accumulator_micro_c = internal
            .coulomb_accumulator_micro_c
            .saturating_add(coulomb_delta_micro_c);

        // --- Correct step: OCV-SOC lookup ---
        //
        // Subtract IR drop: V_ocv ≈ V_meas - I × R
        let ir_drop_micro_v: i64 = data_in
            .current_micro_a
            .saturating_mul(params.internal_resistance_micro_ohm)
            / 1_000_000;
        let v_ocv = data_in.voltage_micro_v.saturating_sub(ir_drop_micro_v);
        let ocv_soc_observed = soc_for_ocv(v_ocv, params);

        // Blend: SOC = (1 - K) × predicted + K × observed
        // (using milli-percent throughout; gain 0..1000)
        let k = params.correction_gain_milli as i64;
        let one_minus_k = 1000_i64 - k;
        let blended: i64 = (predicted_soc as i64).saturating_mul(one_minus_k)
            + (ocv_soc_observed as i64).saturating_mul(k);
        let new_soc = (blended / 1000).clamp(0, 100_000) as i32;
        internal.soc_milli_pct = new_soc;

        // Confidence: increases by 10/tick up to 1000.
        let new_conf = internal.confidence_milli.saturating_add(10).min(1000);
        internal.confidence_milli = new_conf;

        let ocv_estimated = ocv_for_soc(new_soc, params);
        let state = if new_conf < 200 {
            EccState::LowConfidence
        } else if new_soc <= 5_000 || new_soc >= 95_000 {
            EccState::Saturated
        } else {
            EccState::Tracking
        };

        let mut r = TickResult::new(state);
        let _ = r.emitted.push((
            EventOut::Cnf,
            DataOut {
                soc_milli_pct: new_soc,
                ocv_estimated_micro_v: ocv_estimated,
                coulomb_delta_micro_c,
                confidence_milli: new_conf,
            },
        ));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn soc_kalman_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = SocKalman::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn soc_kalman_tick(
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
        1 => EccState::Tracking,
        2 => EccState::LowConfidence,
        3 => EccState::Saturated,
        4 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = SocKalman::tick(event_in, data_in, ecc_state, internal, params, super_step);
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
        match kani::any::<u8>() % 5 {
            0 => EccState::Idle,
            1 => EccState::Tracking,
            2 => EccState::LowConfidence,
            3 => EccState::Saturated,
            _ => EccState::Alarm,
        }
    }

    #[kani::proof]
    fn tick_never_panics() {
        let data_in = DataIn {
            voltage_micro_v: kani::any(),
            current_micro_a: kani::any(),
            temp_milli_c: kani::any(),
            voltage_quality: arbitrary_signal_quality(),
            current_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        };
        let mut internal = Internal {
            soc_milli_pct: kani::any(),
            coulomb_accumulator_micro_c: kani::any(),
            confidence_milli: kani::any(),
            initialized: kani::any(),
        };
        let params = Params {
            capacity_micro_c: kani::any(),
            internal_resistance_micro_ohm: kani::any(),
            ocv_at_0_pct_micro_v: kani::any(),
            ocv_at_100_pct_micro_v: kani::any(),
            correction_gain_milli: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = SocKalman::tick(
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
            capacity_micro_c: kani::any(),
            internal_resistance_micro_ohm: kani::any(),
            ocv_at_0_pct_micro_v: kani::any(),
            ocv_at_100_pct_micro_v: kani::any(),
            correction_gain_milli: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = SocKalman::init(&params);
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
            // 100 Ah pack = 100 × 3600 × 1e6 = 3.6e11 µC.
            capacity_micro_c: 360_000_000_000,
            // 1 mΩ internal resistance.
            internal_resistance_micro_ohm: 1_000,
            // LFP pack: 2.5 V @ 0 %, 3.6 V @ 100 % per cell × 16 cells = 40 V → 57.6 V.
            ocv_at_0_pct_micro_v: 40_000_000,
            ocv_at_100_pct_micro_v: 57_600_000,
            correction_gain_milli: 100, // 10 % blend
            cycle_period_ms: 1_000,
        }
    }

    fn data(v: i64, i: i64) -> DataIn {
        DataIn {
            voltage_micro_v: v,
            current_micro_a: i,
            temp_milli_c: 25_000,
            voltage_quality: SignalQuality::Good,
            current_quality: SignalQuality::Good,
            enable: true,
        }
    }

    #[test]
    fn cold_start_bootstraps_soc_from_ocv() {
        let p = default_params();
        let mut i = SocKalman::init(&p);
        // Mid-pack voltage at 50 % SOC: (40 + 57.6) / 2 = 48.8 V.
        // Idle (i=0) so V == OCV.
        let r = SocKalman::tick(EventIn::Req, &data(48_800_000, 0), EccState::Idle, &mut i, &p, 0);
        let (_, out) = *r.emitted.first().unwrap();
        // After cold-start, SOC ≈ 50 %.
        assert!((out.soc_milli_pct - 50_000).abs() < 1_000);
        assert!(i.initialized);
    }

    #[test]
    fn discharge_decreases_soc() {
        let p = default_params();
        let mut i = Internal {
            soc_milli_pct: 50_000,
            confidence_milli: 1000,
            initialized: true,
            ..Default::default()
        };
        // 100 A discharge at 48 V × 1 s → -100 A × 1 s = -100 As = -360 µAh.
        // Δsoc = -(100 × 1_000_000 × 1000 / 1000) / 360_000_000_000 × 100_000
        //      = -100_000_000 µC × 100_000 / 360e9 ≈ -27.8 milli-pct
        let r = SocKalman::tick(
            EventIn::Req,
            &data(48_800_000, 100_000_000), // 100 A discharge
            EccState::Tracking,
            &mut i,
            &p,
            1,
        );
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.soc_milli_pct < 50_000);
    }

    #[test]
    fn charge_increases_soc() {
        let p = default_params();
        let mut i = Internal {
            soc_milli_pct: 50_000,
            confidence_milli: 1000,
            initialized: true,
            ..Default::default()
        };
        // Negative current = charging.
        let r = SocKalman::tick(
            EventIn::Req,
            &data(48_800_000, -100_000_000),
            EccState::Tracking,
            &mut i,
            &p,
            1,
        );
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.soc_milli_pct > 50_000);
    }

    #[test]
    fn ocv_lookup_full_charge() {
        let p = default_params();
        assert_eq!(soc_for_ocv(p.ocv_at_100_pct_micro_v, &p), 100_000);
    }

    #[test]
    fn ocv_lookup_empty() {
        let p = default_params();
        assert_eq!(soc_for_ocv(p.ocv_at_0_pct_micro_v, &p), 0);
    }

    #[test]
    fn ocv_lookup_midpoint() {
        let p = default_params();
        let mid = (p.ocv_at_0_pct_micro_v + p.ocv_at_100_pct_micro_v) / 2;
        assert!((soc_for_ocv(mid, &p) - 50_000).abs() < 100);
    }

    #[test]
    fn soc_clamps_at_zero() {
        let p = default_params();
        // Below 0 % OCV → returns 0.
        assert_eq!(soc_for_ocv(p.ocv_at_0_pct_micro_v - 1_000_000, &p), 0);
    }

    #[test]
    fn soc_clamps_at_full() {
        let p = default_params();
        assert_eq!(soc_for_ocv(p.ocv_at_100_pct_micro_v + 1_000_000, &p), 100_000);
    }

    #[test]
    fn near_empty_saturates() {
        let p = default_params();
        let mut i = Internal {
            soc_milli_pct: 3_000,
            confidence_milli: 1000,
            initialized: true,
            ..Default::default()
        };
        let r = SocKalman::tick(
            EventIn::Req,
            &data(41_000_000, 0), // OCV near 0%
            EccState::Tracking,
            &mut i,
            &p,
            1,
        );
        assert_eq!(r.next_state, EccState::Saturated);
    }

    #[test]
    fn bad_quality_alarms() {
        let p = default_params();
        let mut i = SocKalman::init(&p);
        let mut d = data(48_800_000, 0);
        d.voltage_quality = SignalQuality::Bad;
        let r = SocKalman::tick(EventIn::Req, &d, EccState::Tracking, &mut i, &p, 0);
        assert_eq!(r.next_state, EccState::Alarm);
    }

    #[test]
    fn invalid_capacity_alarms() {
        let mut p = default_params();
        p.capacity_micro_c = 0;
        let mut i = SocKalman::init(&p);
        let r = SocKalman::tick(
            EventIn::Req,
            &data(48_800_000, 0),
            EccState::Tracking,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Alarm);
    }
}
