"""Generated from `cells/anti-islanding-rocof/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : ANTI_ISLANDING_ROCOF
ECC      : ['Idle', 'Monitoring', 'Warning', 'Tripped', 'Alarm'] (initial=Idle)
ABI      : init=anti_islanding_rocof_init  tick=anti_islanding_rocof_tick
Tick caps: max_emitted=2  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'anti-islanding-rocof'
CELL_SYMBOL  = 'anti_islanding_rocof'
FBTYPE       = 'ANTI_ISLANDING_ROCOF'
INIT_EXPORT  = 'anti_islanding_rocof_init'
TICK_EXPORT  = 'anti_islanding_rocof_tick'
ECC_STATES   = ['Idle', 'Monitoring', 'Warning', 'Tripped', 'Alarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 2
TICK_MAX_NEIGHBOR_MSGS  = 0


DATA_IN_FMT = '<qqqqBBB5x'
DATA_IN_SIZE = 40
DATA_IN_OFFSETS: dict[str, int] = {
    'grid_freq': 0,
    'freq_nominal': 8,
    'grid_voltage': 16,
    'voltage_nominal': 24,
    'freq_quality': 32,
    'voltage_quality': 33,
    'enable': 34,
}


@dataclass
class DataIn:
    grid_freq: int  # i64 (wire: valueMicroUnit)
    freq_nominal: int  # i64 (wire: valueMicroUnit)
    grid_voltage: int  # i64 (wire: valueMicroUnit)
    voltage_nominal: int  # i64 (wire: valueMicroUnit)
    freq_quality: int  # u8 (wire: qualityEnum)
    voltage_quality: int  # u8 (wire: qualityEnum)
    enable: bool  # bool (wire: boolean)


def pack_data_in(d: DataIn) -> bytes:
    return struct.pack(DATA_IN_FMT, d.grid_freq, d.freq_nominal, d.grid_voltage, d.voltage_nominal, d.freq_quality, d.voltage_quality, (1 if d.enable else 0))


def unpack_data_in(buf: bytes) -> DataIn:
    _t = struct.unpack(DATA_IN_FMT, buf)
    return DataIn(grid_freq=_t[0], freq_nominal=_t[1], grid_voltage=_t[2], voltage_nominal=_t[3], freq_quality=_t[4], voltage_quality=_t[5], enable=bool(_t[6]))



DATA_OUT_FMT = '<BB6xqi4xqBBB5x'
DATA_OUT_SIZE = 40
DATA_OUT_OFFSETS: dict[str, int] = {
    'trip': 0,
    'trip_reason': 1,
    'rocof_micro_hz_per_s': 8,
    'voltage_deviation_milli_pct': 16,
    'freq_deviation_micro_hz': 24,
    'rocof_violation_count': 32,
    'voltage_violation_count': 33,
    'freq_violation_count': 34,
}


@dataclass
class DataOut:
    trip: bool  # bool (wire: boolean)
    trip_reason: int  # u8 (wire: tripReasonEnum)
    rocof_micro_hz_per_s: int  # i64 (wire: valueMicroUnit)
    voltage_deviation_milli_pct: int  # i32 (wire: valueMilliPct)
    freq_deviation_micro_hz: int  # i64 (wire: valueMicroUnit)
    rocof_violation_count: int  # u8 (wire: counter)
    voltage_violation_count: int  # u8 (wire: counter)
    freq_violation_count: int  # u8 (wire: counter)


def pack_data_out(d: DataOut) -> bytes:
    return struct.pack(DATA_OUT_FMT, (1 if d.trip else 0), d.trip_reason, d.rocof_micro_hz_per_s, d.voltage_deviation_milli_pct, d.freq_deviation_micro_hz, d.rocof_violation_count, d.voltage_violation_count, d.freq_violation_count)


def unpack_data_out(buf: bytes) -> DataOut:
    _t = struct.unpack(DATA_OUT_FMT, buf)
    return DataOut(trip=bool(_t[0]), trip_reason=_t[1], rocof_micro_hz_per_s=_t[2], voltage_deviation_milli_pct=_t[3], freq_deviation_micro_hz=_t[4], rocof_violation_count=_t[5], voltage_violation_count=_t[6], freq_violation_count=_t[7])



PARAMS_FMT = '<qI4xqqI4xqqII'
PARAMS_SIZE = 64
PARAMS_OFFSETS: dict[str, int] = {
    'rocof_threshold_micro_hz_per_s': 0,
    'rocof_window_samples': 8,
    'voltage_min_micro_v': 16,
    'voltage_max_micro_v': 24,
    'voltage_window_samples': 32,
    'freq_min_micro_hz': 40,
    'freq_max_micro_hz': 48,
    'freq_window_samples': 56,
    'cycle_period_ms': 60,
}


@dataclass
class Params:
    rocof_threshold_micro_hz_per_s: int  # i64
    rocof_window_samples: int  # u32
    voltage_min_micro_v: int  # i64
    voltage_max_micro_v: int  # i64
    voltage_window_samples: int  # u32
    freq_min_micro_hz: int  # i64
    freq_max_micro_hz: int  # i64
    freq_window_samples: int  # u32
    cycle_period_ms: int  # u32


def pack_params(d: Params) -> bytes:
    return struct.pack(PARAMS_FMT, d.rocof_threshold_micro_hz_per_s, d.rocof_window_samples, d.voltage_min_micro_v, d.voltage_max_micro_v, d.voltage_window_samples, d.freq_min_micro_hz, d.freq_max_micro_hz, d.freq_window_samples, d.cycle_period_ms)


def unpack_params(buf: bytes) -> Params:
    _t = struct.unpack(PARAMS_FMT, buf)
    return Params(rocof_threshold_micro_hz_per_s=_t[0], rocof_window_samples=_t[1], voltage_min_micro_v=_t[2], voltage_max_micro_v=_t[3], voltage_window_samples=_t[4], freq_min_micro_hz=_t[5], freq_max_micro_hz=_t[6], freq_window_samples=_t[7], cycle_period_ms=_t[8])



INTERNAL_FMT = '<qIIIBB2x'
INTERNAL_SIZE = 24
INTERNAL_OFFSETS: dict[str, int] = {
    'last_freq_micro_hz': 0,
    'rocof_violation_count': 8,
    'voltage_violation_count': 12,
    'freq_violation_count': 16,
    'last_trip_reason': 20,
    'initialized': 21,
}


@dataclass
class Internal:
    last_freq_micro_hz: int  # i64
    rocof_violation_count: int  # u32
    voltage_violation_count: int  # u32
    freq_violation_count: int  # u32
    last_trip_reason: int  # u8
    initialized: bool  # bool


def pack_internal(d: Internal) -> bytes:
    return struct.pack(INTERNAL_FMT, d.last_freq_micro_hz, d.rocof_violation_count, d.voltage_violation_count, d.freq_violation_count, d.last_trip_reason, (1 if d.initialized else 0))


def unpack_internal(buf: bytes) -> Internal:
    _t = struct.unpack(INTERNAL_FMT, buf)
    return Internal(last_freq_micro_hz=_t[0], rocof_violation_count=_t[1], voltage_violation_count=_t[2], freq_violation_count=_t[3], last_trip_reason=_t[4], initialized=bool(_t[5]))


