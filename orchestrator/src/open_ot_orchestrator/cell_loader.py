"""Wasmtime wrapper for an open-ot BFB cell.

Mirrors the layout used by `risk1/gate-a-rig/src/main.rs`: scratch buffers
at offset 0x10_0000 (1 MiB), inside a memory grown to 32 pages (2 MiB).
The C ABI exposed by each cell follows the convention in
`cells/CLAUDE.md` — `<name>_init(params_ptr, internal_ptr) -> i32` and
`<name>_tick(event_in, data_in_ptr, ecc_state, internal_ptr, params_ptr,
super_step_lo, super_step_hi, data_out_ptr, out_event_ptr) -> u8`.

Per-cell struct layouts are defined out-of-band in `microgrid_pregel.py`
(this module is layout-agnostic — it shuttles raw bytes).
"""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

import wasmtime


PAGE_SIZE = 65_536
SCRATCH_BASE = 0x10_0000  # 1 MiB
OFFSET_PARAMS = SCRATCH_BASE
OFFSET_INTERNAL = SCRATCH_BASE + 0x100
OFFSET_DATA_IN = SCRATCH_BASE + 0x200
OFFSET_DATA_OUT = SCRATCH_BASE + 0x300
OFFSET_OUT_EVENT = SCRATCH_BASE + 0x400
REQUIRED_PAGES = 32  # 2 MiB

# Out-event width — anti-islanding-rocof packs two events into u16; others u8.
OUT_EVENT_WIDTH_DEFAULT = 1
OUT_EVENT_WIDTH_PACKED_U16 = 2


@dataclass(frozen=True)
class TickResult:
    """Result of one cell tick."""

    next_ecc_state: int
    out_event_raw: int  # u8 or u16, depending on cell convention
    data_out_bytes: bytes


class CellLoader:
    """Loads one open-ot cell `.wasm` artefact and provides init / tick."""

    def __init__(
        self,
        wasm_path: str | Path,
        cell_symbol: str,
        out_event_width: int = OUT_EVENT_WIDTH_DEFAULT,
    ) -> None:
        wasm_path = Path(wasm_path)
        if not wasm_path.exists():
            raise FileNotFoundError(
                f"cell wasm not found at {wasm_path} — build it first:\n"
                f"  cd ../cells && cargo build --release --no-default-features "
                f"--target wasm32-unknown-unknown -p {cell_symbol.replace('_', '-')}"
            )
        self.wasm_path = wasm_path
        self.cell_symbol = cell_symbol
        self.out_event_width = out_event_width

        self._engine = wasmtime.Engine()
        self._module = wasmtime.Module.from_file(self._engine, str(wasm_path))
        self._store = wasmtime.Store(self._engine)
        self._instance = wasmtime.Instance(self._store, self._module, [])

        exports = self._instance.exports(self._store)
        self._memory: wasmtime.Memory = exports["memory"]
        cur_pages = self._memory.size(self._store)
        if cur_pages < REQUIRED_PAGES:
            self._memory.grow(self._store, REQUIRED_PAGES - cur_pages)

        self._init_fn = exports[f"{cell_symbol}_init"]
        self._tick_fn = exports[f"{cell_symbol}_tick"]

    # -- raw memory helpers ------------------------------------------------

    def _read(self, offset: int, length: int) -> bytes:
        return bytes(self._memory.read(self._store, offset, offset + length))

    def _write(self, offset: int, data: bytes) -> None:
        self._memory.write(self._store, data, offset)

    # -- ABI ---------------------------------------------------------------

    def init(self, params_bytes: bytes, internal_size: int) -> None:
        self._write(OFFSET_PARAMS, params_bytes)
        self._write(OFFSET_INTERNAL, b"\x00" * internal_size)
        rc = self._init_fn(self._store, OFFSET_PARAMS, OFFSET_INTERNAL)
        if rc != 0:
            raise RuntimeError(f"{self.cell_symbol}_init returned {rc}")

    def tick(
        self,
        event_in: int,
        data_in_bytes: bytes,
        ecc_state: int,
        super_step: int,
        data_out_size: int,
    ) -> TickResult:
        self._write(OFFSET_DATA_IN, data_in_bytes)
        # Zero out_event slot before call so a no-emit case reads as 0.
        self._write(OFFSET_OUT_EVENT, b"\x00" * self.out_event_width)
        next_ecc = self._tick_fn(
            self._store,
            event_in,
            OFFSET_DATA_IN,
            ecc_state,
            OFFSET_INTERNAL,
            OFFSET_PARAMS,
            super_step & 0xFFFF_FFFF,
            (super_step >> 32) & 0xFFFF_FFFF,
            OFFSET_DATA_OUT,
            OFFSET_OUT_EVENT,
        )
        out_event_bytes = self._read(OFFSET_OUT_EVENT, self.out_event_width)
        if self.out_event_width == 1:
            out_event_raw = out_event_bytes[0]
        elif self.out_event_width == 2:
            out_event_raw = int.from_bytes(out_event_bytes, "little", signed=False)
        else:
            raise ValueError(f"unsupported out_event_width={self.out_event_width}")
        data_out_bytes = self._read(OFFSET_DATA_OUT, data_out_size)
        return TickResult(
            next_ecc_state=next_ecc,
            out_event_raw=out_event_raw,
            data_out_bytes=data_out_bytes,
        )

    # -- checkpoint helpers -----------------------------------------------

    def get_internal_bytes(self, internal_size: int) -> bytes:
        return self._read(OFFSET_INTERNAL, internal_size)

    def set_internal_bytes(self, data: bytes) -> None:
        self._write(OFFSET_INTERNAL, data)

    def get_params_bytes(self, params_size: int) -> bytes:
        return self._read(OFFSET_PARAMS, params_size)
