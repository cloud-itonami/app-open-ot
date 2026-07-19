# droop-p-f — IEC 61499 DROOP_P_F BFB

Active-power frequency-droop response with symmetric deadband (P-f droop). Steady-state law:

```
ΔP / P_rated = -(f - f_nominal) / (droop · f_nominal)
```

Pure proportional — no integral accumulator. Reference cell #2 in `cells/`; validates the `openot-bfb-rs` trait against a different ECC shape (5 states) and different math (i128 intermediate) than `pid-limited` (4 states, i64 intermediate).

## Build

```bash
# Host-side test (default-on std feature)
cargo test                     # 9 tests

# Embedded build for Cortex-M7 (Giemon Te)
cargo build --release --no-default-features --target wasm32-wasi
wamrc \
  --target=thumbv7em --target-abi=eabihf --opt-level=3 \
  --enable-aot --disable-bulk-memory \
  -o droop_p_f.aot \
  ../../target/wasm32-wasi/release/droop_p_f.wasm
```

## ABI

| Export | Signature |
|---|---|
| `droop_p_f_init` | `(params: *const Params, internal: *mut Internal) -> i32` |
| `droop_p_f_tick` | `(event_in: u8, data_in: *const DataIn, ecc_state: u8, internal: *mut Internal, params: *const Params, super_step_lo: u32, super_step_hi: u32, data_out: *mut DataOut, out_event: *mut u8) -> u8 (next ecc_state)` |

## Microgrid binding

Used by `:loop:freq-droop` per `PROTOTYPE-MICROGRID.md` §2.3. One cell per dispatchable asset (BESS, diesel, controllable PV); a `freq-aggregator` LangGraph node sums responses and reports loop health.

## Differences from pid-limited (framework validation)

| Aspect | pid-limited | droop-p-f |
|---|---|---|
| ECC states | 4 (Idle, Running, Saturated, Alarm) | 5 (+ WithinDeadband) |
| Math intermediate | i64 | i128 (i64 overflows at 10 MW × 5 Hz) |
| Internal accumulator | yes (integral) | no (pure proportional, last_setpoint for telemetry) |
| Param sanity gate | none beyond range | divide-by-zero (droop > 0, f_nom > 0) |
| Deadband | none | symmetric ±dead_band_micro_hz |
