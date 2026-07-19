#![cfg_attr(not(feature = "std"), no_std)]

//! IEC 61499 Basic Function Block (BFB) trait surface for open-ot WASM cells.
//!
//! One `tick` invocation == one IEC 61499 event tick == one Pregel super-step
//! contribution from this cell. See ADR-2605151200 §LangGraph + Pregel binding.

pub use heapless;

/// Single-source panic handler for `no_std` wasm32 builds. Cells must NOT
/// define their own — Rust's link-time uniqueness rule for `#[panic_handler]`
/// makes this crate the canonical owner. Reaching the handler implies a
/// determinism-contract violation in a cell (we use saturating math
/// throughout, so panics are not expected). On wasm32 we trap; the host
/// runtime (WAMR / Wasmtime) surfaces the trap as a tick failure.
#[cfg(all(not(feature = "std"), target_arch = "wasm32"))]
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // `wasm32` traps on `unreachable`. This is well-defined behaviour and
    // host-observable, unlike `loop {}` which would hang.
    core::arch::wasm32::unreachable()
}

/// Marker trait for IEC 61499 event enums (event input variables / event outputs).
///
/// Implementations are typically small `#[repr(u8)]` enums.
pub trait EventEnum: Copy + Sized {
    fn name(self) -> &'static str;
}

/// Marker trait for typed signal records (BFB DataIn / DataOut structs).
///
/// Implementations must be `Copy` so the host can pass them by value across the
/// WAMR ABI without ambient allocation.
pub trait TypedSignals: Copy + Sized {}

/// Marker trait for in-cell state.
///
/// Implementations must contain no heap allocations. The host pre-allocates one
/// `Internal` per cell in linear memory and re-uses it across ticks.
pub trait LinearMemory: Sized {}

/// Marker trait for immutable BFB parameter records.
///
/// Parameters are frozen at `pinModule` time and never change while a cell is
/// running. Hot reconfiguration requires a new pin and a two-phase swap.
pub trait ConfigOnly: Copy + Sized {}

/// One typed Pregel message destined for a sibling cell in the cohort.
///
/// `to_cell_idx` indexes into the cell's cohort table, which the host fills in
/// at `defineCell` time so cells stay decoupled from concrete neighbor DIDs.
#[derive(Copy, Clone, Debug)]
pub struct NeighborMsg<E, D> {
    pub to_cell_idx: u8,
    pub event: E,
    pub data: D,
}

/// Outcome of one `tick` invocation.
///
/// `MAX_E` and `MAX_M` are const-generic capacity bounds chosen by each BFB
/// impl so the compiler can size the `heapless::Vec` storage statically.
pub struct TickResult<S, E, D, const MAX_E: usize, const MAX_M: usize> {
    pub next_state: S,
    pub emitted: heapless::Vec<(E, D), MAX_E>,
    pub neighbor_msgs: heapless::Vec<NeighborMsg<E, D>, MAX_M>,
}

impl<S, E, D, const MAX_E: usize, const MAX_M: usize> TickResult<S, E, D, MAX_E, MAX_M> {
    #[inline]
    pub fn new(next_state: S) -> Self {
        Self {
            next_state,
            emitted: heapless::Vec::new(),
            neighbor_msgs: heapless::Vec::new(),
        }
    }
}

/// IEC 61499 Basic Function Block, projected as a Rust trait.
///
/// Determinism contract (see SPEC §4.2): for fixed
/// `(event_in, data_in, ecc_state, internal_pre, params, super_step)`,
/// `tick` MUST produce a fixed `(next_state, emitted, neighbor_msgs, internal_post)`.
/// Implementations may not call wall-clock APIs, RNG, or any other ambient source.
pub trait BasicFunctionBlock {
    type EventIn: EventEnum;
    type EventOut: EventEnum + Copy;
    type DataIn: TypedSignals;
    type DataOut: TypedSignals;
    type EccState: Copy + Eq;
    type Internal: LinearMemory;
    type Params: ConfigOnly;
    /// Concrete `TickResult<EccState, EventOut, DataOut, MAX_E, MAX_M>` chosen
    /// by each BFB impl.
    type TickReturn;

    /// IEC 61499 ECC initial state.
    const INITIAL_STATE: Self::EccState;
    /// 4diac FBType identifier (e.g. `"PID_LIMITED"`).
    const FBTYPE: &'static str;

    fn init(params: &Self::Params) -> Self::Internal;

    fn tick(
        event_in: Self::EventIn,
        data_in: &Self::DataIn,
        ecc_state: Self::EccState,
        internal: &mut Self::Internal,
        params: &Self::Params,
        super_step: u64,
    ) -> Self::TickReturn;
}
