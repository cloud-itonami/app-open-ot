"""Generated from `cells/pid-stack-100/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : PID_STACK_100
ECC      : ['Idle', 'Healthy', 'Degraded', 'AllAlarm'] (initial=Idle)
ABI      : init=pid_stack_100_init  tick=pid_stack_100_tick
Tick caps: max_emitted=1  max_neighbor_msgs=0
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = 'pid-stack-100'
CELL_SYMBOL  = 'pid_stack_100'
FBTYPE       = 'PID_STACK_100'
INIT_EXPORT  = 'pid_stack_100_init'
TICK_EXPORT  = 'pid_stack_100_tick'
ECC_STATES   = ['Idle', 'Healthy', 'Degraded', 'AllAlarm']
ECC_INITIAL  = 'Idle'
TICK_MAX_EMITTED        = 1
TICK_MAX_NEIGHBOR_MSGS  = 0


# data_in: empty — no struct emitted.


# data_out: empty — no struct emitted.


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



# internal: empty — no struct emitted.

