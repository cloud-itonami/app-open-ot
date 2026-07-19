# Prototype: Community Microgrid

**Status**: spec only (2026-05-15). Implementation begins on Risk-1 PASS (per ADR-2605151200 §R3 / §R4).

100 kW–10 MW class community microgrid. Demonstrates the full open-ot stack: WAMR-AOT cells on Giemon Mimi/Te (field), LangGraph Pregel orchestrator on Giemon Atama (edge), atproto record SSoT on VKE LangServer pods.

Cross-link with `etzhayyim-project-open-denki` — open-denki owns the CIM topology vocabulary; open-ot adds the **control verbs** that act on it.

## 1. Asset inventory (target pilot site)

A representative 1 MW class community microgrid:

| Asset | Type | Capacity | open-denki record (config SSoT) |
|---|---|---|---|
| PV array A | rooftop solar string × 4 | 400 kW peak | `did:web:open-denki.etzhayyim.com:gen:pv-roof-a` |
| PV array B | carport bifacial × 2 | 200 kW peak | `did:web:open-denki.etzhayyim.com:gen:pv-carport-b` |
| BESS-1 | LFP battery + PCS | 500 kWh / 250 kW | `did:web:open-denki.etzhayyim.com:gen:bess-1` |
| Diesel genset | backup, black-start | 300 kW | `did:web:open-denki.etzhayyim.com:gen:diesel-1` |
| Substation | 6.6 kV / 400 V transformer | 1 MVA | `did:web:open-denki.etzhayyim.com:sub:main` |
| Feeders | 4 × LV distribution | — | `did:web:open-denki.etzhayyim.com:feeder:f01..f04` |
| Smart meters | AMI per delivery point | ~80 | `did:web:open-denki.etzhayyim.com:meter:m001..m080` |
| Grid-tie | utility import/export | 800 kVA | `did:web:open-denki.etzhayyim.com:gen:grid-tie` |

## 2. Loop catalogue

7 loops total, mapping to ADR §R3. Loop DID format: `did:web:open-ot.etzhayyim.com:loop:<loopCode>`. Cell DID format: `did:web:open-ot.etzhayyim.com:cell:<cellCode>`. All cells use `pinModule` to bind to a content-addressed `.aot` artefact.

### 2.1 `:loop:pv-array-mppt-{id}` — PV inverter MPPT (field-only)

| Field | Value |
|---|---|
| Loop kind | `pid` |
| Super-step rate | n/a (field-only inner loop, 100 Hz on Mimi) |
| Cells | 1 per inverter — `did:web:open-ot.etzhayyim.com:cell:pv-mppt-{id}` |
| 4diac FBType | `MPPT_PERTURB_OBSERVE` (etzhayyim-owned, future) |
| Sourced from | open-denki `pv-roof-a`, `pv-carport-b` `recordRenewableOutput` polling |
| LangGraph involvement | observation only — telemetry into checkpointer at 1 Hz |

### 2.2 `:loop:bess-charge-discharge` — BESS PCS control

| Field | Value |
|---|---|
| Loop kind | `pid` |
| Super-step rate | 1 Hz orchestrator + 10 Hz field |
| Cells (field, 10 Hz on Te) | `cell:bess-pcs-1` (`PID_LIMITED` v1 on cv = power), `cell:bess-soc-est-1` (`SOC_KALMAN`, future) |
| Cells (orchestrator, 1 Hz) | `cell:bess-dispatch-coord` (LangGraph node, schedules charge/discharge against forecast) |
| 4diac FBType | `PID_LIMITED`, `SOC_KALMAN` |
| Inputs | open-denki meter readings, PV output, grid-tie state |
| Outputs | PCS power setpoint via `setpointChange` |

### 2.3 `:loop:freq-droop` — frequency support, P-f droop

| Field | Value |
|---|---|
| Loop kind | `coordination` |
| Super-step rate | 1 Hz orchestrator + 10 Hz field |
| Cells (field) | one `cell:freq-droop-{asset}` per dispatchable asset (BESS, diesel, controllable PV) |
| Cells (orchestrator) | `cell:freq-aggregator` (sums droop response, monitors grid frequency) |
| 4diac FBType | `DROOP_P_F` |
| Inputs | grid frequency (PMU or inverter-derived) |
| Outputs | per-asset P setpoint adjustment |

### 2.4 `:loop:volt-var` — voltage / VAR support

| Field | Value |
|---|---|
| Loop kind | `coordination` |
| Super-step rate | 1 Hz orchestrator + 10 Hz field |
| Cells (field) | one `cell:vv-{inverter}` per smart inverter, plus `cell:ltc-tap-control` for transformer tap |
| Cells (orchestrator) | `cell:vv-aggregator` |
| 4diac FBType | `VV_CURVE` (volt-var curve), `LTC_TAP_FSM` |
| Inputs | per-feeder voltage (smart meters), substation voltage |
| Outputs | per-inverter Q setpoint, LTC tap position |

### 2.5 `:loop:islanding-decision` — anti-islanding + black-start

| Field | Value |
|---|---|
| Loop kind | `interlock` |
| Super-step rate | event-driven |
| Cells (field) | `cell:gt-protect` (grid-tie protection BFB on Te), `cell:bus-tie-fsm` (bus-tie sequence) |
| Cells (orchestrator) | `cell:island-decision` (LangGraph node: weighs grid health, BESS SoC, load) |
| 4diac FBType | `ANTI_ISLANDING_ROCOF`, `BLACK_START_SEQ` |
| Inputs | grid voltage / frequency / phase, BESS SoC, critical-load flag |
| Outputs | bus-tie open/close, mode propagation to all loops |
| Latency budget | 100 ms decision → bus-tie open (utility safety requirement) |

### 2.6 `:loop:dr-response` — DR event distribution

| Field | Value |
|---|---|
| Loop kind | `coordination` |
| Super-step rate | event-driven |
| Cells (field) | none — purely orchestrator |
| Cells (orchestrator) | `cell:dr-distributor` (consumes open-denki `recordDemandResponse`, fans out setpoint cascade) |
| 4diac FBType | n/a (LangGraph-only) |
| Inputs | open-denki DR event, asset availability, customer opt-in |
| Outputs | `setpointChange` cascade across BESS / curtailable PV / controllable load |

### 2.7 `:loop:peak-shave-economic` — economic dispatch (orchestrator-only)

| Field | Value |
|---|---|
| Loop kind | `economic` |
| Super-step rate | 1 Hz orchestrator |
| Cells (field) | none |
| Cells (orchestrator) | `cell:dispatch-optim` (LP / heuristic dispatch), `cell:price-feed` (utility tariff + JEPX feed), `cell:forecast-pv` (24 h PV forecast), `cell:forecast-load` |
| Inputs | telemetry, tariffs, weather forecast, BESS SoC |
| Outputs | `setpointChange` to BESS dispatch, PV curtailment recommendation, import/export schedule |
| Notes | Demonstrates LangGraph value at the multi-loop coordination layer without contaminating inner loops. |

## 3. 4diac FBType library at MVP

| FBType | Origin | Status |
|---|---|---|
| `PID_LIMITED` | etzhayyim-owned, in `cells/pid-limited/` | implemented (2026-05-15) — 5 tests PASS incl. replay determinism |
| `DROOP_P_F` | etzhayyim-owned, in `cells/droop-p-f/` | implemented (2026-05-15) — 10 tests PASS incl. droop proportionality + replay determinism |
| `ANTI_ISLANDING_ROCOF` | etzhayyim-owned, in `cells/anti-islanding-rocof/` | implemented (2026-05-15) — 14 tests PASS incl. all 5 trip reasons + RESET handling + replay determinism |
| `MPPT_PERTURB_OBSERVE` | etzhayyim-owned | future |
| `SOC_KALMAN` | etzhayyim-owned | future |
| `VV_CURVE` | etzhayyim-owned | future |
| `LTC_TAP_FSM` | etzhayyim-owned | future |
| `BLACK_START_SEQ` | etzhayyim-owned | future |

`PID_LIMITED` is the only cell required for Risk-1 Gate A. The other two implemented cells validate the `openot-bfb-rs` trait against progressively more elaborate shapes — `DROOP_P_F` adds a 5th ECC state + i128 intermediate, `ANTI_ISLANDING_ROCOF` adds multi-event-input (REQ + RESET), multi-event-output (CNF + TRIP + ALM), latched state, time-derivative, and N-sample debounce. The trait absorbs all three without modification. Remaining FBTypes are scaffolded for the 90-day pilot; their implementation begins after Gate A PASS.

## 4. Acceptance — 90-day pilot

Per ADR §R3 acceptance:

- Zero unplanned islanding.
- ≥ 99 % uptime of the orchestrator (NixOS + LangGraph) measured at the XRPC `getLoop` endpoint.
- ≥ 95 % of `setpointChange` events landing within their declared deadline (1 s for orchestrator-issued, 100 ms for islanding sequence).
- Audit trail (`vertex_open_ot_loop_checkpoint` stream) reconstructable to per-second granularity for any 24 h window in the 90 days, with no gaps > 5 s.

## 5. Pilot site selection (open question, ADR carry-over)

Candidates:

| Site type | Pros | Cons |
|---|---|---|
| University campus microgrid | non-safety-critical, research-friendly, public-good narrative | smaller scale (~100 kW), less production stress |
| Small industrial site (rooftop PV + diesel backup) | real economic value, paying customer signal | safety / liability scrutiny, longer sales cycle |
| Remote island grid (Okinawa / Ogasawara) | islanding is the *normal* state, strong value story | logistics / regulatory complexity, partner dependency |

Decision deferred — drives 802.1Qbv TSN profile, NTP/PTP source choice, and OPC UA FX peer count.

## 6. Out of scope at prototype

- Volt-Var optimization (VVO) at multi-substation scale — single-substation only.
- Wholesale market bidding (JEPX) — read-only price feed for `peak-shave-economic` is OK; bid submission is not.
- Real-time OPF (Newton-Raphson power flow) — economic dispatch uses LP + heuristics.
- Battery state-of-health degradation modelling — SoC only.
- DERMS integration with utility ADMS — defer to MVP+1.
