# Risk-1 Gate B — host simulator report

**Wasm artefact**: `../../cells/target/wasm32-unknown-unknown/release/droop_p_f.wasm`

**Field cells**: 12 (droop_p_f)

**Super-steps**: 1000

**Cycle period**: 100 ms (informational)

**Crashes injected**: 4 of 4 scheduled — at super-steps [65, 135, 661, 971]

**Checkpoint dir**: `./gate-b-checkpoints`

**Total wall-clock**: 4.331 s

## Super-step latency

| Stat | Value (ns) | Value (ms) |
|---|---|---|
| n        | 1000 | 0.001 |
| min      | 1000 | 0.001 |
| mean     | 4026 | 0.004 |
| p50      | 2959 | 0.003 |
| p90      | 6333 | 0.006 |
| p99      | 21083 | 0.021 |
| p99.9    | 28292 | 0.028 |
| max      | 72917 | 0.073 |

- Misses (>50 ms): 0 / 1000

## Checkpoint write latency

| Stat | Value (ns) | Value (ms) |
|---|---|---|
| n        | 1000 | 0.001 |
| min      | 1680958 | 1.681 |
| mean     | 4311552 | 4.312 |
| p50      | 4170375 | 4.170 |
| p90      | 5103167 | 5.103 |
| p99      | 7329209 | 7.329 |
| p99.9    | 9009875 | 9.010 |
| max      | 9993583 | 9.994 |

- Misses (>100 ms): 0 / 1000

## Resume-from-checkpoint latency

| Stat | Value (ns) | Value (ms) |
|---|---|---|
| n        | 4 | 0.000 |
| min      | 414334 | 0.414 |
| mean     | 712833 | 0.713 |
| p50      | 675375 | 0.675 |
| p90      | 1231375 | 1.231 |
| p99      | 1231375 | 1.231 |
| p99.9    | 1231375 | 1.231 |
| max      | 1231375 | 1.231 |

- Misses (>5 s): 0 / 4

## Message-loss check

- Mismatched aggregates (live vs. reference): 0 / 1000

## Verdict

- Host verdict: **PASS**
- SPEC §14.2 PASS thresholds:
  - super-step p99 ≤ 50 ms
  - checkpoint p99 ≤ 100 ms
  - zero in-flight message loss across crashes
  - resume ≤ 5 s
- Observed: step p99 = 0.021 ms; ckpt p99 = 7.329 ms; resume max = 1.231 ms; messages_lost = 0.

## Notes

- This is a **host simulator**, not the SPEC §14.2 fleet test. Real Gate B requires 3 × Atama + 12 × Mimi/Te + TSN switch + 24 h soak (per SPEC §14.2 table).
- The simulator approximates the four PASS criteria; in particular it skips genuine TSN gate windows and process-level kill -9 (crashes are in-process drops). A host PASS is necessary but not sufficient.
- Checkpoint format is a fixed-size binary (magic + super_step + N × Internal + aggregate). Production uses RisingWave-backed sqlite stand-in (`orchestrator/src/open_ot_orchestrator/checkpointer.py`); the host rig substitutes fsync on local fs.
- Aggregator value is the cohort Δp sum across 12 cells (mirrors the `:loop:freq-droop` topology in PROTOTYPE-MICROGRID §2.3 §13.2).
