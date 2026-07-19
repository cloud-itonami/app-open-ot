# pid-stack-100 — Risk-1 Gate A workload BFB

100 independent saturating PI controllers per tick. Same math as `pid-limited`, replicated across `N=100` instances with shared params (kp / ki / clamps / cycle period) and per-instance state (pv / sp / quality / enable / integral / last_pv / cv / error / saturated / alarm).

Purpose: matches SPEC §14.1 Gate A workload spec — **100 DataIn / 100 DataOut signals at 1 ms cycle**. The cell exists to exercise the realistic memory-access pattern (100 reads + math + 100 writes per tick) that Gate A measures; it is not deployed in microgrid loops.

## Build

```bash
cd 60-apps/etzhayyim-project-open-ot/cells
cargo test -p pid-stack-100                                      # 5 unit tests
cargo build --release --no-default-features --target wasm32-unknown-unknown -p pid-stack-100
```

## Run via Gate A rig

```bash
cd ../risk1/gate-a-rig
cargo run --release -- \
  --cell pid_stack_100 \
  --wasm-path ../../cells/target/wasm32-unknown-unknown/release/pid_stack_100.wasm \
  --iterations 100000 \
  --report ../gate-a-stack100-report.md
```

The default rig is wired for `pid-limited` (12-byte DataIn / 12-byte DataOut). For `pid-stack-100` (1000-byte DataIn / 1000-byte DataOut) the rig needs the `--cell` / `--wasm-path` flags AND its struct-layout constants need to match. The current rig hard-codes `pid-limited` layouts; running pid-stack-100 through it requires a follow-up rig change (out of scope for this cell's PR — see "Rig integration" below).

## ABI

| Export | Signature |
|---|---|
| `pid_stack_100_init` | `(params: *const Params, internal: *mut Internal) -> i32` |
| `pid_stack_100_tick` | `(event_in: u8, data_in: *const DataIn, ecc_state: u8, internal: *mut Internal, params: *const Params, super_step_lo: u32, super_step_hi: u32, data_out: *mut DataOut, out_event: *mut u8) -> u8 (next ECC)` |

## Struct sizes (Rust `#[repr(C)]` layout)

| Struct | Size (bytes) | Layout |
|---|---|---|
| `Params` | 20 | i32 × 4 + u32 (align 4, no tail pad) |
| `Internal` | 1300 | `[i64; 100]` (800) + `[i32; 100]` (400) + `[u8; 100]` (100) |
| `DataIn` | 1000 | `[i32; 100]` × 2 (800) + `[u8; 100]` × 2 (200) |
| `DataOut` | 4000 | `[i32; 100]` × 2 (800) + `[u8; 100]` × 2 (200). Wait — let codegen verify. |

Codegen emits PARAMS_FMT = `<iiiiI`, PARAMS_SIZE = 20 (matches the table). Internal/DataIn/DataOut use fixed-length arrays not modelled by the current manifest schema, so codegen produces no DataIn/DataOut/Internal classes for this cell — see "Manifest exception" below.

Total per-tick scratch: ~3 KB. Comfortably within Mimi's 1 MB SRAM (per `cad-spec/giemon-mimi/SPEC.md`).

## Differences from pid-limited (framework validation #4)

| Aspect | pid-limited | pid-stack-100 |
|---|---|---|
| Instances per tick | 1 | **100** |
| Data per tick | ~24 bytes | **~2 KB in + 2 KB out** |
| Loop iterations in `tick` | 0 | **100** |
| ECC states | 4 | 4 (Idle / Healthy / Degraded / AllAlarm) |
| Replay-determinism complexity | trivial | **100-instance integral / last-pv equality** |

## Manifest exception

`data_in_schema` / `data_out_schema` / `internal_schema` are empty in `manifest.json` — the schemas use fixed-length arrays which the current `manifest.json` schema doesn't model. Codegen skips emitting DataIn / DataOut / Internal classes for this cell. This is an intentional gap; adding `array` rust_type to the manifest schema + corresponding codegen support is a future framework deliverable.

## Rig integration (carry-over)

The existing `risk1/gate-a-rig/src/main.rs` hard-codes `pid-limited` struct layouts. Running `pid-stack-100` through the same rig requires:

1. Either: extend the rig with cell-specific layout tables (per-cell impl block selected by `--cell` flag).
2. Or: build a separate `risk1/gate-a-stack100-rig/` binary tuned for `pid-stack-100` (parallel to the existing gate-a-rig).

The cell + tests landed first (this PR); the rig extension is a separate, smaller change. Tracking it as carry-over.
