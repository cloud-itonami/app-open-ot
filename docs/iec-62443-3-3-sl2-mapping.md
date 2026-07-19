# IEC 62443-3-3 SL-2 Foundational Requirements traceability matrix

Gate C §2.6 deliverable per `risk1/gate-c-estimate/gate-c-report.md`. Requirement-by-requirement mapping from IEC 62443-3-3 Security Level 2 (SL-2) onto today's open-ot stack.

## Scope and reading guide

IEC 62443-3-3 defines seven Foundational Requirements (FR) plus a set of System Requirements (SR) and per-requirement Security Levels (SL-1 .. SL-4). open-ot targets **SL-2** from day one (per SPEC §8).

This matrix is **draft 2026-05-21**; final acceptance requires the external industrial-cyber consultant review per SPEC §14.3 reviewer table. Statuses:

| Status | Meaning |
|---|---|
| ✅ | Covered by today's stack with verifiable evidence |
| 🟡 | Specified but not yet implemented; tracked in §2.5 / §2.6 effort estimate |
| ⏳ | Depends on Mimi Rev-1 hardware or Council artifacts; planned post-Risk-1 PASS |

## FR 1 — Identification and Authentication

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 1.1 | Human user identification | atproto DID + WebAuthn passkey per ADR-2605172700 | ✅ |
| SR 1.2 | Software process / device identification | path-based DID (`did:web:etzhayyim.com:openot:device:<serial>`) | ✅ |
| SR 1.3 | Account management | atproto records under `com.etzhayyim.apps.openOt` Lexicons | ✅ |
| SR 1.4 | Identifier management | DID is the identifier; identifier rotation = new DID record | ✅ |
| SR 1.5 | Authenticator management | Ed25519 key per device; key generation in TF-M secure storage | ⏳ Mimi Rev-1 |
| SR 1.6 | Wireless authentication management | not applicable to MVP (no wireless field protocols per SPEC §11) | — |
| SR 1.7 | Strength of password-based authentication | passkey-only; no passwords (per ADR-2605172700) | ✅ |
| SR 1.8 | PKI certificates | TLS 1.3 with mbedTLS-issued certs; DID resolution provides the trust anchor | 🟡 docs/zenoh-tls-profile.md (future) |
| SR 1.9 | Strength of public key authentication | Ed25519 (NIST-equivalent SL ≥ 128 bits) | ✅ |
| SR 1.10 | Authenticator feedback | TF-M secure storage on STM32H753 (Mimi) / i.MX RT1170 HABv4 (Te) | ⏳ Mimi Rev-1 |
| SR 1.11 | Unsuccessful login attempts | exponential back-off in the XRPC gateway; alerting via atproto record | 🟡 |
| SR 1.12 | System use notification | banner on edge HMI (Atama Svelte editor, post-Risk-1) | ⏳ |
| SR 1.13 | Access via untrusted networks | XRPC mutual-TLS required; capability tokens scoped per session | ✅ (TLS today, capability spec future) |

## FR 2 — Use Control

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 2.1 | Authorisation enforcement | capability-based imports + no ambient authority (SPEC §8 + cells/CLAUDE.md) | ✅ |
| SR 2.2 | Wireless use control | not applicable (no wireless field per SPEC §11) | — |
| SR 2.3 | Use control for portable / mobile devices | not applicable to non-mobile PLC | — |
| SR 2.4 | Mobile code | WASM modules pinned by CID + Ed25519 signature; only signed modules execute | ✅ |
| SR 2.5 | Session lock | 1-hour XRPC session token TTL | 🟡 spec todo |
| SR 2.6 | Remote session termination | per-session revocation list on the cloud gateway | 🟡 spec todo |
| SR 2.7 | Concurrent session control | per-DID active session limit on the cloud gateway | 🟡 |
| SR 2.8 | Auditable events | every state-changing XRPC call writes an atproto audit record | ✅ |
| SR 2.9 | Audit storage capacity | atproto records are content-addressed via IPFS pinning; retention ≥ 90 days per Gate C §2.1 | ✅ |
| SR 2.10 | Response to audit processing failures | audit failure → controller switches to safe-state output (`Alarm` ECC) | ✅ (SPEC §3.5 latched-Alarm semantics) |
| SR 2.11 | Timestamps | atproto records carry RFC 3339 timestamps; PTP / TSN gPTP planned for field clocks | 🟡 (PTP profile TBD) |
| SR 2.12 | Non-repudiation | Ed25519 signature on every audit record + write-once IPFS pin | ✅ |
| SR 2.13 | Role-based use control | SBT ↔ role binding (etzhayyim adherent / steward / council Lv6+) per ADR-2605192100 | 🟡 SBT↔role audit verifier (§2.6 todo) |

**SL-2 gap closure**: SR 2.13 role-based use control needs the SBT ↔ role binding implementation in `etzhayyim-charters-compliance` contract. Spec exists; integration with open-ot's XRPC handlers is ~0.5 PM of the 1.5 PM §2.6 estimate.

## FR 3 — System Integrity

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 3.1 | Communication integrity | TLS 1.3 (mbedTLS) on every XRPC / Zenoh link | 🟡 docs/zenoh-tls-profile.md (future) |
| SR 3.2 | Malicious code protection | only signed AOT modules execute on Mimi/Te (MCUboot verify at load) | ⏳ Mimi Rev-1 |
| SR 3.3 | Security functionality verification | self-test on boot; cell init returns non-zero → halt; periodic capability re-verification | ✅ (cell init contract) |
| SR 3.4 | Software / information integrity | BLAKE3 CID + Ed25519 signature on every module (per SPEC §9, builder-sign-rs) | ✅ (cargo/WASM path; AOT path post-Mimi) |
| SR 3.5 | Input validation | every BFB cell validates `DataIn.quality` + sensor envelope; out-of-range → `Alarm` | ✅ (cells/<cell>/src/lib.rs) |
| SR 3.6 | Deterministic output | by-construction tick(...) determinism contract (SPEC §3); replay tests cover this | ✅ |
| SR 3.7 | Error handling | safe-state output on any invariant violation (`Alarm` ECC + latched trip on anti-islanding-rocof) | ✅ |
| SR 3.8 | Session integrity | XRPC sessions over mutual TLS; replay-resistant nonces | 🟡 spec todo |
| SR 3.9 | Protection of audit information | atproto records are content-addressed + IPFS-pinned (immutable) | ✅ |

## FR 4 — Data Confidentiality

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 4.1 | Information confidentiality | TLS 1.3 in transit; XChaCha20-Poly1305 envelope at rest (per ADR-2605181100) for sensitive cells | ✅ (envelope), 🟡 (TLS profile doc) |
| SR 4.2 | Information persistence | encrypted at-rest layer per ADR-2605181100 | ✅ |
| SR 4.3 | Use of cryptography | Ed25519 + BLAKE3 + XChaCha20-Poly1305 + TLS 1.3 — all standard primitives | ✅ |

## FR 5 — Restricted Data Flow

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 5.1 | Network segmentation | TSN gate windows segment field tier from edge tier per SPEC §4.3 | 🟡 (TSN profile post-Mimi) |
| SR 5.2 | Zone boundary protection | Zenoh-TLS mTLS at each tier boundary | 🟡 docs/zenoh-tls-profile.md (future) |
| SR 5.3 | General purpose person-to-person communication restrictions | not applicable to PLC | — |
| SR 5.4 | Application partitioning | each cell is one WASM instance with linear-memory isolation | ✅ |

## FR 6 — Timely Response to Events

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 6.1 | Audit log accessibility | atproto records queryable via XRPC `getLoop` / `getFault` Lexicons | ✅ |
| SR 6.2 | Continuous monitoring | telemetry stream via Zenoh + RW Hyperdrive checkpoint stream | ✅ (orchestrator/) |

## FR 7 — Resource Availability

| SR | SL-2 Requirement (paraphrased) | Mapping in open-ot | Status |
|---|---|---|---|
| SR 7.1 | Denial-of-service protection | per-loop rate limits on `setpointChange` events; capability tokens scope-bound | 🟡 spec todo |
| SR 7.2 | Resource management | heapless::Vec capacity is const-generic; no `alloc` after init (memory-safety doc) | ✅ |
| SR 7.3 | Control system backup | atproto records are IPFS-pinned; checkpoint stream is RW-persisted | ✅ |
| SR 7.4 | Control system recovery and reconstitution | resume-from-checkpoint validated by `gate-b-rig` (1.328 ms max on host) | ✅ |
| SR 7.5 | Emergency power | not in scope of open-ot software stack; deployment-site responsibility | — |
| SR 7.6 | Network and security configurations | configuration is atproto records; revision history is the audit trail | ✅ |
| SR 7.7 | Least functionality | each cell exposes only `_init` + `_tick` symbols; no ambient runtime | ✅ |
| SR 7.8 | Control system component inventory | atproto records under `com.etzhayyim.apps.openOt.pinModule` list all deployed CIDs | ✅ |

## Summary

| FR | ✅ | 🟡 | ⏳ | — | Total |
|---|---:|---:|---:|---:|---:|
| FR 1 | 7 | 2 | 2 | 2 | 13 |
| FR 2 | 6 | 4 | 0 | 3 | 13 |
| FR 3 | 6 | 3 | 0 | 0 | 9 |
| FR 4 | 3 | 0 | 0 | 0 | 3 |
| FR 5 | 1 | 2 | 0 | 1 | 4 |
| FR 6 | 2 | 0 | 0 | 0 | 2 |
| FR 7 | 6 | 1 | 0 | 1 | 8 |
| **Total** | **31** | **12** | **2** | **7** | **52** |

`52` requirements in scope. `31` (60 %) are ✅ today. `12` (23 %) are spec-defined but pending integration. `2` await Mimi Rev-1 hardware. `7` are not applicable (wireless, mobile, emergency power, person-to-person comms).

The 12 🟡 items are the bulk of the 1.5 PM §2.6 estimate; the 2 ⏳ items overlap with §2.5 (signing/pinning on Mimi).

## Out of scope (explicit)

- IEC 62443-2-4 (service provider requirements) — applies to operators, not the stack itself.
- IEC 62443-4-1 (secure product development lifecycle) — covered separately by lefthook hooks + GitHub Actions CI; not catalogued here.
- IEC 62443-4-2 (component-level technical security requirements) — SL-2 cert at the **system** level (62443-3-3) is the scope; component cert is a future item if a deployment partner requests it.
- IEC 61508 / 61511 functional safety — explicitly excluded by SPEC §11.

## References

- `risk1/gate-c-estimate/gate-c-report.md` §2.6 — parent estimate (1.5 PM)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §8 — IEC 62443-aligned controls overview
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §9 — build / sign / pin pipeline
- `60-apps/etzhayyim-project-open-ot/docs/openot-bfb-rs-memory-safety.md` — FR 3 / FR 7.2 evidence
- `60-apps/etzhayyim-project-open-ot/builder-sign-rs/` — FR 3.4 / SR 2.4 implementation
- IEC 62443-3-3:2013 — *Industrial communication networks – Network and system security – Part 3-3: System security requirements and security levels*
