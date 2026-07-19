//! Risk-1 Gate B simulator (host approximation of SPEC §14.2).
//!
//! Models the `:loop:freq-droop` analogue from PROTOTYPE-MICROGRID.md §2.3:
//! N field cells (default 12 instances of `droop_p_f.wasm`) running in
//! sequence per super-step, plus an aggregator that sums per-asset Δp into a
//! cohort setpoint, plus a per-super-step checkpoint write, plus randomised
//! controller crashes that drop every cell instance and force a
//! resume-from-checkpoint.
//!
//! What this rig validates on the host:
//!
//! 1. **Super-step latency** (`p99 ≤ 50 ms` per SPEC §14.2). The host is much
//!    faster than the Mimi+TSN target, so a host run that already misses
//!    50 ms means the per-super-step Pregel cost is structurally wrong —
//!    independent of TSN gating.
//! 2. **Checkpoint write latency** (`p99 ≤ 100 ms`). Disk file + rename;
//!    the rig writes to `--checkpoint-dir` (defaults to a temp dir under the
//!    current cwd). For SPEC the target is the RisingWave-backed sqlite
//!    stand-in; on host we substitute fsync on local fs.
//! 3. **Zero in-flight message loss across crash**. The injected crash
//!    happens *between* super-steps, after the checkpoint is durable. On
//!    resume the rig re-instantiates 12 cells from a fresh `Module`, copies
//!    the saved `Internal` bytes back into linear memory, and runs the
//!    next super-step. The aggregator output is compared against a
//!    deterministic reference (same DataIn synthesis function); any
//!    mismatch is counted as message loss.
//! 4. **Resume latency** (`≤ 5 s` per SPEC). Measured from crash signal to
//!    first successful post-resume super-step.
//!
//! Out of scope: real TSN gate windows, network-side message delivery,
//! genuine kill-9 of a separate process. The rig is a single in-process
//! simulator. The decision matrix in SPEC §14.2 still ranks "host PASS, HW
//! TBD" above "host FAIL".
//!
//! Per ADR-2605151200 + SPEC §14.2.

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::time::Instant;
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(name = "gate-b-rig", version, about = "Risk-1 Gate B simulator")]
struct Cli {
    /// Path to `droop_p_f.wasm`. The 12-cell `:loop:freq-droop` analogue
    /// uses identical cell code per asset; field-side heterogeneity is
    /// expressed via per-instance Params (rated power, droop) rather than
    /// distinct cell types.
    #[arg(
        long,
        default_value = "../../cells/target/wasm32-unknown-unknown/release/droop_p_f.wasm"
    )]
    wasm_path: PathBuf,

    /// Number of field cells. SPEC §14.2 default is 12.
    #[arg(long, default_value_t = 12u32)]
    num_cells: u32,

    /// Number of super-steps to run.
    #[arg(long, default_value_t = 1000u64)]
    super_steps: u64,

    /// Cycle period in milliseconds (Params.cycle_period_ms; informational —
    /// the host rig runs super-steps as fast as possible).
    #[arg(long, default_value_t = 100u32)]
    cycle_period_ms: u32,

    /// Number of crash events to inject across the run. SPEC §14.2 fault
    /// injection: 1 controller crash + 3 device crashes = 4 events.
    #[arg(long, default_value_t = 4u32)]
    crash_count: u32,

    /// Seed for the deterministic crash-event scheduler.
    #[arg(long, default_value_t = 0x4753ABCD_u64)]
    seed: u64,

    /// Directory for checkpoint files. Created if missing.
    #[arg(long, default_value = "./gate-b-checkpoints")]
    checkpoint_dir: PathBuf,

    /// Super-step deadline in nanoseconds. SPEC §14.2 PASS: p99 ≤ 50 ms.
    #[arg(long, default_value_t = 50_000_000u64)]
    deadline_superstep_ns: u64,

    /// Checkpoint-write deadline in nanoseconds. SPEC §14.2 PASS: p99 ≤ 100 ms.
    #[arg(long, default_value_t = 100_000_000u64)]
    deadline_checkpoint_ns: u64,

    /// Max post-crash resume duration in nanoseconds. SPEC §14.2 PASS: ≤ 5 s.
    #[arg(long, default_value_t = 5_000_000_000u64)]
    max_resume_ns: u64,

    /// Markdown report output.
    #[arg(long, default_value = "../gate-b-report.md")]
    report: PathBuf,
}

// ---------------------------------------------------------------------------
// Cell layout (droop_p_f only — single-cell-type loop)
// ---------------------------------------------------------------------------

// Layouts mirror cells/droop-p-f/src/lib.rs. Kept in sync by hand; gate-a-rig
// has the same constants. A future refactor should publish these from
// `openot-bfb-rs` so both rigs and the production runtime read one source.
const PARAMS_SIZE: u32 = 24;
const INTERNAL_SIZE: u32 = 8;
const DATA_IN_SIZE: u32 = 24;
const DATA_OUT_SIZE: u32 = 24;
const OUT_EVENT_SIZE: u32 = 1;
const SCRATCH_BASE: u32 = 0x10_0000; // 1 MiB
const REQUIRED_PAGES: u64 = 32; // 2 MiB

#[derive(Copy, Clone)]
struct MemMap {
    params: u32,
    internal: u32,
    data_in: u32,
    data_out: u32,
    out_event: u32,
}

fn lay_out() -> MemMap {
    fn pad16(o: u32) -> u32 {
        (o + 15) & !15
    }
    let params = SCRATCH_BASE;
    let internal = pad16(params + PARAMS_SIZE);
    let data_in = pad16(internal + INTERNAL_SIZE);
    let data_out = pad16(data_in + DATA_IN_SIZE);
    let out_event = pad16(data_out + DATA_OUT_SIZE);
    MemMap {
        params,
        internal,
        data_in,
        data_out,
        out_event,
    }
}

fn build_params(cell_idx: u32, cycle_period_ms: u32) -> Vec<u8> {
    // Per-asset rated power varies so the cohort isn't 12× the same Δp.
    // 50–600 kW spread (12 assets × 50 kW step) — realistic community
    // microgrid asset mix per PROTOTYPE-MICROGRID §1.
    let p_rated_micro_kw: i32 = 50_000_000_i32.saturating_add((cell_idx as i32) * 50_000_000);
    let mut buf = Vec::with_capacity(PARAMS_SIZE as usize);
    buf.extend_from_slice(&p_rated_micro_kw.to_le_bytes());
    buf.extend_from_slice(&(-p_rated_micro_kw).to_le_bytes()); // p_min_micro_kw
    buf.extend_from_slice(&p_rated_micro_kw.to_le_bytes()); // p_max_micro_kw
    buf.extend_from_slice(&50_i32.to_le_bytes()); // 5 % droop
    buf.extend_from_slice(&200_000_i32.to_le_bytes()); // 0.2 Hz deadband
    buf.extend_from_slice(&cycle_period_ms.to_le_bytes());
    buf
}

fn synthesize_data_in(super_step: u64, cell_idx: u32) -> Vec<u8> {
    // Grid frequency is shared across all assets in a super-step; per-asset
    // current_p is staggered so each cell sees a different setpoint pressure.
    let drift_micro_hz =
        ((super_step as i64).wrapping_mul(311) % 1_000_001) - 500_000;
    let grid_freq_micro_hz: i64 = 50_000_000_i64.saturating_add(drift_micro_hz);
    let freq_nominal_micro_hz: i64 = 50_000_000;
    let p_rated: i32 = 50_000_000_i32.saturating_add((cell_idx as i32) * 50_000_000);
    let current_p_micro_kw: i32 = p_rated / 2; // running at 50 % each
    let mut buf = Vec::with_capacity(DATA_IN_SIZE as usize);
    buf.extend_from_slice(&grid_freq_micro_hz.to_le_bytes());
    buf.extend_from_slice(&freq_nominal_micro_hz.to_le_bytes());
    buf.extend_from_slice(&current_p_micro_kw.to_le_bytes());
    buf.push(0); // freq_quality = Good
    buf.push(1); // enable
    buf.extend_from_slice(&[0u8; 2]); // tail-pad → 24
    buf
}

fn read_data_out_delta_p(memory: &Memory, store: &mut Store<()>, base: u32) -> Result<i32> {
    // delta_p_micro_kw is the second i32 of DataOut (offset 4..8).
    let mut buf = [0u8; 4];
    memory.read(store, (base + 4) as usize, &mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

// ---------------------------------------------------------------------------
// Per-cell state — one Wasmtime store per field device
// ---------------------------------------------------------------------------

type TickArgs = (i32, i32, i32, i32, i32, i32, i32, i32, i32);

struct FieldCell {
    cell_idx: u32,
    store: Store<()>,
    memory: Memory,
    tick_fn: TypedFunc<TickArgs, i32>,
    mem: MemMap,
    last_ecc: u8,
}

impl FieldCell {
    fn spawn(
        engine: &Engine,
        module: &Module,
        cell_idx: u32,
        cycle_period_ms: u32,
        seed_internal: Option<&[u8]>,
    ) -> Result<Self> {
        let mut store: Store<()> = Store::new(engine, ());
        let instance = Instance::new(&mut store, module, &[])
            .context("Instance::new for droop_p_f")?;
        let memory: Memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| anyhow!("droop_p_f did not export `memory`"))?;
        let cur_pages = memory.size(&mut store);
        if cur_pages < REQUIRED_PAGES {
            memory
                .grow(&mut store, REQUIRED_PAGES - cur_pages)
                .context("memory.grow")?;
        }
        let init_fn: TypedFunc<(i32, i32), i32> = instance
            .get_typed_func(&mut store, "droop_p_f_init")
            .context("get droop_p_f_init")?;
        let tick_fn: TypedFunc<TickArgs, i32> = instance
            .get_typed_func(&mut store, "droop_p_f_tick")
            .context("get droop_p_f_tick")?;

        let mem = lay_out();
        let params_bytes = build_params(cell_idx, cycle_period_ms);
        memory
            .write(&mut store, mem.params as usize, &params_bytes)
            .context("write Params")?;
        if let Some(blob) = seed_internal {
            if blob.len() != INTERNAL_SIZE as usize {
                bail!(
                    "checkpoint Internal size {} != expected {}",
                    blob.len(),
                    INTERNAL_SIZE
                );
            }
            memory
                .write(&mut store, mem.internal as usize, blob)
                .context("write Internal from checkpoint")?;
            // Skip init — checkpoint is authoritative.
        } else {
            memory
                .write(
                    &mut store,
                    mem.internal as usize,
                    &vec![0u8; INTERNAL_SIZE as usize],
                )
                .context("zero Internal")?;
            let rc = init_fn
                .call(&mut store, (mem.params as i32, mem.internal as i32))
                .context("call droop_p_f_init")?;
            if rc != 0 {
                bail!("droop_p_f_init returned {}", rc);
            }
        }
        Ok(Self {
            cell_idx,
            store,
            memory,
            tick_fn,
            mem,
            last_ecc: 0,
        })
    }

    fn tick(&mut self, super_step: u64) -> Result<i32> {
        let data_in = synthesize_data_in(super_step, self.cell_idx);
        self.memory
            .write(&mut self.store, self.mem.data_in as usize, &data_in)?;
        self.memory.write(
            &mut self.store,
            self.mem.out_event as usize,
            &[0u8; OUT_EVENT_SIZE as usize],
        )?;
        let next_ecc = self.tick_fn.call(
            &mut self.store,
            (
                0i32, // EventIn::Req
                self.mem.data_in as i32,
                self.last_ecc as i32,
                self.mem.internal as i32,
                self.mem.params as i32,
                (super_step & 0xFFFF_FFFF) as i32,
                (super_step >> 32) as i32,
                self.mem.data_out as i32,
                self.mem.out_event as i32,
            ),
        )?;
        self.last_ecc = next_ecc as u8;
        read_data_out_delta_p(&self.memory, &mut self.store, self.mem.data_out)
    }

    fn snapshot_internal(&mut self) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; INTERNAL_SIZE as usize];
        self.memory
            .read(&mut self.store, self.mem.internal as usize, &mut buf)?;
        Ok(buf)
    }
}

// ---------------------------------------------------------------------------
// Checkpoint format
// ---------------------------------------------------------------------------

// Hand-rolled binary: 1 + 8 + 4 + N×INTERNAL_SIZE + 4 (aggregate) = small,
// fixed, easy to fsync atomically via tmp + rename.
//
// Layout:
//   u8 magic = 0x42 ('B' for Gate B)
//   u64 super_step (last completed)
//   u32 num_cells
//   [num_cells] × INTERNAL_SIZE bytes
//   i32 last_aggregate_delta_p
//   i64 wall_ns at write

const CKPT_MAGIC: u8 = 0x42;

struct Checkpoint {
    super_step: u64,
    num_cells: u32,
    internals: Vec<Vec<u8>>,
    last_aggregate: i32,
}

fn write_checkpoint(dir: &Path, ck: &Checkpoint) -> Result<u64> {
    std::fs::create_dir_all(dir).context("create checkpoint dir")?;
    let path = dir.join("ckpt.bin");
    let tmp = dir.join("ckpt.bin.tmp");
    let mut buf: Vec<u8> = Vec::with_capacity(
        1 + 8 + 4 + (ck.num_cells as usize) * (INTERNAL_SIZE as usize) + 4 + 8,
    );
    buf.push(CKPT_MAGIC);
    buf.extend_from_slice(&ck.super_step.to_le_bytes());
    buf.extend_from_slice(&ck.num_cells.to_le_bytes());
    for blob in &ck.internals {
        if blob.len() != INTERNAL_SIZE as usize {
            bail!("internal blob size mismatch");
        }
        buf.extend_from_slice(blob);
    }
    buf.extend_from_slice(&ck.last_aggregate.to_le_bytes());
    buf.extend_from_slice(&(0i64).to_le_bytes()); // reserved

    let t0 = Instant::now();
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp).context("open ckpt tmp")?;
        f.write_all(&buf).context("write ckpt tmp")?;
        f.sync_all().context("fsync ckpt tmp")?;
    }
    std::fs::rename(&tmp, &path).context("rename ckpt")?;
    Ok(t0.elapsed().as_nanos() as u64)
}

fn read_checkpoint(dir: &Path) -> Result<Checkpoint> {
    let path = dir.join("ckpt.bin");
    let buf = std::fs::read(&path).context("read ckpt")?;
    if buf.first().copied() != Some(CKPT_MAGIC) {
        bail!("checkpoint magic byte mismatch");
    }
    let super_step = u64::from_le_bytes(buf[1..9].try_into().unwrap());
    let num_cells = u32::from_le_bytes(buf[9..13].try_into().unwrap());
    let mut internals = Vec::with_capacity(num_cells as usize);
    let base = 13;
    for i in 0..num_cells as usize {
        let off = base + i * INTERNAL_SIZE as usize;
        internals.push(buf[off..off + INTERNAL_SIZE as usize].to_vec());
    }
    let agg_off = base + (num_cells as usize) * (INTERNAL_SIZE as usize);
    let last_aggregate = i32::from_le_bytes(buf[agg_off..agg_off + 4].try_into().unwrap());
    Ok(Checkpoint {
        super_step,
        num_cells,
        internals,
        last_aggregate,
    })
}

// ---------------------------------------------------------------------------
// Crash schedule (deterministic from --seed + --super-steps + --crash-count)
// ---------------------------------------------------------------------------

fn crash_super_steps(seed: u64, super_steps: u64, crash_count: u32) -> Vec<u64> {
    // splitmix64 to pick K distinct super-steps in [1, super_steps - 1].
    // Deterministic schedule so reruns produce identical reports.
    let mut x = seed.wrapping_add(0x9E3779B97F4A7C15);
    let mut out: Vec<u64> = Vec::with_capacity(crash_count as usize);
    let mut tries = 0;
    while out.len() < crash_count as usize && tries < crash_count as usize * 16 {
        x = x.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z = z ^ (z >> 31);
        if super_steps < 2 {
            break;
        }
        let candidate = (z % (super_steps - 1)) + 1;
        if !out.contains(&candidate) {
            out.push(candidate);
        }
        tries += 1;
    }
    out.sort();
    out
}

// ---------------------------------------------------------------------------
// Histogram (copy of gate-a-rig)
// ---------------------------------------------------------------------------

struct Histogram {
    samples_ns: Vec<u64>,
}

impl Histogram {
    fn new(cap: usize) -> Self {
        Self {
            samples_ns: Vec::with_capacity(cap),
        }
    }
    fn record(&mut self, ns: u64) {
        self.samples_ns.push(ns);
    }
    fn summary(mut self) -> HistogramSummary {
        self.samples_ns.sort_unstable();
        let n = self.samples_ns.len();
        let pct = |p: f64| -> u64 {
            if n == 0 {
                return 0;
            }
            let idx = (((n - 1) as f64) * p / 100.0).round() as usize;
            self.samples_ns[idx]
        };
        let sum: u128 = self.samples_ns.iter().map(|&x| x as u128).sum();
        let mean = if n > 0 { (sum / n as u128) as u64 } else { 0 };
        HistogramSummary {
            n: n as u64,
            min: *self.samples_ns.first().unwrap_or(&0),
            max: *self.samples_ns.last().unwrap_or(&0),
            mean_ns: mean,
            p50_ns: pct(50.0),
            p90_ns: pct(90.0),
            p99_ns: pct(99.0),
            p99_9_ns: pct(99.9),
        }
    }
}

struct HistogramSummary {
    n: u64,
    min: u64,
    max: u64,
    mean_ns: u64,
    p50_ns: u64,
    p90_ns: u64,
    p99_ns: u64,
    p99_9_ns: u64,
}

// ---------------------------------------------------------------------------
// Driver
// ---------------------------------------------------------------------------

fn spawn_field(
    engine: &Engine,
    module: &Module,
    num_cells: u32,
    cycle_period_ms: u32,
    seeds: Option<&[Vec<u8>]>,
) -> Result<Vec<FieldCell>> {
    let mut cells = Vec::with_capacity(num_cells as usize);
    for i in 0..num_cells {
        let seed = seeds.map(|s| s[i as usize].as_slice());
        cells.push(FieldCell::spawn(engine, module, i, cycle_period_ms, seed)?);
    }
    Ok(cells)
}

fn run_one_super_step(cells: &mut [FieldCell], super_step: u64) -> Result<i32> {
    // Aggregator: sum delta_p across cells. In production this is one Pregel
    // super-step where each field cell's emitted Δp is fanned into the
    // orchestrator node; here we approximate by direct sum.
    let mut agg: i32 = 0;
    for c in cells.iter_mut() {
        let dp = c.tick(super_step)?;
        agg = agg.saturating_add(dp);
    }
    Ok(agg)
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if !cli.wasm_path.exists() {
        bail!(
            "wasm artefact not found at {}\n\
             Build it first:\n  cd ../../cells && cargo build --release \\\n    --no-default-features --target wasm32-unknown-unknown -p droop-p-f",
            cli.wasm_path.display()
        );
    }

    eprintln!("[gate-b-rig] wasm={}", cli.wasm_path.display());
    eprintln!(
        "[gate-b-rig] num_cells={} super_steps={} crashes={}",
        cli.num_cells, cli.super_steps, cli.crash_count
    );
    eprintln!("[gate-b-rig] checkpoint_dir={}", cli.checkpoint_dir.display());

    let engine = Engine::default();
    let module = Module::from_file(&engine, &cli.wasm_path).context("load module")?;

    // Crash schedule.
    let crashes = crash_super_steps(cli.seed, cli.super_steps, cli.crash_count);
    eprintln!(
        "[gate-b-rig] crash super-steps: {:?}",
        crashes.iter().collect::<Vec<_>>()
    );

    // Reference run: a parallel non-crashed shadow of the same cells, used to
    // verify zero in-flight loss across crash boundaries. The reference run
    // is also the source of expected aggregate per super-step.
    let mut reference = spawn_field(&engine, &module, cli.num_cells, cli.cycle_period_ms, None)?;
    let mut expected: Vec<i32> = Vec::with_capacity(cli.super_steps as usize);
    for s in 0..cli.super_steps {
        let agg = run_one_super_step(&mut reference, s)?;
        expected.push(agg);
    }
    drop(reference);

    // Live run with crashes.
    let mut cells = spawn_field(&engine, &module, cli.num_cells, cli.cycle_period_ms, None)?;
    let mut hist_step = Histogram::new(cli.super_steps as usize);
    let mut hist_ckpt = Histogram::new(cli.super_steps as usize);
    let mut hist_resume = Histogram::new(cli.crash_count as usize);
    let mut step_misses: u64 = 0;
    let mut ckpt_misses: u64 = 0;
    let mut resume_misses: u64 = 0;
    let mut messages_lost: u64 = 0;
    let mut crashes_done: u32 = 0;

    let mut crash_iter = crashes.iter().peekable();

    let total_start = Instant::now();
    for s in 0..cli.super_steps {
        let step_start = Instant::now();
        let agg = run_one_super_step(&mut cells, s)?;
        let step_ns = step_start.elapsed().as_nanos() as u64;
        hist_step.record(step_ns);
        if step_ns > cli.deadline_superstep_ns {
            step_misses += 1;
        }

        if agg != expected[s as usize] {
            messages_lost += 1;
            eprintln!(
                "[gate-b-rig] WARN super_step={} agg={} expected={}",
                s, agg, expected[s as usize]
            );
        }

        // Snapshot all 12 internals + write checkpoint.
        let mut internals: Vec<Vec<u8>> = Vec::with_capacity(cli.num_cells as usize);
        for c in cells.iter_mut() {
            internals.push(c.snapshot_internal()?);
        }
        let ck = Checkpoint {
            super_step: s,
            num_cells: cli.num_cells,
            internals,
            last_aggregate: agg,
        };
        let ckpt_ns = write_checkpoint(&cli.checkpoint_dir, &ck)?;
        hist_ckpt.record(ckpt_ns);
        if ckpt_ns > cli.deadline_checkpoint_ns {
            ckpt_misses += 1;
        }

        // Inject crash if this super-step is on the schedule.
        if crash_iter.peek().copied() == Some(&s) {
            crash_iter.next();
            let crash_start = Instant::now();
            // Drop the live cells and re-instantiate from scratch + checkpoint.
            drop(cells);
            let ck_disk = read_checkpoint(&cli.checkpoint_dir)?;
            if ck_disk.super_step != s {
                bail!(
                    "checkpoint super_step {} != current {}",
                    ck_disk.super_step,
                    s
                );
            }
            cells = spawn_field(
                &engine,
                &module,
                cli.num_cells,
                cli.cycle_period_ms,
                Some(&ck_disk.internals),
            )?;
            let resume_ns = crash_start.elapsed().as_nanos() as u64;
            hist_resume.record(resume_ns);
            if resume_ns > cli.max_resume_ns {
                resume_misses += 1;
            }
            crashes_done += 1;
            eprintln!(
                "[gate-b-rig] crash {} at super_step={} → resume in {:.3} ms",
                crashes_done,
                s,
                resume_ns as f64 / 1e6
            );
        }
    }
    let total_elapsed = total_start.elapsed();

    let step_sum = hist_step.summary();
    let ckpt_sum = hist_ckpt.summary();
    let resume_sum = hist_resume.summary();

    let mut pass = true;
    if step_sum.p99_ns > cli.deadline_superstep_ns {
        pass = false;
    }
    if ckpt_sum.p99_ns > cli.deadline_checkpoint_ns {
        pass = false;
    }
    if messages_lost > 0 {
        pass = false;
    }
    if resume_sum.max > cli.max_resume_ns {
        pass = false;
    }

    // Report.
    let mut out = String::new();
    out.push_str("# Risk-1 Gate B — host simulator report\n\n");
    out.push_str(&format!(
        "**Wasm artefact**: `{}`\n\n",
        cli.wasm_path.display()
    ));
    out.push_str(&format!("**Field cells**: {} (droop_p_f)\n\n", cli.num_cells));
    out.push_str(&format!("**Super-steps**: {}\n\n", cli.super_steps));
    out.push_str(&format!(
        "**Cycle period**: {} ms (informational)\n\n",
        cli.cycle_period_ms
    ));
    out.push_str(&format!(
        "**Crashes injected**: {} of {} scheduled — at super-steps {:?}\n\n",
        crashes_done, cli.crash_count, crashes
    ));
    out.push_str(&format!(
        "**Checkpoint dir**: `{}`\n\n",
        cli.checkpoint_dir.display()
    ));
    out.push_str(&format!(
        "**Total wall-clock**: {:.3} s\n\n",
        total_elapsed.as_secs_f64()
    ));

    out.push_str("## Super-step latency\n\n");
    out.push_str("| Stat | Value (ns) | Value (ms) |\n|---|---|---|\n");
    fmt_row(&mut out, &step_sum);
    out.push_str(&format!(
        "\n- Misses (>{} ms): {} / {}\n\n",
        cli.deadline_superstep_ns / 1_000_000,
        step_misses,
        cli.super_steps
    ));

    out.push_str("## Checkpoint write latency\n\n");
    out.push_str("| Stat | Value (ns) | Value (ms) |\n|---|---|---|\n");
    fmt_row(&mut out, &ckpt_sum);
    out.push_str(&format!(
        "\n- Misses (>{} ms): {} / {}\n\n",
        cli.deadline_checkpoint_ns / 1_000_000,
        ckpt_misses,
        cli.super_steps
    ));

    out.push_str("## Resume-from-checkpoint latency\n\n");
    if resume_sum.n == 0 {
        out.push_str("- (no crashes injected — set --crash-count > 0 to populate this section)\n\n");
    } else {
        out.push_str("| Stat | Value (ns) | Value (ms) |\n|---|---|---|\n");
        fmt_row(&mut out, &resume_sum);
        out.push_str(&format!(
            "\n- Misses (>{} s): {} / {}\n\n",
            cli.max_resume_ns / 1_000_000_000,
            resume_misses,
            resume_sum.n
        ));
    }

    out.push_str("## Message-loss check\n\n");
    out.push_str(&format!(
        "- Mismatched aggregates (live vs. reference): {} / {}\n\n",
        messages_lost, cli.super_steps
    ));

    out.push_str("## Verdict\n\n");
    out.push_str(&format!(
        "- Host verdict: **{}**\n",
        if pass { "PASS" } else { "FAIL" }
    ));
    out.push_str("- SPEC §14.2 PASS thresholds:\n");
    out.push_str("  - super-step p99 ≤ 50 ms\n");
    out.push_str("  - checkpoint p99 ≤ 100 ms\n");
    out.push_str("  - zero in-flight message loss across crashes\n");
    out.push_str("  - resume ≤ 5 s\n");
    out.push_str(&format!(
        "- Observed: step p99 = {:.3} ms; ckpt p99 = {:.3} ms; resume max = {:.3} ms; messages_lost = {}.\n\n",
        step_sum.p99_ns as f64 / 1e6,
        ckpt_sum.p99_ns as f64 / 1e6,
        resume_sum.max as f64 / 1e6,
        messages_lost
    ));

    out.push_str("## Notes\n\n");
    out.push_str("- This is a **host simulator**, not the SPEC §14.2 fleet test. Real Gate B requires 3 × Atama + 12 × Mimi/Te + TSN switch + 24 h soak (per SPEC §14.2 table).\n");
    out.push_str("- The simulator approximates the four PASS criteria; in particular it skips genuine TSN gate windows and process-level kill -9 (crashes are in-process drops). A host PASS is necessary but not sufficient.\n");
    out.push_str("- Checkpoint format is a fixed-size binary (magic + super_step + N × Internal + aggregate). Production uses RisingWave-backed sqlite stand-in (`orchestrator/src/open_ot_orchestrator/checkpointer.py`); the host rig substitutes fsync on local fs.\n");
    out.push_str("- Aggregator value is the cohort Δp sum across 12 cells (mirrors the `:loop:freq-droop` topology in PROTOTYPE-MICROGRID §2.3 §13.2).\n");

    std::fs::write(&cli.report, out).context("write report")?;
    eprintln!("[gate-b-rig] report written: {}", cli.report.display());
    eprintln!(
        "[gate-b-rig] step p99={:.3} ms  ckpt p99={:.3} ms  resume max={:.3} ms  messages_lost={}",
        step_sum.p99_ns as f64 / 1e6,
        ckpt_sum.p99_ns as f64 / 1e6,
        resume_sum.max as f64 / 1e6,
        messages_lost
    );

    if !pass {
        bail!(
            "Gate B host check failed (step p99 {} ns, ckpt p99 {} ns, resume max {} ns, messages_lost {})",
            step_sum.p99_ns,
            ckpt_sum.p99_ns,
            resume_sum.max,
            messages_lost
        );
    }
    Ok(())
}

fn fmt_row(out: &mut String, s: &HistogramSummary) {
    let r = |label: &str, v: u64| {
        format!("| {:<8} | {} | {:.3} |\n", label, v, v as f64 / 1e6)
    };
    out.push_str(&r("n", s.n));
    out.push_str(&r("min", s.min));
    out.push_str(&r("mean", s.mean_ns));
    out.push_str(&r("p50", s.p50_ns));
    out.push_str(&r("p90", s.p90_ns));
    out.push_str(&r("p99", s.p99_ns));
    out.push_str(&r("p99.9", s.p99_9_ns));
    out.push_str(&r("max", s.max));
}
