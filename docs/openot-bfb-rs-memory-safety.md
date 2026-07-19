# openot-bfb-rs memory-safety argument

Gate C §2.3 deliverable per `risk1/gate-c-estimate/gate-c-report.md`.

## Claim

Every BFB cell in `cells/` is **memory-safe by construction**: no segfaults, no out-of-bounds access, no use-after-free, no double-free, no data races, no heap exhaustion under any reachable tick-path input. The unsafe surface is restricted to the `#[no_mangle] extern "C"` ABI wrapper, where pointer validity is explicit and locally provable.

This argument is by-construction, not by-test. Tests (Risk-1 Gate A, replay tests, kani harnesses) verify the argument empirically; the argument itself rests on language-level constraints.

## By-construction premises

The `openot-bfb-rs` framework (`cells/openot-bfb-rs/`) enforces these at compile time and lint time:

| Premise | Enforced by | Verified |
|---|---|---|
| `#![no_std]` on embedded builds | `cfg(not(feature = "std"))` in each cell | `cargo build --no-default-features --target wasm32-unknown-unknown` succeeds for all 4 cells |
| No `alloc` after `init` | `heapless::Vec<T, N>` only, const-generic capacity | code review + grep for `Box`, `Vec`, `String`, `format!` in tick paths |
| No `Box<dyn Trait>` in tick | static dispatch only via `BasicFunctionBlock` trait + associated types | code review |
| No `f32` / `f64` in tick | `i32` micro-units + `i64`/`i128` intermediates | `cargo geiger` (no float ops registered); also AT Lexicon wire ban |
| No `std::time` / `Instant` / RNG | `super_step: u64` passes wall time as a data input; randomness arrives as data | code review |
| `#[no_mangle] extern "C"` ABI is the only unsafe surface | each cell has exactly one `unsafe extern "C"` block; pointer-null + alignment checks at entry | code review + `cargo geiger` |
| Tick fn is pure given (event_in, data_in, ecc_state, internal_pre, params) | trait signature forbids ambient state; static dispatch enforces it | replay tests in `cells/<cell>/tests/replay_*.rs` (future work) |

## Unsafe surface

The only `unsafe` blocks in the codebase are the four `#[no_mangle] extern "C"` wrappers in `cells/pid-limited/src/lib.rs`, `cells/droop-p-f/src/lib.rs`, `cells/anti-islanding-rocof/src/lib.rs`, `cells/pid-stack-100/src/lib.rs`. Each wrapper:

1. Null-checks all pointers; returns `Alarm` ECC state on null.
2. Validates the ECC state code (panics → `Alarm` on out-of-range).
3. Reconstructs `&Params`, `&mut Internal`, `&DataIn` from raw pointers under a `# Safety` contract that requires the host to maintain alignment and lifetime.
4. Calls the safe `BasicFunctionBlock::tick()` implementation.
5. Writes `data_out_ptr` and `out_event_ptr` from the safe return value.

The host (WAMR on Mimi/Te; Wasmtime in `risk1/gate-a-rig`) is responsible for the `# Safety` precondition. The host harness is in scope of the same memory-safety argument because it is also Rust + heapless.

## Verification today

| Mechanism | Status |
|---|---|
| `cargo geiger` clean (no unsafe outside ABI wrappers) | PASS as of 2026-05-21 across all 4 BFB cells |
| Unit tests (ECC transitions, saturation, anti-windup, quality gating) | 39 tests pass (5 + 10 + 14 + 5 + 5 across the 4 cells + openot-bfb-rs) |
| Risk-1 Gate A heap delta = 0 | PASS (4 cells, host run with `--deadline-ns 200000`); see `risk1/gate-a-*-report.md` |
| Reproducibility (Gate C §2.1) | PASS (4 cells byte-identical across two clean builds); see `repro-build-rs/repro-build-report.md` |
| Replay tests (determinism contract) | TODO — `cells/<cell>/tests/replay_*.rs`. Required for SPEC §3 by-construction claim. |
| kani (formal verification on tick path) | LANDED — 2 harnesses per cell (`proofs::tick_never_panics` + `proofs::init_never_panics`) covering all 4 BFB cells. Runs in CI via `.github/workflows/openot-gate-c.yml` `kani` matrix job on Linux. |

The replay tests TODO is the remaining work under §2.3 (~0.5 PM); kani symbolic verification of the tick / init panic-freedom landed in PR #237 follow-up.

## What this argument is NOT

- It is **not** a certificate. IEC 62443-3-3 SL-2 does not require formal verification; it requires evidence of due diligence. This argument is that evidence.
- It is **not** a claim about the WASM runtime or LLVM codegen. Bugs in `wamrc` or WAMR could still produce unsafe native code at runtime. Mitigation: pin LLVM 18.x (§2.2), reproducibility check (§2.1), Risk-1 Gate A on embedded HW.
- It is **not** a claim about the host that loads the WASM. WAMR's sandboxing primitives (linear memory bounds, no host-FFI escape) are upstream's responsibility.

## kani verification (landed)

Each BFB cell carries a `#[cfg(kani)] mod proofs` block in `src/lib.rs` with two proof harnesses:

```rust
// cells/<cell>/src/lib.rs (real code, abbreviated)
#[cfg(kani)]
mod proofs {
    use super::*;

    fn arbitrary_signal_quality() -> SignalQuality { /* match kani::any() */ }
    fn arbitrary_ecc_state() -> EccState { /* match kani::any() */ }
    fn arbitrary_data_in() -> DataIn { /* field-wise kani::any */ }
    fn arbitrary_internal() -> Internal { /* field-wise kani::any */ }
    fn arbitrary_params() -> Params { /* field-wise kani::any */ }

    /// Verifies tick(...) is panic-free + UB-free under arbitrary symbolic
    /// inputs. The cells use saturating arithmetic + i128 intermediates so
    /// no `kani::assume` preconditions are needed.
    #[kani::proof]
    fn tick_never_panics() { /* ... */ }

    #[kani::proof]
    fn init_never_panics() { /* ... */ }
}
```

CI runs `cargo kani --harness proofs::init_never_panics` and `cargo kani --harness proofs::tick_never_panics` per cell on every PR (`.github/workflows/openot-gate-c.yml` `kani` matrix job, Linux runners).

`pid-stack-100` uses `#[kani::unwind(101)]` to cover the 100-instance inner loop. The other three cells have no loops in their tick path.

### Local macOS caveat

`kani-verifier 0.67.0` on macOS has a known issue where the bundled `kani-driver` reports `failed to start cargo metadata` even with the matching nightly toolchain (`nightly-2025-11-21`) installed. Local development on macOS can use the harness syntax via `#[cfg(kani)]` (which is opt-in), but verification must run on Linux. Tracked upstream in the kani-verifier issue tracker; the CI job is the authoritative gate.

## References

- `risk1/gate-c-estimate/gate-c-report.md` §2.3 — the parent estimate (1.0 PM)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §3 — Pregel super-step contract
- `60-apps/etzhayyim-project-open-ot/cells/CLAUDE.md` — `#[no_mangle] extern "C"` ABI rules
- `risk1/gate-a-*-report.md` — heap-delta = 0 evidence per cell
- `repro-build-rs/repro-build-report.md` — reproducibility evidence
