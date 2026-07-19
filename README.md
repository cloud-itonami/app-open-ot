# open-ot — WASM-native PLC + Distributed Logic Controller (OSS)

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Reference implementation for **WASM-based industrial PLC and Distributed Logic Controller (DLC)** in non-safety-rated control: process monitoring, energy management, building automation, water / wastewater non-SIL, lab and agricultural automation. Apache-2.0.

**Status (2026-05-15)**: spec / research with 2 reference Function Block cells implemented. No deployed runtime yet. Implementation gated on Risk-1 prototype outcome (Q3 2026).

## What it is

- Each control loop is one **LangGraph graph**; each cell is one **Pregel node**; one **super-step** is one **IEC 61499 event tick**.
- Cells compile to **WASM (`wasm32-wasi`)** and run inside **WAMR AOT** on **Zephyr** (field tier) or **Wasmtime** on **PREEMPT_RT Linux** / **NixOS** (edge tier).
- Logic semantics follow **IEC 61499** (event-driven function blocks). Optional 4diac IDE round-trip via FBType XML.
- Substrate: **Eclipse Zenoh** (data plane), **OPC UA FX over TSN** (cross-vendor interop), **etzhayyim XRPC + MCP** (control-plane / config / audit).
- Configuration / lineage / audit are **atproto records** under `com.etzhayyim.apps.openOt.*` (17 NSIDs).
- Hardware reference: **Giemon Mimi (耳)** sensor RTU / **Te (手)** actuator RTU / **Atama (頭)** edge controller.
- First prototype vertical: **community microgrid (100 kW–10 MW)** in collaboration with [`open-denki`](../etzhayyim-project-open-denki).

## Authoritative source

| Topic | File |
|---|---|
| Architecture decision | [`90-docs/adr/2605151200-open-ot-wasm-plc-dlc.md`](../../90-docs/adr/2605151200-open-ot-wasm-plc-dlc.md) (ADR-2605151200, status: proposed) |
| Detailed spec (NSIDs / FB API / Pregel binding) | [`SPEC.md`](SPEC.md) |
| Project conventions | [`CLAUDE.md`](CLAUDE.md) |
| Hardware spec (Mimi / Te / Atama) | [`cad-spec/giemon-{mimi,te,atama}/SPEC.md`](cad-spec/) |
| Cell cargo workspace | [`cells/`](cells/) |
| Microgrid prototype scope | [`PROTOTYPE-MICROGRID.md`](PROTOTYPE-MICROGRID.md) |
| Lexicon JSON (XRPC contract) | [`../../00-contracts/lexicons/com/etzhayyim/apps/openOt/`](../../00-contracts/lexicons/com/etzhayyim/apps/openOt/) |

## Layout

```
60-apps/etzhayyim-project-open-ot/
├── README.md                    ← you are here
├── CONTRIBUTING.md              monorepo / repo-split contribution policy
├── LICENSE                      Apache-2.0 + dependency attribution
├── OWNERS                       maintainers
├── CLAUDE.md                    project conventions (LLM-readable)
├── SPEC.md                      detailed spec (NSIDs / FB API / Pregel binding)
├── PROTOTYPE-MICROGRID.md       first prototype scope
├── cad-spec/                    hardware reference (Giemon Mimi / Te / Atama)
└── cells/                       Cargo workspace for IEC 61499 BFB cells
    ├── Cargo.toml               workspace root
    ├── openot-bfb-rs/           shared trait crate
    ├── pid-limited/             reference BFB #1 (PID_LIMITED)
    └── droop-p-f/               reference BFB #2 (DROOP_P_F)
```

## Build & test

All cells compile to `wasm32-wasi` for embedded deployment, with default-on `std` for host development:

```bash
cd 60-apps/etzhayyim-project-open-ot/cells

# Host-side test — all 15 unit tests across both cells
cargo test --workspace

# Embedded build for Cortex-M7 (Giemon Mimi / Te), per cell
cargo build --release --no-default-features --target wasm32-wasi -p pid-limited
wamrc \
  --target=thumbv7em --target-abi=eabihf --opt-level=3 \
  --enable-aot --disable-bulk-memory \
  -o pid_limited.aot \
  target/wasm32-wasi/release/pid_limited.wasm
```

## Implemented cells

| FBType | Crate | Tests | Notes |
|---|---|---|---|
| `PID_LIMITED` | `cells/pid-limited` | 5 / 5 | Saturating PI + anti-windup, integer fixed-point |
| `DROOP_P_F` | `cells/droop-p-f` | 10 / 10 | Frequency-droop with deadband, i128 intermediate |
| `ANTI_ISLANDING_ROCOF` | `cells/anti-islanding-rocof` | 14 / 14 | ROCOF + voltage / freq envelopes, latched trip + RESET, multi-event-output |

Risk-1 Gate A simulator: `risk1/gate-a-rig/` — Wasmtime host harness that loads `pid_limited.wasm` and emits a latency report. Run-to-completion validates the .wasm pipeline end-to-end without Mimi HW.

Pregel orchestrator demos: `orchestrator/` — Python implementation of the `:loop:freq-droop` microgrid loop, available in two equivalent variants:

- `microgrid_pregel.py` — minimal Python BSP runner (no LangGraph dep)
- `microgrid_langgraph.py` — real `langgraph.graph.StateGraph` + `MemorySaver` checkpointer

Both load real `droop_p_f.wasm` cells via `wasmtime-py` and produce byte-identical cohort ΔP outputs — the equivalence proof that "IEC 61499 event tick ≡ Pregel super-step" works end-to-end through working code, not just spec. 12 / 12 unit tests pass; see `orchestrator/README.md`.

Roadmap (per `PROTOTYPE-MICROGRID.md`): `MPPT_PERTURB_OBSERVE`, `SOC_KALMAN`, `VV_CURVE`, `LTC_TAP_FSM`, `BLACK_START_SEQ`.

## Safety classification

Non-safety only at MVP. IEC 62443 SL-2 from day one (signed `.aot` modules + capability-based imports + no ambient authority). **IEC 61508 / 61511 functional safety certification is explicitly out of scope** — any Safety Instrumented Function (SIF) requires a separate certified safety PLC running in parallel.

## Issues / contributions

This project currently lives in the [`etzhayyim/etzhayyim-root`](https://github.com/etzhayyim/etzhayyim-root) monorepo. See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the split-repo plan and the contribution flow.

## License

Apache-2.0. See [`LICENSE`](LICENSE) for full text and third-party dependency attribution.
