# gate-c-estimate — Risk-1 Gate C toolchain qualification cost

Written deliverable per SPEC §14.3 / ADR-2605151200 §R4.

| File | Content |
|---|---|
| `gate-c-report.md` | canonical Gate C estimate (PASS / FAIL + 7 sub-items) |

Gate C is a **paper deliverable**, not an executable test. The PASS criterion
is a written estimate that:

1. Covers the seven sub-topics in SPEC §14.3 (WAMR AOT / LLVM 18 / Rust FB
   memory-safety / Zephyr LTS reuse / signing-and-pinning / IEC 62443-3-3
   SL-2 mapping / total effort).
2. Lands at ≤ 6 person-months total effort.
3. Identifies no structural LLVM-side blocker.

Reviewers (per SPEC §14.3): one external industrial-cyber consultant + one
internal review. Reviewer signatures and review notes go in
`gate-c-report.md` § Reviewers when the review is complete.
