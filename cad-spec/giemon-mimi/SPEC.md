# Giemon Mimi (耳) — Sensor RTU SPEC v0.1

**Status**: spec only (2026-05-15). KiCad sources begin post-Risk-1 PASS.

Sensor-side Remote Terminal Unit. Listens to the field — opto-isolated 24 V DI, 4–20 mA AI, RS-485 Modbus RTU — and republishes sample-rate-clean signals onto Zenoh. Hosts up to 8 BFB cells at 1 ms cycle (per ADR-2605151200 §R1).

## 1. Functional block diagram

```
                         ┌──────────────────────────────────────┐
                         │  Giemon Mimi (耳)                    │
                         │  STM32H753ZIT6 @ 480 MHz             │
                         │  Zephyr LTS + WAMR AOT + Zenoh-Pico  │
24 V DI × 16  ───┐       │  ┌────────────────────────────────┐  │
   (opto)       │       │  │ App layer: BFB cells           │  │
                ├─[ISO]─┤  │  - up to 8 instances           │  │
4–20 mA AI × 8 ─┤       │  │  - 1 ms scan, ≤100 µs jitter   │  │
   (per-ch iso) │       │  │  - ECC + tick + heapless       │  │
                ├─[ADC]─┤  └────────────────────────────────┘  │
RS-485 × 2 ─────┤       │  ┌────────────────────────────────┐  │
   (Modbus RTU) │       │  │ HAL: Zenoh-Pico, Modbus, audit │  │
                │       │  └────────────────────────────────┘  │
                │       │  ┌────────────────────────────────┐  │
                │       │  │ TSN PHY: 100BASE-T1 + fallback │  │
                │       │  └────────────────────────────────┘  │
                │       └──────────────────────────────────────┘
                │                 │ 100BASE-T1 (TSN) │ 100BASE-TX
                │                 ▼                  ▼
                │             Site fabric / Giemon Atama
                ▼
         24 V isolated DC-DC, surge clamp
```

## 2. BOM v0.1 (target JPY 28,000)

| Ref | Part | Function | Qty | Source | Origin |
|---|---|---|---|---|---|
| U1 | STM32H753ZIT6 (LQFP144) | MCU, Cortex-M7 @ 480 MHz, 2 MB flash, 1 MB SRAM | 1 | STMicro (via Murata Mfg dist) | EU/CH (chip), JP (board) |
| U2 | TI ADS1118 × 2 | 16-bit ΔΣ ADC, 4 kSPS | 2 | TI (via Tokyo Electron Device) | US (chip) |
| U3 | TI ISO1212 × 4 | 24 V DI digital isolator | 4 | TI | US |
| U4 | Murata MEU1S2415SC | 24 V isolated DC-DC, 1 W | 1 | Murata Mfg | JP |
| U5 | Linear LTC1480 × 2 | RS-485 transceiver, isolated | 2 | ADI / Linear | US |
| U6 | Marvell 88Q2112 | 100BASE-T1 PHY (TSN-capable) | 1 | Marvell | US |
| U7 | TI DP83825I | 100BASE-TX PHY (fallback) | 1 | TI | US |
| F1 | TVS array × 8 | Surge protection on 24 V DI | 8 | ROHM | JP |
| C* | Murata GRM ceramic | Decoupling + PSU bulk | ~120 | Murata Mfg | JP |
| R* | KOA / Susumu thin-film | Precision sense + dividers | ~80 | KOA / Susumu | JP |
| Q* | TI CSD18540 × 8 | Side-driver / DI level shift FETs | 8 | TI | US |
| J1–J6 | Phoenix Contact MC 1.5 | 24 V industrial pluggable terminals | 6 | Phoenix Contact / IDEC dist | DE (part) JP (dist) |
| J7 | Hirose RJ45 + magnetics | Ethernet + 100BASE-T1 (single connector with adapter) | 1 | Hirose | JP |
| | 4-layer FR4 PCB, 100×80 mm, ENIG, IPC class 3 | | 1 | P-ban Suntsu (Suntsu Elec) | JP |
| | DIN-rail clip + Polycarbonate enclosure (35 mm wide) | | 1 | Misumi standard | JP |

JP-fab assembly via Suntsu (KIT) or P-ban; SoC + PHY + DC-DC are imported but soldered on JP boards per the open-robo precedent.

## 3. Pin assignment (preliminary)

### MCU (STM32H753ZIT6) bank summary

| GPIO bank | Use | Count |
|---|---|---|
| PA0–PA7  | RS-485-A: USART2 (TX/RX/DE/RE), Modbus RTU master | 4 |
| PA8–PA15 | 100BASE-TX RMII pins to DP83825I | 8 |
| PB0–PB15 | 24 V DI 0–7 (via ISO1212 #1, #2) | 16 lines (8 input lanes + 8 status) |
| PC0–PC15 | 24 V DI 8–15 (via ISO1212 #3, #4) | 16 lines |
| PD0–PD7  | RS-485-B: USART3, Modbus RTU master/slave | 4 |
| PD8–PD15 | TSN MII pins to 88Q2112 | 8 |
| PE0–PE7  | ADS1118 #1: SPI1 + DRDY + nCS | 6 |
| PE8–PE15 | ADS1118 #2: SPI4 + DRDY + nCS | 6 |
| PF0–PF7  | Status LEDs (per-cell run / fault), watchdog stroke | 8 |
| PG0–PG15 | reserved for QSPI flash + debug headers | 16 |
| PH0–PH1  | 32.768 kHz LSE (RTC + PTP) | 2 |

### External terminal map (J1–J6)

| Terminal | Function | Range | Isolation |
|---|---|---|---|
| J1 (8 pos) | DI 0–7 | 24 V nominal, opto | 1.5 kV per ch |
| J2 (8 pos) | DI 8–15 | 24 V nominal, opto | 1.5 kV per ch |
| J3 (5 pos) | AI 0–3 + AI ground | 4–20 mA loop | 250 V per ch |
| J4 (5 pos) | AI 4–7 + AI ground | 4–20 mA loop | 250 V per ch |
| J5 (4 pos) | RS-485-A: A, B, GND, shield | 32 nodes max, 9.6k–115.2k baud | 2.5 kV |
| J6 (4 pos) | RS-485-B: A, B, GND, shield | same | 2.5 kV |
| J7 (RJ45)  | 100BASE-T1 (TSN) primary, 100BASE-TX fallback (auto-select) | — | per IEEE |
| J8 (3 pos) | 24 V DC supply + GND + earth | 18–36 V tolerant | — |

## 4. Software stack contract

| Layer | Component | Notes |
|---|---|---|
| RTOS | Zephyr LTS 4.x | SCHED_FIFO control thread @ priority -10 |
| WASM runtime | WAMR AOT (LLVM 18 backend, no GC, `-O3`) | Pre-loaded `.aot` from QSPI; verified against `pinModule` signature |
| Substrate | Zenoh-Pico | UDP unicast on 100BASE-T1; topic format per SPEC §4.1 |
| Field protocols | OpenAPI Modbus RTU master/slave | RS-485-A and RS-485-B independent contexts |
| HAL | Zephyr drivers (custom for ISO1212, ADS1118 SPI tier) | Audit log via `log:event` capability |

## 5. Engineering targets

- WCET (1 BFB tick @ 1 ms cycle): ≤ 200 µs p99.9 — Risk-1 Gate A
- Heap delta after `init`: 0 bytes — enforced by build-time `--no-heap` Zephyr config
- Power: ≤ 6 W typical, ≤ 8 W transient
- MTBF target: 200,000 h @ 40 °C ambient (industrial grade)
- Surge withstand: IEC 61000-4-5 Level 4 on 24 V DI / DO terminals
