# open-ot — Detailed Spec (Draft)

**Status**: draft, 2026-05-15. Companion to ADR-2605151200. NSIDs, schemas, and FB API surface listed here are for review; nothing is implemented.

## 1. Identity topology

Path-based DIDs under `did:web:open-ot.etzhayyim.com`:

| DID prefix | Entity | Example |
|---|---|---|
| `:device:` | Physical controller / RTU / edge gateway | `did:web:open-ot.etzhayyim.com:device:rtu-east-01` |
| `:cell:` | WASM module instance (one control loop / FB network) | `did:web:open-ot.etzhayyim.com:cell:pid-tank3-level` |
| `:signal:` | Signal point (analog / digital / string) | `did:web:open-ot.etzhayyim.com:signal:tank3-level-pv` |
| `:loop:` | Logical control loop (≥ 1 cell + signals) | `did:web:open-ot.etzhayyim.com:loop:tank3-level-control` |
| `:fault:` | Fault record | `did:web:open-ot.etzhayyim.com:fault:2026-05-15-001` |

Devices, cells, signals, and loops are AT records under `com.etzhayyim.apps.openOt.*`. Fault records carry a DMN-evaluated severity.

## 2. NSID surface (MVP)

Procedures (write):

| NSID | Purpose | Notes |
|---|---|---|
| `com.etzhayyim.apps.openOt.defineDevice` | register physical controller | manufacturer / model / firmware / location |
| `com.etzhayyim.apps.openOt.defineCell` | declare WASM cell instance | references pinned module CID, host capabilities, cohort links |
| `com.etzhayyim.apps.openOt.defineSignal` | analog / digital / string signal | EU range, deadband, sample period |
| `com.etzhayyim.apps.openOt.defineLoop` | PID / sequence / interlock | references cells + signals |
| `com.etzhayyim.apps.openOt.pinModule` | pin `.aot` / `.wasm` artefact | content-addressed CID, signed by builder DID |
| `com.etzhayyim.apps.openOt.grantCapability` | host capability grant | I/O, network peer, neighbor cell, time source |
| `com.etzhayyim.apps.openOt.setpointChange` | operator / agent setpoint | requires `setpoint:write` capability + audit reason |
| `com.etzhayyim.apps.openOt.modeChange` | auto / manual / cascade / safe | interlock-checked DMN |
| `com.etzhayyim.apps.openOt.recordTelemetryBatch` | pod-side ingest from Zenoh aggregator | array of `(signal_did, ts, value, quality)` |
| `com.etzhayyim.apps.openOt.reportFault` | fault with severity DMN | optional public notice |

Queries (read):

| NSID | Purpose |
|---|---|
| `com.etzhayyim.apps.openOt.getDevice` | device detail incl. attached cells |
| `com.etzhayyim.apps.openOt.getCell` | cell detail incl. pinned module + capabilities |
| `com.etzhayyim.apps.openOt.getLoop` | loop detail incl. cells + signals + setpoint |
| `com.etzhayyim.apps.openOt.listSignals` | by device / cell / since |
| `com.etzhayyim.apps.openOt.listLoops` | by device / status |
| `com.etzhayyim.apps.openOt.listFaults` | by device / loop / since / minSeverity |
| `com.etzhayyim.apps.openOt.listReadings` | telemetry by signal / since (paged) |

All NSIDs in `camelCase` per platform identifier convention.

**Status (2026-05-15)**: All 17 Lexicon JSON files authored under `00-contracts/lexicons/com/etzhayyim/apps/openOt/`. Bundle (`50-infra/cloudflare/workers/atproto/src/lexicon/bundled.ts`) and registry (`50-infra/cloudflare/workers/atproto/src/generated/lexicon-registry.gen.ts`, `10-protocol/xrpc/src/lexicon-types.gen.ts`) regenerated. **Wrangler deploy deferred** until the open-ot Worker has handlers — no point shipping a bundle whose new NSIDs have no responder. When implementation begins: `cd 50-infra/cloudflare/workers/atproto && npx wrangler deploy` (per CLAUDE.md root rule). All values use `integer` (no `number` per AT Lexicon float-prohibition); analog values are scaled to micro-units (1e-6) with UCUM `unitCode` separately; super-step rate uses `millihertz` (1000 = 1 Hz). Array-of-object always uses `items: { type: "ref", "ref": "#typeName" }` per the AT Lexicon validator.

## 3. Function-block API (Rust, Tier 1) — IEC 61499-compatible

Greenfield cells implement a typed Basic Function Block (BFB) trait targeting `wasm32-wasi`. The trait is a Rust projection of IEC 61499 BFB semantics: typed event inputs / outputs, typed data inputs / outputs, ECC (Execution Control Chart) state, and algorithms triggered by ECC transitions.

```
trait BasicFunctionBlock {
    type EventIn:  EventEnum;       // IEC 61499 event input variables
    type EventOut: EventEnum;       // IEC 61499 event output variables
    type DataIn:   TypedSignals;    // associated WITH event inputs
    type DataOut:  TypedSignals;    // associated WITH event outputs
    type EccState: Copy + Eq;       // ECC state enum (e.g. Idle / Running / Alarm)
    type Internal: LinearMemory;    // BFB internal vars, no GC, fixed-size
    type Params:   ConfigOnly;      // immutable after pinModule

    const INITIAL_STATE: Self::EccState;

    fn init(params: &Self::Params) -> Self::Internal;

    /// One IEC 61499 event tick == one Pregel super-step.
    /// Returns: next ECC state, emitted output events with their data,
    /// and any neighbor messages destined for sibling cells in the graph.
    fn tick(
        &mut self,
        event_in: Self::EventIn,
        data_in: &Self::DataIn,
        ecc_state: Self::EccState,
        internal: &mut Self::Internal,
        params: &Self::Params,
        super_step: u64,            // monotonic super-step id from orchestrator
    ) -> TickResult<Self::EccState, Self::EventOut, Self::DataOut>;
}

struct TickResult<S, E, D> {
    next_state: S,
    emitted: Vec<(E, D)>,            // event + associated data, fixed-cap Vec
    neighbor_msgs: Vec<NeighborMsg>, // typed Pregel messages to sibling cell DIDs
}
```

Composite Function Blocks (CFB — IEC 61499 networks of BFBs) are expressed at the **LangGraph graph level**, not as a Rust composition. A CFB === a LangGraph subgraph; this keeps the Pregel binding uniform and avoids two layers of FB composition.

Constraints enforced by the build pipeline:

- No `alloc` after `init`. All buffers sized at compile time. `emitted` and `neighbor_msgs` use fixed-capacity `heapless::Vec`.
- No `gc` feature, no `Box<dyn Trait>` in `tick` path.
- No `std::time` — `super_step` and any required wall time arrive as data inputs.
- No RNG — randomness arrives as a data input (replay-deterministic).
- Host imports limited to capabilities listed in the cell's `grantCapability` records.

Manifest (per-cell, analogue of `kotodama.jsonld`):

```jsonc
{
  "@context": "https://etzhayyim.com/ns/open-ot/cell/v1",
  "@id": "did:web:open-ot.etzhayyim.com:cell:pid-tank3-level",
  "module": { "cid": "bafy…", "signedBy": "did:web:builder.etzhayyim.com" },
  "fb_kind": "BFB",
  "iec61499_fbtype": "PID_LIMITED",
  "ecc": {
    "states": ["Idle", "Running", "Saturated", "Alarm"],
    "initial": "Idle",
    "transitions": [
      { "from": "Idle",      "event": "REQ", "guard": "enable",         "to": "Running" },
      { "from": "Running",   "event": "REQ", "guard": "out>=out_max",   "to": "Saturated" },
      { "from": "Running",   "event": "REQ", "guard": "pv_quality<good", "to": "Alarm" }
    ]
  },
  "cycle_period_ms": 100,
  "deadline_ms": 100,
  "events_in":  [{ "name": "REQ", "with": ["pv", "sp"] }],
  "events_out": [{ "name": "CNF", "with": ["cv"] }, { "name": "ALM", "with": [] }],
  "data_in":  [
    { "name": "pv", "signal": "did:web:open-ot.etzhayyim.com:signal:tank3-level-pv" },
    { "name": "sp", "signal": "did:web:open-ot.etzhayyim.com:signal:tank3-level-sp" }
  ],
  "data_out": [{ "name": "cv", "signal": "did:web:open-ot.etzhayyim.com:signal:tank3-level-cv" }],
  "params":  { "kp": 1.2, "ki": 0.05, "kd": 0.0, "out_min": 0.0, "out_max": 100.0 },
  "capabilities": ["io:tank3-level-pv:read", "io:tank3-level-sp:read", "io:tank3-level-cv:write"],
  "cohort": ["did:web:open-ot.etzhayyim.com:loop:tank3-level-control"]
}
```

Round-trip with **Eclipse 4diac IDE**: `4diac FBType XML ↔ open-ot manifest + Rust BFB skeleton`. etzhayyim-owned codegen lives in `70-tools/scripts/open-ot/fbtype-codegen/`; engineers can model graphically in 4diac, export FBType XML, and the codegen emits a Rust project ready to compile to `wasm32-wasi`.

## 4. LangGraph + Pregel binding

A control **loop** is a LangGraph graph; each **cell** in the loop is a Pregel node; one **super-step** is one IEC 61499 event tick across all cells in the loop. The orchestrator runs on the edge-controller tier (NixOS / Talos with PREEMPT_RT), not on the field device.

### 4.1 Mapping table

| LangGraph / Pregel | Open-OT | Storage |
|---|---|---|
| Graph definition (per ADR-2605082000) | `defineLoop` record with cell DIDs + edges | atproto `com.etzhayyim.apps.openOt.loop` |
| Pregel node | one cell DID | `vertex_open_ot_cell` |
| Pregel message | typed IEC 61499 event + data | Zenoh key `open-ot/{site}/{loop}/msg/{from_cell}→{to_cell}/{event}` |
| Super-step id | monotonic `super_step: u64` per loop | column in checkpoint row |
| Super-step barrier | TSN `802.1Qbv` gate event when present, else software barrier | — |
| Checkpoint (per ADR-2605082100) | `(loop_did, super_step, ecc_state[], internal[], in_flight_msgs[])` | `vertex_open_ot_loop_checkpoint` (RW) |
| Single-task / row-driven (per ADR-2605082200) | one signal change row → one task → one super-step on affected loop | `vertex_open_ot_signal_change` |
| Operator / agent intervention | `setpointChange` / `modeChange` injected as `EXT` event into next super-step | atproto record + checkpoint entry |
| Resume from failure | load latest `loop_checkpoint`, reissue in-flight msgs, continue | RW SELECT |

### 4.2 Determinism contract

A cell's `tick(event_in, data_in, ecc_state, internal, params, super_step)` is **a pure function** modulo `internal` mutation. All wall-clock side effects (timers, RNG, external I/O, sensor reads beyond `data_in`, neighbor reads beyond inbound messages) must enter as explicit data inputs supplied by the orchestrator.

Replay test: given a checkpoint stream `(super_step_n)_n`, replaying `tick` on each `(event_in, data_in, ecc_state, internal_pre, params)` MUST reproduce the same `(next_state, emitted, neighbor_msgs, internal_post)`. CI gate enforces this for all cells in `60-apps/etzhayyim-project-open-ot/cells/`.

### 4.3 Cycle-rate split

| Loop class | Field-device tick | Orchestrator super-step | Checkpoint cadence |
|---|---|---|---|
| Tight inner loop (PID, velocity, current) | 100 Hz–1 kHz | not in loop | none (state lives on device, snapshot at fault) |
| Process / sequence loop | 10 Hz | 1–10 Hz | every 1 s |
| Plant-coordination loop | event-driven | event-driven | every super-step |
| Fault / interlock | event-driven | every event | every super-step |

Tight inner loops execute **entirely on the field device** — the LangGraph orchestrator is **not** in the per-cycle critical path. The orchestrator participates at slower coordination rates (mode changes, setpoint cascades, fault propagation, multi-loop sequencing) and at every operator / agent interaction. This is the only configuration that keeps Pregel BSP overhead off the control path while still giving multi-loop coordination the checkpoint / resume / audit benefits.

### 4.4 Orchestrator placement

| Loop placement | Orchestrator host |
|---|---|
| Single device | LangGraph runs on the same NixOS / Talos edge controller; super-steps coordinate cells on the local Zephyr device(s) over Zenoh shm / UDP |
| Multi-device, single site | LangGraph on a site edge controller; Zenoh UDP across LAN; OPC UA FX bridge if cross-vendor devices participate |
| Multi-site | LangServer pod in VKE; site edge controllers act as Pregel workers; super-step barrier is software (no TSN across WAN) |

The orchestrator host **must** run CPython 3.11+ / Granian per ADR-2605080600. QNX, VxWorks, FreeRTOS hosts are excluded — they cannot run the LangGraph stack.

## 5. Distribution substrate

| Plane | Substrate | Topic / channel form | Latency target |
|---|---|---|---|
| Cohort data | Zenoh | `open-ot/{site}/{device}/{cell}/{signal}` | 100 µs–1 ms |
| Industrial interop | OPC UA FX over TSN | bridged 1:1 from Zenoh keys | 1–10 ms |
| Control plane | XRPC + MCP → atproto | NSIDs above | seconds (eventual) |
| Telemetry persist | Zenoh aggregator pod → XRPC `recordTelemetryBatch` → RW | batched per-second | seconds |

Wire encoding: CBOR for Zenoh payloads, AT Protocol Lexicon JSON for atproto records. (No `type: "number"` — see CLAUDE.md root rule on AT Lexicon float.)

## 6. RisingWave projection (read tier)

Vertices:

- `vertex_open_ot_device`, `vertex_open_ot_cell`, `vertex_open_ot_signal`, `vertex_open_ot_loop`, `vertex_open_ot_fault`

Pregel orchestrator state (per ADR-2605082100):

- `vertex_open_ot_loop_checkpoint` (`loop_did`, `super_step`, `ts`, `ecc_state[]`, `internal_blob[]`, `in_flight_msgs[]`, `params_rev`) — one row per super-step per loop; SSoT for resume / replay / audit.
- `vertex_open_ot_signal_change` (`signal_did`, `ts`, `value`, `quality`, `loop_did_affected[]`) — single-task / row-driven trigger source per ADR-2605082200.

Telemetry:

- `vertex_open_ot_reading` (`signal_did`, `ts`, `value_f64`, `quality`, `aggregator_did`) — high-volume; bulk ingest uses `SET dml_rate_limit` per CLAUDE.md `[[conventions]] rw-bulk-insert-throttle`.

Pre-computed MVs (streaming):

- `mv_open_ot_loop_health` (per-loop deadline-miss rate, last 1m / 5m / 1h)
- `mv_open_ot_signal_last` (latest reading per signal)
- `mv_open_ot_fault_active` (open faults by severity / device)
- `mv_open_ot_loop_super_step_latency` (per-loop super-step duration p50/p95/p99 — observability for the Pregel binding)

Archive: long-tail readings to Iceberg on B2 per ADR-0048.

## 7. Capability model

Host imports a cell may request, granted via `grantCapability`:

| Capability | Form | Notes |
|---|---|---|
| `io:{signal_did}:read` | read latest value + quality | scoped per signal DID |
| `io:{signal_did}:write` | publish setpoint / output | requires interlock DMN pass |
| `peer:{cell_did}:msg` | send typed message to neighbor cell | Zenoh shm in same device, UDP across |
| `time:cycle` | receive `cycle_time_us` from host | always granted |
| `log:event` | emit structured log to host audit | always granted |

No file system, no network beyond explicit `peer:` and `io:`, no clock other than `time:cycle`. Sandboxing matches IEC 62443 SL-2 expectations.

## 8. Safety boundary

- Cells declared with `safety_class: none` (default) run normally.
- Cells declared with `safety_class: safety_observed` may **read** safety-rated signals from a co-located certified safety PLC but cannot write to safety I/O.
- Cells with `safety_class: sif` are **rejected at `defineCell`** until IEC 61508 toolchain qualification is in place (post-Risk-1 resolution).

## 9. Build / sign / pin pipeline

1. Author Rust FB → `cargo build --target wasm32-wasi`.
2. AOT compile on trusted builder: `wamrc --target=thumbv7em --enable-sgx=false --opt-level=3 cell.wasm -o cell.aot`.
3. Sign artefact with builder DID; upload to content-addressed store (B2 / IPFS); receive CID.
4. `pinModule` — atproto record links cell DID → CID + signature.
5. Edge device pulls CID over XRPC, verifies signature against builder DID resolved from atproto, loads via WAMR.
6. Hot-swap is **two-phase**: new cell instance comes up in shadow mode (compute outputs, do not publish), atomic swap on operator confirm via `modeChange`.

## 10. Open spec questions

- Time-source contract for cells (PTP / NTP / TSN gPTP?). Likely defer to per-deployment TSN profile. Required for §4.1 super-step barrier when no TSN gate is available.
- Cell-to-cell cross-device messaging schema — typed Zenoh keys vs. component-model resources. Bias: typed Zenoh keys at MVP; revisit when WASI-P2 components stabilize on embedded.
- Engineering surface — Beremiz fork? Svelte editor under `60-apps/etzhayyim-project-open-ot/svelte/`? **Resolved: 4diac IDE as primary** (per §3); Svelte editor for loop-level (LangGraph graph) composition and HMI is still open.
- Telemetry schema versioning — Lexicon revision strategy when EU range or quality codes evolve. Need ADR.
- Checkpoint compaction — how often to compact `vertex_open_ot_loop_checkpoint` history; balance audit retention vs. RW storage cost.
- Single-task scheduler tuning — whether one row in `vertex_open_ot_signal_change` triggers exactly one super-step, or coalesces signal changes within a debounce window. Bias: per-loop debounce config in `defineLoop`.

## 11. Out of scope (explicit, MVP)

- IEC 61508 / 61511 functional safety certification.
- Hard-RT motion / servo loops inside WASM.
- Centralized HMI replacing CENTUM Vnet/IP screens.
- Wireless field protocols (WirelessHART, ISA100.11a) — treat as future PLC4X driver work.
- Historian replacement at petabyte scale — Iceberg + RW MVs cover the read tier; long-term plant historian (PI System scale) is a separate project.

## 12. Hardware reference: Giemon Mimi / Te / Atama

Per ADR §R1. Three reference boards under the Giemon brand (ADR-2605142200), body-part naming (mimi=ear / te=hand / atama=head) consistent with `etzhayyim-project-open-robo` (Giemon Otete).

### 12.1 Giemon Mimi (耳) — sensor RTU

| Parameter | Value |
|---|---|
| SoC | STMicro STM32H753ZIT6, Cortex-M7 @ 480 MHz, 2 MB flash, 1 MB SRAM |
| OS / runtime | Zephyr LTS 4.x + WAMR AOT (LLVM 18, `-O3`, no GC) + Zenoh-Pico |
| Power | 24 V DC industrial, isolated DC-DC, max 6 W |
| Digital input | 16 ch × 24 V, opto-isolated, software-debounce, surge-protected |
| Analog input | 8 ch × 4–20 mA, 16-bit ADS1118, 4 kSPS, individually isolated |
| Serial | 2 × RS-485 (Modbus RTU master/slave), galvanic isolation |
| Network | 1 × 100BASE-T1 (TSN-capable, Marvell PHY) + 1 × 100BASE-TX fallback |
| Storage | 8 MB QSPI flash for AOT artefacts + manifest cache |
| Mechanical | DIN-rail, 35 mm wide, IP20, –20 °C to +60 °C |
| Cells per board | up to 8 BFB instances, 1 ms cycle each |
| Cost target | JPY 28,000 BOM |

### 12.2 Giemon Te (手) — actuator RTU

| Parameter | Value |
|---|---|
| SoC | NXP i.MX RT1170 dual-core (Cortex-M7 @ 1 GHz + Cortex-M4 @ 400 MHz), 2 MB on-die SRAM, 16 MB external HyperRAM |
| OS / runtime (M7) | Zephyr LTS + WAMR AOT + Zenoh-Pico (soft-RT cells) |
| OS / runtime (M4) | bare-metal C (hard-RT motion / safety interlock) — see open question on second WAMR |
| Power | 24 V DC industrial, max 12 W |
| Digital output | 8 ch × 24 V, sourcing, 1 A per ch, short-circuit + thermal protected |
| Analog output | 4 ch × 0–10 V, 12-bit DAC, 5 kSPS |
| CAN | 2 × CAN-FD (motor drive / safety bus), galvanic isolation |
| Network | 1 × 100BASE-T1 (TSN) + 1 × 100BASE-TX |
| Storage | 16 MB QSPI flash |
| Mechanical | DIN-rail, 45 mm wide, IP20 |
| Cells per board (M7) | up to 8 BFB instances, 1 ms cycle each |
| Cost target | JPY 38,000 BOM |

### 12.3 Giemon Atama (頭) — edge controller

| Parameter | Value |
|---|---|
| SoC | Rockchip RK3588, ARMv8.2, 4× Cortex-A76 @ 2.4 GHz + 4× Cortex-A55 @ 1.8 GHz |
| Memory | 16 GB LPDDR5 |
| Storage | 128 GB eMMC + M.2 NVMe slot |
| OS | NixOS 25.05 + linuxPackages_rt (PREEMPT_RT 6.6 LTS) |
| Runtime | CPython 3.11+ / Granian / LangGraph (per ADR-2605080600) + Wasmtime sidecar |
| Persistence | RisingWave checkpointer client (per ADR-2605082100) over Hyperdrive to RW Vultr |
| Network | 1 × 2.5 GbE WAN, 4-port TSN switch (802.1Qbv) — Marvell 88Q5050 default; Microchip LAN9692 / NXP SJA1110 alternates |
| OPC UA FX | bridge sidecar exposing local Zenoh keys as FX subscriptions |
| Power | 24 V DC + UPS input (Panasonic 18650 pack, 30 min hold-up) |
| Mechanical | DIN-rail or 1U rack adapter, fanless ≤ 25 W TDP |
| Loops per controller | 4–8 (rate-dependent, see SPEC §4.3) |
| Cost target | JPY 95,000 BOM |

### 12.4 Sourcing convention

JP-domestic per the open-robo precedent: Misumi / Meviy structure, IDEC / Bosch Rexroth switching gear, ROHM / Toshiba driver ICs, Murata / TDK passives, Panasonic 18650 cells, JP-fab PCB (P-ban / Suntsu). SoCs (STM32, i.MX RT, RK3588) are imported but soldered on JP boards.

### 12.5 Repo layout (post-Risk-1 hardware spin)

```
60-apps/etzhayyim-project-open-ot/
├── cad-spec/
│   ├── giemon-mimi/{schematic.kicad_sch, pcb.kicad_pcb, BOM.md}
│   ├── giemon-te/{...}
│   └── giemon-atama/{...}
├── firmware/
│   ├── mimi-zephyr/    # Zephyr west workspace, board overlay, WAMR config
│   └── te-zephyr/
└── nixos/
    └── atama/          # NixOS module + linuxPackages_rt + LangGraph service
```

Created only after Risk-1 PASS.

## 13. Prototype: community microgrid

Per ADR §R3. First production-class deployment — 100 kW–10 MW class community microgrid.

### 13.1 Cross-link with `etzhayyim-project-open-denki`

Configuration SSoT lives in open-denki. open-ot adds **control verbs** on top:

| open-denki (already MVP) | open-ot (new) |
|---|---|
| `defineGenerationNode` (PV / wind / BESS / generator) | `defineCell` per inverter / PCS controller |
| `defineSubstation` / `defineFeeder` | `defineLoop` per voltage / frequency control loop |
| `registerSmartMeter`, `recordMeterReading` | `recordTelemetryBatch` (1 Hz aggregated) |
| `recordRenewableOutput` | (consumed, not duplicated) |
| `recordDemandResponse` | `setpointChange` writes to inverter / BESS setpoints in response |
| `reportFault` (CIM) | `reportFault` (control-side, links to open-denki fault by `cim_fault_did`) |

Implementation mode: `60-apps/etzhayyim-project-open-ot/PROTOTYPE-MICROGRID.md` written at Risk-1 PASS captures the per-asset cell DIDs, loop DIDs, and 4diac FBType library used.

### 13.2 Loop catalogue (target)

| Loop DID stub | Cells | Rate | Purpose |
|---|---|---|---|
| `:loop:pv-array-mppt-{id}` | inverter MPPT BFB | 100 Hz on Mimi | Maximum-power-point tracking — field-only |
| `:loop:bess-charge-discharge` | BESS PCS BFB + SoC estimator BFB | 10 Hz on Te + 1 Hz orchestrator | Charge / discharge schedule |
| `:loop:freq-droop` | inverter droop BFBs (per asset) + aggregator | 10 Hz field + 1 Hz orchestrator | Frequency support, P-f droop |
| `:loop:volt-var` | inverter Q control BFBs + LTC tap BFB | 10 Hz field + 1 Hz orchestrator | Voltage-VAR support |
| `:loop:islanding-decision` | grid-tie protection BFB + sequence cell | event-driven | Anti-islanding + black-start sequence |
| `:loop:dr-response` | DR setpoint distribution cell | event-driven | open-denki `recordDemandResponse` event → setpoint cascade |
| `:loop:peak-shave-economic` | LangGraph-only (no field BFB) | 1 Hz | Economic dispatch across BESS + PV curtailment + import limit |

`peak-shave-economic` is intentionally orchestrator-only — it consumes telemetry, computes setpoints, and emits `setpointChange` events to per-asset loops. It demonstrates the LangGraph/Pregel value at the multi-loop coordination layer without contaminating the inner control loops.

### 13.3 Acceptance for the prototype (separate from Risk-1)

Run for 90 days at a partner site:

- Zero unplanned islanding.
- ≥ 99 % uptime of the orchestrator (NixOS + LangGraph) measured at the XRPC `getLoop` endpoint.
- ≥ 95 % of `setpointChange` events landing within their declared deadline (1 s for orchestrator-issued, 100 ms for islanding sequence).
- Audit trail (`vertex_open_ot_loop_checkpoint` stream) reconstructable to per-second granularity for any 24 h window in the 90 days, with no gaps > 5 s.

Pilot site selection deferred — candidates: a university campus microgrid, a small industrial site with rooftop PV + diesel backup, or a remote island grid (Okinawa / Ogasawara). Selection drives 802.1Qbv TSN profile and NTP/PTP source.

## 14. Risk-1 acceptance test plan

Per ADR §R4. Reference test rigs and pass/fail thresholds, restated for engineering use.

### 14.1 Gate A — WAMR AOT WCET on representative PID loop

| Parameter | Value |
|---|---|
| Hardware | Giemon Mimi prototype board (STM32H753 @ 480 MHz) |
| OS | Zephyr LTS 4.x, single-core, SCHED_FIFO control thread @ priority -10 |
| WASM toolchain | wasi-sdk → `wasm32-wasi` → `wamrc -O3 --opt-level=3 --enable-aot --no-gc` (LLVM 18) |
| Workload | typed BFB: 100 DataIn signals, 100 DataOut signals, ECC with 4 states, internal PID computation |
| Cycle | 1 ms |
| Duration | 10 hours continuous |
| Instrumentation | DWT cycle counter at tick entry / exit; histogram bucketed at 1 µs |
| **PASS** | p99.9 tick latency ≤ 200 µs, zero deadline misses, observed heap delta = 0 bytes after `init` |
| **FAIL** | any deadline miss, p99.9 > 500 µs, or any heap growth after `init` |
| Artefact | `60-apps/etzhayyim-project-open-ot/risk1/gate-a-report.md` |

### 14.2 Gate B — Pregel super-step latency end-to-end

| Parameter | Value |
|---|---|
| Hardware | 3 × Giemon Atama prototype + 12 × Giemon Mimi/Te prototype |
| Network | TSN switch (Marvell 88Q5050), 802.1Qbv schedule with 1 ms gate window |
| Loop | 12-cell `:loop:freq-droop` analogue across the 12 field devices |
| Super-step rate | 1 Hz (orchestrator), 100 Hz (field-only inner loops) |
| Duration | 24 hours continuous |
| Fault injection | 1 controller crash (kill -9 LangGraph at random t), 3 device crashes (power-cycle Mimi/Te at random t) |
| **PASS** | super-step duration p99 ≤ 50 ms; checkpoint write p99 ≤ 100 ms; zero in-flight message loss across crashes; resume-from-checkpoint within 5 s of controller restart |
| **FAIL** | any in-flight message loss, p99 super-step > 200 ms, or resume > 30 s |
| Artefact | `60-apps/etzhayyim-project-open-ot/risk1/gate-b-report.md` |

### 14.3 Gate C — Toolchain qualification cost estimate

| Parameter | Value |
|---|---|
| Deliverable | written estimate covering: WAMR AOT compiler, LLVM 18 dependency mapping, Rust FB framework memory-safety claims, Zephyr LTS vendor safety package reuse, signing / pinning workflow, IEC 62443-3-3 SL-2 requirements mapping |
| Reviewers | external industrial-cyber consultant + internal review |
| **PASS** | estimated effort ≤ 6 person-months and no LLVM-side blocker identified |
| **FAIL** | structural blocker (e.g., LLVM versioning policy incompatible with cyber-cert requirements) or estimate > 12 person-months |
| Artefact | `60-apps/etzhayyim-project-open-ot/risk1/gate-c-report.md` |

### 14.4 Gate-failure decision matrix

Reproduced from the ADR for reference; the ADR is authoritative.

| A | B | C | Outcome |
|---|---|---|---|
| PASS | PASS | PASS | Promote to MVP build, commission Mimi/Te/Atama Rev-1, start microgrid pilot, begin Svelte editor |
| PASS | PASS | FAIL | Promote runtime, defer industrial-cyber-cert path, ship community-microgrid + research-only positioning |
| PASS | FAIL | * | Re-architect orchestrator (lower super-step rate, alternative checkpointer); rerun Gate B |
| FAIL | * | * | Re-evaluate WAMR AOT vs. Wasmtime-on-RT-Linux on a single Atama (no Mimi/Te separation); rerun Gate A |
