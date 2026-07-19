# pid-limited — IEC 61499 PID_LIMITED reference BFB

Reference cell for ADR-2605151200 §R4 Risk-1 Gate A. Saturating PI controller with anti-windup, integer-only fixed-point math (i32 micro-units, i64 intermediate).

## Build

```bash
# Host-side test (default-on std feature)
cargo test                     # 5 tests, includes replay_determinism

# Embedded build for Cortex-M7 (Giemon Mimi / Te) — opt out of std
cargo build --release --no-default-features --target wasm32-wasi
wamrc \
  --target=thumbv7em --target-abi=eabihf --opt-level=3 \
  --enable-aot --disable-bulk-memory \
  -o pid_limited.aot \
  ../../target/wasm32-wasi/release/pid_limited.wasm
```

## ABI

| Export | Signature |
|---|---|
| `pid_limited_init` | `(params: *const Params, internal: *mut Internal) -> i32` |
| `pid_limited_tick` | `(event_in: u8, data_in: *const DataIn, ecc_state: u8, internal: *mut Internal, params: *const Params, super_step_lo: u32, super_step_hi: u32, data_out: *mut DataOut, out_event: *mut u8) -> u8 (next ecc_state)` |

`out_event`: `0 = none`, `1 = CNF`, `2 = ALM`.

## Determinism

Tick is a pure function of `(event_in, data_in, ecc_state, internal_pre, params, super_step)`. The `replay_determinism` test enforces this against a fixed input sequence. CI gate.

## Risk-1 Gate A workload

For Gate A measurement (per SPEC §14.1), the cell is wrapped with 100 dummy DataIn / 100 dummy DataOut signals to match the workload spec. The wrapper lives in `risk1/gate-a-rig/` (created post-Risk-1 prep).
