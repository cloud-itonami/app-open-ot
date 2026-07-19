#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `LTC_TAP_FSM` BFB — Load Tap Changer finite state machine.
//!
//! Used by `:loop:volt-var` per PROTOTYPE-MICROGRID.md §13.2. Drives a tap
//! changer on a transformer to keep terminal voltage within a deadband
//! around a target. Issues `Raise` / `Lower` / `Hold` commands subject to
//! a tap dwell timer (so the FSM doesn't hammer the physical tap).
//!
//! All math is fixed-point integer per cells/CLAUDE.md.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct LtcTapFsm;

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
pub enum TapCommand {
    #[default]
    Hold = 0,
    Raise = 1,
    Lower = 2,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataIn {
    /// Measured terminal voltage in µV (1_000_000_000 == 1.0 V × 10⁻⁹? — actually
    /// micro-volts; see PROTOTYPE-MICROGRID §13.2). For an 11 kV bus, 11_000_000_000
    /// = 11 kV.
    pub voltage_meas_micro_v: i64,
    /// Target voltage in µV.
    pub voltage_target_micro_v: i64,
    /// Current tap position, signed (negative = below neutral).
    pub tap_position: i16,
    pub voltage_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    /// Tap movement command for this tick.
    pub command: TapCommand,
    /// Computed voltage error (`voltage_meas - voltage_target`).
    pub voltage_error_micro_v: i64,
    /// Remaining dwell time in ms before the next command can be issued.
    pub dwell_remaining_ms: u32,
    /// `true` when at tap_min / tap_max and further movement would clip.
    pub at_limit: bool,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Symmetric deadband around the target, in µV. Movements only issue
    /// when |error| > deadband.
    pub dead_band_micro_v: i64,
    /// Minimum dwell time after each tap movement, in ms. Typical 30_000 (30 s).
    pub dwell_ms: u32,
    /// Lower tap limit (most-buck position; e.g. -8).
    pub tap_min: i16,
    /// Upper tap limit (most-boost position; e.g. +8).
    pub tap_max: i16,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    /// Remaining dwell in ms. Decrements by cycle_period_ms each tick.
    pub dwell_remaining_ms: u32,
    /// Last command issued (for telemetry + repeat-suppression).
    pub last_command: TapCommand,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Holding = 1,
    Raising = 2,
    Lowering = 3,
    Limit = 4,
    Alarm = 5,
}

impl BasicFunctionBlock for LtcTapFsm {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "LTC_TAP_FSM";

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
        if matches!(
            data_in.voltage_quality,
            SignalQuality::Bad | SignalQuality::Stale
        ) {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }
        if !data_in.enable {
            internal.dwell_remaining_ms = 0;
            internal.last_command = TapCommand::Hold;
            internal.initialized = false;
            return TickResult::new(EccState::Idle);
        }
        if params.tap_min >= params.tap_max || params.dead_band_micro_v < 0 {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Decrement dwell timer.
        internal.dwell_remaining_ms = internal
            .dwell_remaining_ms
            .saturating_sub(params.cycle_period_ms);
        internal.initialized = true;

        let error = data_in
            .voltage_meas_micro_v
            .saturating_sub(data_in.voltage_target_micro_v);
        let abs_error = error.saturating_abs();

        // In deadband → hold.
        if abs_error <= params.dead_band_micro_v {
            internal.last_command = TapCommand::Hold;
            let mut r = TickResult::new(EccState::Holding);
            let _ = r.emitted.push((
                EventOut::Cnf,
                DataOut {
                    command: TapCommand::Hold,
                    voltage_error_micro_v: error,
                    dwell_remaining_ms: internal.dwell_remaining_ms,
                    at_limit: false,
                },
            ));
            return r;
        }

        // Outside deadband but waiting for dwell → hold (without command).
        if internal.dwell_remaining_ms > 0 {
            internal.last_command = TapCommand::Hold;
            let mut r = TickResult::new(EccState::Holding);
            let _ = r.emitted.push((
                EventOut::Cnf,
                DataOut {
                    command: TapCommand::Hold,
                    voltage_error_micro_v: error,
                    dwell_remaining_ms: internal.dwell_remaining_ms,
                    at_limit: false,
                },
            ));
            return r;
        }

        // Decide direction: error > 0 means voltage too high → lower tap (buck).
        // error < 0 means voltage too low → raise tap (boost).
        let (cmd, state, at_limit) = if error > 0 {
            // Want to lower; check tap_min limit.
            if data_in.tap_position <= params.tap_min {
                (TapCommand::Hold, EccState::Limit, true)
            } else {
                (TapCommand::Lower, EccState::Lowering, false)
            }
        } else {
            // Want to raise; check tap_max limit.
            if data_in.tap_position >= params.tap_max {
                (TapCommand::Hold, EccState::Limit, true)
            } else {
                (TapCommand::Raise, EccState::Raising, false)
            }
        };

        if matches!(cmd, TapCommand::Raise | TapCommand::Lower) {
            internal.dwell_remaining_ms = params.dwell_ms;
        }
        internal.last_command = cmd;

        let mut r = TickResult::new(state);
        let _ = r.emitted.push((
            EventOut::Cnf,
            DataOut {
                command: cmd,
                voltage_error_micro_v: error,
                dwell_remaining_ms: internal.dwell_remaining_ms,
                at_limit,
            },
        ));
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn ltc_tap_fsm_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = LtcTapFsm::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn ltc_tap_fsm_tick(
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
        1 => EccState::Holding,
        2 => EccState::Raising,
        3 => EccState::Lowering,
        4 => EccState::Limit,
        5 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = LtcTapFsm::tick(event_in, data_in, ecc_state, internal, params, super_step);
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

    fn arbitrary_tap_command() -> TapCommand {
        match kani::any::<u8>() % 3 {
            0 => TapCommand::Hold,
            1 => TapCommand::Raise,
            _ => TapCommand::Lower,
        }
    }

    fn arbitrary_ecc_state() -> EccState {
        match kani::any::<u8>() % 6 {
            0 => EccState::Idle,
            1 => EccState::Holding,
            2 => EccState::Raising,
            3 => EccState::Lowering,
            4 => EccState::Limit,
            _ => EccState::Alarm,
        }
    }

    #[kani::proof]
    fn tick_never_panics() {
        let data_in = DataIn {
            voltage_meas_micro_v: kani::any(),
            voltage_target_micro_v: kani::any(),
            tap_position: kani::any(),
            voltage_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        };
        let mut internal = Internal {
            dwell_remaining_ms: kani::any(),
            last_command: arbitrary_tap_command(),
            initialized: kani::any(),
        };
        let params = Params {
            dead_band_micro_v: kani::any(),
            dwell_ms: kani::any(),
            tap_min: kani::any(),
            tap_max: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = LtcTapFsm::tick(
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
            dead_band_micro_v: kani::any(),
            dwell_ms: kani::any(),
            tap_min: kani::any(),
            tap_max: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = LtcTapFsm::init(&params);
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
            dead_band_micro_v: 100_000_000, // ±0.1 V on a 1 V scale; for 11 kV, set higher
            dwell_ms: 30_000,
            tap_min: -8,
            tap_max: 8,
            cycle_period_ms: 1_000,
        }
    }

    fn data(meas: i64, target: i64, tap: i16) -> DataIn {
        DataIn {
            voltage_meas_micro_v: meas,
            voltage_target_micro_v: target,
            tap_position: tap,
            voltage_quality: SignalQuality::Good,
            enable: true,
        }
    }

    fn run_one(d: DataIn, internal: &mut Internal) -> (EccState, DataOut) {
        let p = default_params();
        let r = LtcTapFsm::tick(EventIn::Req, &d, EccState::Idle, internal, &p, 0);
        let (_, data) = *r.emitted.first().unwrap();
        (r.next_state, data)
    }

    #[test]
    fn within_deadband_holds() {
        let mut i = Internal::default();
        let (state, data) = run_one(data(11_000_000_000, 11_000_050_000, 0), &mut i);
        assert_eq!(state, EccState::Holding);
        assert!(matches!(data.command, TapCommand::Hold));
    }

    #[test]
    fn over_voltage_lowers() {
        let mut i = Internal::default();
        // Meas > target by more than deadband.
        let (state, data) = run_one(data(11_500_000_000, 11_000_000_000, 0), &mut i);
        assert_eq!(state, EccState::Lowering);
        assert!(matches!(data.command, TapCommand::Lower));
        // Dwell timer armed.
        assert_eq!(i.dwell_remaining_ms, 30_000);
    }

    #[test]
    fn under_voltage_raises() {
        let mut i = Internal::default();
        let (state, data) = run_one(data(10_500_000_000, 11_000_000_000, 0), &mut i);
        assert_eq!(state, EccState::Raising);
        assert!(matches!(data.command, TapCommand::Raise));
    }

    #[test]
    fn at_max_tap_limits() {
        let mut i = Internal::default();
        let (state, data) = run_one(data(10_500_000_000, 11_000_000_000, 8), &mut i);
        assert_eq!(state, EccState::Limit);
        assert!(matches!(data.command, TapCommand::Hold));
        assert!(data.at_limit);
    }

    #[test]
    fn at_min_tap_limits() {
        let mut i = Internal::default();
        let (state, data) = run_one(data(11_500_000_000, 11_000_000_000, -8), &mut i);
        assert_eq!(state, EccState::Limit);
        assert!(matches!(data.command, TapCommand::Hold));
        assert!(data.at_limit);
    }

    #[test]
    fn dwell_blocks_repeat_command() {
        let mut i = Internal {
            dwell_remaining_ms: 15_000,
            last_command: TapCommand::Lower,
            initialized: true,
        };
        let (state, data) = run_one(data(11_500_000_000, 11_000_000_000, 0), &mut i);
        // Still want to lower, but dwell not elapsed → holding.
        assert_eq!(state, EccState::Holding);
        assert!(matches!(data.command, TapCommand::Hold));
        // Dwell decrements by cycle_period_ms (1000).
        assert_eq!(data.dwell_remaining_ms, 14_000);
    }

    #[test]
    fn bad_quality_alarms() {
        let mut i = Internal::default();
        let mut d = data(11_000_000_000, 11_000_000_000, 0);
        d.voltage_quality = SignalQuality::Bad;
        let p = default_params();
        let r = LtcTapFsm::tick(EventIn::Req, &d, EccState::Idle, &mut i, &p, 0);
        assert_eq!(r.next_state, EccState::Alarm);
    }
}
