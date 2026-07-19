# orchestrator — Pregel demos for open-ot

Demonstrates ADR-2605151200's central architectural claim — **IEC 61499 event tick ≡ Pregel super-step** — with cells running as real WASM modules and the orchestrator coordinating multi-cell loops.

| File | Variant | Loop | Cells | Tests |
|---|---|---|---|---|
| `microgrid_pregel.py` | minimal Python BSP | `:loop:freq-droop` | DROOP_P_F × N | 7 / 7 |
| `microgrid_langgraph.py` | LangGraph SDK | `:loop:freq-droop` | DROOP_P_F × N | 5 / 5 |
| `microgrid_async_langgraph.py` | LangGraph async | `:loop:freq-droop` | DROOP_P_F × N | covered |
| `microgrid_islanding_langgraph.py` | multi-event | `:loop:islanding-decision` | ANTI_ISLANDING_ROCOF | covered |
| `microgrid_volt_var_langgraph.py` | multi-cell-type, supervisory | `:loop:volt-var` | VV_CURVE × N + LTC_TAP_FSM | 7 / 7 |
| `microgrid_bess_langgraph.py` | **2-stage chain** with SOC-aware clamps | `:loop:bess-charge-discharge` | **SOC_KALMAN → DROOP_P_F** | 7 / 7 |
| `microgrid_pv_mppt_langgraph.py` | N strings + aggregator | `:loop:pv-mppt` | MPPT_PERTURB_OBSERVE × N | 7 / 7 |
| `microgrid_islanding_blackstart_langgraph.py` | **chained-FSM coordination** | `:loop:islanding-decision` ext. | **ANTI_ISLANDING_ROCOF → BLACK_START_SEQ** | 5 / 5 |

All 7 PROTOTYPE-MICROGRID §13.2 field loops have orchestrator references now (dr-response and peak-shave-economic are orchestrator-only / no field cells).

Patterns:
- `microgrid_volt_var_*`  — **fan-out → aggregate → supervisory cell**: N parallel inverter cells driving a single LTC controller.
- `microgrid_bess_*`      — **per-asset 2-stage chain**: SOC estimator feeds clamps into droop controller, then all assets fan into a cohort aggregator.
- `microgrid_pv_mppt_*`   — **fan-out → aggregate**: N PV-string MPPT cells + total power aggregator.
- `microgrid_islanding_blackstart_*` — **chained-FSM**: protection FSM output gates a restart FSM.

The freq-droop demos produce **byte-identical cohort ΔP outputs** for the same input schedule, which is the equivalence proof for the IEC 61499 ⇄ Pregel binding.

## Build & run

```bash
# 1. Build cells once (one-time per cell change).
cd ../cells
cargo build --release --no-default-features --target wasm32-unknown-unknown \
  -p droop-p-f -p pid-limited -p anti-islanding-rocof \
  -p vv-curve -p ltc-tap-fsm
# Other microgrid cells (mppt-perturb-observe, black-start-seq, soc-kalman)
# build on the same pattern — add `-p <cell>` for the loops you want to run.

# 2. Set up Python venv (uv).
cd ../orchestrator
uv sync

# 3. Run all 12 unit tests.
uv run pytest

# 4. Demo: minimal Pregel runner (#3a).
uv run python -m open_ot_orchestrator.microgrid_pregel

# 5. Demo: real LangGraph SDK (#3b).
uv run python -m open_ot_orchestrator.microgrid_langgraph
```

## Demo output (both variants identical)

```
[step  1] f=50.000 Hz  cohort ΔP =    +0.00 kW   ← nominal
[step  2] f=50.050 Hz  cohort ΔP =    +0.00 kW   ← inside ±0.2 Hz deadband
[step  3] f=50.300 Hz  cohort ΔP =  -180.00 kW   ← over-freq, droop responds DOWN
[step  4] f=50.500 Hz  cohort ΔP =  -300.00 kW   ← bigger excursion
[step  5] f=50.300 Hz  cohort ΔP =  -180.00 kW   ← recovering
[step  6] f=50.050 Hz  cohort ΔP =    +0.00 kW   ← back in deadband
[step  7] f=50.000 Hz  cohort ΔP =    +0.00 kW   ← fully recovered
```

Math check (5 % droop → 1 % freq dev = 20 % rated power change):
- step 4: f=50.5 Hz → +0.5 Hz dev = +1 % deviation; 1 MW asset = -200 kW, 500 kW asset = -100 kW; total -300 kW ✓
- step 3: f=50.3 Hz → +0.3 Hz dev = +0.6 % deviation; 1 MW asset = -120 kW, 500 kW asset = -60 kW; total -180 kW ✓

## What each test validates

### `test_pregel_runner.py` (#3a)

| Test | What it proves |
|---|---|
| `test_single_cell_step` | one cell tick via WASM, deadband hold |
| `test_two_cells_cohort_response` | two cells sum correctly |
| `test_single_task_skips_unaffected_cells` | row-driven trigger pattern (ADR-2605082200) |
| `test_checkpoint_stream_monotonic` | super_step ids 0..N |
| `test_replay_determinism_after_checkpoint_restore` | **SPEC §4.2 determinism contract** — restore from mid-stream checkpoint reproduces all subsequent outputs byte-identical |
| `test_restore_rejects_mismatched_cell_set` | safety guard |
| `test_loop_runner_rejects_duplicate_dids` | safety guard |

### `test_microgrid_langgraph.py` (#3b)

| Test | What it proves |
|---|---|
| `test_single_super_step_within_deadband` | LangGraph node = Pregel node = cell |
| `test_two_cells_cohort_sum` | parallel fan-out + aggregator fan-in |
| `test_checkpoint_history_grows_per_invocation` | `MemorySaver` per-thread history |
| `test_replay_determinism_via_thread_isolation` | same schedule × different threads = identical outputs |
| `test_thread_isolation_keeps_internals_separate` | per-thread `cell_internals` independence |

## What this is **not**

- **Not production**: in-memory checkpointer only. Real LangGraph runs inside the etzhayyim LangServer pod (per ADR-2605080600) on a Giemon Atama (per `cad-spec/giemon-atama/`) with the RisingWave checkpointer (per ADR-2605082100).
- **Not WAMR**: this loads cells via Wasmtime (Python). Embedded path remains WAMR AOT on Zephyr (Mimi/Te). See `cells/CLAUDE.md` for why each runtime fits its tier.
- **Not the full microgrid**: only `:loop:freq-droop` with 2 BESS cells. The other 6 loops in `PROTOTYPE-MICROGRID.md` (mppt / bess / volt-var / islanding / dr / peak-shave-economic) are scoped post-Risk-1.
