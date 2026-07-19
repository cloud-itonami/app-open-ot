# Zephyr LTS supply-chain doc

Gate C §2.4 deliverable per `risk1/gate-c-estimate/gate-c-report.md`.

## Scope

The Mimi (sensor RTU) and Te (actuator RTU) field devices run **Zephyr LTS 4.x** + **WAMR AOT**. The Atama edge controller runs **NixOS + linuxPackages_rt**, which is separately tracked. This doc covers the Zephyr side only.

## Modules in use

| Module | Version pin | Upstream | Role |
|---|---|---|---|
| Zephyr RTOS | LTS 4.x (latest 4.x patch) | Linux Foundation Zephyr Project | base RTOS, scheduler, drivers |
| MCUboot | follow Zephyr LTS submodule | mcu-tools/mcuboot | signed-binary boot chain |
| TF-M (Trusted Firmware-M) | follow Zephyr LTS submodule | TrustedFirmware-M | secure storage + isolated services on STM32H753 (TrustZone-M) |
| mbedTLS 3.x | follow Zephyr LTS submodule | Trusted Firmware | TLS 1.3 stack for Zenoh + atproto |
| WAMR | tag pinned to LLVM 18.x build | Bytecode Alliance | WASM AOT runtime |
| Zenoh-Pico | pin per `west.yml` | Eclipse Zenoh | data-plane |

`west.yml` lives at `firmware/mimi-zephyr/west.yml` and `firmware/te-zephyr/west.yml`. Both will be created at the Mimi Rev-1 firmware spin (post-Risk-1 PASS).

## Upstream support SLAs

| Project | Support SLA |
|---|---|
| Zephyr LTS 4.x | Linux Foundation Zephyr Project: 2.5 years from initial release. LTS 4.0 was released 2024-11; 4.x supported through ~2027. Subsequent LTS (5.x) expected 2026-Q4. |
| MCUboot | upstream-driven; tracks Zephyr LTS branch directly |
| TF-M | TrustedFirmware project; quarterly releases; LTS branches available |
| mbedTLS | Trusted Firmware; long-term support branches |
| WAMR | Bytecode Alliance; quarterly releases; no formal LTS but issue tracker active |

## Safety status (informational; not used at MVP)

The Zephyr Project hosts an **IEC 61508 SIL2 / EN 50129 safety certification path** via the Zephyr Safety Working Group (Nordic Semiconductor + Intel + Linaro members). This is **not** in scope for open-ot MVP per SPEC §11 — open-ot stays non-SIL and delegates Safety Instrumented Functions to certified parallel safety PLCs.

If a future deployment partner requires SIL, the path is:

1. Adopt the Zephyr Safety Working Group's qualified configuration.
2. Limit WAMR cells to non-SIL classification (per existing SPEC §11).
3. Add a certified safety PLC in parallel (HIMA / S7-1500F / GuardLogix).
4. Wire the open-ot cells to issue **advisory** outputs only; safety functions live on the parallel PLC.

This is reflected in `risk1/gate-c-estimate/gate-c-report.md` §2.4 risk register — no work today, ~0.25 PM upgrade if requested.

## Security-package reuse

| Use case | Component | Notes |
|---|---|---|
| Signed boot chain | MCUboot | verifies builder signature on AOT module at boot (per SPEC §9 step 5) |
| Key storage | TF-M secure storage | builder DID's verifying-key stored in TF-M, attested |
| TLS 1.3 to Zenoh + atproto | mbedTLS 3.x | mTLS profile spec in `docs/zenoh-tls-profile.md` (future work, IEC 62443-3-3 SL-2 FR 4) |
| HW root-of-trust | STM32H753 TrustZone-M / i.MX RT1170 HABv4 | varies by SoC; both reachable from Mimi/Te |

## Update procedure

Zephyr LTS minor bumps (e.g. 4.1 → 4.2) follow the LLVM 18 pin update procedure in `docs/llvm-version-policy.md` §"Update procedure":

1. Wait for the LTS release.
2. Bump pin in `west.yml`.
3. Run reproducibility check (`repro-build-rs` — once Mimi build is wired in).
4. Run Gate A on Mimi prototype.
5. Commit new BLAKE3 baseline + Zephyr revision pin.

LTS major bumps (4.x → 5.x) trigger a full Risk-1 re-run.

## What this doc is NOT

- It is **not** a code change. Today there is no `firmware/` directory yet; it lands at Mimi Rev-1 spin.
- It is **not** a SIL cert path. Per SPEC §11, IEC 61508 / 61511 are explicitly out of scope.
- It is **not** an exhaustive vendor list. Only the components in the trusted boot / WASM / TLS path are tracked here. Other Zephyr drivers (UART, ADC, CAN) are upstream-trusted with no project-side pin policy.

## References

- `risk1/gate-c-estimate/gate-c-report.md` §2.4 — the parent estimate (0.25 PM)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §11 — explicit out-of-scope (SIL)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §12 — hardware spec (Mimi / Te / Atama)
- `docs/llvm-version-policy.md` — LLVM 18 pin policy (parallel pin)
- `docs/iec-62443-3-3-sl2-mapping.md` — FR mapping that depends on these packages
