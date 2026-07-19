# Contributing to open-ot

## Where this code lives

`open-ot` currently develops inside the `etzhayyim/etzhayyim-root` monorepo at `60-apps/etzhayyim-project-open-ot/`. A standalone OSS repo (`etzhayyim/etzhayyim-project-open-ot`) is planned for split after **Risk-1 PASS** (per ADR-2605151200 §R4, target Q3 2026).

Until split, all contributions go through the monorepo: open issues against `etzhayyim/etzhayyim-root` with the label `area/open-ot`. The same Apache-2.0 license applies.

## Repo split plan (post-Risk-1)

When the standalone repo is created, it will export the following subset:

| In monorepo | In split repo |
|---|---|
| `60-apps/etzhayyim-project-open-ot/{README,CLAUDE,SPEC,PROTOTYPE-MICROGRID,LICENSE,OWNERS}.md` | root |
| `60-apps/etzhayyim-project-open-ot/cad-spec/` | `cad-spec/` |
| `60-apps/etzhayyim-project-open-ot/cells/` | `cells/` |
| `00-contracts/lexicons/com/etzhayyim/apps/openOt/` | `lexicons/openOt/` |
| `90-docs/adr/2605151200-open-ot-wasm-plc-dlc.md` | `docs/adr/0001-open-ot.md` |

Internal-only files (etzhayyim platform conventions, `deps.toml`, registry sidecars) will **not** be exported. The split is one-way (monorepo → standalone); upstream changes flow back through PR.

## Build & test

```bash
cd 60-apps/etzhayyim-project-open-ot/cells
cargo test --workspace                                          # 15 unit tests, host
cargo build --release --no-default-features --target wasm32-wasi # embedded
```

## Required for any cell PR

Per ADR-2605151200 §LangGraph + Pregel binding determinism contract:

1. **Replay-determinism test** — given a recorded super-step stream, replaying `tick` MUST reproduce the same `(next_state, emitted, neighbor_msgs, internal_post)`. This is the CI gate; no cell merges without it.
2. **No `f32` / `f64` in the tick path** — use fixed-point `i32` µ-units, `i64` or `i128` intermediates with saturating arithmetic.
3. **No allocation after `init`** — fixed-cap `heapless::Vec`, no `Box<dyn Trait>`, no `gc` feature.
4. **No `std::time` / RNG** — wall time and randomness arrive as data inputs (replay-deterministic).
5. **`#[no_mangle] extern "C"` ABI** — `<cell>_init` and `<cell>_tick` exported with `#[repr(C)]` structs.
6. **Manifest** — `manifest.json` describing FBType / ECC / events / data / params / capacity bounds.
7. **`no_std`-compatible** — `#![cfg_attr(not(feature = "std"), no_std)]`. Default-on `std` for host tests; embedded opts out via `--no-default-features`.

## Code style

Standard `rustfmt`. No manual `unsafe` outside the C ABI surface. `clippy::pedantic` is not enforced; common-sense lints are.

## Pre-commit (recommended)

Add to `.pre-commit-config.yaml` at the repo root (create if absent):

```yaml
repos:
  - repo: local
    hooks:
      - id: open-ot-validate-cell-abi
        name: open-ot Lexicon × manifest validator
        entry: python3 70-tools/scripts/open-ot/validate-cell-abi.py
        language: system
        files: ^(60-apps/etzhayyim-project-open-ot/cells/.*/manifest\.json|00-contracts/lexicons/com/etzhayyim/apps/openOt/.*\.json)$
        pass_filenames: false

      - id: open-ot-codegen-check
        name: open-ot generated packers up to date
        entry: python3 70-tools/scripts/open-ot/codegen-cell-types.py --check
        language: system
        files: ^60-apps/etzhayyim-project-open-ot/cells/.*/manifest\.json$
        pass_filenames: false
```

Run `pre-commit install` once per clone. CI enforces the same checks via `.github/workflows/open-ot-validate.yml`.

## CI

`.github/workflows/open-ot-validate.yml` runs on any PR / push touching open-ot sources, the openOt Lexicon directory, or the open-ot tool scripts. The workflow has three jobs:

1. `static-validators` — Lexicon × manifest validator + `codegen-cell-types.py --check`.
2. `rust-cells` — `cargo test --workspace` for `cells/` + builds wasm32-unknown-unknown artefacts as a job artifact.
3. `orchestrator-pytest` — `uv run pytest` for orchestrator + validator + codegen tool tests; consumes the wasm artefacts from job #2.

A failing `static-validators` job is the most common drift signal: it means a Lexicon or a manifest changed but the generated packers / validator output didn't catch up. Re-run `python3 70-tools/scripts/open-ot/codegen-cell-types.py` (no `--check`) to regenerate, then commit.

## Issue triage

| Label | Meaning |
|---|---|
| `area/open-ot` | scope is open-ot |
| `area/open-ot/framework` | `openot-bfb-rs` trait surface |
| `area/open-ot/cell/<name>` | a specific cell |
| `area/open-ot/hardware/<board>` | Giemon Mimi / Te / Atama |
| `area/open-ot/risk1` | Risk-1 prototype gate (Q3 2026) |

## Maintainers

See [`OWNERS`](OWNERS).
