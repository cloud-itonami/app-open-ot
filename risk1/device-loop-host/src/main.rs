//! device-loop-host — run a REAL open-ot BFB cell (the deployment WASM
//! artefact, wasm32-unknown-unknown cdylib) under Wasmtime and let an external
//! plant simulation close the control loop over a line protocol on
//! stdin/stdout. The host knows nothing about plants or scenarios: it is the
//! device tier only (same artefact class WAMR executes on the Giemon Mimi/Te
//! Cortex-M7 field hardware).
//!
//! Protocol (one request line -> one reply line):
//!
//!   LOAD <wasm_path> <init_export> <tick_export> <out_event_width>
//!     -> OK loaded
//!   INIT <params_hex> <internal_size>
//!     -> OK rc=<i32>
//!   TICK <event_code> <ecc_state> <super_step> <data_out_size> <data_in_hex>
//!     -> OK state=<u8> event=<u16> out=<data_out_hex>
//!   QUIT
//!     -> OK bye
//!
//! Struct layouts are the caller's contract (use the generated pack/unpack
//! from 70-tools/scripts/open-ot/codegen-cell-types.py). Memory map mirrors
//! gate-a-rig: fixed high offsets well above the module's data region.

use anyhow::{anyhow, bail, Context, Result};
use std::io::{BufRead, Write};
use wasmtime::{Engine, Instance, Memory, Module, Store, TypedFunc};

const PAGE_SIZE: u64 = 65_536;
const PARAMS_OFF: u32 = 0x10_0000;
const INTERNAL_OFF: u32 = 0x11_0000;
const DATA_IN_OFF: u32 = 0x12_0000;
const DATA_OUT_OFF: u32 = 0x13_0000;
const OUT_EVENT_OFF: u32 = 0x14_0000;
const NEED_BYTES: u64 = 0x15_0000;

struct Cell {
    store: Store<()>,
    memory: Memory,
    init: TypedFunc<(i32, i32), i32>,
    #[allow(clippy::type_complexity)]
    tick: TypedFunc<(i32, i32, i32, i32, i32, i32, i32, i32, i32), i32>,
    out_event_width: u32,
}

fn hex_decode(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        bail!("odd hex length");
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex"))
        .collect()
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn load(path: &str, init_export: &str, tick_export: &str, out_event_width: u32) -> Result<Cell> {
    let engine = Engine::default();
    let module = Module::from_file(&engine, path).with_context(|| format!("load {path}"))?;
    let mut store: Store<()> = Store::new(&engine, ());
    let instance = Instance::new(&mut store, &module, &[])
        .context("Instance::new — no imports expected for wasm32-unknown-unknown cdylib")?;
    let memory = instance
        .get_memory(&mut store, "memory")
        .ok_or_else(|| anyhow!("module did not export `memory`"))?;

    let need_pages = NEED_BYTES.div_ceil(PAGE_SIZE);
    let cur = memory.size(&store);
    if cur < need_pages {
        memory
            .grow(&mut store, need_pages - cur)
            .context("memory.grow")?;
    }

    let init = instance
        .get_typed_func::<(i32, i32), i32>(&mut store, init_export)
        .with_context(|| format!("init export {init_export}"))?;
    let tick = instance
        .get_typed_func::<(i32, i32, i32, i32, i32, i32, i32, i32, i32), i32>(
            &mut store,
            tick_export,
        )
        .with_context(|| format!("tick export {tick_export}"))?;
    Ok(Cell { store, memory, init, tick, out_event_width })
}

fn handle(cell: &mut Option<Cell>, line: &str) -> Result<String> {
    let mut it = line.split_whitespace();
    match it.next().unwrap_or("") {
        "LOAD" => {
            let path = it.next().context("LOAD: missing wasm_path")?;
            let init_export = it.next().context("LOAD: missing init_export")?;
            let tick_export = it.next().context("LOAD: missing tick_export")?;
            let width: u32 = it.next().context("LOAD: missing out_event_width")?.parse()?;
            if width != 1 && width != 2 {
                bail!("out_event_width must be 1 or 2");
            }
            *cell = Some(load(path, init_export, tick_export, width)?);
            Ok("OK loaded".into())
        }
        "INIT" => {
            let c = cell.as_mut().context("INIT before LOAD")?;
            let params = hex_decode(it.next().context("INIT: missing params_hex")?)?;
            let internal_size: usize = it.next().context("INIT: missing internal_size")?.parse()?;
            c.memory.write(&mut c.store, PARAMS_OFF as usize, &params)?;
            c.memory
                .write(&mut c.store, INTERNAL_OFF as usize, &vec![0u8; internal_size])?;
            let rc = c
                .init
                .call(&mut c.store, (PARAMS_OFF as i32, INTERNAL_OFF as i32))?;
            Ok(format!("OK rc={rc}"))
        }
        "TICK" => {
            let c = cell.as_mut().context("TICK before LOAD")?;
            let event_code: i32 = it.next().context("TICK: missing event_code")?.parse()?;
            let ecc_state: i32 = it.next().context("TICK: missing ecc_state")?.parse()?;
            let super_step: u64 = it.next().context("TICK: missing super_step")?.parse()?;
            let data_out_size: usize = it.next().context("TICK: missing data_out_size")?.parse()?;
            let data_in = hex_decode(it.next().context("TICK: missing data_in_hex")?)?;
            c.memory.write(&mut c.store, DATA_IN_OFF as usize, &data_in)?;
            // Zero the out-event slot before the call (u16 covers both widths).
            c.memory
                .write(&mut c.store, OUT_EVENT_OFF as usize, &[0u8, 0u8])?;
            let state = c.tick.call(
                &mut c.store,
                (
                    event_code,
                    DATA_IN_OFF as i32,
                    ecc_state,
                    INTERNAL_OFF as i32,
                    PARAMS_OFF as i32,
                    (super_step & 0xffff_ffff) as i32,
                    (super_step >> 32) as i32,
                    DATA_OUT_OFF as i32,
                    OUT_EVENT_OFF as i32,
                ),
            )?;
            let mut out = vec![0u8; data_out_size];
            c.memory.read(&c.store, DATA_OUT_OFF as usize, &mut out)?;
            let mut ev = [0u8; 2];
            c.memory.read(&c.store, OUT_EVENT_OFF as usize, &mut ev)?;
            let event: u16 = if c.out_event_width == 1 {
                ev[0] as u16
            } else {
                u16::from_le_bytes(ev)
            };
            Ok(format!("OK state={state} event={event} out={}", hex_encode(&out)))
        }
        "QUIT" => Ok("OK bye".into()),
        other => bail!("unknown command {other:?}"),
    }
}

fn main() -> Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    let mut cell: Option<Cell> = None;
    for line in stdin.lock().lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let reply = match handle(&mut cell, trimmed) {
            Ok(r) => r,
            Err(e) => format!("ERR {e:#}"),
        };
        writeln!(stdout, "{reply}")?;
        stdout.flush()?;
        if trimmed == "QUIT" {
            break;
        }
    }
    Ok(())
}
