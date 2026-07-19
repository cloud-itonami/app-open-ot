# gate-b-rig — Risk-1 Gate B host simulator

Host approximation of the SPEC §14.2 fleet test (3 × Atama + 12 × Mimi/Te + TSN, 24 h soak). The rig is a single-process Wasmtime simulator that drives N field cells × 1 aggregator across configurable super-steps, writes a checkpoint after each super-step, and injects crashes that force a resume from checkpoint.

**Real Gate B still requires the hardware fleet.** A host PASS here is a necessary precondition, not a sufficient one.

## What it validates

| SPEC §14.2 criterion | Host approximation |
|---|---|
| Super-step duration p99 ≤ 50 ms | per-super-step wall-clock histogram, deadline enforced |
| Checkpoint write p99 ≤ 100 ms | fsync'd binary file rewrite, deadline enforced |
| Zero in-flight message loss across crashes | live aggregator vs. deterministic reference run — any mismatch counts as loss |
| Resume from checkpoint ≤ 5 s | wall-clock from in-process drop to resumed first super-step |

## Build

```bash
# 1. Build droop_p_f.wasm (Gate B uses one cell type across all 12 field
#    instances per `:loop:freq-droop` in PROTOTYPE-MICROGRID §2.3).
cd 60-apps/etzhayyim-project-open-ot/cells
cargo build --release --no-default-features --target wasm32-unknown-unknown -p droop-p-f

# 2. Build + run the rig.
cd ../risk1/gate-b-rig
cargo run --release -- \
  --num-cells 12 \
  --super-steps 1000 \
  --crash-count 4 \
  --deadline-superstep-ns 50000000 \
  --deadline-checkpoint-ns 100000000 \
  --max-resume-ns 5000000000
```

The rig exits non-zero on any criterion failure, so CI just runs it.

## CLI

```
gate-b-rig [OPTIONS]

  --wasm-path <PATH>                droop_p_f.wasm (default points at cells/target/...)
  --num-cells <N>                   field cells (default 12, per SPEC §14.2)
  --super-steps <N>                 super-steps to run (default 1000)
  --cycle-period-ms <MS>            Params.cycle_period_ms (informational, default 100)
  --crash-count <N>                 crash events to inject (default 4 = 1 ctrl + 3 dev)
  --seed <N>                        deterministic crash-event scheduler seed
  --checkpoint-dir <PATH>           checkpoint dir (default ./gate-b-checkpoints)
  --deadline-superstep-ns <NS>      per-super-step budget (default 50 ms)
  --deadline-checkpoint-ns <NS>     per-checkpoint budget (default 100 ms)
  --max-resume-ns <NS>              max resume time (default 5 s)
  --report <PATH>                   markdown output (default ../gate-b-report.md)
```

## Architecture

- 1 Wasmtime `Module`, 12 `Store`s (one per field cell) — mirrors the production "one Pregel node = one WASM instance" rule per ADR-2605151200 §4.
- Per super-step: synthesize 12 `DataIn`s (deterministic from `super_step + cell_idx`), tick each cell, sum `delta_p_micro_kw` into the cohort aggregate, snapshot 12 `Internal` blobs, write fsynced checkpoint.
- Reference run: a parallel non-crashed shadow that produces the expected aggregate stream. Live aggregator value is compared per super-step; any mismatch is logged as an in-flight message loss.
- Crash injection: at scheduled super-steps the rig drops all 12 cells, reads the checkpoint, re-instantiates 12 fresh cells with `Internal` seeded from the checkpoint, and resumes. Wall-clock from drop to first resumed super-step is the "resume" latency.

## What this **doesn't** cover

- Real TSN gate windows (the rig runs as fast as possible).
- Genuine process-level `kill -9` — crashes are in-process drops.
- Network-side message delivery — single-process aggregator.
- 24 h soak — defaults to 1000 super-steps; raise `--super-steps` for longer.
- Real RisingWave / RW Hyperdrive — substitutes local fs + fsync.

## Output

`gate-b-report.md` contains:

- Per-super-step latency histogram (n / min / mean / p50 / p90 / p99 / p99.9 / max)
- Checkpoint write latency histogram
- Resume-from-checkpoint latency histogram (one sample per crash)
- Message-loss counter
- PASS / FAIL verdict against SPEC §14.2 thresholds
