"""Generated from `cells/soc-kalman/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : SOC_KALMAN
ECC      : ['Idle', 'Tracking', 'LowConfidence', 'Saturated', 'Alarm'] (initial=Idle)
ABI      : init=soc_kalman_init  tick=soc_kalman_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'soc-kalman'
CELL_SYMBOL  = 'soc_kalman'
FBTYPE       = 'SOC_KALMAN'
INIT_EXPORT  = 'soc_kalman_init'
TICK_EXPORT  = 'soc_kalman_tick'
ECC_STATES   = ['Idle', 'Tracking', 'LowConfidence', 'Saturated', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<qqiBBB1x'
DATA_IN_SIZE = 24
DATA_IN_OFFSETS: dict[str, int] = {
    'voltage_micro_v': 0,
    'current_micro_a': 8,
    'temp_milli_c': 16,
    'voltage_quality': 20,
    'current_quality': 21,
    'enable': 22,
}


@dataclass
class DataIn:
    voltage_micro_v: int  # i64 (wire: valueMicroUnit)
    current_micro_a: int  # i64 (wire: valueMicroUnit)
    temp_milli_c: int  # i32 (wire: valueMilliUnit)
    voltage_quality: int  # u8 (wire: qualityEnum)
    current_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.voltage_micro_v, d.current_micro_a, d.temp_milli_c, d.voltage_quality, d.current_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(voltage_micro_v=_t[0], current_micro_a=_t[1], temp_milli_c=_t[2], voltage_quality=_t[3], current_quality=_t[4], enable=bool(_t[5]))



DATA_OUT_FMT = '<i4xqqH6x'
DATA_OUT_SIZE = 32
DATA_OUT_OFFSETS: dict[str, int] = {
    'soc_milli_pct': 0,
    'ocv_estimated_micro_v': 8,
    'coulomb_delta_micro_c': 16,
    'confidence_milli': 24,
}


@dataclass
class DataOut:
    soc_milli_pct: int  # i32 (wire: valueMilliUnit)
    ocv_estimated_micro_v: int  # i64 (wire: valueMicroUnit)
    coulomb_delta_micro_c: int  # i64 (wire: valueMicroUnit)
    confidence_milli: int  # u16 (wire: valueMilliUnit)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, d.soc_milli_pct, d.ocv_estimated_micro_v, d.coulomb_delta_micro_c, d.confidence_milli)


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(soc_milli_pct=_t[0], ocv_estimated_micro_v=_t[1], coulomb_delta_micro_c=_t[2], confidence_milli=_t[3])



PARAMS_FMT = '<qqqqH2xI'
PARAMS_SIZE = 40
PARAMS_OFFSETS: dict[str, int] = {
    'capacity_micro_c': 0,
    'internal_resistance_micro_ohm': 8,
    'ocv_at_0_pct_micro_v': 16,
    'ocv_at_100_pct_micro_v': 24,
    'correction_gain_milli': 32,
    'cycle_period_ms': 36,
}


@dataclass
class Params:
    capacity_micro_c: int  # i64
    internal_resistance_micro_ohm: int  # i64
    ocv_at_0_pct_micro_v: int  # i64
    ocv_at_100_pct_micro_v: int  # i64
    correction_gain_milli: int  # u16
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.capacity_micro_c, d.internal_resistance_micro_ohm, d.ocv_at_0_pct_micro_v, d.ocv_at_100_pct_micro_v, d.correction_gain_milli, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(capacity_micro_c=_t[0], internal_resistance_micro_ohm=_t[1], ocv_at_0_pct_micro_v=_t[2], ocv_at_100_pct_micro_v=_t[3], correction_gain_milli=_t[4], cycle_period_ms=_t[5])



INTERNAL_FMT = '<i4xqHB5x'
INTERNAL_SIZE = 24
INTERNAL_OFFSETS: dict[str, int] = {
    'soc_milli_pct': 0,
    'coulomb_accumulator_micro_c': 8,
    'confidence_milli': 16,
    'initialized': 18,
}


@dataclass
class Internal:
    soc_milli_pct: int  # i32
    coulomb_accumulator_micro_c: int  # i64
    confidence_milli: int  # u16
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.soc_milli_pct, d.coulomb_accumulator_micro_c, d.confidence_milli, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(soc_milli_pct=_t[0], coulomb_accumulator_micro_c=_t[1], confidence_milli=_t[2], initialized=bool(_t[3]))


