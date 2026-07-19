"""Generated from `cells/black-start-seq/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : BLACK_START_SEQ
ECC      : ['Idle', 'Detecting', 'StartingGen', 'EnergizingBus', 'Syncing', 'Connected', 'Alarm'] (initial=Idle)
ABI      : init=black_start_seq_init  tick=black_start_seq_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'black-start-seq'
CELL_SYMBOL  = 'black_start_seq'
FBTYPE       = 'BLACK_START_SEQ'
INIT_EXPORT  = 'black_start_seq_init'
TICK_EXPORT  = 'black_start_seq_tick'
ECC_STATES   = ['Idle', 'Detecting', 'StartingGen', 'EnergizingBus', 'Syncing', 'Connected', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<BBBBB'
DATA_IN_SIZE = 5
DATA_IN_OFFSETS: dict[str, int] = {
    'grid_present': 0,
    'gen_ready': 1,
    'bus_voltage_stable': 2,
    'voltage_synced': 3,
    'authorised': 4,
}


@dataclass
class DataIn:
    grid_present: bool  # bool (wire: boolean)
    gen_ready: bool  # bool (wire: boolean)
    bus_voltage_stable: bool  # bool (wire: boolean)
    voltage_synced: bool  # bool (wire: boolean)
    authorised: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, (1 if d.grid_present else 0), (1 if d.gen_ready else 0), (1 if d.bus_voltage_stable else 0), (1 if d.voltage_synced else 0), (1 if d.authorised else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(grid_present=bool(_t[0]), gen_ready=bool(_t[1]), bus_voltage_stable=bool(_t[2]), voltage_synced=bool(_t[3]), authorised=bool(_t[4]))



DATA_OUT_FMT = '<BB2xIB3x'
DATA_OUT_SIZE = 12
DATA_OUT_OFFSETS: dict[str, int] = {
    'stage': 0,
    'command': 1,
    'dwell_remaining_ms': 4,
    'connected': 8,
}


@dataclass
class DataOut:
    stage: int  # u8 (wire: stageEnum)
    command: int  # u8 (wire: blackStartCommandEnum)
    dwell_remaining_ms: int  # u32 (wire: durationMs)
    connected: bool  # bool (wire: boolean)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.stage, d.command, d.dwell_remaining_ms, (1 if d.connected else 0))


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(stage=_t[0], command=_t[1], dwell_remaining_ms=_t[2], connected=bool(_t[3]))



PARAMS_FMT = '<IIIII'
PARAMS_SIZE = 20
PARAMS_OFFSETS: dict[str, int] = {
    'detect_dwell_ms': 0,
    'gen_timeout_ms': 4,
    'bus_timeout_ms': 8,
    'sync_timeout_ms': 12,
    'cycle_period_ms': 16,
}


@dataclass
class Params:
    detect_dwell_ms: int  # u32
    gen_timeout_ms: int  # u32
    bus_timeout_ms: int  # u32
    sync_timeout_ms: int  # u32
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.detect_dwell_ms, d.gen_timeout_ms, d.bus_timeout_ms, d.sync_timeout_ms, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(detect_dwell_ms=_t[0], gen_timeout_ms=_t[1], bus_timeout_ms=_t[2], sync_timeout_ms=_t[3], cycle_period_ms=_t[4])



INTERNAL_FMT = '<IBB2x'
INTERNAL_SIZE = 8
INTERNAL_OFFSETS: dict[str, int] = {
    'stage_timer_ms': 0,
    'current_stage': 4,
    'initialized': 5,
}


@dataclass
class Internal:
    stage_timer_ms: int  # u32
    current_stage: int  # u8
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.stage_timer_ms, d.current_stage, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(stage_timer_ms=_t[0], current_stage=_t[1], initialized=bool(_t[2]))


