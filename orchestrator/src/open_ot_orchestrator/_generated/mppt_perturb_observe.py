"""Generated from `cells/mppt-perturb-observe/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : MPPT_PERTURB_OBSERVE
ECC      : ['Idle', 'Searching', 'AtMpp', 'Alarm'] (initial=Idle)
ABI      : init=mppt_perturb_observe_init  tick=mppt_perturb_observe_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'mppt-perturb-observe'
CELL_SYMBOL  = 'mppt_perturb_observe'
FBTYPE       = 'MPPT_PERTURB_OBSERVE'
INIT_EXPORT  = 'mppt_perturb_observe_init'
TICK_EXPORT  = 'mppt_perturb_observe_tick'
ECC_STATES   = ['Idle', 'Searching', 'AtMpp', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<iiBBB1x'
DATA_IN_SIZE = 12
DATA_IN_OFFSETS: dict[str, int] = {
    'pv_voltage_micro_v': 0,
    'pv_current_micro_a': 4,
    'voltage_quality': 8,
    'current_quality': 9,
    'enable': 10,
}


@dataclass
class DataIn:
    pv_voltage_micro_v: int  # i32 (wire: valueMicroUnit)
    pv_current_micro_a: int  # i32 (wire: valueMicroUnit)
    voltage_quality: int  # u8 (wire: qualityEnum)
    current_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.pv_voltage_micro_v, d.pv_current_micro_a, d.voltage_quality, d.current_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(pv_voltage_micro_v=_t[0], pv_current_micro_a=_t[1], voltage_quality=_t[2], current_quality=_t[3], enable=bool(_t[4]))



DATA_OUT_FMT = '<i4xqBB6x'
DATA_OUT_SIZE = 24
DATA_OUT_OFFSETS: dict[str, int] = {
    'voltage_setpoint_micro_v': 0,
    'power_pw': 8,
    'direction': 16,
    'mpp_reached': 17,
}


@dataclass
class DataOut:
    voltage_setpoint_micro_v: int  # i32 (wire: valueMicroUnit)
    power_pw: int  # i64 (wire: valueMicroUnit)
    direction: int  # u8 (wire: perturbDirEnum)
    mpp_reached: bool  # bool (wire: boolean)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.voltage_setpoint_micro_v, d.power_pw, d.direction, (1 if d.mpp_reached else 0))


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(voltage_setpoint_micro_v=_t[0], power_pw=_t[1], direction=_t[2], mpp_reached=bool(_t[3]))



PARAMS_FMT = '<iii4xqI4x'
PARAMS_SIZE = 32
PARAMS_OFFSETS: dict[str, int] = {
    'perturb_step_micro_v': 0,
    'v_min_micro_v': 4,
    'v_max_micro_v': 8,
    'mpp_tolerance_pw': 16,
    'cycle_period_ms': 24,
}


@dataclass
class Params:
    perturb_step_micro_v: int  # i32
    v_min_micro_v: int  # i32
    v_max_micro_v: int  # i32
    mpp_tolerance_pw: int  # i64
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.perturb_step_micro_v, d.v_min_micro_v, d.v_max_micro_v, d.mpp_tolerance_pw, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(perturb_step_micro_v=_t[0], v_min_micro_v=_t[1], v_max_micro_v=_t[2], mpp_tolerance_pw=_t[3], cycle_period_ms=_t[4])



INTERNAL_FMT = '<i4xqBB6x'
INTERNAL_SIZE = 24
INTERNAL_OFFSETS: dict[str, int] = {
    'last_voltage_setpoint_micro_v': 0,
    'last_power_pw': 8,
    'direction': 16,
    'initialized': 17,
}


@dataclass
class Internal:
    last_voltage_setpoint_micro_v: int  # i32
    last_power_pw: int  # i64
    direction: int  # u8
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.last_voltage_setpoint_micro_v, d.last_power_pw, d.direction, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(last_voltage_setpoint_micro_v=_t[0], last_power_pw=_t[1], direction=_t[2], initialized=bool(_t[3]))


