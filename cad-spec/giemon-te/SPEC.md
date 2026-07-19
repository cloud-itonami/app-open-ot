# Giemon Te (手) — Actuator RTU SPEC v0.1

**Status**: spec only (2026-05-15). KiCad sources begin post-Risk-1 PASS.

Actuator-side RTU with dual-core SoC. M7 hosts soft-RT BFB cells (24 V DO, 0–10 V AO, CAN-FD). M4 reserved for hard-RT motion / safety interlock — see open question on whether M4 hosts a separate WAMR instance or stays bare-metal C.

## 1. Functional block diagram

```
                         ┌────────────────────────────────────────────┐
                         │  Giemon Te (手)                            │
                         │  NXP i.MX RT1170 (M7@1GHz + M4@400MHz)     │
                         │                                            │
                         │  ┌── M7 ────────────────────────────────┐  │
24 V DO × 8  ◄──[FET]────┤  │ Zephyr LTS + WAMR AOT + Zenoh-Pico   │  │
   (sourcing)            │  │  - up to 8 BFB cells, 1 ms cycle     │  │
0–10 V AO × 4 ◄──[DAC]───┤  │  - actuator scope: DO/AO/CAN write   │  │
                         │  └──────────────────────────────────────┘  │
                         │  ┌── M4 ────────────────────────────────┐  │
CAN-FD × 2 ◄──[ISO]──────┤  │ bare-metal C (option: WAMR for hard-RT)  │
   (motor / safety)      │  │  - hard-RT motion / safety interlock │  │
                         │  └──────────────────────────────────────┘  │
                         │  ┌── PHY ───────────────────────────────┐  │
                         │  │ 100BASE-T1 (TSN) + 100BASE-TX        │  │
                         │  └──────────────────────────────────────┘  │
                         └────────────────────────────────────────────┘
                                  │ 100BASE-T1 (TSN)        │ 100BASE-TX
                                  ▼                          ▼
                              Site fabric / Giemon Atama
```

## 2. BOM v0.1 (target JPY 38,000)

| Ref | Part | Function | Qty | Source | Origin |
|---|---|---|---|---|---|
| U1 | NXP MIMXRT1176DVMAA (LFBGA289) | Dual-core SoC, M7@1 GHz + M4@400 MHz, 2 MB SRAM | 1 | NXP (via Avnet JP) | NL/US (chip), JP (board) |
| U2 | ISSI IS66WVH16M8DBLL (HyperRAM 16 MB) | External XIP/data RAM | 1 | ISSI | US |
| U3 | TI DAC8554 × 1 | 4-ch 12-bit DAC, 0–10 V via opamp | 1 | TI | US |
| U4 | TI OPA2197 × 2 | DAC output buffer, ±36 V rail | 2 | TI | US |
| U5 | TI CSD17313Q2 × 8 | 24 V DO sourcing FET, 1 A, OCP/OTP | 8 | TI | US |
| U6 | NXP TJA1463 × 2 | CAN-FD transceiver, isolated | 2 | NXP | NL |
| U7 | Marvell 88Q2112 | 100BASE-T1 PHY | 1 | Marvell | US |
| U8 | TI DP83825I | 100BASE-TX PHY (fallback) | 1 | TI | US |
| U9 | Murata MEU2S2415SC × 2 | 24 V isolated DC-DC, 2 W (DO + DAC) | 2 | Murata Mfg | JP |
| F1 | TVS array × 8 | Surge protection on DO terminals | 8 | ROHM | JP |
| Q* | discrete JFET / clamp | 0–10 V AO output protection | 12 | ROHM / Toshiba | JP |
| C* | Murata GRM ceramic | Decoupling + PSU bulk | ~150 | Murata Mfg | JP |
| R* | KOA precision thin-film | Sense + DAC reference | ~100 | KOA | JP |
| J1 | Phoenix Contact MC 1.5 (8 pos) | 24 V DO terminals + common | 1 | Phoenix Contact | DE/JP |
| J2 | Phoenix Contact MC 1.5 (5 pos) | AO 0–3 + AGND | 1 | Phoenix Contact | DE/JP |
| J3 | Phoenix Contact MC 1.5 (4 pos) | CAN-A: H, L, GND, shield | 1 | Phoenix Contact | DE/JP |
| J4 | Phoenix Contact MC 1.5 (4 pos) | CAN-B: H, L, GND, shield | 1 | Phoenix Contact | DE/JP |
| J5 | Hirose RJ45 + magnetics | Ethernet (TSN + TX) | 1 | Hirose | JP |
| J6 | Phoenix Contact MC 1.5 (3 pos) | 24 V DC supply + GND + earth | 1 | Phoenix Contact | DE/JP |
| | 6-layer FR4 PCB, 110×80 mm, ENIG | | 1 | P-ban Suntsu | JP |
| | DIN-rail clip + Polycarbonate enclosure (45 mm wide) | | 1 | Misumi standard | JP |

## 3. Pin assignment (preliminary)

### M7 core ownership

| GPIO bank | Use | Count |
|---|---|---|
| GPIO_AD_00–07  | LPSPI for DAC8554 (CS, MOSI, MISO, SCK) + DAC LDAC | 6 |
| GPIO_AD_08–15  | 24 V DO 0–7 gate drives via CSD17313Q2 | 8 |
| GPIO_DISP_00–11 | RGMII to 88Q2112 (TSN PHY) | 12 |
| GPIO_DISP_12–13 | RMII to DP83825I (fallback PHY) | 8 |
| GPIO_LPSR_00–07 | M7↔M4 IPC mailbox + status LEDs | 8 |

### M4 core ownership (hard-RT)

| GPIO bank | Use | Count |
|---|---|---|
| GPIO_EMC_B1_00–03 | CAN-FD-A (CAN1) TX/RX + EN/STB | 4 |
| GPIO_EMC_B1_04–07 | CAN-FD-B (CAN2) TX/RX + EN/STB | 4 |
| GPIO_EMC_B2_00–03 | safety interlock inputs (4 lines, 24 V opto via shared ISO1212 on Mimi-style sub-board) | 4 |
| GPIO_EMC_B2_04   | watchdog stroke (independent of M7 watchdog) | 1 |

### External terminal map

| Terminal | Function | Range | Isolation |
|---|---|---|---|
| J1 (8 pos) | DO 0–7 + DO common | 24 V sourcing, 1 A/ch, OCP/OTP | 1.5 kV per ch |
| J2 (5 pos) | AO 0–3 + AGND | 0–10 V, 12-bit, ±0.5 % FS | 250 V per ch |
| J3 (4 pos) | CAN-FD-A (motor drive) | 5 Mbps, ISO 11898-2 | 2.5 kV |
| J4 (4 pos) | CAN-FD-B (safety) | 5 Mbps, ISO 26262 ASIL-B-friendly transceiver | 2.5 kV |
| J5 (RJ45)  | TSN + TX Ethernet | per IEEE | per IEEE |
| J6 (3 pos) | 24 V DC supply | 18–36 V tolerant | — |

## 4. Software stack contract

| Core | Stack |
|---|---|
| M7 | Zephyr LTS 4.x + WAMR AOT + Zenoh-Pico — soft-RT BFB cells (1 ms cycle, ≤100 µs jitter) |
| M4 | bare-metal C (default) — hard-RT motion / safety interlock at 100 µs cycle. Optional second WAMR instance behind a build flag (carry-over open question per CLAUDE.md) |

IPC: NXP RPMsg-Lite mailbox between M7 and M4. M4 publishes telemetry to M7's Zenoh aggregator at ≤ 100 Hz.

## 5. Engineering targets

- M7 WCET (1 BFB tick @ 1 ms cycle): ≤ 200 µs p99.9 — Risk-1 Gate A
- M4 hard-RT loop deadline: 100 µs, jitter ≤ 5 µs
- Power: ≤ 12 W typical, ≤ 18 W transient (all DO sourcing 1 A simultaneous)
- MTBF target: 150,000 h @ 40 °C
- DO output: short-circuit, over-temperature, reverse-polarity protected; CSD17313Q2 OC trip ≤ 3 A
- AO output: floating differential, 12-bit, monotonic, ±0.5 % full-scale accuracy at 25 °C
