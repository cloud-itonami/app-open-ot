#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `DROOP_P_F` BFB — active-power frequency-droop response with deadband.
//!
//! Steady-state law: `ΔP / P_rated = -(f - f_nominal) / (droop · f_nominal)`.
//! Pure proportional — no integral accumulator (the grid is the integrator).
//! All math is fixed-point: `i32` µ-units at the wire boundary, `i128`
//! intermediates so realistic microgrid asset ratings (≤ 10 MW) and
//! frequency excursions (≤ 5 Hz) cannot overflow before the saturating
//! clamp back to `i32`.
//!
//! 4diac FBType: `DROOP_P_F`. Reference cell #2 — validates the
//! `openot-bfb-rs` trait against a different ECC shape than `pid-limited`.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct DroopPF;

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
    /// Grid frequency in µHz (50_000_000 == 50.000_000 Hz). `i64` because
    /// the deadband / error arithmetic is more convenient in 64-bit.
    pub grid_freq_micro_hz: i64,
    /// Nominal frequency in µHz. Typically 50_000_000 (JP East / Europe) or
    /// 60_000_000 (JP West / US).
    pub freq_nominal_micro_hz: i64,
    /// Current asset active-power output in µkW (1_000_000_000 == 1 MW).
    pub current_p_micro_kw: i32,
    pub freq_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    /// Commanded active-power setpoint in µkW.
    pub p_setpoint_micro_kw: i32,
    /// `p_setpoint_micro_kw - current_p_micro_kw` (post-clamp).
    pub delta_p_micro_kw: i32,
    /// Frequency error in µHz (signed: positive = over-frequency).
    pub freq_error_micro_hz: i64,
    pub dead_band_active: bool,
    pub saturated: bool,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Asset rated active power in µkW. Positive.
    pub p_rated_micro_kw: i32,
    /// Lower output clamp in µkW (may be negative for power-absorbing assets).
    pub p_min_micro_kw: i32,
    /// Upper output clamp in µkW.
    pub p_max_micro_kw: i32,
    /// Droop coefficient in per-mille (1/1000). 50 == 5 % droop, meaning a
    /// 5 % frequency deviation produces a 100 % rated power change. Must
    /// be > 0; 0 disables droop response (treated as Alarm).
    pub droop_permille: i32,
    /// Symmetric deadband around nominal frequency, in µHz. Typical 200_000
    /// (= 0.2 Hz) for ENTSO-E primary frequency response.
    pub dead_band_micro_hz: i32,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    /// Last commanded setpoint, retained for telemetry and future slew-rate
    /// limiting. NOT a control accumulator — droop is purely proportional.
    pub last_setpoint_micro_kw: i32,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    WithinDeadband = 1,
    Responding = 2,
    Saturated = 3,
    Alarm = 4,
}

impl BasicFunctionBlock for DroopPF {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "DROOP_P_F";

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
        // Quality gate.
        if matches!(data_in.freq_quality, SignalQuality::Bad | SignalQuality::Stale) {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Mode gate.
        if !data_in.enable {
            internal.initialized = false;
            return TickResult::new(EccState::Idle);
        }

        // Param / input sanity gate. Droop = 0 has no defined response;
        // f_nominal = 0 would divide by zero. Both → Alarm.
        if params.droop_permille <= 0 || data_in.freq_nominal_micro_hz <= 0 {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        if !internal.initialized {
            internal.last_setpoint_micro_kw = data_in.current_p_micro_kw;
            internal.initialized = true;
        }

        let freq_error_micro_hz: i64 = data_in
            .grid_freq_micro_hz
            .saturating_sub(data_in.freq_nominal_micro_hz);
        let abs_freq_error: i64 = freq_error_micro_hz.saturating_abs();

        // Deadband short-circuit.
        if abs_freq_error <= params.dead_band_micro_hz as i64 {
            // Hold current output; report deadband.
            let p_setpoint = data_in.current_p_micro_kw;
            internal.last_setpoint_micro_kw = p_setpoint;
            let mut r = TickResult::new(EccState::WithinDeadband);
            let _ = r.emitted.push((
                EventOut::Cnf,
                DataOut {
                    p_setpoint_micro_kw: p_setpoint,
                    delta_p_micro_kw: 0,
                    freq_error_micro_hz,
                    dead_band_active: true,
                    saturated: false,
                },
            ));
            return r;
        }

        // Droop response (i128 intermediate to avoid overflow at realistic
        // microgrid scales).
        let numerator: i128 = (params.p_rated_micro_kw as i128)
            .saturating_mul(freq_error_micro_hz as i128)
            .saturating_mul(1_000)
            .saturating_neg();
        let denominator: i128 = (params.droop_permille as i128)
            .saturating_mul(data_in.freq_nominal_micro_hz as i128);
        // denominator > 0 by gates above.
        let delta_p_i128: i128 = numerator / denominator;

        let target_i128: i128 = (data_in.current_p_micro_kw as i128).saturating_add(delta_p_i128);

        let (p_setpoint, saturated) = if target_i128 > params.p_max_micro_kw as i128 {
            (params.p_max_micro_kw, true)
        } else if target_i128 < params.p_min_micro_kw as i128 {
            (params.p_min_micro_kw, true)
        } else {
            // target_i128 is within i32 by clamp; safe cast.
            (target_i128 as i32, false)
        };

        let delta_p_actual: i32 = p_setpoint.saturating_sub(data_in.current_p_micro_kw);
        internal.last_setpoint_micro_kw = p_setpoint;

        let next = if saturated {
            EccState::Saturated
        } else {
            EccState::Responding
        };

        let mut r = TickResult::new(next);
        let _ = r.emitted.push((
            EventOut::Cnf,
            DataOut {
                p_setpoint_micro_kw: p_setpoint,
                delta_p_micro_kw: delta_p_actual,
                freq_error_micro_hz,
                dead_band_active: false,
                saturated,
            },
        ));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI surface (parallel structure to pid-limited).
// ---------------------------------------------------------------------------

/// # Safety
/// Both pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn droop_p_f_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = DroopPF::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn droop_p_f_tick(
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
        1 => EccState::WithinDeadband,
        2 => EccState::Responding,
        3 => EccState::Saturated,
        4 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = DroopPF::tick(event_in, data_in, ecc_state, internal, params, super_step);
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
            1 => EccState::WithinDeadband,
            2 => EccState::Responding,
            3 => EccState::Saturated,
            _ => EccState::Alarm,
        }
    }

    fn arbitrary_data_in() -> DataIn {
        DataIn {
            grid_freq_micro_hz: kani::any(),
            freq_nominal_micro_hz: kani::any(),
            current_p_micro_kw: kani::any(),
            freq_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        }
    }

    fn arbitrary_internal() -> Internal {
        Internal {
            last_setpoint_micro_kw: kani::any(),
            initialized: kani::any(),
        }
    }

    fn arbitrary_params() -> Params {
        Params {
            p_rated_micro_kw: kani::any(),
            p_min_micro_kw: kani::any(),
            p_max_micro_kw: kani::any(),
            droop_permille: kani::any(),
            dead_band_micro_hz: kani::any(),
            cycle_period_ms: kani::any(),
        }
    }

    /// `tick` is total: saturating arithmetic + i128 intermediate ensure no
    /// arithmetic overflow, and the early-return gates (quality, droop=0,
    /// freq_nominal=0) prevent division by zero in the deadband branch.
    #[kani::proof]
    fn tick_never_panics() {
        let data_in = arbitrary_data_in();
        let mut internal = arbitrary_internal();
        let params = arbitrary_params();
        let super_step: u64 = kani::any();
        let _ = DroopPF::tick(
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
        let _ = DroopPF::init(&arbitrary_params());
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_params() -> Params {
        Params {
            p_rated_micro_kw: 1_000_000_000, // 1 MW
            p_min_micro_kw: 0,
            p_max_micro_kw: 1_000_000_000,
            droop_permille: 50, // 5 % droop
            dead_band_micro_hz: 200_000, // ±0.2 Hz
            cycle_period_ms: 100,
        }
    }

    fn data_in(grid_micro_hz: i64, current_p_micro_kw: i32, enable: bool) -> DataIn {
        DataIn {
            grid_freq_micro_hz: grid_micro_hz,
            freq_nominal_micro_hz: 50_000_000,
            current_p_micro_kw,
            freq_quality: SignalQuality::Good,
            enable,
        }
    }

    #[test]
    fn quality_bad_emits_alarm() {
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        let mut d = data_in(50_000_000, 500_000_000, true);
        d.freq_quality = SignalQuality::Bad;
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Responding, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Alarm);
        assert!(matches!(r.emitted.first().unwrap().0, EventOut::Alm));
    }

    #[test]
    fn disabled_returns_idle() {
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        let d = data_in(50_000_000, 500_000_000, false);
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Responding, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Idle);
        assert!(!internal.initialized);
    }

    #[test]
    fn zero_droop_emits_alarm() {
        let mut params = default_params();
        params.droop_permille = 0;
        let mut internal = DroopPF::init(&params);
        let d = data_in(50_100_000, 500_000_000, true);
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Alarm);
    }

    #[test]
    fn within_deadband_holds_current() {
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        // 50.000_100 Hz — 0.0001 Hz over nominal, well within the 0.2 Hz deadband.
        let d = data_in(50_000_100, 500_000_000, true);
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::WithinDeadband);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.dead_band_active);
        assert_eq!(out.delta_p_micro_kw, 0);
        assert_eq!(out.p_setpoint_micro_kw, 500_000_000);
    }

    #[test]
    fn over_frequency_drives_p_down() {
        // 50.500 Hz — 0.5 Hz over nominal, outside the 0.2 Hz deadband.
        // Expect setpoint < current.
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        let d = data_in(50_500_000, 500_000_000, true);
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Responding);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(!out.dead_band_active);
        assert!(out.delta_p_micro_kw < 0, "expected P down, got {}", out.delta_p_micro_kw);
        assert!(out.p_setpoint_micro_kw < 500_000_000);
        assert_eq!(out.freq_error_micro_hz, 500_000);
    }

    #[test]
    fn under_frequency_drives_p_up() {
        // 49.500 Hz — 0.5 Hz under, outside deadband. Expect setpoint > current.
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        let d = data_in(49_500_000, 500_000_000, true);
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Responding);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.delta_p_micro_kw > 0);
        assert!(out.p_setpoint_micro_kw > 500_000_000);
    }

    #[test]
    fn droop_proportionality() {
        // 5 % droop, 1 MW asset. A 1 % freq deviation = 0.5 Hz away from
        // 50 Hz nominal must produce a 100 / 5 = 20 % rated power change
        // = 200 kW = 200_000_000 µkW.
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        // 49.500 Hz — under-frequency by 0.5 Hz = 1 %.
        let d = data_in(49_500_000, 500_000_000, true);
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        let (_, out) = *r.emitted.first().unwrap();
        // Expect ΔP ≈ +200 kW.
        let expected_delta = 200_000_000_i32;
        let actual = out.delta_p_micro_kw;
        // Allow ±0.1 % rounding.
        let tolerance = 1_000_000_i32; // 1 kW
        assert!(
            (actual - expected_delta).abs() <= tolerance,
            "expected ΔP ≈ {}, got {}",
            expected_delta,
            actual
        );
    }

    #[test]
    fn saturation_clamps_at_p_max() {
        // Huge under-frequency at low current power. Should clamp at p_max.
        let mut params = default_params();
        params.p_max_micro_kw = 800_000_000;
        let mut internal = DroopPF::init(&params);
        let d = data_in(45_000_000, 100_000_000, true); // 5 Hz under = 10 % deviation
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Saturated);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.saturated);
        assert_eq!(out.p_setpoint_micro_kw, 800_000_000);
    }

    #[test]
    fn saturation_clamps_at_p_min() {
        // Huge over-frequency at low current power and p_min = 0 (no
        // absorption). Should clamp at p_min.
        let params = default_params();
        let mut internal = DroopPF::init(&params);
        let d = data_in(55_000_000, 100_000_000, true); // 5 Hz over
        let r = DroopPF::tick(EventIn::Req, &d, EccState::Idle, &mut internal, &params, 1);
        assert_eq!(r.next_state, EccState::Saturated);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.saturated);
        assert_eq!(out.p_setpoint_micro_kw, 0);
    }

    #[test]
    fn replay_determinism() {
        // Determinism contract per SPEC §4.2 — same as pid-limited.
        let params = default_params();
        let inputs = [
            (50_000_000_i64, 500_000_000_i32),
            (50_100_000, 500_000_000),
            (50_300_000, 500_000_000),
            (49_700_000, 500_000_000),
            (49_400_000, 500_000_000),
            (50_000_000, 500_000_000),
        ];
        let mut int_a = DroopPF::init(&params);
        let mut int_b = DroopPF::init(&params);
        let mut s_a = EccState::Idle;
        let mut s_b = EccState::Idle;
        for (i, (f, p)) in inputs.iter().enumerate() {
            let d = data_in(*f, *p, true);
            let ra = DroopPF::tick(EventIn::Req, &d, s_a, &mut int_a, &params, i as u64);
            let rb = DroopPF::tick(EventIn::Req, &d, s_b, &mut int_b, &params, i as u64);
            assert_eq!(ra.next_state, rb.next_state);
            assert_eq!(int_a.last_setpoint_micro_kw, int_b.last_setpoint_micro_kw);
            let (_, oa) = *ra.emitted.first().unwrap();
            let (_, ob) = *rb.emitted.first().unwrap();
            assert_eq!(oa.p_setpoint_micro_kw, ob.p_setpoint_micro_kw);
            assert_eq!(oa.delta_p_micro_kw, ob.delta_p_micro_kw);
            s_a = ra.next_state;
            s_b = rb.next_state;
        }
    }
}
