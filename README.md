# open-ot — WASM-native PLC + Distributed Logic Controller (OSS)

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Reference implementation for **WASM-based industrial PLC and Distributed Logic Controller (DLC)** in non-safety-rated control: process monitoring, energy management, building automation, water / wastewater non-SIL, lab and agricultural automation. Apache-2.0.

**Status**: spec / research. Nine reference Function Block cells are implemented
(`cells/`), with a Wasmtime host harness (`risk1/gate-a-rig/`) and eight Pregel /
LangGraph orchestrator demos (`orchestrator/`). **No deployed runtime.**
Implementation is gated on the Risk-1 prototype outcome.

New here? Read [`docs/operator-quickstart.md`](docs/operator-quickstart.md) — it
lists only steps that have actually been executed, and says which ones cannot be.

## What it is

- Each control loop is one **LangGraph graph**; each cell is one **Pregel node**; one **super-step** is one **IEC 61499 event tick**.
- Cells compile to **WASM (`wasm32-wasi`)** and run inside **WAMR AOT** on **Zephyr** (field tier) or **Wasmtime** on **PREEMPT_RT Linux** / **NixOS** (edge tier).
- Logic semantics follow **IEC 61499** (event-driven function blocks). Optional 4diac IDE round-trip via FBType XML.
- Substrate: **Eclipse Zenoh** (data plane), **OPC UA FX over TSN** (cross-vendor interop), **etzhayyim XRPC + MCP** (control-plane / config / audit).
- Configuration / lineage / audit are **atproto records** under `com.etzhayyim.apps.openOt.*`; the NSID surface is specified in [`SPEC.md`](SPEC.md) §2. The Lexicon JSON artefacts are *not* in this repository and are not at the monorepo path older documents cite — see [`docs/operator-quickstart.md`](docs/operator-quickstart.md) §4.
- Hardware reference: **Giemon Mimi (耳)** sensor RTU / **Te (手)** actuator RTU / **Atama (頭)** edge controller.
- First prototype vertical: **community microgrid (100 kW–10 MW)** in collaboration with `open-denki`.

## Authoritative source

This repository was split out of the `etzhayyim/root` monorepo (see
[`migration.edn`](migration.edn)). Documents that still carry
`60-apps/etzhayyim-project-open-ot/...` paths predate the split; **this repository
root *is* that directory**.

| Topic | Where |
|---|---|
| Operator quickstart (what actually runs) | [`docs/operator-quickstart.md`](docs/operator-quickstart.md) |
| Detailed spec (NSIDs / FB API / Pregel binding) | [`SPEC.md`](SPEC.md) |
| Project conventions | [`CLAUDE.md`](CLAUDE.md) |
| Hardware spec (Mimi / Te / Atama) | [`cad-spec/`](cad-spec/) |
| Cell cargo workspace | [`cells/`](cells/) |
| Orchestrator demos + tests | [`orchestrator/README.md`](orchestrator/README.md) |
| Risk-1 gate reports | [`risk1/`](risk1/) |
| Microgrid prototype scope | [`PROTOTYPE-MICROGRID.md`](PROTOTYPE-MICROGRID.md) |
| Architecture decision (ADR-2605151200) | not in this repo — [`etzhayyim/root` `90-docs/adr/2605151200-open-ot-wasm-plc-dlc.edn`](https://github.com/etzhayyim/root/blob/main/90-docs/adr/2605151200-open-ot-wasm-plc-dlc.edn) |
| Lexicon JSON (XRPC contract) | **missing** — see [`docs/operator-quickstart.md`](docs/operator-quickstart.md) §4 |

## Layout

```
.
├── README.md                    ← you are here
├── CLAUDE.md                    project conventions (LLM-readable)
├── SPEC.md                      detailed spec (NSIDs / FB API / Pregel binding)
├── PROTOTYPE-MICROGRID.md       first prototype scope
├── CONTRIBUTING.md              contribution policy
├── LICENSE, OWNERS              Apache-2.0 + attribution, maintainers
├── README.edn, migration.edn    provenance of the split from etzhayyim/root
├── docs/                        operator-quickstart + supply-chain / compliance notes
├── cells/                       Cargo workspace — 9 BFB cells + shared trait crate
├── orchestrator/                Pregel / LangGraph demos (Python) + .cljc port
├── risk1/                       Gate A/B/C rigs and their reports
├── nixos/atama/                 NixOS modules for the Atama edge controller
├── cad-spec/                    hardware reference (Giemon Mimi / Te / Atama)
├── repro-build-rs/              reproducible-build checker
├── builder-sign-rs/             build attestation signer
└── tools/codegen-cell-types.py  manifest → Python cell type codegen
```

## Build & test

The full path needs a Rust toolchain. **One step does not** — start there:

```bash
cd orchestrator && bb run_tests.clj    # 16 tests, 53 assertions, 0 failures
```

Two WASM targets exist and are not interchangeable: `wasm32-unknown-unknown`
for the host harnesses (`orchestrator/`, `risk1/gate-a-rig/`) and `wasm32-wasi`
for the embedded WAMR AOT path. Cell crates default to `std` for host tests and
`#![no_std]` for embedded builds.

```bash
cd cells

# Host-side unit tests for every cell
cargo test --workspace

# Host harness artefacts (what orchestrator/ and risk1/ load)
cargo build --release --no-default-features --target wasm32-unknown-unknown -p droop-p-f

# Embedded build for Cortex-M7 (Giemon Mimi / Te), per cell
cargo build --release --no-default-features --target wasm32-wasi -p pid-limited
wamrc \
  --target=thumbv7em --target-abi=eabihf --opt-level=3 \
  --enable-aot --disable-bulk-memory \
  -o pid_limited.aot \
  target/wasm32-wasi/release/pid_limited.wasm
```

Full walkthrough, including what the Python suite reports when the cells have
**not** been built: [`docs/operator-quickstart.md`](docs/operator-quickstart.md).

## Implemented cells

Nine BFB cells plus the shared `openot-bfb-rs` trait crate. The counts below are
`#[test]` functions **counted in source** — this repository does not publish an
observed pass count, because no run of `cargo test --workspace` is recorded here.

| FBType | Crate | `#[test]` fns | Notes |
|---|---|---|---|
| `PID_LIMITED` | `cells/pid-limited` | 5 | Saturating PI + anti-windup, integer fixed-point |
| `PID_STACK_100` | `cells/pid-stack-100` | 5 | 100-loop PID stack for the Gate A density test |
| `DROOP_P_F` | `cells/droop-p-f` | 10 | Frequency-droop with deadband, i128 intermediate |
| `ANTI_ISLANDING_ROCOF` | `cells/anti-islanding-rocof` | 14 | ROCOF + voltage / freq envelopes, latched trip + RESET, multi-event-output |
| `VV_CURVE` | `cells/vv-curve` | 8 | Volt-VAR curve, piecewise-linear |
| `LTC_TAP_FSM` | `cells/ltc-tap-fsm` | 7 | On-load tap-changer FSM, supervisory |
| `MPPT_PERTURB_OBSERVE` | `cells/mppt-perturb-observe` | 7 | Perturb-and-observe MPPT |
| `SOC_KALMAN` | `cells/soc-kalman` | 11 | State-of-charge estimator |
| `BLACK_START_SEQ` | `cells/black-start-seq` | 11 | Black-start staging FSM |

Risk-1 rigs: `risk1/gate-a-rig/` (Wasmtime host harness that loads a cell and
emits a latency report), `risk1/gate-b-rig/`, `risk1/gate-c-estimate/`. Gate
reports for each cell are checked in alongside them as `gate-*-report.md`.

Pregel orchestrator demos: `orchestrator/` holds eight demo modules covering the
seven `PROTOTYPE-MICROGRID.md` §13.2 field loops — freq-droop (minimal BSP,
LangGraph sync, LangGraph async), volt-var, BESS charge/discharge, PV MPPT,
islanding, and islanding→black-start.
The freq-droop variants produce byte-identical cohort ΔP outputs for the same
input schedule, which is the equivalence proof that "IEC 61499 event tick ≡
Pregel super-step" holds in working code and not only in the spec. Per-loop
detail and per-test intent: [`orchestrator/README.md`](orchestrator/README.md).

## Safety classification

Non-safety only at MVP. IEC 62443 SL-2 from day one (signed `.aot` modules + capability-based imports + no ambient authority). **IEC 61508 / 61511 functional safety certification is explicitly out of scope** — any Safety Instrumented Function (SIF) requires a separate certified safety PLC running in parallel.

## Issues / contributions

This project currently lives in the [`etzhayyim/etzhayyim-root`](https://github.com/etzhayyim/etzhayyim-root) monorepo. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the split-repo plan and the contribution flow.

## License

Apache-2.0. See [`LICENSE`](LICENSE) for full text and third-party dependency attribution.
