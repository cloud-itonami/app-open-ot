//! repro-build — open-ot BFB cell reproducibility harness.
//!
//! Compile each cell twice with `cargo clean` between, BLAKE3-hash output,
//! diff. PASS iff every cell produces byte-identical .wasm across two
//! clean builds. Per SPEC §14.3 / Gate C §2.1.

use anyhow::{bail, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

const DEFAULT_CELLS: &[&str] = &[
    "pid-limited",
    "droop-p-f",
    "anti-islanding-rocof",
    "pid-stack-100",
];

#[derive(Parser, Debug)]
#[command(name = "repro-build", version, about = "open-ot BFB cell reproducibility harness")]
struct Cli {
    /// Path to the cells/ workspace root.
    #[arg(long, default_value = "../cells")]
    cells_dir: PathBuf,

    /// Cells to test (default: all four BFB cells).
    #[arg(long, num_args = 1.., default_values = DEFAULT_CELLS)]
    cells: Vec<String>,

    /// Markdown report output.
    #[arg(long, default_value = "./repro-build-report.md")]
    report: PathBuf,
}

fn run(cmd: &mut Command, label: &str) -> Result<()> {
    let status = cmd
        .status()
        .with_context(|| format!("spawn {}", label))?;
    if !status.success() {
        bail!("{} failed: exit code {:?}", label, status.code());
    }
    Ok(())
}

fn force_clean_artefact(cells_dir: &Path, cell: &str) -> Result<()> {
    // Belt-and-braces: `cargo clean -p <cell>` reports "Removed 0 files" in
    // some cargo versions for cross-target builds, leaving stale artefacts.
    // Remove the specific .wasm + deps file directly to guarantee a fresh
    // codegen.
    let underscored = cell.replace('-', "_");
    let target_dir = cells_dir.join("target/wasm32-unknown-unknown/release");
    let _ = std::fs::remove_file(target_dir.join(format!("{}.wasm", underscored)));
    let _ = std::fs::remove_file(
        target_dir
            .join("deps")
            .join(format!("{}.wasm", underscored)),
    );
    Ok(())
}

fn cargo_clean_cell(cells_dir: &Path, cell: &str) -> Result<()> {
    run(
        Command::new("cargo")
            .arg("clean")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("-p")
            .arg(cell)
            .current_dir(cells_dir),
        &format!("cargo clean -p {}", cell),
    )?;
    force_clean_artefact(cells_dir, cell)?;
    Ok(())
}

fn cargo_build_cell(cells_dir: &Path, cell: &str) -> Result<()> {
    run(
        Command::new("cargo")
            .arg("build")
            .arg("--release")
            .arg("--no-default-features")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("-p")
            .arg(cell)
            .current_dir(cells_dir),
        &format!("cargo build -p {}", cell),
    )
}

fn artefact_path(cells_dir: &Path, cell: &str) -> PathBuf {
    let underscored = cell.replace('-', "_");
    cells_dir
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", underscored))
}

fn hash_artefact(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let h = blake3::hash(&bytes);
    Ok(hex::encode(h.as_bytes()))
}

fn build_and_hash(cells_dir: &Path, cells: &[String]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::with_capacity(cells.len());
    for cell in cells {
        cargo_clean_cell(cells_dir, cell)?;
        cargo_build_cell(cells_dir, cell)?;
        let path = artefact_path(cells_dir, cell);
        let hash = hash_artefact(&path)?;
        eprintln!("[repro-build]   {} → {}", cell, &hash[..16]);
        out.push((cell.clone(), hash));
    }
    Ok(out)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if !cli.cells_dir.exists() {
        bail!("cells dir not found: {}", cli.cells_dir.display());
    }
    let cells_dir = cli
        .cells_dir
        .canonicalize()
        .context("canonicalize cells dir")?;

    eprintln!("[repro-build] cells_dir={}", cells_dir.display());
    eprintln!("[repro-build] cells={:?}", cli.cells);

    let t0 = Instant::now();
    eprintln!("[repro-build] === run 1 ===");
    let run1 = build_and_hash(&cells_dir, &cli.cells)?;
    eprintln!("[repro-build] === run 2 ===");
    let run2 = build_and_hash(&cells_dir, &cli.cells)?;
    let total = t0.elapsed();

    let mut mismatches: Vec<&str> = Vec::new();
    let mut out = String::new();
    out.push_str("# open-ot reproducibility harness report\n\n");
    out.push_str(&format!("**Cells dir**: `{}`\n\n", cells_dir.display()));
    out.push_str(&format!("**Cells**: {:?}\n\n", cli.cells));
    out.push_str(&format!(
        "**Total wall-clock**: {:.3} s (both runs + cargo clean × 2 each)\n\n",
        total.as_secs_f64()
    ));
    out.push_str("## Results\n\n");
    out.push_str("| Cell | Run 1 BLAKE3 | Run 2 BLAKE3 | Match |\n");
    out.push_str("|---|---|---|---|\n");
    for ((c1, h1), (_c2, h2)) in run1.iter().zip(run2.iter()) {
        let ok = h1 == h2;
        if !ok {
            mismatches.push(c1);
        }
        out.push_str(&format!(
            "| `{}` | `{}` | `{}` | {} |\n",
            c1,
            &h1[..32],
            &h2[..32],
            if ok { "✓" } else { "✗" }
        ));
    }
    out.push_str("\n## Verdict\n\n");
    if mismatches.is_empty() {
        out.push_str("**PASS** — all cells produced byte-identical artefacts across two clean builds.\n\n");
    } else {
        out.push_str(&format!(
            "**FAIL** — mismatches in: {}\n\n",
            mismatches.join(", ")
        ));
    }
    out.push_str("## Notes\n\n");
    out.push_str("- Scope: `cargo build --release --target wasm32-unknown-unknown` only. The full SPEC §14.3 §2.1 deliverable also covers the WASM → AOT step via `wamrc`; that gets added post-Risk-1 PASS when Mimi Rev-1 hardware lands and the AOT build is wired in CI.\n");
    out.push_str("- The harness exits non-zero on any mismatch, so CI can gate on it directly.\n");
    out.push_str("- Cargo is normally deterministic given a pinned `Cargo.lock`. The harness exists to **prove** this empirically and to catch any future toolchain regression that breaks it.\n");

    std::fs::write(&cli.report, out).context("write report")?;
    eprintln!("[repro-build] report written: {}", cli.report.display());

    if !mismatches.is_empty() {
        bail!(
            "Reproducibility FAIL — mismatches in: {}",
            mismatches.join(", ")
        );
    }
    eprintln!("[repro-build] PASS — all cells byte-identical across two clean builds");
    Ok(())
}
