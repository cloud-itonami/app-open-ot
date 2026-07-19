# anti-islanding-rocof — IEC 61499 ANTI_ISLANDING_ROCOF BFB

Grid-tie protection. Trips the bus-tie breaker when any of three conditions is sustained for `N` consecutive samples:

1. **ROCOF** (Rate of Change of Frequency) exceeds threshold (typical 0.5 Hz/s).
2. **Voltage envelope** — outside `[v_min, v_max]` (typical ±10 % of nominal).
3. **Frequency envelope** — outside `[f_min, f_max]` (typical ±0.5 Hz of nominal).

Latched: once tripped, stays tripped until `RESET` event from operator / agent.

Reference cell #3 — validates `openot-bfb-rs` against the most-elaborate cell shape so far:

| Aspect | pid-limited | droop-p-f | anti-islanding-rocof |
|---|---|---|---|
| ECC states | 4 | 5 | 5 |
| EventIn variants | 1 (REQ) | 1 (REQ) | **2 (REQ + RESET)** |
| EventOut variants | 2 (CNF / ALM) | 2 (CNF / ALM) | **3 (CNF / TRIP / ALM)** |
| Max emitted per tick | 1 | 1 | **2 (CNF + TRIP same tick)** |
| Latched state | no | no | **yes (Tripped)** |
| Time-derivative | no | no | **yes (ROCOF via last_freq)** |
| N-sample debounce | no | no | **yes, 3 independent counters** |
| Math intermediate | i64 | i128 | i64 + i128 (voltage %) |

Trait worked unchanged — the framework absorbs all of these without modification.

## Build

```bash
# Host-side test (default-on std feature) — 13 tests
cargo test

# Embedded build for Cortex-M7 (Giemon Te) — gridtie protection lives on actuator side
cargo build --release --no-default-features --target wasm32-unknown-unknown
wamrc \
  --target=thumbv7em --target-abi=eabihf --opt-level=3 \
  --enable-aot --disable-bulk-memory \
  -o anti_islanding_rocof.aot \
  ../target/wasm32-unknown-unknown/release/anti_islanding_rocof.wasm
```

## ABI

| Export | Signature |
|---|---|
| `anti_islanding_rocof_init` | `(params: *const Params, internal: *mut Internal) -> i32` |
| `anti_islanding_rocof_tick` | `(event_in: u8, data_in: *const DataIn, ecc_state: u8, internal: *mut Internal, params: *const Params, super_step_lo: u32, super_step_hi: u32, data_out: *mut DataOut, out_event: *mut u16) -> u8` |

`out_event` is **`u16` packed**: low byte = first emitted event (1 CNF / 2 TRIP / 3 ALM / 0 none), high byte = second emitted event. The Gate B harness will need to be aware of this — `pid-limited` and `droop-p-f` use `u8`. Future framework work: replace ad-hoc per-cell event packing with a typed result struct in `openot-bfb-rs`.

## Microgrid binding

Used by `:loop:islanding-decision` per `PROTOTYPE-MICROGRID.md` §2.5. Trip latency budget: **100 ms** decision → bus-tie open (utility safety requirement).

## Latched semantics

- `Tripped` state survives across ticks; `REQ` alone keeps it Tripped (re-emitting CNF with `trip=true`).
- `RESET` event clears the latch. Behaviour:
  - `enable=true` → `Monitoring`, all violation counters cleared. `last_freq_micro_hz` is **retained** (avoids spurious ROCOF on the post-reset tick).
  - `enable=false` → `Idle`, counters cleared.
- `RESET` does NOT bypass quality / mode gates that would otherwise force `Alarm` / `Idle`.

## Future extensions (deferred to MVP+1)

- Cross-cell typed neighbor messages to bus-tie controller (currently emitted via orchestrator-mediated path; see open-ot framework note in `cells/CLAUDE.md`).
- Phase-jump (vector-shift) detection — adds 4th violation counter.
- Rolling-mean ROCOF instead of single-sample diff (smoother, less false-trip).
