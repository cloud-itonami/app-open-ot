# Risk-1 Gate A — Wasmtime harness report

**Cell**: `droop_p_f`

**Wasm artefact**: `../../cells/target/wasm32-unknown-unknown/release/droop_p_f.wasm`

**Iterations**: 50000

**Cycle period**: 1 ms

**Layout**: params=24 B  internal=8 B  data_in=24 B  data_out=24 B

**Total wall-clock**: 0.005 s

## Tick latency (host, x86_64 / aarch64 — not embedded)

| Stat | Value (ns) |
|---|---|
| n        | 50000 |
| min      | 0 |
| mean     | 46 |
| p50      | 42 |
| p90      | 83 |
| p99      | 84 |
| p99.9    | 125 |
| p99.99   | 208 |
| max      | 8833 |

## Counters

- ALM-emitting ticks: 0
- Unexpected `out_event`: 0
- Deadline (200000 ns) misses: 0
- Memory pages: initial=16 final=32 delta=0 (1 page = 64 KB)

## Notes

- This is **host harness validation**, not Mimi WCET measurement.
- Real Gate A criteria (per SPEC §14.1): STM32H753 @ 480 MHz, Zephyr LTS, WAMR AOT, 1 ms cycle, 10 h continuous. **PASS**: p99.9 tick latency ≤ 200 µs, zero deadline misses, observed heap delta = 0 bytes.
- Host run validates: artefact load, ABI surface, struct layouts (cell ⇄ rig), determinism, report pipeline.
- `--deadline-ns 200_000` approximates the SPEC §14.1 budget on the host. PASS on host is necessary but not sufficient: a host p99.9 already above budget would mean the cell can't possibly meet it on Mimi.
