# open-ot reproducibility harness report

**Cells dir**: `/private/tmp/etzhayyim-gate-c-wt/60-apps/etzhayyim-project-open-ot/cells`

**Cells**: ["pid-limited", "droop-p-f", "anti-islanding-rocof", "pid-stack-100"]

**Total wall-clock**: 1.166 s (both runs + cargo clean × 2 each)

## Results

| Cell | Run 1 BLAKE3 | Run 2 BLAKE3 | Match |
|---|---|---|---|
| `pid-limited` | `8bbd6f184dad73ca34e84348bbb2e8df` | `8bbd6f184dad73ca34e84348bbb2e8df` | ✓ |
| `droop-p-f` | `e22f92f9328b6bad6810a7a5df242406` | `e22f92f9328b6bad6810a7a5df242406` | ✓ |
| `anti-islanding-rocof` | `46a1d463b1aaa46c80a13402f7e6b436` | `46a1d463b1aaa46c80a13402f7e6b436` | ✓ |
| `pid-stack-100` | `f4c427c0e7162ebc033097d5fbbcf7c5` | `f4c427c0e7162ebc033097d5fbbcf7c5` | ✓ |

## Verdict

**PASS** — all cells produced byte-identical artefacts across two clean builds.

## Notes

- Scope: `cargo build --release --target wasm32-unknown-unknown` only. The full SPEC §14.3 §2.1 deliverable also covers the WASM → AOT step via `wamrc`; that gets added post-Risk-1 PASS when Mimi Rev-1 hardware lands and the AOT build is wired in CI.
- The harness exits non-zero on any mismatch, so CI can gate on it directly.
- Cargo is normally deterministic given a pinned `Cargo.lock`. The harness exists to **prove** this empirically and to catch any future toolchain regression that breaks it.
