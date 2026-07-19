# repro-build-rs — open-ot reproducibility harness

Compiles the BFB cells in `cells/` **twice** with a `cargo clean` between runs, hashes each resulting `.wasm` with BLAKE3, and diffs the hashes. PASS means every cell produces a byte-identical artefact across two clean builds — the by-construction prerequisite for the SPEC §9 / Gate C §2.1 `wamrc` AOT determinism claim.

## Status

`v0.1.0` — Tier 1 deliverable for Gate C follow-up. Today: covers the `cargo build --release --target wasm32-unknown-unknown` step only. Post-Risk-1 PASS the same harness wraps `wamrc` AOT output, which is the actual SPEC §14.3 §2.1 deliverable. The `cargo`-side determinism is the harder gate — LLVM upstream has well-known sources of non-determinism (file paths in debug info, parallel codegen).

## What it does

```
for cell in pid-limited droop-p-f anti-islanding-rocof pid-stack-100:
    cargo clean -p <cell>
    cargo build --release --no-default-features --target wasm32-unknown-unknown -p <cell>
    record BLAKE3(output.wasm) as run-1
for cell in <same list>:
    cargo clean -p <cell>
    cargo build --release --no-default-features --target wasm32-unknown-unknown -p <cell>
    record BLAKE3(output.wasm) as run-2
diff run-1 vs run-2
exit 0 if all cells identical, non-zero otherwise
```

## CLI

```
repro-build [OPTIONS]

  --cells-dir <PATH>    Path to the cells/ workspace (default: ../cells)
  --cells <NAMES>...    Cell names (default: pid-limited droop-p-f anti-islanding-rocof pid-stack-100)
  --report <PATH>       Markdown report output (default: ./repro-build-report.md)
```

## Build & run

```bash
cd 60-apps/etzhayyim-project-open-ot/repro-build-rs
cargo build --release
./target/release/repro-build
```

The harness exits non-zero on any byte mismatch, so CI just runs it.

## Output

`repro-build-report.md` contains:

- per-cell BLAKE3 hashes from run-1 and run-2
- pass/fail per cell
- overall verdict

## Caveats

- This validates the **Rust → WASM** path only. The remaining gap (and the actual Gate C §2.1 deliverable) is the **WASM → AOT** via `wamrc`. The same harness pattern applies; we add `wamrc` invocation between `cargo build` and the hash step once Mimi Rev-1 hardware lands.
- LLVM debug-info non-determinism is sidestepped here because the BFB cell crates compile in release mode with `lto = true` and no debug info.
- `cargo` itself is normally deterministic given pinned `Cargo.lock`. The harness exists to **prove** this empirically, not to debug it; if a future Rust upgrade introduces non-determinism, this harness will catch it.
