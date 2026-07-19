# risk1 — Risk-1 prototype gates

Risk-1 quantitative gates per ADR-2605151200 §R4. Three gates A / B / C with PASS/FAIL thresholds; outcome drives the Q3 2026 implementation go/no-go decision.

## Layout

| Path | Gate | Purpose |
|---|---|---|
| `gate-a-rig/` | A — WAMR AOT WCET | Wasmtime simulator covering 4 cells (pid_limited / droop_p_f / anti_islanding_rocof / pid_stack_100). Host run with `--deadline-ns 200000` enforces the §14.1 budget — non-zero exit on miss. Embedded measurement on Mimi HW in Q3 |
| `gate-b-rig/` | B — Pregel super-step latency | Host simulator: N field cells + 1 aggregator + checkpoint + crash injection, approximating SPEC §14.2 PASS criteria without HW |
| `gate-c-estimate/` | C — Toolchain qualification cost | Written deliverable: WAMR / LLVM 18 / Rust FB / Zephyr LTS / signing / IEC 62443-3-3 SL-2 effort estimate per SPEC §14.3 |
| `gate-*-report.md` | reports | per-gate measurement output, committed |

## Scope discipline

risk1 tools are **harness / measurement only** — they must not be confused with production runtime. The `cells/` workspace is the production code path; `risk1/` is the *measurement* path that consumes `cells/` outputs.

## Workspace separation

`risk1/` is its own Cargo workspace (heavy host deps like wasmtime). Keep separate from `cells/` so embedded-target builds stay lean.
