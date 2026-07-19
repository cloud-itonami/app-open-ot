#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 `BLACK_START_SEQ` BFB — multi-stage black-start FSM.
//!
//! Used by `:loop:islanding-decision` per PROTOTYPE-MICROGRID.md §13.2. On
//! detection of a grid outage, walks through 5 stages:
//!
//! ```text
//!   0 Idle            — grid present, nothing to do
//!   1 Detecting       — outage observed, dwell to confirm
//!   2 StartingGen     — issue StartGen command, wait for ready
//!   3 EnergizingBus   — close isolated breaker, wait for voltage stable
//!   4 Syncing         — wait for grid return + frequency/phase match
//!   5 Connected       — close tie breaker, normal operation
//!   * Alarm           — invariant violation
//! ```
//!
//! Each stage emits a `command` for the operator / dispatcher to consume.

use openot_bfb_rs::{
    BasicFunctionBlock, ConfigOnly, EventEnum, LinearMemory, TickResult, TypedSignals,
};

pub struct BlackStartSeq;

#[derive(Copy, Clone, Debug)]
#[repr(u8)]
pub enum EventIn {
    Req = 0,
    /// Operator-initiated abort; forces Alarm (latched until external reset).
    Abort = 1,
}

impl EventEnum for EventIn {
    fn name(self) -> &'static str {
        match self {
            EventIn::Req => "REQ",
            EventIn::Abort => "ABORT",
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

#[derive(Copy, Clone, Debug, Default)]
#[repr(u8)]
pub enum Command {
    #[default]
    None = 0,
    StartGen = 1,
    EnergizeBus = 2,
    WaitSync = 3,
    CloseTieBreaker = 4,
    HoldConnected = 5,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct DataIn {
    pub grid_present: bool,
    pub gen_ready: bool,
    pub bus_voltage_stable: bool,
    pub voltage_synced: bool,
    /// `true` if the upstream operator believes a black start is authorised.
    /// Without this the FSM stays in Idle even if the grid is gone (manual
    /// gating per SPEC §3.5 latched-Alarm semantics).
    pub authorised: bool,
}
impl TypedSignals for DataIn {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct DataOut {
    /// Current stage 0..5 (Idle/Detecting/StartingGen/EnergizingBus/Syncing/Connected).
    pub stage: u8,
    /// Command for this tick — what the operator / dispatcher should issue.
    pub command: Command,
    /// Remaining dwell in ms before the FSM advances to the next stage on a
    /// stuck input.
    pub dwell_remaining_ms: u32,
    /// `true` once `Stage::Connected` is reached.
    pub connected: bool,
}
impl TypedSignals for DataOut {}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct Params {
    /// Dwell after outage detection before issuing StartGen, in ms.
    /// Filters transient dips. Typical 5_000 (= 5 s).
    pub detect_dwell_ms: u32,
    /// Max wait for gen_ready, in ms. Typical 60_000 (= 60 s).
    pub gen_timeout_ms: u32,
    /// Max wait for bus_voltage_stable, in ms. Typical 30_000 (= 30 s).
    pub bus_timeout_ms: u32,
    /// Max wait for voltage_synced, in ms. Typical 120_000 (= 2 min).
    pub sync_timeout_ms: u32,
    pub cycle_period_ms: u32,
}
impl ConfigOnly for Params {}

#[derive(Copy, Clone, Debug, Default)]
#[repr(C)]
pub struct Internal {
    /// Timer for the current stage, ms remaining.
    pub stage_timer_ms: u32,
    /// Sticky stage; cleared only on Abort or restoration to Idle.
    pub current_stage: u8,
    pub initialized: bool,
}
impl LinearMemory for Internal {}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum EccState {
    Idle = 0,
    Detecting = 1,
    StartingGen = 2,
    EnergizingBus = 3,
    Syncing = 4,
    Connected = 5,
    Alarm = 6,
}

fn ecc_to_stage(ecc: EccState) -> u8 {
    match ecc {
        EccState::Idle => 0,
        EccState::Detecting => 1,
        EccState::StartingGen => 2,
        EccState::EnergizingBus => 3,
        EccState::Syncing => 4,
        EccState::Connected => 5,
        EccState::Alarm => u8::MAX,
    }
}

impl BasicFunctionBlock for BlackStartSeq {
    type EventIn = EventIn;
    type EventOut = EventOut;
    type DataIn = DataIn;
    type DataOut = DataOut;
    type EccState = EccState;
    type Internal = Internal;
    type Params = Params;
    type TickReturn = TickResult<EccState, EventOut, DataOut, 1, 0>;

    const INITIAL_STATE: EccState = EccState::Idle;
    const FBTYPE: &'static str = "BLACK_START_SEQ";

    fn tick(
        event_in: EventIn,
        data_in: &DataIn,
        ecc_state: EccState,
        internal: &mut Internal,
        params: &Params,
        _super_step: u64,
    ) -> Self::TickReturn {
        // Abort from any state → latched Alarm (cleared only by external
        // reset — modelled here as a fresh init).
        if matches!(event_in, EventIn::Abort) {
            internal.current_stage = u8::MAX;
            internal.stage_timer_ms = 0;
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Latched Alarm sticks; consumer must call init() to recover.
        if matches!(ecc_state, EccState::Alarm) {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        // Param sanity.
        if params.cycle_period_ms == 0 {
            let mut r = TickResult::new(EccState::Alarm);
            let _ = r.emitted.push((EventOut::Alm, DataOut::default()));
            return r;
        }

        internal.initialized = true;
        internal.stage_timer_ms = internal.stage_timer_ms.saturating_sub(params.cycle_period_ms);

        // Stage dispatch.
        let (next_state, command, event_out) = match ecc_state {
            EccState::Idle => {
                if data_in.grid_present || !data_in.authorised {
                    (EccState::Idle, Command::None, EventOut::Cnf)
                } else {
                    // Outage + authorised → enter Detecting with dwell.
                    internal.stage_timer_ms = params.detect_dwell_ms;
                    (EccState::Detecting, Command::None, EventOut::Cnf)
                }
            }
            EccState::Detecting => {
                if data_in.grid_present {
                    // Grid came back during dwell → return to Idle.
                    internal.stage_timer_ms = 0;
                    (EccState::Idle, Command::None, EventOut::Cnf)
                } else if internal.stage_timer_ms == 0 {
                    internal.stage_timer_ms = params.gen_timeout_ms;
                    (EccState::StartingGen, Command::StartGen, EventOut::Cnf)
                } else {
                    (EccState::Detecting, Command::None, EventOut::Cnf)
                }
            }
            EccState::StartingGen => {
                if data_in.gen_ready {
                    internal.stage_timer_ms = params.bus_timeout_ms;
                    (
                        EccState::EnergizingBus,
                        Command::EnergizeBus,
                        EventOut::Cnf,
                    )
                } else if internal.stage_timer_ms == 0 {
                    // Generator timed out → Alarm (operator intervention).
                    (EccState::Alarm, Command::None, EventOut::Alm)
                } else {
                    (EccState::StartingGen, Command::StartGen, EventOut::Cnf)
                }
            }
            EccState::EnergizingBus => {
                if data_in.bus_voltage_stable {
                    internal.stage_timer_ms = params.sync_timeout_ms;
                    (EccState::Syncing, Command::WaitSync, EventOut::Cnf)
                } else if internal.stage_timer_ms == 0 {
                    (EccState::Alarm, Command::None, EventOut::Alm)
                } else {
                    (
                        EccState::EnergizingBus,
                        Command::EnergizeBus,
                        EventOut::Cnf,
                    )
                }
            }
            EccState::Syncing => {
                if data_in.grid_present && data_in.voltage_synced {
                    internal.stage_timer_ms = 0;
                    (
                        EccState::Connected,
                        Command::CloseTieBreaker,
                        EventOut::Trip, // signal that the tie breaker should close
                    )
                } else if internal.stage_timer_ms == 0 {
                    (EccState::Alarm, Command::None, EventOut::Alm)
                } else {
                    (EccState::Syncing, Command::WaitSync, EventOut::Cnf)
                }
            }
            EccState::Connected => {
                if data_in.grid_present {
                    (
                        EccState::Connected,
                        Command::HoldConnected,
                        EventOut::Cnf,
                    )
                } else {
                    // Lost grid post-connect → restart from Detecting.
                    internal.stage_timer_ms = params.detect_dwell_ms;
                    (EccState::Detecting, Command::None, EventOut::Cnf)
                }
            }
            EccState::Alarm => unreachable!("handled above"),
        };

        internal.current_stage = ecc_to_stage(next_state);

        let mut r = TickResult::new(next_state);
        let connected = matches!(next_state, EccState::Connected);
        let _ = r.emitted.push((
            event_out,
            DataOut {
                stage: ecc_to_stage(next_state),
                command,
                dwell_remaining_ms: internal.stage_timer_ms,
                connected,
            },
        ));
        r
    }

    fn init(_params: &Params) -> Internal {
        Internal::default()
    }
}

// ---------------------------------------------------------------------------
// C ABI
// ---------------------------------------------------------------------------

#[no_mangle]
pub unsafe extern "C" fn black_start_seq_init(
    params_ptr: *const Params,
    internal_ptr: *mut Internal,
) -> i32 {
    if params_ptr.is_null() || internal_ptr.is_null() {
        return -1;
    }
    *internal_ptr = BlackStartSeq::init(&*params_ptr);
    0
}

/// # Safety
/// All pointers must be non-null, aligned, and live for the call's duration.
#[no_mangle]
pub unsafe extern "C" fn black_start_seq_tick(
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
        1 => EventIn::Abort,
        _ => return EccState::Alarm as u8,
    };
    let ecc_state = match ecc_state_code {
        0 => EccState::Idle,
        1 => EccState::Detecting,
        2 => EccState::StartingGen,
        3 => EccState::EnergizingBus,
        4 => EccState::Syncing,
        5 => EccState::Connected,
        6 => EccState::Alarm,
        _ => return EccState::Alarm as u8,
    };
    let super_step: u64 = ((super_step_hi as u64) << 32) | (super_step_lo as u64);
    let data_in = &*data_in_ptr;
    let internal = &mut *internal_ptr;
    let params = &*params_ptr;
    let result = BlackStartSeq::tick(event_in, data_in, ecc_state, internal, params, super_step);
    if let Some((event, data)) = result.emitted.first() {
        *data_out_ptr = *data;
        *out_event_ptr = match event {
            EventOut::Cnf => 1,
            EventOut::Trip => 2,
            EventOut::Alm => 3,
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

    fn arbitrary_event_in() -> EventIn {
        match kani::any::<u8>() & 1 {
            0 => EventIn::Req,
            _ => EventIn::Abort,
        }
    }

    fn arbitrary_command() -> Command {
        match kani::any::<u8>() % 6 {
            0 => Command::None,
            1 => Command::StartGen,
            2 => Command::EnergizeBus,
            3 => Command::WaitSync,
            4 => Command::CloseTieBreaker,
            _ => Command::HoldConnected,
        }
    }

    fn arbitrary_ecc_state() -> EccState {
        match kani::any::<u8>() % 7 {
            0 => EccState::Idle,
            1 => EccState::Detecting,
            2 => EccState::StartingGen,
            3 => EccState::EnergizingBus,
            4 => EccState::Syncing,
            5 => EccState::Connected,
            _ => EccState::Alarm,
        }
    }

    #[kani::proof]
    fn tick_never_panics() {
        let data_in = DataIn {
            grid_present: kani::any(),
            gen_ready: kani::any(),
            bus_voltage_stable: kani::any(),
            voltage_synced: kani::any(),
            authorised: kani::any(),
        };
        // Bound stage_timer_ms loosely; sat sub means any starting value is fine.
        let mut internal = Internal {
            stage_timer_ms: kani::any(),
            current_stage: kani::any(),
            initialized: kani::any(),
        };
        let _ = arbitrary_command(); // exercise helper
        let params = Params {
            detect_dwell_ms: kani::any(),
            gen_timeout_ms: kani::any(),
            bus_timeout_ms: kani::any(),
            sync_timeout_ms: kani::any(),
            // cycle_period_ms = 0 explicitly handled → Alarm path; kani::any
            // covers both branches.
            cycle_period_ms: kani::any(),
        };
        let _ = BlackStartSeq::tick(
            arbitrary_event_in(),
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
            detect_dwell_ms: kani::any(),
            gen_timeout_ms: kani::any(),
            bus_timeout_ms: kani::any(),
            sync_timeout_ms: kani::any(),
            cycle_period_ms: kani::any(),
        };
        let _ = BlackStartSeq::init(&params);
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
            detect_dwell_ms: 5_000,
            gen_timeout_ms: 60_000,
            bus_timeout_ms: 30_000,
            sync_timeout_ms: 120_000,
            cycle_period_ms: 1_000,
        }
    }

    fn data(
        grid: bool,
        gen: bool,
        bus: bool,
        sync: bool,
        auth: bool,
    ) -> DataIn {
        DataIn {
            grid_present: grid,
            gen_ready: gen,
            bus_voltage_stable: bus,
            voltage_synced: sync,
            authorised: auth,
        }
    }

    #[test]
    fn idle_with_grid_stays_idle() {
        let p = default_params();
        let mut i = Internal::default();
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(true, false, false, false, true),
            EccState::Idle,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Idle);
    }

    #[test]
    fn idle_without_grid_unauthorised_stays_idle() {
        let p = default_params();
        let mut i = Internal::default();
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(false, false, false, false, false), // not authorised
            EccState::Idle,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Idle);
    }

    #[test]
    fn idle_without_grid_authorised_enters_detecting() {
        let p = default_params();
        let mut i = Internal::default();
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(false, false, false, false, true),
            EccState::Idle,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Detecting);
        assert_eq!(i.stage_timer_ms, 5_000);
    }

    #[test]
    fn detecting_grid_returns_goes_idle() {
        let p = default_params();
        let mut i = Internal {
            stage_timer_ms: 3_000,
            ..Default::default()
        };
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(true, false, false, false, true),
            EccState::Detecting,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Idle);
    }

    #[test]
    fn detecting_dwell_expired_starts_gen() {
        let p = default_params();
        let mut i = Internal {
            // After dwell decrement by cycle_period_ms (1000) this hits 0.
            stage_timer_ms: 1_000,
            ..Default::default()
        };
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(false, false, false, false, true),
            EccState::Detecting,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::StartingGen);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(matches!(out.command, Command::StartGen));
    }

    #[test]
    fn starting_gen_ready_advances_to_energizing() {
        let p = default_params();
        let mut i = Internal {
            stage_timer_ms: 30_000,
            ..Default::default()
        };
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(false, true, false, false, true),
            EccState::StartingGen,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::EnergizingBus);
    }

    #[test]
    fn starting_gen_timeout_alarms() {
        let p = default_params();
        let mut i = Internal {
            stage_timer_ms: 500, // less than cycle_period_ms → underflow → 0
            ..Default::default()
        };
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(false, false, false, false, true),
            EccState::StartingGen,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Alarm);
    }

    #[test]
    fn syncing_with_voltage_match_connects() {
        let p = default_params();
        let mut i = Internal {
            stage_timer_ms: 60_000,
            ..Default::default()
        };
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(true, true, true, true, true),
            EccState::Syncing,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Connected);
        let (_, out) = *r.emitted.first().unwrap();
        assert!(matches!(out.command, Command::CloseTieBreaker));
        assert!(out.connected);
    }

    #[test]
    fn connected_loses_grid_restarts_sequence() {
        let p = default_params();
        let mut i = Internal::default();
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(false, true, true, false, true),
            EccState::Connected,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Detecting);
    }

    #[test]
    fn abort_from_any_state_alarms() {
        let p = default_params();
        let mut i = Internal::default();
        let r = BlackStartSeq::tick(
            EventIn::Abort,
            &data(false, true, true, true, true),
            EccState::Syncing,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Alarm);
    }

    #[test]
    fn alarm_is_latched() {
        let p = default_params();
        let mut i = Internal::default();
        let r = BlackStartSeq::tick(
            EventIn::Req,
            &data(true, true, true, true, true),
            EccState::Alarm,
            &mut i,
            &p,
            0,
        );
        assert_eq!(r.next_state, EccState::Alarm);
    }
}
