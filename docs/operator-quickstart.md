# Operator quickstart

Everything below was executed on a clean checkout of this repository before it was
written down. Where a step cannot be run without a toolchain this repo does not
vendor, that is said plainly rather than hidden behind a command that will fail.

**Measured 2026-09-01** on macOS (arm64), against commit `2ef002f`.

## 0. What this repository is, in one screen

`open-ot` is a **specification plus reference cells** for a WASM-native PLC /
Distributed Logic Controller. It is not a deployed runtime and there is nothing
to install. Three things are actually executable today:

| Layer | Directory | Runs today? |
|---|---|---|
| IEC 61499 BFB cells (Rust → WASM) | `cells/` | needs a Rust toolchain |
| Pregel / LangGraph orchestrator (Python) | `orchestrator/` | partly — see §2 |
| Wasm-free codec + runner port (Clojure) | `orchestrator/` (`.cljc`) | **yes, no extra toolchain** |

Start with the third one. It is the only step with no prerequisites beyond a
`bb` on `PATH`, and it is the only step that is green end to end.

## 1. The one step that needs nothing (30 seconds)

```bash
cd orchestrator
bb run_tests.clj
```

Actual output:

```
Testing open-ot-orchestrator.core-test

Ran 16 tests containing 53 assertions.
0 failures, 0 errors.
```

Exit code `0`. `bb test` runs the same suite through `bb.edn` and prints the
same three lines. This exercises the `.cljc` port of the droop codec and the Pregel
runner (`src/open_ot_orchestrator/droop_codec.cljc`,
`src/open_ot_orchestrator/pregel_runner.cljc`) — the parts of the orchestrator
that do not touch WASM.

## 2. The Python orchestrator (2 minutes, and it will not be all green)

```bash
cd orchestrator
uv sync --frozen
uv run --frozen pytest -q
```

`uv sync --frozen` succeeds and resolves the lockfile. `pytest` does **not**
come back clean on a machine without a Rust toolchain:

```
26 failed, 14 passed, 25 skipped
```

(exit code `1`; the trailing wall-clock in pytest's real summary line varies and
is omitted here.)

That result is not a defect in the code under test. 51 of those 65 tests load a
`.wasm` cell artefact from
`cells/target/wasm32-unknown-unknown/release/<cell>.wasm`, and those artefacts do
not exist until §3 has been run. The split is:

| Outcome | Count | Modules |
|---|---|---|
| passed | 14 | `test_generated_byte_equivalence.py` (10), `test_checkpointer.py` (3), `test_pregel_runner.py` (1) |
| skipped | 25 | `test_pregel_runner.py`, `test_microgrid_langgraph.py`, `test_microgrid_islanding_langgraph.py`, `test_microgrid_async_langgraph.py`, `test_checkpointer.py` |
| **failed** | 26 | `test_microgrid_bess_langgraph.py` (7), `test_microgrid_pv_mppt_langgraph.py` (7), `test_microgrid_volt_var_langgraph.py` (7), `test_microgrid_islanding_blackstart_langgraph.py` (5) |

**Known gap — the 26 failures are a missing guard, not a missing feature.** Five
test modules declare

```python
requires_wasm = pytest.mark.skipif(...)
```

and skip cleanly with a message telling you what to build. The four modules in
the "failed" row never adopted that guard, so the same missing artefact raises
`FileNotFoundError` from `cell_loader.py:56` instead. The only difference between
a skip and a failure here is which module the test lives in. Until that is
fixed, read `26 failed` on a cargo-less machine as `26 skipped`; a red run is not
evidence of a regression unless §3 has been run first.

To see only the part that is meaningful without a Rust toolchain:

```bash
uv run --frozen pytest -q tests/test_generated_byte_equivalence.py tests/test_checkpointer.py
```

Actual output: `13 passed, 4 skipped`, exit code `0`.

## 3. Building the cells (requires Rust; not runnable in this environment)

**There are two different WASM targets in this repository and they are not
interchangeable.** Picking the wrong one produces artefacts that nothing loads.

| Target | Consumer | Where it is documented |
|---|---|---|
| `wasm32-unknown-unknown` | `orchestrator/` (wasmtime-py), `risk1/gate-a-rig/` | `risk1/gate-a-rig/build-wasm.sh` |
| `wasm32-wasi` | embedded path — `wamrc` AOT → Zephyr on Giemon Mimi / Te | `cells/CLAUDE.md` |

Host-side, for the orchestrator and the Gate A rig:

```bash
cd cells
cargo build --release --no-default-features --target wasm32-unknown-unknown \
  -p droop-p-f -p pid-limited -p anti-islanding-rocof \
  -p vv-curve -p ltc-tap-fsm -p mppt-perturb-observe \
  -p black-start-seq -p soc-kalman
```

`risk1/gate-a-rig/build-wasm.sh <cell>` does the single-cell version of this.

Embedded, for the field tier:

```bash
cd cells
cargo build --release --no-default-features --target wasm32-wasi -p pid-limited
wamrc --target=thumbv7em --target-abi=eabihf --opt-level=3 \
      --enable-aot --disable-bulk-memory \
      -o pid_limited.aot \
      target/wasm32-wasi/release/pid_limited.wasm
```

Cell unit tests are host-side (`std` is on by default):

```bash
cd cells
cargo test --workspace
```

**Not verified here.** This machine has no `cargo` and no `wamrc`, so none of
§3 was executed and no pass/fail count for it is asserted anywhere in this
repository's documentation. What *is* measurable statically is that the nine
cell crates contain 78 `#[test]` functions between them (see the table in
`README.md`) — that is a count of test functions in source, not a count of
tests observed passing.

## 4. Where the authority actually lives

- **CI is not GitHub Actions.** `.github/workflows/openot-gate-c.yml` exists but
  Actions is **disabled** on this repository — `GET /repos/cloud-itonami/app-open-ot/actions/permissions`
  returns `{"enabled": false}` (measured 2026-09-01). One run has ever been
  recorded, on 2026-07-19, and it failed. Do not read the absence of a red check
  mark as a passing build; nothing runs. Workspace CI is the murakumo fleet
  (`scripts/fleet-ci/` in the superproject).
- **The architecture decision is not in this repository.** ADR-2605151200 lives
  in the `etzhayyim/root` monorepo this project was split out of:
  <https://github.com/etzhayyim/root/blob/main/90-docs/adr/2605151200-open-ot-wasm-plc-dlc.edn>
  (23,502 bytes, verified reachable 2026-09-01). It is `.edn`, not `.md`; older
  documents in this repo link to a `.md` path that no longer exists.
- **The Lexicon JSON files are not reachable.** `SPEC.md` §2 records that 17
  Lexicon files were authored under
  `00-contracts/lexicons/com/etzhayyim/apps/openOt/` in the monorepo. That
  directory does not exist in `etzhayyim/root` today — the GitHub contents API
  returns 404 for it and it is absent from a full local checkout, whose
  `com/etzhayyim/apps/` holds only `etzhayyim`, `hakken`, `kotoba`, `maps3d` and
  `murakumoFleet`. The NSID surface itself is still specified, in `SPEC.md` §2;
  the JSON artefacts are not where any document in this repo says they are.

## 5. Known documentation gaps not fixed by this file

- `CLAUDE.md` and `SPEC.md` still carry monorepo-relative paths
  (`60-apps/etzhayyim-project-open-ot/...`) from before the split.
- `cells/Cargo.toml` declares `repository = "https://github.com/etzhayyim/etzhayyim-root"`,
  which 404s; the monorepo is `etzhayyim/root`.
- `SPEC.md` says "17 Lexicon JSON files" while its own §2 tables name 18
  distinct NSIDs. Neither number has been reconciled against artefacts, because
  the artefacts are missing (§4).
