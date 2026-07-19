"""Generated from `cells/pid-limited/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : PID_LIMITED
ECC      : ['Idle', 'Running', 'Saturated', 'Alarm'] (initial=Idle)
ABI      : init=pid_limited_init  tick=pid_limited_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'pid-limited'
CELL_SYMBOL  = 'pid_limited'
FBTYPE       = 'PID_LIMITED'
INIT_EXPORT  = 'pid_limited_init'
TICK_EXPORT  = 'pid_limited_tick'
ECC_STATES   = ['Idle', 'Running', 'Saturated', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<iiBB2x'
DATA_IN_SIZE = 12
DATA_IN_OFFSETS: dict[str, int] = {
    'pv': 0,
    'sp': 4,
    'pv_quality': 8,
    'enable': 9,
}


@dataclass
class DataIn:
    pv: int  # i32 (wire: valueMicroUnit)
    sp: int  # i32 (wire: valueMicroUnit)
    pv_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.pv, d.sp, d.pv_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(pv=_t[0], sp=_t[1], pv_quality=_t[2], enable=bool(_t[3]))



DATA_OUT_FMT = '<iiB3x'
DATA_OUT_SIZE = 12
DATA_OUT_OFFSETS: dict[str, int] = {
    'cv': 0,
    'error': 4,
    'saturated': 8,
}


@dataclass
class DataOut:
    cv: int  # i32 (wire: valueMicroUnit)
    error: int  # i32 (wire: valueMicroUnit)
    saturated: bool  # bool (wire: boolean)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.cv, d.error, (1 if d.saturated else 0))


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(cv=_t[0], error=_t[1], saturated=bool(_t[2]))



PARAMS_FMT = '<iiiiI'
PARAMS_SIZE = 20
PARAMS_OFFSETS: dict[str, int] = {
    'kp_micro': 0,
    'ki_micro': 4,
    'out_min_micro': 8,
    'out_max_micro': 12,
    'cycle_period_ms': 16,
}


@dataclass
class Params:
    kp_micro: int  # i32
    ki_micro: int  # i32
    out_min_micro: int  # i32
    out_max_micro: int  # i32
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.kp_micro, d.ki_micro, d.out_min_micro, d.out_max_micro, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(kp_micro=_t[0], ki_micro=_t[1], out_min_micro=_t[2], out_max_micro=_t[3], cycle_period_ms=_t[4])



INTERNAL_FMT = '<qiB3x'
INTERNAL_SIZE = 16
INTERNAL_OFFSETS: dict[str, int] = {
    'integral_micro': 0,
    'last_pv_micro': 8,
    'initialized': 12,
}


@dataclass
class Internal:
    integral_micro: int  # i64
    last_pv_micro: int  # i32
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.integral_micro, d.last_pv_micro, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(integral_micro=_t[0], last_pv_micro=_t[1], initialized=bool(_t[2]))


