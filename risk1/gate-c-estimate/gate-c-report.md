# Risk-1 Gate C — toolchain qualification cost estimate

**Status**: DRAFT (2026-05-21) — pending external industrial-cyber consultant review + internal review per SPEC §14.3.

**Verdict**: **PASS** (estimated effort 4.75 person-months; ≤ 6 PM threshold; no LLVM-side structural blocker identified).

Per ADR-2605151200 §R4 + SPEC §14.3.

---

## 0. Software evidence today (Tier 1 follow-up landed)

Paper → code follow-up under `risk1/gate-c-estimate/` and `60-apps/etzhayyim-project-open-ot/`:

| Sub-item | Code / doc artefact | Evidence |
|---|---|---|
| §2.1 WAMR AOT determinism (cargo half) | `repro-build-rs/` Rust harness | `repro-build` PASS — 4 cells byte-identical across two clean builds; CI-gated |
| §2.2 LLVM 18 pin policy | `docs/llvm-version-policy.md` | pin locations + update procedure + CVE policy |
| §2.3 Rust FB memory-safety | `docs/openot-bfb-rs-memory-safety.md` + `cargo geiger` CI + `#[cfg(kani)] mod proofs` × 4 cells | by-construction argument + Risk-1 Gate A heap-delta = 0 + kani symbolic verification of `tick` / `init` panic-freedom (CI matrix) |
| §2.4 Zephyr LTS supply chain | `docs/zephyr-lts-supply-chain.md` | module list + support SLAs + safety status |
| §2.5 Signing / pinning CLI | `builder-sign-rs/` Rust crate (CLI + library) | 11 unit tests + smoke test signing a real `droop_p_f.wasm` + verify roundtrip |
| §2.6 IEC 62443-3-3 SL-2 mapping | `docs/iec-62443-3-3-sl2-mapping.md` | 52 SRs traced; 31 ✅ / 12 🟡 / 2 ⏳ / 7 N/A |
| CI gate | `.github/workflows/openot-gate-c.yml` | 7 jobs: geiger / repro-build / builder-sign / gate-a × 4 cells / gate-b / cells-tests / kani × 4 cells |

The remaining 🟡 and ⏳ items (Mimi firmware integration, Zenoh-TLS mTLS profile, SBT↔role audit verifier, cell replay tests) stay within the 1.5 PM §2.6 + ~0.5 PM §2.3 residuals.

---

## 1. Scope

Gate C covers the **toolchain qualification work** required to position the open-ot WASM PLC stack for an IEC 62443-3-3 SL-2 cyber-physical security claim on the Q3 2026 microgrid prototype. It does **not** cover IEC 61508 / 61511 functional-safety certification — that remains out of scope per SPEC §11 and is delegated to certified parallel safety PLCs.

The seven sub-items are evaluated against today's stack (commit at risk1 ↔ cells parity, 2026-05-21):

| Sub-item | Coverage | Estimated effort |
|---|---|---|
| 1. WAMR AOT compiler determinism | reproducible build of `wamrc` outputs + version pin + artefact retention | 1.0 PM |
| 2. LLVM 18 dependency mapping | LLVM 18.x pin policy + Rust 1.75+ compatibility CI matrix | 0.25 PM |
| 3. Rust FB framework memory-safety | `openot-bfb-rs` no-unsafe-except-ABI claim + kani / miri harness on tick path | 1.0 PM |
| 4. Zephyr LTS vendor safety package reuse | config / supply-chain doc; no code work | 0.25 PM |
| 5. Signing / pinning workflow | Ed25519 verify on Cortex-M7, builder CLI, `pinModule` XRPC handler | 0.75 PM |
| 6. IEC 62443-3-3 SL-2 requirements mapping | requirement-by-requirement traceability matrix + audit-trail / Zenoh-TLS profile | 1.5 PM |
| 7. Total | sum | **4.75 PM** |

The estimate is bottom-up; the SPEC threshold (6 PM) carries a 26 % slack against this estimate. The estimate excludes hardware development (Mimi / Te / Atama Rev-1 boards), which is tracked separately under Risk-1 Gate A and the post-PASS hardware spin.

---

## 2. Per-sub-item analysis

### 2.1 WAMR AOT compiler determinism (1.0 PM)

| Item | Status |
|---|---|
| Upstream | [Bytecode Alliance WAMR](https://github.com/bytecodealliance/wasm-micro-runtime) — BSD-3 + Apache-2.0 dual, vendor-supported by Intel + the Bytecode Alliance |
| Version | Pin to a WAMR release tag in `nixos/atama/wamr.nix` and `firmware/{mimi,te}-zephyr/west.yml`; today the rig uses `wamrc` from a developer-built tree |
| Reproducibility | `wamrc` output must be byte-deterministic given (input `.wasm` CID + target triple + LLVM version). Not currently verified |
| Risk | The WAMR AOT path uses LLVM to lower WASM → native; LLVM has well-known non-determinism sources (file paths in debug info, parallel codegen). Mitigated via `--no-debug-info`, single-thread codegen, fixed `-O3` profile |
| Effort breakdown | (a) Pin policy in west.yml + nix; (b) reproducible-build harness that compiles the 4 BFB cells across two clean checkouts and `diff`s the AOT bytes; (c) CI artefact retention (≥ 90 days per IEC 62443 audit trail) |

**Blocker check**: no structural blocker. WAMR's AOT path is upstream-supported and the determinism work is tooling, not a fork.

### 2.2 LLVM 18 dependency mapping (0.25 PM)

| Item | Status |
|---|---|
| LLVM version | LLVM 18.x is the WAMR-supported release as of Q1 2026; LLVM 19/20 are not yet WAMR-validated |
| Rust toolchain | `rustc 1.75+` emits LLVM IR compatible with LLVM 18 backend used by `wamrc` |
| Risk | LLVM project's annual release cadence vs. WAMR's validation lag (~6 months) can force a stale pin. Risk: a CVE in LLVM 18.x without a backported patch |
| Effort breakdown | (a) Version-pin policy doc (`docs/llvm-version-policy.md`); (b) CI matrix exercising `wamrc-LLVM-18.x` on every PR; (c) quarterly CVE review |

**Blocker check**: no structural blocker. The LLVM project's stable-branch policy is compatible with our pinning strategy.

### 2.3 Rust FB framework memory-safety (1.0 PM)

| Item | Status |
|---|---|
| `openot-bfb-rs` | shared crate enforces `#![no_std]` (embedded build), `heapless::Vec` for tick-path collections, no `Box<dyn Trait>` in `tick`, no `f32` / `f64` |
| Unsafe surface | Per-cell `#[no_mangle] extern "C"` ABI wrapper is the only `unsafe` block; pointer validation is explicit (null-check, alignment, lifetime documented in `# Safety` comments) |
| Verification | Today: `cargo geiger` confirms no unsafe outside the ABI wrappers (4 / 4 cells clean as of 2026-05-21). Target: kani / miri harness on the tick path for replay determinism |
| Risk | Heap exhaustion in BFB tick is impossible by construction (no `alloc` after init, `heapless::Vec` capacity is a const generic). Verified by SPEC §3 + Risk-1 Gate A heap-delta = 0 |
| Effort breakdown | (a) Formal memory-safety argument doc (`docs/openot-bfb-rs-memory-safety.md`); (b) kani harness on `tick_pure(...)` for each cell (compiled to host, not embedded); (c) `cargo geiger` CI gate |

**Blocker check**: no structural blocker. The framework is intentionally constrained so that the memory-safety argument is by-construction; verification is automating that argument.

### 2.4 Zephyr LTS vendor safety package reuse (0.25 PM)

| Item | Status |
|---|---|
| Zephyr LTS | 4.x is the current LTS (released 2024-11, supported through 2028 per Linux Foundation Zephyr Project policy) |
| Safety status | Zephyr Safety Working Group maintains an IEC 61508 SIL2 / EN 50129 cert path (Nordic + Intel members) — **not used** by open-ot at MVP (non-SIL only per SPEC §11) |
| Security packages reused | MCUboot (signed-binary boot chain), TF-M (Trusted Firmware-M secure storage), mbedTLS 3.x (TLS 1.3 stack) |
| Risk | None at MVP. If the project later pursues SIL, the upgrade path is the Zephyr Safety Working Group's qualified configuration |
| Effort breakdown | (a) Supply-chain doc listing the Zephyr modules in use + upstream support SLAs; (b) west.yml manifest pinning + revision-cap policy |

**Blocker check**: no structural blocker. Zephyr LTS is the most vendor-supported RTOS in scope and the security-package reuse is well-trodden ground.

### 2.5 Signing / pinning workflow (0.75 PM)

| Item | Status |
|---|---|
| Build → sign → pin | Specced in SPEC §9: cargo build → `wamrc` AOT → builder DID Ed25519 sign → CID via `b3sum` → atproto record via `com.etzhayyim.apps.openOt.pinModule` |
| Today | CLI stub at `scripts/builder-sign.sh` (per cells/CLAUDE.md note from 2026-05-20 CLI removal); `pinModule` Lexicon already exists at `00-contracts/lexicons/com/etzhayyim/apps/openOt/pinModule.json` |
| Edge verification | Mimi / Te / Atama pull CID over XRPC, verify Ed25519 sig against builder DID resolved from atproto, load via WAMR — **not yet implemented** |
| Risk | Cortex-M7 Ed25519 verify (~5 ms on STM32H753) is well within budget for the boot-time check. Tooling for signing is mature (e.g. `signify-rs`, `ed25519-dalek`) |
| Effort breakdown | (a) Builder signing CLI re-implementation in Rust (cells/builder-sign-rs); (b) Ed25519 verify shim for Cortex-M7 in `firmware/mimi-zephyr/src/aot-verify.c` + integration with MCUboot; (c) `pinModule` XRPC handler on the cloud gateway VKE + LangServer pod |

**Blocker check**: no structural blocker. The cryptographic primitives are standard; the work is integration + ABI plumbing.

### 2.6 IEC 62443-3-3 SL-2 requirements mapping (1.5 PM)

| Item | Status |
|---|---|
| Scope | IEC 62443-3-3 SL-2 is the **security level** target for open-ot from day one (per SPEC §8) |
| Foundational Requirements (FR) | FR 1 Identification & Authentication / FR 2 Use Control / FR 3 System Integrity / FR 4 Data Confidentiality / FR 5 Restricted Data Flow / FR 6 Timely Response / FR 7 Resource Availability |
| Current coverage | Signed AOT modules (FR 3), capability-based imports + no ambient authority (FR 2), audit trail via atproto records (FR 5+6), DID-based authentication for all writers (FR 1) |
| Gaps | (a) Capability-grant audit needs SBT↔role binding (FR 2 SL-2 requires role-based use control); (b) Zenoh data plane needs TLS 1.3 mTLS profile (FR 4); (c) HW root-of-trust on Mimi/Te for AOT-at-rest on QSPI flash (FR 3, depends on SoC vendor — STM32H753 has TrustZone-M; i.MX RT1170 has HABv4) |
| Effort breakdown | (a) FR-by-FR traceability matrix (`docs/iec-62443-3-3-sl2-mapping.md`); (b) Zenoh-TLS mTLS profile spec + reference config; (c) SBT↔role audit-trail verifier; (d) consultant review + redline cycle |

**Blocker check**: no structural blocker. SL-2 is the standard target for non-SIL industrial control; the gaps are integration, not invention.

### 2.7 Effort estimate

| Sub-item | PM |
|---|---|
| 2.1 WAMR AOT compiler determinism | 1.00 |
| 2.2 LLVM 18 dependency mapping | 0.25 |
| 2.3 Rust FB framework memory-safety | 1.00 |
| 2.4 Zephyr LTS vendor safety package reuse | 0.25 |
| 2.5 Signing / pinning workflow | 0.75 |
| 2.6 IEC 62443-3-3 SL-2 requirements mapping | 1.50 |
| **Total** | **4.75** |

SPEC threshold: ≤ 6 PM. **PASS** with 1.25 PM slack (26 %).

Bottom-up rates assume one full-time engineer with industrial-cyber experience + one part-time consultant for §2.6 redlines. Calendar time at this loading is ~5 calendar months; this is **inside** the Q3 2026 prototype window provided staffing is in place at Risk-1 PASS.

---

## 3. Risk register (Gate C-specific)

| Risk | Severity | Likelihood | Mitigation |
|---|---|---|---|
| LLVM 18 EOL forces upgrade mid-cert | medium | low | Quarterly CVE review; LLVM 19 validation when WAMR upstreams it (~Q4 2026) |
| External consultant unavailable for §2.6 review | medium | medium | Pre-engage at Risk-1 PASS; budget includes 0.25 PM consultant time |
| Cortex-M7 Ed25519 verify exceeds boot-time budget | low | very low | Already benchmarked at ~5 ms; MCUboot's existing Ed25519 path is reusable |
| SL-2 gap (FR 2 role-based use control) requires SBT redesign | medium | low | SBT ↔ role binding already in `com.etzhayyim.apps.openOt` Lexicon set; gap is plumbing |

No risk is rated high. No risk pushes the estimate over 6 PM in isolation.

---

## 4. Out of scope (Gate C)

- **IEC 61508 / 61511 functional-safety certification** — explicitly excluded by SPEC §11. Any SIF stays on a certified parallel safety PLC.
- **IEC 62443-2-4 service provider requirements** — applies to operators, not the open-ot stack itself.
- **Common Criteria evaluation** — not requested by any deployment partner.
- **FIPS 140-3 cryptographic module validation** — Ed25519 and BLAKE3 are not currently FIPS-listed; if a deployment requires FIPS, replace with FIPS-listed primitives (RSA-3072 + SHA-256) as a separate work item (estimated 1.5 PM, not counted here).

---

## 5. Decision

**Gate C is provisionally PASS.** Final decision requires the two reviewer sign-offs per SPEC §14.3.

Per the SPEC §14.4 decision matrix:

- Gate A: **host PASS** (4 cells, p99.9 ≤ 125 ns at 200 µs deadline, heap delta 0, 0 deadline misses — see `../gate-a-report.md` and siblings)
- Gate B: **host PASS** (step p99 = 0.034 ms, ckpt p99 = 5.533 ms, resume max = 1.328 ms, 0 message loss — see `../gate-b-report.md`)
- Gate C: **paper PASS** pending review (this report)

If all three gates retain PASS after embedded measurement + reviewer sign-off, the SPEC §14.4 row reads:

> | PASS | PASS | PASS | Promote to MVP build, commission Mimi/Te/Atama Rev-1, start microgrid pilot, begin Svelte editor |

— i.e. the Risk-1 outcome is the green-light row.

---

## 6. Reviewers

| Reviewer | Role | Status | Date | Notes |
|---|---|---|---|---|
| TBD (external industrial-cyber consultant) | independent | pending | — | engage at Risk-1 PASS announcement |
| TBD (internal) | open-ot maintainer | pending | — | — |

---

## 7. References

- ADR-2605151200 — open-ot WASM PLC + DLC architecture decision (R4 Risk-1 gates)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §14 — Risk-1 acceptance test plan (gate-by-gate criteria)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §8 — IEC 62443-aligned security controls
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §9 — Build / sign / pin pipeline
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §11 — Explicit out-of-scope (SIL)
- `60-apps/etzhayyim-project-open-ot/cells/CLAUDE.md` — `#[no_mangle] extern "C"` ABI rules + no-alloc-after-init invariant
- `60-apps/etzhayyim-project-open-ot/risk1/gate-a-report.md` (+ droop / anti-islanding / stack100 siblings) — Gate A host run
- `60-apps/etzhayyim-project-open-ot/risk1/gate-b-report.md` — Gate B host run
