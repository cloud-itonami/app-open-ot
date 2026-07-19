# open-ot.etzhayyim.com — WASM-native PLC + Distributed Logic Controller (OSS)

**Status**: spec / research only (2026-05-15). No runtime artefacts yet (no `kotodama.jsonld`, no `src/`, no `wrangler.jsonc`). Apache-2.0.

Reference implementation for **WASM-based industrial PLC and DLC** in non-safety-rated control: process monitoring, energy management, building automation, water / wastewater non-SIL, lab and agricultural automation.

## SSoT

| 文書 | パス |
|---|---|
| Architecture decision | `90-docs/adr/2605151200-open-ot-wasm-plc-dlc.md` |
| Detailed spec (NSIDs / FB API / Zenoh schemas) | `60-apps/etzhayyim-project-open-ot/SPEC.md` |
| Lexicon contract (17 NSID, authored 2026-05-15) | `00-contracts/lexicons/com/etzhayyim/apps/openOt/*.json` |
| BFB cells (Cargo workspace, 3 cells, 29 tests) | `60-apps/etzhayyim-project-open-ot/cells/` |
| Risk-1 Gate A Wasmtime harness | `60-apps/etzhayyim-project-open-ot/risk1/gate-a-rig/` |
| Pregel orchestrator demos (Python, 3 variants, 25 tests) | `60-apps/etzhayyim-project-open-ot/orchestrator/` |
| SPEC §6 checkpointer (sqlite stand-in for RW) | `orchestrator/src/open_ot_orchestrator/checkpointer.py` |
| Hardware reference spec (Mimi / Te / Atama) | `60-apps/etzhayyim-project-open-ot/cad-spec/` |
| NixOS module spec for Atama edge controller | `60-apps/etzhayyim-project-open-ot/nixos/atama/` |
| Lexicon × manifest CI validator | `70-tools/scripts/open-ot/validate-cell-abi.py` (12 tests) |
| Microgrid prototype scope | `60-apps/etzhayyim-project-open-ot/PROTOTYPE-MICROGRID.md` |

## Scope summary

| Tier | Pick | Why |
|---|---|---|
| Field device OS | **Zephyr LTS** | RTOS, vendor-supported, Zenoh-Pico |
| Field device runtime | **WAMR AOT** | 60–95 % native, 20–80 µs jitter |
| Edge controller OS | **NixOS + linuxPackages_rt** (greenfield) / **Talos + RT kernel** (K8s flow) | runs CPython for LangGraph + Wasmtime + RW checkpointer |
| Edge controller runtime | **CPython 3.11+ / Granian / LangGraph** + **Wasmtime** sidecar | Pregel orchestrator, multi-cell coordination |
| Cloud gateway | **VKE + LangServer pod** | XRPC ↔ MCP ↔ atproto |
| Hard-RT (sub-10 µs) | **NOT WASM, NOT LangGraph** — RTOS / RT-Linux only | WCET unbounded with current WASM toolchain |
| Logic model | **IEC 61499 (event + data + ECC)** | semantically isomorphic to Pregel super-step |
| Logic Tier 1 (greenfield) | **Rust FB API, IEC 61499 FBType-compatible** → `wasm32-wasi` | typed FB, no GC, etzhayyim-native |
| Logic Tier 1b (graphical) | **Eclipse 4diac IDE → FBType XML → Rust codegen** | engineering surface without FORTE runtime debt |
| Logic Tier 2 (migration) | **matiec fork → ST → WASM**, wrapped as one BFB | IEC 61131-3 compatibility |
| Data plane | **Eclipse Zenoh** (UDP / shm) | Pregel message transport, lowest overhead |
| Industrial interop | **OPC UA FX over TSN** bridge | standards-aligned, TSN gate event = super-step barrier |
| Control plane | **etzhayyim XRPC + MCP** → atproto records | config, lineage, audit, capability grants |
| Orchestration | **LangGraph + RW checkpointer** (per ADR-2605082000/2100/2200) | multi-loop coordination, plant restart, audit trail |
| Southbound legacy | **Apache PLC4X** | S7, EtherNet/IP, Modbus, BACnet, DNP3 |
| Safety | **non-SIL only at MVP**; IEC 62443 SL-2 from day one | IEC 61508 cert out of scope |

## Boundary

- **In scope**: non-safety control loops, telemetry aggregation, configuration SSoT, capability grants, audit, OPC UA FX bridge, Zenoh substrate, IEC 62443-aligned signed module workflow.
- **Out of scope** (MVP): IEC 61508 / 61511 safety certification, hard-RT motion / servo loops in WASM, replacement of dedicated safety PLCs (HIMA / S7-1500F / GuardLogix), CENTUM / Experion migration tooling.

## Project conventions

- All entities use path-based DIDs: `did:web:open-ot.etzhayyim.com:{device|cell|signal|loop|fault}:{id}`.
- Telemetry is Zenoh + RisingWave (pod-side ingest), **not** atproto records.
- Control writes go XRPC → bpmn-dispatcher → AgentGateway MCP → LangServer pod → Zenoh publish (per ADR-2605111200 / ADR-2605091400).
- WASM artefacts are pinned by content hash (CID) on `pinModule`; AOT compile happens on a trusted builder, not on edge devices.
- GC-proposal modules and Component-Model dynamic linking are **disallowed in control-data path** at MVP.
- **One loop = one LangGraph graph; one cell = one Pregel node; one super-step = one IEC 61499 event tick.** Cells must be deterministic given (state, inbound events, params); wall-clock side effects enter as explicit messages.
- Tight inner loops (≥ 100 Hz PID math) run **on the field device only**. LangGraph orchestrator observes / coordinates at 1–10 Hz checkpointer cadence — never inside the per-cycle critical path.

## Resolutions (2026-05-15 follow-up)

1. **Hardware reference** — Giemon brand adopted (ADR §R1). Three boards: **Mimi (耳)** sensor RTU [STM32H753 + Zephyr + WAMR], **Te (手)** actuator RTU [i.MX RT1170 + Zephyr + WAMR], **Atama (頭)** edge controller [RK3588 + NixOS RT + LangGraph + Wasmtime]. KiCad / firmware / NixOS module under `cad-spec/` `firmware/` `nixos/` post-Risk-1.
2. **Loop-level Svelte editor** — deferred to post-Risk-1, scope spec'd (ADR §R2): read-only loop visualization + operator HMI first, engineering-write at MVP+1. Per-cell FB editing remains 4diac IDE.
3. **First prototype vertical** — **community microgrid** (100 kW–10 MW) (ADR §R3). Cross-link with `etzhayyim-project-open-denki` for CIM config SSoT; open-ot adds control verbs. Building HVAC and water utility deferred to MVP+1 / MVP+2.
4. **Risk-1 gate** — three quantitative gates A/B/C with PASS/FAIL thresholds (ADR §R4 + SPEC §14). Q3 2026 prototype.

## Open questions (carry to next session)

- ST→WASM matiec port — defer until paying design partner asks?
- TSN switch silicon for Atama — Marvell 88Q5050 vs Microchip LAN9692 vs NXP SJA1110, decision pending Q3 supply quotes.
- Giemon Te M4 core — separate WAMR instance for hard-RT cells or stays bare-metal C only?
- Pilot site for microgrid prototype — university campus / industrial site / remote island grid?
