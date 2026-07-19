# LLVM version pin policy

Gate C §2.2 deliverable per `risk1/gate-c-estimate/gate-c-report.md`.

## TL;DR

| Property | Value |
|---|---|
| LLVM version | **18.x** (pinned, latest patch) |
| Rust toolchain | **1.75 or newer** (any minor; LLVM 18 IR ABI-compatible) |
| WAMR validation lag | ~6 months behind LLVM mainline; LLVM 19/20 not yet WAMR-validated |
| Re-validation cadence | quarterly CVE review; major-version bump when WAMR upstream validates |
| CVE policy | LLVM 18.x CVE without a backported patch → pin to latest 18.x + document; if no fix exists in 18.x → trigger emergency Risk-1 re-run, do not auto-bump to 19.x |

## Why this matters for open-ot

The build → sign → pin pipeline (SPEC §9) ends with `wamrc` doing the WASM → AOT lowering on a trusted builder. `wamrc` is built on LLVM. For:

- **Reproducibility** (Gate C §2.1): two clean builds must produce byte-identical AOT artefacts. LLVM is the single biggest non-determinism source (file paths in debug info, parallel codegen). Pinning the LLVM version is a necessary precondition.
- **Cyber-cert audit retention** (IEC 62443-3-3): the AOT artefact's provenance must include the exact compiler version. Audit retention is ≥ 90 days.
- **Memory-safety claims** (Gate C §2.3): the Rust source is `#![no_std]` + heapless. A buggy LLVM codegen pass could violate that at the WASM level. Pinning a vetted LLVM version is the only way to make this claim stable.

## Pin locations (single source of truth)

The pin lives in **two places**, kept in sync:

1. `firmware/{mimi,te}-zephyr/west.yml` — Zephyr west manifest. WAMR submodule revision is pinned to a tag built against the LLVM version we ship.
2. `nixos/atama/wamr.nix` — NixOS module for Atama edge controller. `wamrc` package version pinned via `nixpkgs` overlay.

Both are validated by CI (`.github/workflows/openot-gate-c.yml`):

- Build the 4 BFB cells.
- Diff against the previous CI run's BLAKE3 hashes (committed to `risk1/repro-baseline.txt`, future work).
- Mismatch → CI fails → operator inspects whether the LLVM update was intentional.

Until the Mimi Rev-1 firmware spin lands, the harness in `repro-build-rs/` validates the cargo → WASM half of this; the WASM → AOT half lands at the same time as Mimi.

## CI matrix

`.github/workflows/openot-gate-c.yml` runs against:

| Toolchain | Status |
|---|---|
| `rustc 1.75.x` (release) | required |
| `rustc 1.83.x` (latest stable as of 2026-05) | required |
| `rustc nightly` | informational (FAIL allowed) |

Cargo workspace `Cargo.lock` is committed.

## Update procedure

1. **Watch upstream WAMR** — every quarterly release.
2. **Verify LLVM version** in the new WAMR release's `CMakeLists.txt`.
3. **Bump the pin** in both `west.yml` and `wamr.nix` simultaneously, one PR.
4. **Run `repro-build` + `gate-a-rig` × 4 cells** against the new LLVM; require PASS.
5. **Commit the new BLAKE3 baseline** in `risk1/repro-baseline.txt`.
6. **Update `gate-c-estimate/gate-c-report.md` §2.2** if the LLVM version moves to a major.

The first three steps are paper; the last three are CI-enforced.

## What this policy is NOT

- It is **not** a freeze: LLVM 18.x patches are pulled in routinely.
- It is **not** a long-term commitment to LLVM 18: when LLVM 19 is WAMR-validated (~Q4 2026 by current upstream cadence), we will bump.
- It is **not** a cert claim: IEC 62443-3-3 SL-2 does not require a specific LLVM version, only a stable provenance chain. The pin is part of that chain.

## References

- `risk1/gate-c-estimate/gate-c-report.md` §2.2 — the parent estimate (0.25 PM)
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §9 — build / sign / pin pipeline
- `60-apps/etzhayyim-project-open-ot/SPEC.md` §14.3 — Risk-1 Gate C criteria
- Upstream: https://github.com/bytecodealliance/wasm-micro-runtime — WAMR release notes (cross-reference LLVM version per tag)
