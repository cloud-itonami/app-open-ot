"""Generated from `cells/droop-p-f/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : DROOP_P_F
ECC      : ['Idle', 'WithinDeadband', 'Responding', 'Saturated', 'Alarm'] (initial=Idle)
ABI      : init=droop_p_f_init  tick=droop_p_f_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'droop-p-f'
CELL_SYMBOL  = 'droop_p_f'
FBTYPE       = 'DROOP_P_F'
INIT_EXPORT  = 'droop_p_f_init'
TICK_EXPORT  = 'droop_p_f_tick'
ECC_STATES   = ['Idle', 'WithinDeadband', 'Responding', 'Saturated', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<qqiBB2x'
DATA_IN_SIZE = 24
DATA_IN_OFFSETS: dict[str, int] = {
    'grid_freq': 0,
    'freq_nominal': 8,
    'current_p': 16,
    'freq_quality': 20,
    'enable': 21,
}


@dataclass
class DataIn:
    grid_freq: int  # i64 (wire: valueMicroUnit)
    freq_nominal: int  # i64 (wire: valueMicroUnit)
    current_p: int  # i32 (wire: valueMicroUnit)
    freq_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.grid_freq, d.freq_nominal, d.current_p, d.freq_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(grid_freq=_t[0], freq_nominal=_t[1], current_p=_t[2], freq_quality=_t[3], enable=bool(_t[4]))



DATA_OUT_FMT = '<iiqBB6x'
DATA_OUT_SIZE = 24
DATA_OUT_OFFSETS: dict[str, int] = {
    'p_setpoint': 0,
    'delta_p': 4,
    'freq_error': 8,
    'dead_band_active': 16,
    'saturated': 17,
}


@dataclass
class DataOut:
    p_setpoint: int  # i32 (wire: valueMicroUnit)
    delta_p: int  # i32 (wire: valueMicroUnit)
    freq_error: int  # i64 (wire: valueMicroUnit)
    dead_band_active: bool  # bool (wire: boolean)
    saturated: bool  # bool (wire: boolean)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.p_setpoint, d.delta_p, d.freq_error, (1 if d.dead_band_active else 0), (1 if d.saturated else 0))


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(p_setpoint=_t[0], delta_p=_t[1], freq_error=_t[2], dead_band_active=bool(_t[3]), saturated=bool(_t[4]))



PARAMS_FMT = '<iiiiiI'
PARAMS_SIZE = 24
PARAMS_OFFSETS: dict[str, int] = {
    'p_rated_micro_kw': 0,
    'p_min_micro_kw': 4,
    'p_max_micro_kw': 8,
    'droop_permille': 12,
    'dead_band_micro_hz': 16,
    'cycle_period_ms': 20,
}


@dataclass
class Params:
    p_rated_micro_kw: int  # i32
    p_min_micro_kw: int  # i32
    p_max_micro_kw: int  # i32
    droop_permille: int  # i32
    dead_band_micro_hz: int  # i32
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.p_rated_micro_kw, d.p_min_micro_kw, d.p_max_micro_kw, d.droop_permille, d.dead_band_micro_hz, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(p_rated_micro_kw=_t[0], p_min_micro_kw=_t[1], p_max_micro_kw=_t[2], droop_permille=_t[3], dead_band_micro_hz=_t[4], cycle_period_ms=_t[5])



INTERNAL_FMT = '<iB3x'
INTERNAL_SIZE = 8
INTERNAL_OFFSETS: dict[str, int] = {
    'last_setpoint_micro_kw': 0,
    'initialized': 4,
}


@dataclass
class Internal:
    last_setpoint_micro_kw: int  # i32
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.last_setpoint_micro_kw, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(last_setpoint_micro_kw=_t[0], initialized=bool(_t[1]))


