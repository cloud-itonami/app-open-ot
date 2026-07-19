"""Generated from `cells/ltc-tap-fsm/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : LTC_TAP_FSM
ECC      : ['Idle', 'Holding', 'Raising', 'Lowering', 'Limit', 'Alarm'] (initial=Idle)
ABI      : init=ltc_tap_fsm_init  tick=ltc_tap_fsm_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'ltc-tap-fsm'
CELL_SYMBOL  = 'ltc_tap_fsm'
FBTYPE       = 'LTC_TAP_FSM'
INIT_EXPORT  = 'ltc_tap_fsm_init'
TICK_EXPORT  = 'ltc_tap_fsm_tick'
ECC_STATES   = ['Idle', 'Holding', 'Raising', 'Lowering', 'Limit', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<qqhBB4x'
DATA_IN_SIZE = 24
DATA_IN_OFFSETS: dict[str, int] = {
    'voltage_meas_micro_v': 0,
    'voltage_target_micro_v': 8,
    'tap_position': 16,
    'voltage_quality': 18,
    'enable': 19,
}


@dataclass
class DataIn:
    voltage_meas_micro_v: int  # i64 (wire: valueMicroUnit)
    voltage_target_micro_v: int  # i64 (wire: valueMicroUnit)
    tap_position: int  # i16 (wire: integerSigned)
    voltage_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.voltage_meas_micro_v, d.voltage_target_micro_v, d.tap_position, d.voltage_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(voltage_meas_micro_v=_t[0], voltage_target_micro_v=_t[1], tap_position=_t[2], voltage_quality=_t[3], enable=bool(_t[4]))



DATA_OUT_FMT = '<B7xqIB3x'
DATA_OUT_SIZE = 24
DATA_OUT_OFFSETS: dict[str, int] = {
    'command': 0,
    'voltage_error_micro_v': 8,
    'dwell_remaining_ms': 16,
    'at_limit': 20,
}


@dataclass
class DataOut:
    command: int  # u8 (wire: tapCommandEnum)
    voltage_error_micro_v: int  # i64 (wire: valueMicroUnit)
    dwell_remaining_ms: int  # u32 (wire: durationMs)
    at_limit: bool  # bool (wire: boolean)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.command, d.voltage_error_micro_v, d.dwell_remaining_ms, (1 if d.at_limit else 0))


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(command=_t[0], voltage_error_micro_v=_t[1], dwell_remaining_ms=_t[2], at_limit=bool(_t[3]))



PARAMS_FMT = '<qIhhI4x'
PARAMS_SIZE = 24
PARAMS_OFFSETS: dict[str, int] = {
    'dead_band_micro_v': 0,
    'dwell_ms': 8,
    'tap_min': 12,
    'tap_max': 14,
    'cycle_period_ms': 16,
}


@dataclass
class Params:
    dead_band_micro_v: int  # i64
    dwell_ms: int  # u32
    tap_min: int  # i16
    tap_max: int  # i16
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.dead_band_micro_v, d.dwell_ms, d.tap_min, d.tap_max, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(dead_band_micro_v=_t[0], dwell_ms=_t[1], tap_min=_t[2], tap_max=_t[3], cycle_period_ms=_t[4])



INTERNAL_FMT = '<IBB2x'
INTERNAL_SIZE = 8
INTERNAL_OFFSETS: dict[str, int] = {
    'dwell_remaining_ms': 0,
    'last_command': 4,
    'initialized': 5,
}


@dataclass
class Internal:
    dwell_remaining_ms: int  # u32
    last_command: int  # u8
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.dwell_remaining_ms, d.last_command, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(dwell_remaining_ms=_t[0], last_command=_t[1], initialized=bool(_t[2]))


