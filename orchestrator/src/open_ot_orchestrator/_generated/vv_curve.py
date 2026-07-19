"""Generated from `cells/vv-curve/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : VV_CURVE
ECC      : ['Idle', 'InDeadBand', 'Absorbing', 'Injecting', 'Saturated', 'Alarm'] (initial=Idle)
ABI      : init=vv_curve_init  tick=vv_curve_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'vv-curve'
CELL_SYMBOL  = 'vv_curve'
FBTYPE       = 'VV_CURVE'
INIT_EXPORT  = 'vv_curve_init'
TICK_EXPORT  = 'vv_curve_tick'
ECC_STATES   = ['Idle', 'InDeadBand', 'Absorbing', 'Injecting', 'Saturated', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<iiBB2x'
DATA_IN_SIZE = 12
DATA_IN_OFFSETS: dict[str, int] = {
    'voltage_micro_pu': 0,
    'q_max_micro_var': 4,
    'voltage_quality': 8,
    'enable': 9,
}


@dataclass
class DataIn:
    voltage_micro_pu: int  # i32 (wire: valueMicroUnit)
    q_max_micro_var: int  # i32 (wire: valueMicroUnit)
    voltage_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.voltage_micro_pu, d.q_max_micro_var, d.voltage_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(voltage_micro_pu=_t[0], q_max_micro_var=_t[1], voltage_quality=_t[2], enable=bool(_t[3]))



DATA_OUT_FMT = '<iiBB2x'
DATA_OUT_SIZE = 12
DATA_OUT_OFFSETS: dict[str, int] = {
    'q_setpoint_micro_var': 0,
    'voltage_deviation_micro_pu': 4,
    'in_dead_band': 8,
    'saturated': 9,
}


@dataclass
class DataOut:
    q_setpoint_micro_var: int  # i32 (wire: valueMicroUnit)
    voltage_deviation_micro_pu: int  # i32 (wire: valueMicroUnit)
    in_dead_band: bool  # bool (wire: boolean)
    saturated: bool  # bool (wire: boolean)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.q_setpoint_micro_var, d.voltage_deviation_micro_pu, (1 if d.in_dead_band else 0), (1 if d.saturated else 0))


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(q_setpoint_micro_var=_t[0], voltage_deviation_micro_pu=_t[1], in_dead_band=bool(_t[2]), saturated=bool(_t[3]))



PARAMS_FMT = '<iiiiI'
PARAMS_SIZE = 20
PARAMS_OFFSETS: dict[str, int] = {
    'v_dead_high_micro_pu': 0,
    'v_full_high_micro_pu': 4,
    'v_dead_low_micro_pu': 8,
    'v_full_low_micro_pu': 12,
    'cycle_period_ms': 16,
}


@dataclass
class Params:
    v_dead_high_micro_pu: int  # i32
    v_full_high_micro_pu: int  # i32
    v_dead_low_micro_pu: int  # i32
    v_full_low_micro_pu: int  # i32
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.v_dead_high_micro_pu, d.v_full_high_micro_pu, d.v_dead_low_micro_pu, d.v_full_low_micro_pu, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(v_dead_high_micro_pu=_t[0], v_full_high_micro_pu=_t[1], v_dead_low_micro_pu=_t[2], v_full_low_micro_pu=_t[3], cycle_period_ms=_t[4])



INTERNAL_FMT = '<iB3x'
INTERNAL_SIZE = 8
INTERNAL_OFFSETS: dict[str, int] = {
    'last_setpoint_micro_var': 0,
    'initialized': 4,
}


@dataclass
class Internal:
    last_setpoint_micro_var: int  # i32
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.last_setpoint_micro_var, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(last_setpoint_micro_var=_t[0], initialized=bool(_t[1]))


