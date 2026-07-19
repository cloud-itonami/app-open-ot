# Giemon Atama (頭) — Edge Controller SPEC v0.1

**Status**: spec only (2026-05-15). KiCad / NixOS module work begins post-Risk-1 PASS.

Edge controller. Hosts the LangGraph Pregel orchestrator (CPython 3.11+ / Granian per ADR-2605080600), the RisingWave checkpointer client (per ADR-2605082100), and the OPC UA FX ↔ Zenoh bridge. Acts as TSN switch for the local Mimi/Te fabric.

## 1. Functional block diagram

```
   2.5 GbE WAN ──────┐
                     ▼
          ┌─────────────────────────────────────────────────────┐
          │  Giemon Atama (頭)                                  │
          │  Rockchip RK3588 (4×A76@2.4 GHz + 4×A55@1.8 GHz)    │
          │  16 GB LPDDR5 + 128 GB eMMC                         │
          │                                                     │
          │  ┌── NixOS + linuxPackages_rt ───────────────────┐  │
          │  │ ┌── User-space ────────────────────────────┐  │  │
          │  │ │ CPython / Granian / LangGraph (Pregel)   │  │  │
          │  │ │ Wasmtime sidecar (tier-2 cells)          │  │  │
          │  │ │ Zenoh aggregator + OPC UA FX bridge      │  │  │
          │  │ │ RisingWave checkpointer client (asyncpg) │  │  │
          │  │ └──────────────────────────────────────────┘  │  │
          │  │ ┌── Kernel ────────────────────────────────┐  │  │
          │  │ │ PREEMPT_RT 6.6 LTS, isolcpus=4-7,        │  │  │
          │  │ │ irqaffinity, full_nohz, mlockall         │  │  │
          │  │ └──────────────────────────────────────────┘  │  │
          │  └──────────────────────────────────────────────┘  │
          │                                                     │
          │  ┌── 4-port TSN switch (802.1Qbv) ───────────────┐  │
          │  │ Marvell 88Q5050 (default) /                    │  │
          │  │ Microchip LAN9692 / NXP SJA1110 (alt)          │  │
          │  └──────────────────────────────────────────────┘  │
          └─────────────────────────────────────────────────────┘
              │ Port 0   │ Port 1   │ Port 2   │ Port 3
              ▼          ▼          ▼          ▼
          Mimi/Te × N  Mimi/Te × N  Mimi/Te × N  Mimi/Te × N
                                                        OR
                                          OPC UA FX peer (cross-vendor)
```

## 2. BOM v0.1 (target JPY 95,000)

| Ref | Part | Function | Qty | Source | Origin |
|---|---|---|---|---|---|
| U1 | Rockchip RK3588 (FCBGA1144) | 8-core ARMv8.2 SoC, Mali-G610 GPU | 1 | Rockchip (via Avnet) | CN (chip), JP (board) |
| U2 | Samsung K3KL9L90DM-MGCT × 4 | LPDDR5 4 GB ×4 = 16 GB | 4 | Samsung | KR |
| U3 | Sandisk SDIN8DE4-128G | eMMC 5.1 128 GB | 1 | Sandisk | US/JP |
| U4 | M.2 2280 NVMe socket | optional NVMe expansion | 1 | Hirose | JP |
| U5 | TSN switch SoC: Marvell 88Q5050 (default) | 4 × 1G-T1 + 1 × 2.5G-T (host) | 1 | Marvell | US |
|   | OR Microchip LAN9692 (alt 1) | same class, 5-port TSN | 1 | Microchip | US |
|   | OR NXP SJA1110 (alt 2) | 11-port TSN switch | 1 | NXP | NL |
| U6 | Microchip KSZ9477RTX | 2.5GbE WAN PHY | 1 | Microchip | US |
| U7 | TI TPS65219 | PMIC for SoC rails | 1 | TI | US |
| U8 | Murata 12 V → 24 V DC-DC + Panasonic 18650 × 4 | UPS hold-up (≥ 30 min @ idle) | — | Murata Mfg + Panasonic | JP |
| U9 | TI BQ40Z80 | UPS battery management | 1 | TI | US |
| U10 | Microchip ATSAMD20 (sidecar) | independent watchdog + PoE/UPS supervisor | 1 | Microchip | US |
| C* | Murata GRM ceramic | Decoupling | ~300 | Murata Mfg | JP |
| L* | TDK / Coilcraft inductors | DC-DC + filter | ~40 | TDK / Coilcraft | JP/US |
| J1 | Hirose RJ45 × 4 | TSN ports 0–3 to field | 4 | Hirose | JP |
| J2 | Hirose RJ45 × 1 | 2.5 GbE WAN | 1 | Hirose | JP |
| J3 | Phoenix Contact (4 pos) | 24 V DC + UPS-OK signal | 1 | Phoenix Contact | DE/JP |
| J4 | USB-C (debug + power-in fallback) | USB 2.0 + 100 W PD | 1 | Hirose | JP |
| J5 | M.2 NVMe slot | storage expansion | 1 | Hirose | JP |
| J6 | μSD card socket | recovery / boot media | 1 | Hirose | JP |
| | 8-layer FR4 PCB, 160×120 mm, ENIG | | 1 | P-ban Suntsu | JP |
| | DIN-rail (96 mm wide) + 1U rack adapter, fanless aluminium chassis | | 1 | Misumi + Meviy custom | JP |

## 3. Connectivity matrix

| Function | Interface | Notes |
|---|---|---|
| Field fabric | 4 × 100/1000 BASE-T1 (TSN), 802.1Qbv | Marvell 88Q5050 default; Microchip LAN9692 / NXP SJA1110 alternates pending Q3 supply quotes |
| WAN | 1 × 2.5 GbE (RJ45) | XRPC ↔ AgentGateway MCP ↔ atproto (per ADR-2605091400) |
| OPC UA FX | runs as sidecar pod inside NixOS | binds to TSN ports 0–3 |
| Storage | 128 GB eMMC + optional NVMe | NixOS root + RW checkpointer client cache |
| UPS | Panasonic 18650 × 4 (12 V pack) | ≥ 30 min idle hold-up; graceful shutdown on `UPS-LOW` |
| Time sync | PTP IEEE 1588v2 + 802.1AS gPTP | TSN switch is grandmaster candidate; falls back to NTP from WAN |

## 4. Software stack contract

| Layer | Component | Notes |
|---|---|---|
| OS | NixOS 25.05 + linuxPackages_rt (PREEMPT_RT 6.6 LTS) | Declarative config under `60-apps/etzhayyim-project-open-ot/nixos/atama/` post-Risk-1 |
| Runtime | CPython 3.11+ / Granian / LangGraph + Wasmtime | Pregel orchestrator + tier-2 cells |
| Persistence | RisingWave checkpointer client (asyncpg, SQLAlchemy Core per ADR-2605080300) | Writes `vertex_open_ot_loop_checkpoint` |
| Substrate | Zenoh router (zenohd) | Aggregates field Zenoh-Pico publishers; exposes OPC UA FX bridge |
| Bridge | OPC UA FX gateway (open62541-based) | Cross-vendor interop |

## 5. Engineering targets

- Pregel super-step end-to-end latency p99 ≤ 50 ms (Gate B)
- Checkpoint write p99 ≤ 100 ms (Gate B)
- cyclictest jitter @ 1 kHz, isolated A76 cores: ≤ 30 µs worst-case
- Power: ≤ 25 W TDP, fanless under 40 °C ambient
- UPS hold-up: ≥ 30 min idle, ≥ 5 min full load
- Resume from checkpoint: ≤ 5 s after controller restart (Gate B)

## 6. Open hardware decisions (carry-over)

- TSN switch silicon — Marvell 88Q5050 vs Microchip LAN9692 vs NXP SJA1110, decision pending Q3 2026 supply quotes (per ADR §Open questions).
