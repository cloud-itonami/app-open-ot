# gate-a-rig — Risk-1 Gate A Wasmtime simulator

Host-side harness for Risk-1 Gate A (per SPEC §14.1). Loads a BFB cell as `wasm32-unknown-unknown`, runs N ticks, emits a Markdown latency report.

**This is harness validation, not the embedded measurement.** Real Gate A runs the same cell via WAMR AOT on Cortex-M7 (Giemon Mimi). Host latency on x86_64 / Apple Silicon is nanosecond-class — useful only for sanity-checking that the pipeline works end-to-end.

## What it validates

1. The cell's `.wasm` artefact loads cleanly (memory export present, no unresolved imports).
2. The `#[no_mangle] extern "C"` ABI surface (`<cell>_init` / `<cell>_tick`) is callable from a real WASM runtime.
3. Struct layouts (`Params` / `Internal` / `DataIn` / `DataOut`) match between cell and host. ABI mismatches surface as `unexpected out_event` or panics during the loop.
4. The latency / counter / report pipeline works.

## Build

```bash
# 1. Build all four cells as wasm32-unknown-unknown (no WASI imports).
cd 60-apps/etzhayyim-project-open-ot/cells
cargo build --release --no-default-features --target wasm32-unknown-unknown \
  -p pid-limited -p droop-p-f -p anti-islanding-rocof -p pid-stack-100

# 2. Build + run the rig once per cell (PASS gate at 200 µs).
cd ../risk1/gate-a-rig
for cell in pid_limited droop_p_f anti_islanding_rocof pid_stack_100; do
  cargo run --release -- --cell "$cell" --iterations 50000 --deadline-ns 200000
done
```

The rig exits non-zero on the first cell that misses the deadline, so the
loop fails fast in CI.

## CLI

```
gate-a-rig [OPTIONS]

  --cell <NAME>               One of `pid_limited` (default), `pid_stack_100`,
                              `droop_p_f`, `anti_islanding_rocof`.
  --wasm-path <PATH>          Cell .wasm artefact (default: ../../cells/target/wasm32-unknown-unknown/release/<cell>.wasm)
  --iterations <N>            Tick iterations (default: 100_000)
  --cycle-period-ms <MS>      Params.cycle_period_ms (default: 1)
  --deadline-ns <NS>          Per-tick deadline in ns. 0 disables enforcement
                              (default). On Mimi the SPEC §14.1 budget is
                              200_000 (200 µs) at p99.9; pass `--deadline-ns
                              200000` to make the rig exit non-zero if any tick
                              exceeds the host approximation.
  --report <PATH>             Markdown report output (default: ../gate-a-<cell>-report.md
                              — `pid_limited` keeps the legacy `../gate-a-report.md` name)
```

Adding a new cell: declare its `CellLayout` and `synthesize_data_in_*` function in `src/main.rs`, register both in `select_cell`. The cell-agnostic main loop handles everything else.

## Report

Latency stats (host nanoseconds, not Gate A criteria):

| Stat | Meaning |
|---|---|
| min / mean / max | bounds |
| p50, p90, p99 | typical / busy / tail latency |
| p99.9, p99.99 | hard tail — relevant on embedded |

Counters:

- `ECC=Alarm` ticks
- Saturated output ticks
- Unexpected `out_event` — non-zero is an ABI mismatch (rig fails with non-zero exit code).

## What this **doesn't** measure

- Embedded WCET — that requires WAMR AOT + Cortex-M7 + DWT cycle counter.
- WAMR-specific overhead — Wasmtime uses Cranelift; WAMR AOT uses LLVM directly. Different codegen.
- Heap behaviour — host has full malloc; embedded has none.
- Real 10 h soak — the rig defaults to 100k iterations; raise `--iterations` for longer runs.
