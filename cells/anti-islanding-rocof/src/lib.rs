#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `ANTI_ISLANDING_ROCOF` BFB — grid-tie protection.
//!
//! Trips the grid-tie breaker when any of three conditions is sustained
//! for `N` consecutive samples:
//!
//! 1. **ROCOF** (Rate of Change of Frequency) exceeds threshold.
//!    `rocof = (f_now - f_last) / dt`, sign-significant.
//! 2. **Voltage envelope** — grid voltage outside `[v_min, v_max]`.
//! 3. **Frequency envelope** — grid frequency outside `[f_min, f_max]`.
//!
//! The trip is **latched**: once entered, the cell stays in `Tripped` until
//! a `RESET` event arrives from the operator / agent. This is the first BFB
//! in `cells/` that exercises:
//!
//! - Multi-event-input (`REQ` + `RESET`)
//! - Multi-event-output (`CNF` + `TRIP` + `ALM`)
//! - `MAX_E > 1` — `CNF` + `TRIP` emitted on the same tick when a violation
//!   crosses the debounce threshold
//! - Latched ECC state (`Tripped` survives across ticks)
//! - Time-derivative computation (`last_freq_micro_hz` retained)
//! - Multi-counter N-sample debounce
//!
//! 4diac FBType: `ANTI_ISLANDING_ROCOF`. Used by microgrid
//! `:loop:islanding-decision` (per `PROTOTYPE-MICROGRID.md` §2.5). Trip
//! latency budget: 100 ms decision → bus-tie open.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct AntiIslandingRocof;

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum EventIn {
    Req = 0,
    Reset = 1,
}

impl EventEnum for EventIn {
    fn name(self) -> &'static str {
        match self {
            EventIn::Req => "REQ",
            EventIn::Reset => "RESET",
        }
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum EventOut {
    Cnf = 0,
    Trip = 1,
    Alm = 2,
}

impl EventEnum for EventOut {
    fn name(self) -> &'static str {
        match self {
            EventOut::Cnf => "CNF",
            EventOut::Trip => "TRIP",
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
#[repr(u8)]
pub enum TripReason {
    None = 0,
    Rocof = 1,
    Overvoltage = 2,
    Undervoltage = 3,
    Overfrequency = 4,
    Underfrequency = 5,
}

impl Default for TripReason {
    fn default() -> Self {
        TripReason::None
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataIn {
    pub grid_freq_micro_hz: i64,
    pub freq_nominal_micro_hz: i64,
    pub grid_voltage_micro_v: i64,
    pub voltage_nominal_micro_v: i64,
    pub freq_quality: SignalQuality,
    pub voltage_quality: SignalQuality,
    pub enable: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    pub trip: bool,
    pub trip_reason: TripReason,
    pub rocof_micro_hz_per_s: i64,
    /// Voltage deviation in milli-percent: 1234 == 1.234 % above nominal.
    pub voltage_deviation_milli_pct: i32,
    pub freq_deviation_micro_hz: i64,
    pub rocof_violation_count: u8,
    pub voltage_violation_count: u8,
    pub freq_violation_count: u8,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// ROCOF threshold in µHz/s. Typical 500_000 (= 0.5 Hz/s).
    pub rocof_threshold_micro_hz_per_s: i64,
    /// Consecutive samples above ROCOF threshold required to trip.
    pub rocof_window_samples: u32,
    /// Voltage envelope, µV. Typical for 230 V system: 207_000_000 / 253_000_000.
    pub voltage_min_micro_v: i64,
    pub voltage_max_micro_v: i64,
    pub voltage_window_samples: u32,
    /// Frequency envelope, µHz. Typical for 50 Hz: 49_500_000 / 50_500_000.
    pub freq_min_micro_hz: i64,
    pub freq_max_micro_hz: i64,
    pub freq_window_samples: u32,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    pub last_freq_micro_hz: i64,
    pub rocof_violation_count: u32,
    pub voltage_violation_count: u32,
    pub freq_violation_count: u32,
    pub last_trip_reason: TripReason,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Monitoring = 1,
    Warning = 2,
    Tripped = 3,
    Alarm = 4,
}

impl BasicFunctionBlock for AntiIslandingRocof {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    /// `MAX_E = 2` because a violation crossing the debounce threshold emits
    /// `CNF` + `TRIP` on the same tick.
    type TickReturn = TickResult<EccState, EventOut, DataOut, 2, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "ANTI_ISLANDING_ROCOF";

    fn init(_params: &Params) -> Internal {
        Internal::default()
    }

    fn tick(
        event_in: EventIn,
        data_in: &DataIn,
        ecc_state: EccState,
        internal: &mut Internal,
        params: &Params,
        _super_step: u64,
    ) -> Self::TickReturn {
        // RESET event from any state clears the trip latch and the violation
        // counters, returning to Monitoring (or Idle if not enabled).
        if matches!(event_in, EventIn::Reset) {
            internal.rocof_violation_count = 0;
            internal.voltage_violation_count = 0;
            internal.freq_violation_count = 0;
            internal.last_trip_reason = TripReason::None;
            // last_freq is intentionally retained — RESET shouldn't blank
            // history and create a spurious ROCOF spike on the next tick.
            let next = if data_in.enable {
                EccState::Monitoring
            } else {
                EccState::Idle
            };
            let mut r = TickResult::new(next);
            let _ = r.emitted.push((EventOut::Cnf, DataOut::default()));
            return r;
        }

        // Quality gate.
        if matches!(
            data_in.freq_quality,
            SignalQuality::Bad | SignalQuality::Stale
        ) || matches!(
            data_in.voltage_quality,
            SignalQuality::Bad | SignalQuality::Stale
        ) {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Mode gate.
        if !data_in.enable {
            internal.rocof_violation_count = 0;
            internal.voltage_violation_count = 0;
            internal.freq_violation_count = 0;
            internal.initialized = false;
            return TickResult::new(EccState::Idle);
        }

        // Latched trip state — REQ alone does not clear it. Still track
        // `last_freq` here so that a subsequent RESET doesn't see a stale
        // delta and spurious-trip on the next REQ (the gap between
        // pre-trip and post-reset measurements isn't a single cycle).
        if matches!(ecc_state, EccState::Tripped) {
            internal.last_freq_micro_hz = data_in.grid_freq_micro_hz;
            let mut r = TickResult::new(EccState::Tripped);
            let _ = r.emitted.push((
                EventOut::Cnf,
                DataOut {
                    trip: true,
                    trip_reason: internal.last_trip_reason,
                    rocof_micro_hz_per_s: 0,
                    voltage_deviation_milli_pct: 0,
                    freq_deviation_micro_hz: 0,
                    rocof_violation_count: internal.rocof_violation_count.min(255) as u8,
                    voltage_violation_count: internal.voltage_violation_count.min(255) as u8,
                    freq_violation_count: internal.freq_violation_count.min(255) as u8,
                },
            ));
            return r;
        }

        // First-tick init — establish last_freq baseline so the ROCOF on the
        // very first sample is zero (not freq itself), avoiding a spurious
        // trip at startup.
        if !internal.initialized {
            internal.last_freq_micro_hz = data_in.grid_freq_micro_hz;
            internal.rocof_violation_count = 0;
            internal.voltage_violation_count = 0;
            internal.freq_violation_count = 0;
            internal.initialized = true;
            let mut r = TickResult::new(EccState::Monitoring);
            let _ = r.emitted.push((EventOut::Cnf, DataOut::default()));
            return r;
        }

        // ROCOF computation in µHz/s.
        // delta_freq = grid - last  (µHz)
        // rocof = delta_freq * 1000 / cycle_period_ms  (µHz/s)
        let delta_freq: i64 = data_in
            .grid_freq_micro_hz
            .saturating_sub(internal.last_freq_micro_hz);
        let rocof_micro_hz_per_s: i64 = if params.cycle_period_ms == 0 {
            0
        } else {
            delta_freq.saturating_mul(1000) / (params.cycle_period_ms as i64)
        };
        internal.last_freq_micro_hz = data_in.grid_freq_micro_hz;

        // Voltage deviation in milli-percent (signed).
        // delta_v_milli_pct = (grid - nominal) * 100_000 / nominal   (signed, milli-%)
        let voltage_deviation_milli_pct: i32 = if data_in.voltage_nominal_micro_v == 0 {
            0
        } else {
            let dev = data_in
                .grid_voltage_micro_v
                .saturating_sub(data_in.voltage_nominal_micro_v);
            let scaled = (dev as i128).saturating_mul(100_000);
            let q = scaled / (data_in.voltage_nominal_micro_v as i128);
            if q > i32::MAX as i128 {
                i32::MAX
            } else if q < i32::MIN as i128 {
                i32::MIN
            } else {
                q as i32
            }
        };

        let freq_deviation: i64 = data_in
            .grid_freq_micro_hz
            .saturating_sub(data_in.freq_nominal_micro_hz);

        // Violation counters — each independently tracks N consecutive samples.
        // ROCOF: |rocof| over threshold counts.
        let rocof_violated = rocof_micro_hz_per_s.saturating_abs()
            > params.rocof_threshold_micro_hz_per_s;
        if rocof_violated {
            internal.rocof_violation_count = internal.rocof_violation_count.saturating_add(1);
        } else {
            internal.rocof_violation_count = 0;
        }

        // Voltage envelope.
        let voltage_under = data_in.grid_voltage_micro_v < params.voltage_min_micro_v;
        let voltage_over = data_in.grid_voltage_micro_v > params.voltage_max_micro_v;
        let voltage_violated = voltage_under || voltage_over;
        if voltage_violated {
            internal.voltage_violation_count = internal.voltage_violation_count.saturating_add(1);
        } else {
            internal.voltage_violation_count = 0;
        }

        // Frequency envelope.
        let freq_under = data_in.grid_freq_micro_hz < params.freq_min_micro_hz;
        let freq_over = data_in.grid_freq_micro_hz > params.freq_max_micro_hz;
        let freq_violated = freq_under || freq_over;
        if freq_violated {
            internal.freq_violation_count = internal.freq_violation_count.saturating_add(1);
        } else {
            internal.freq_violation_count = 0;
        }

        // Trip decision — first violation type to cross its window wins.
        // Order: ROCOF > voltage > frequency (ROCOF is the fastest islanding
        // signature; voltage envelope is the most physically dangerous).
        let trip_reason = if internal.rocof_violation_count >= params.rocof_window_samples
            && params.rocof_window_samples > 0
        {
            TripReason::Rocof
        } else if internal.voltage_violation_count >= params.voltage_window_samples
            && params.voltage_window_samples > 0
        {
            if voltage_over {
                TripReason::Overvoltage
            } else {
                TripReason::Undervoltage
            }
        } else if internal.freq_violation_count >= params.freq_window_samples
            && params.freq_window_samples > 0
        {
            if freq_over {
                TripReason::Overfrequency
            } else {
                TripReason::Underfrequency
            }
        } else {
            TripReason::None
        };

        let data_out = DataOut {
            trip: !matches!(trip_reason, TripReason::None),
            trip_reason,
            rocof_micro_hz_per_s,
            voltage_deviation_milli_pct,
            freq_deviation_micro_hz: freq_deviation,
            rocof_violation_count: internal.rocof_violation_count.min(255) as u8,
            voltage_violation_count: internal.voltage_violation_count.min(255) as u8,
            freq_violation_count: internal.freq_violation_count.min(255) as u8,
        };

        let next = if !matches!(trip_reason, TripReason::None) {
            internal.last_trip_reason = trip_reason;
            EccState::Tripped
        } else if internal.rocof_violation_count > 0
            || internal.voltage_violation_count > 0
            || internal.freq_violation_count > 0
        {
            EccState::Warning
        } else {
            EccState::Monitoring
        };

        let mut r = TickResult::new(next);
        let _ = r.emitted.push((EventOut::Cnf, data_out));
        // Emit TRIP alongside CNF on the tick where the latch closes.
        if matches!(next, EccState::Tripped) {
            let _ = r.emitted.push((EventOut::Trip, data_out));
        }
        r
    }
}

// ---------------------------------------------------------------------------
// C ABI surface.
// ---------------------------------------------------------------------------

/// # Safety
/// Both pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn anti_islanding_rocof_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = AntiIslandingRocof::init(&*params_ptr);
    0
}

/// `out_event_ptr` receives a packed pair of bytes: low byte = first event
/// (CNF/ALM/none), high byte = second event (TRIP if emitted, else 0).
///
/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn anti_islanding_rocof_tick(
    event_in_code: u8,
    data_in_ptr: *const DataIn,
    ecc_state_code: u8,
    internal_ptr: *mut Internal,
    params_ptr: *const Params,
    super_step_lo: u32,
    super_step_hi: u32,
    data_out_ptr: *mut DataOut,
    out_event_ptr: *mut u16,
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
        1 => EventIn::Reset,
        _ => return EccState::Alarm as u8,
    };
    let ecc_state = match ecc_state_code {
        0 => EccState::Idle,
        1 => EccState::Monitoring,
        2 => EccState::Warning,
        3 => EccState::Tripped,
        4 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = AntiIslandingRocof::tick(event_in, data_in, ecc_state, internal, params, super_step);

    let mut packed_events: u16 = 0;
    if let Some((event, data)) = result.emitted.first() {
        *data_out_ptr = *data;
        packed_events |= match event {
            EventOut::Cnf => 1,
            EventOut::Trip => 2,
            EventOut::Alm => 3,
        } as u16;
    } else {
        *data_out_ptr = DataOut::default();
    }
    if let Some((event, _)) = result.emitted.get(1) {
        let code: u16 = match event {
            EventOut::Cnf => 1,
            EventOut::Trip => 2,
            EventOut::Alm => 3,
        };
        packed_events |= code << 8;
    }
    *out_event_ptr = packed_events;
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

    fn arbitrary_trip_reason() -> TripReason {
        match kani::any::<u8>() % 6 {
            0 => TripReason::None,
            1 => TripReason::Rocof,
            2 => TripReason::Overvoltage,
            3 => TripReason::Undervoltage,
            4 => TripReason::Overfrequency,
            _ => TripReason::Underfrequency,
        }
    }

    fn arbitrary_event_in() -> EventIn {
        match kani::any::<u8>() & 1 {
            0 => EventIn::Req,
            _ => EventIn::Reset,
        }
    }

    fn arbitrary_ecc_state() -> EccState {
        match kani::any::<u8>() % 5 {
            0 => EccState::Idle,
            1 => EccState::Monitoring,
            2 => EccState::Warning,
            3 => EccState::Tripped,
            _ => EccState::Alarm,
        }
    }

    fn arbitrary_data_in() -> DataIn {
        DataIn {
            grid_freq_micro_hz: kani::any(),
            freq_nominal_micro_hz: kani::any(),
            grid_voltage_micro_v: kani::any(),
            voltage_nominal_micro_v: kani::any(),
            freq_quality: arbitrary_signal_quality(),
            voltage_quality: arbitrary_signal_quality(),
            enable: kani::any(),
        }
    }

    fn arbitrary_internal() -> Internal {
        Internal {
            last_freq_micro_hz: kani::any(),
            rocof_violation_count: kani::any(),
            voltage_violation_count: kani::any(),
            freq_violation_count: kani::any(),
            last_trip_reason: arbitrary_trip_reason(),
            initialized: kani::any(),
        }
    }

    fn arbitrary_params() -> Params {
        Params {
            rocof_threshold_micro_hz_per_s: kani::any(),
            rocof_window_samples: kani::any(),
            voltage_min_micro_v: kani::any(),
            voltage_max_micro_v: kani::any(),
            voltage_window_samples: kani::any(),
            freq_min_micro_hz: kani::any(),
            freq_max_micro_hz: kani::any(),
            freq_window_samples: kani::any(),
            cycle_period_ms: kani::any(),
        }
    }

    /// Multi-event tick: even with REQ + RESET + arbitrary ECC, the tick
    /// path is total. Tested under both event inputs since RESET clears
    /// counters and RESET-from-Tripped is the most logic-heavy branch.
    #[kani::proof]
    fn tick_never_panics() {
        let data_in = arbitrary_data_in();
        let mut internal = arbitrary_internal();
        let params = arbitrary_params();
        let super_step: u64 = kani::any();
        let _ = AntiIslandingRocof::tick(
            arbitrary_event_in(),
            &data_in,
            arbitrary_ecc_state(),
            &mut internal,
            &params,
            super_step,
        );
    }

    #[kani::proof]
    fn init_never_panics() {
        let _ = AntiIslandingRocof::init(&arbitrary_params());
    }
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use openot_bfb_rs::heapless;

    fn default_params() -> Params {
        Params {
            rocof_threshold_micro_hz_per_s: 500_000, // 0.5 Hz/s
            rocof_window_samples: 3,
            voltage_min_micro_v: 207_000_000, // 207 V (90 % of 230)
            voltage_max_micro_v: 253_000_000, // 253 V (110 % of 230)
            voltage_window_samples: 5,
            freq_min_micro_hz: 49_500_000, // 49.5 Hz
            freq_max_micro_hz: 50_500_000, // 50.5 Hz
            freq_window_samples: 5,
            cycle_period_ms: 100,
        }
    }

    fn data_in(
        grid_freq_micro_hz: i64,
        grid_voltage_micro_v: i64,
        enable: bool,
    ) -> DataIn {
        DataIn {
            grid_freq_micro_hz,
            freq_nominal_micro_hz: 50_000_000,
            grid_voltage_micro_v,
            voltage_nominal_micro_v: 230_000_000,
            freq_quality: SignalQuality::Good,
            voltage_quality: SignalQuality::Good,
            enable,
        }
    }

    #[test]
    fn quality_bad_emits_alarm() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        let mut d = data_in(50_000_000, 230_000_000, true);
        d.freq_quality = SignalQuality::Bad;
        let r = AntiIslandingRocof::tick(EventIn::Req, &d, EccState::Monitoring, &mut int_, &p, 1);
        assert_eq!(r.next_state, EccState::Alarm);
        assert!(matches!(r.emitted.first().unwrap().0, EventOut::Alm));
    }

    #[test]
    fn disabled_returns_idle_clears_counters() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        int_.rocof_violation_count = 2;
        let d = data_in(50_000_000, 230_000_000, false);
        let r = AntiIslandingRocof::tick(EventIn::Req, &d, EccState::Warning, &mut int_, &p, 1);
        assert_eq!(r.next_state, EccState::Idle);
        assert_eq!(int_.rocof_violation_count, 0);
    }

    #[test]
    fn first_tick_initializes_no_trip() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        // Even at way off-nominal, first tick just initializes.
        let d = data_in(45_000_000, 230_000_000, true);
        let r = AntiIslandingRocof::tick(EventIn::Req, &d, EccState::Idle, &mut int_, &p, 1);
        assert_eq!(r.next_state, EccState::Monitoring);
        assert!(int_.initialized);
        assert_eq!(int_.last_freq_micro_hz, 45_000_000);
    }

    #[test]
    fn normal_grid_stays_monitoring() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        let d = data_in(50_000_000, 230_000_000, true);
        let _ = AntiIslandingRocof::tick(EventIn::Req, &d, EccState::Idle, &mut int_, &p, 1);
        // Multiple normal samples — should stay Monitoring.
        for i in 2..10 {
            let r = AntiIslandingRocof::tick(EventIn::Req, &d, EccState::Monitoring, &mut int_, &p, i);
            assert_eq!(r.next_state, EccState::Monitoring, "tick {} expected Monitoring", i);
        }
    }

    #[test]
    fn rocof_under_window_no_trip() {
        // Window is 3 samples. Single ROCOF spike must NOT trip.
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        // First tick: init at 50 Hz.
        let _ = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_000_000, 230_000_000, true),
            EccState::Idle,
            &mut int_,
            &p,
            1,
        );
        // Second tick: jump to 50.1 Hz over 100 ms = ROCOF +1 Hz/s, way over 0.5 Hz/s threshold.
        let r = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_100_000, 230_000_000, true),
            EccState::Monitoring,
            &mut int_,
            &p,
            2,
        );
        assert_eq!(r.next_state, EccState::Warning);
        assert_eq!(int_.rocof_violation_count, 1);
    }

    #[test]
    fn rocof_over_window_trips_and_emits_two_events() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        // Init.
        let _ = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_000_000, 230_000_000, true),
            EccState::Idle,
            &mut int_,
            &p,
            1,
        );
        let mut state = EccState::Monitoring;
        // 3 consecutive ticks each with ROCOF over threshold.
        // Each tick the freq jumps another 0.1 Hz over 100 ms = +1 Hz/s.
        for (i, freq) in [50_100_000_i64, 50_200_000, 50_300_000].iter().enumerate() {
            let r = AntiIslandingRocof::tick(
                EventIn::Req,
                &data_in(*freq, 230_000_000, true),
                state,
                &mut int_,
                &p,
                (i + 2) as u64,
            );
            state = r.next_state;
            // First two: Warning. Third: Tripped + 2 emitted events.
            if i < 2 {
                assert_eq!(state, EccState::Warning, "tick {} expected Warning", i + 2);
            } else {
                assert_eq!(state, EccState::Tripped);
                assert_eq!(r.emitted.len(), 2, "expected CNF + TRIP");
                let kinds: openot_bfb_rs::heapless::Vec<_, 2> =
                    r.emitted.iter().map(|(e, _)| *e as u8).collect();
                assert!(kinds.contains(&(EventOut::Cnf as u8)));
                assert!(kinds.contains(&(EventOut::Trip as u8)));
                let (_, out) = r.emitted.first().unwrap();
                assert!(out.trip);
                assert!(matches!(out.trip_reason, TripReason::Rocof));
            }
        }
    }

    #[test]
    fn overvoltage_trips() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        let _ = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_000_000, 230_000_000, true),
            EccState::Idle,
            &mut int_,
            &p,
            1,
        );
        let mut state = EccState::Monitoring;
        // 5 consecutive over-voltage samples (260 V > 253 V max).
        for i in 0..5 {
            let r = AntiIslandingRocof::tick(
                EventIn::Req,
                &data_in(50_000_000, 260_000_000, true),
                state,
                &mut int_,
                &p,
                (i + 2) as u64,
            );
            state = r.next_state;
        }
        assert_eq!(state, EccState::Tripped);
        let r = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_000_000, 260_000_000, true),
            state,
            &mut int_,
            &p,
            10,
        );
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.trip);
        assert!(matches!(out.trip_reason, TripReason::Overvoltage));
    }

    #[test]
    fn undervoltage_trips_with_correct_reason() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        let _ = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_000_000, 230_000_000, true),
            EccState::Idle,
            &mut int_,
            &p,
            1,
        );
        let mut state = EccState::Monitoring;
        let mut last_reason = TripReason::None;
        for i in 0..5 {
            let r = AntiIslandingRocof::tick(
                EventIn::Req,
                &data_in(50_000_000, 200_000_000, true), // 200 V < 207 V min
                state,
                &mut int_,
                &p,
                (i + 2) as u64,
            );
            state = r.next_state;
            if let Some((_, out)) = r.emitted.first() {
                last_reason = out.trip_reason;
            }
        }
        assert_eq!(state, EccState::Tripped);
        assert!(matches!(last_reason, TripReason::Undervoltage));
    }

    #[test]
    fn overfrequency_trips() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        // Init at the upper limit so envelope check kicks in but ROCOF doesn't.
        let _ = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_510_000, 230_000_000, true),
            EccState::Idle,
            &mut int_,
            &p,
            1,
        );
        let mut state = EccState::Monitoring;
        for i in 0..5 {
            let r = AntiIslandingRocof::tick(
                EventIn::Req,
                &data_in(50_510_000, 230_000_000, true),
                state,
                &mut int_,
                &p,
                (i + 2) as u64,
            );
            state = r.next_state;
        }
        assert_eq!(state, EccState::Tripped);
        assert!(matches!(int_.last_trip_reason, TripReason::Overfrequency));
    }

    #[test]
    fn underfrequency_trips() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        let _ = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(49_490_000, 230_000_000, true),
            EccState::Idle,
            &mut int_,
            &p,
            1,
        );
        let mut state = EccState::Monitoring;
        for i in 0..5 {
            let r = AntiIslandingRocof::tick(
                EventIn::Req,
                &data_in(49_490_000, 230_000_000, true),
                state,
                &mut int_,
                &p,
                (i + 2) as u64,
            );
            state = r.next_state;
        }
        assert_eq!(state, EccState::Tripped);
        assert!(matches!(int_.last_trip_reason, TripReason::Underfrequency));
    }

    #[test]
    fn tripped_state_latched_until_reset() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        // Force into Tripped by stuffing internal state.
        int_.initialized = true;
        int_.last_freq_micro_hz = 50_000_000;
        int_.last_trip_reason = TripReason::Rocof;
        // Even with perfectly normal grid, REQ does not clear.
        let r = AntiIslandingRocof::tick(
            EventIn::Req,
            &data_in(50_000_000, 230_000_000, true),
            EccState::Tripped,
            &mut int_,
            &p,
            1,
        );
        assert_eq!(r.next_state, EccState::Tripped);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(out.trip);
        assert!(matches!(out.trip_reason, TripReason::Rocof));
    }

    #[test]
    fn reset_event_clears_trip() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        int_.initialized = true;
        int_.last_freq_micro_hz = 50_000_000;
        int_.last_trip_reason = TripReason::Rocof;
        int_.rocof_violation_count = 5;
        // RESET from Tripped, grid healthy, enabled.
        let r = AntiIslandingRocof::tick(
            EventIn::Reset,
            &data_in(50_000_000, 230_000_000, true),
            EccState::Tripped,
            &mut int_,
            &p,
            1,
        );
        assert_eq!(r.next_state, EccState::Monitoring);
        assert_eq!(int_.rocof_violation_count, 0);
        assert!(matches!(int_.last_trip_reason, TripReason::None));
    }

    #[test]
    fn reset_event_returns_idle_when_disabled() {
        let p = default_params();
        let mut int_ = AntiIslandingRocof::init(&p);
        int_.initialized = true;
        int_.rocof_violation_count = 5;
        let r = AntiIslandingRocof::tick(
            EventIn::Reset,
            &data_in(50_000_000, 230_000_000, false),
            EccState::Tripped,
            &mut int_,
            &p,
            1,
        );
        assert_eq!(r.next_state, EccState::Idle);
        assert_eq!(int_.rocof_violation_count, 0);
    }

    #[test]
    fn replay_determinism() {
        let p = default_params();
        // Build a sequence with a mix of normal / warning / trip-bound samples.
        let inputs: &[(i64, i64)] = &[
            (50_000_000, 230_000_000), // init
            (50_005_000, 230_000_000),
            (50_005_000, 240_000_000), // small over but within envelope
            (50_005_000, 260_000_000), // overvoltage
            (50_005_000, 260_000_000),
            (50_005_000, 260_000_000),
            (50_005_000, 260_000_000),
            (50_005_000, 260_000_000), // 5th — trip
        ];
        let mut int_a = AntiIslandingRocof::init(&p);
        let mut int_b = AntiIslandingRocof::init(&p);
        let mut s_a = EccState::Idle;
        let mut s_b = EccState::Idle;
        for (i, (f, v)) in inputs.iter().enumerate() {
            let d = data_in(*f, *v, true);
            let ra = AntiIslandingRocof::tick(EventIn::Req, &d, s_a, &mut int_a, &p, i as u64);
            let rb = AntiIslandingRocof::tick(EventIn::Req, &d, s_b, &mut int_b, &p, i as u64);
            assert_eq!(ra.next_state, rb.next_state, "tick {} state mismatch", i);
            assert_eq!(int_a.rocof_violation_count, int_b.rocof_violation_count);
            assert_eq!(int_a.voltage_violation_count, int_b.voltage_violation_count);
            assert_eq!(int_a.freq_violation_count, int_b.freq_violation_count);
            assert_eq!(ra.emitted.len(), rb.emitted.len());
            s_a = ra.next_state;
            s_b = rb.next_state;
        }
    }
}
