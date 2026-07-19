"""Microgrid `:loop:volt-var` as a LangGraph `StateGraph`.

Per PROTOTYPE-MICROGRID.md §13.2. Multi-cell topology combining:

  - N × `VV_CURVE` cells (one per inverter): local Volt-VAR Q setpoint
    from each inverter's terminal voltage. 10 Hz field-tier.
  - 1 × `LTC_TAP_FSM` cell: supervisory tap controller acting on the
    cohort-averaged bus voltage. Event-driven (issues tap commands subject
    to dwell timer + tap_min/tap_max).
  - 1 aggregator node: averages inverter voltages → drives LTC input.

The graph layout is:

```text
   START ─┬─→ vv_curve_inv1 ─┐
          ├─→ vv_curve_inv2 ─┤
          │      ...         ├─→ aggregator ─→ ltc_tap_fsm ─→ END
          └─→ vv_curve_invN ─┘
```

This is the **first multi-cell-type** orchestrator loop. The pattern
generalises to `:loop:bess-charge-discharge` (SOC_KALMAN → DROOP_P_F) and
the islanding decision loop (ANTI_ISLANDING_ROCOF + BLACK_START_SEQ).

Run: `uv run python -m open_ot_orchestrator.microgrid_volt_var_langgraph`
"""

from __future__ import annotations

from operator import or_
from pathlib import Path
from typing import Annotated, Any, TypedDict

from langgraph.checkpoint.memory import MemorySaver
from langgraph.graph import END, START, StateGraph

from ._generated import ltc_tap_fsm as ltc_gen
from ._generated import vv_curve as vv_gen
from .cell_loader import CellLoader

REPO_ROOT = Path(__file__).resolve().parents[5]
CELLS_TARGET = (
    REPO_ROOT
    / "60-apps/etzhayyim-project-open-ot/cells/target/wasm32-unknown-unknown/release"
)
VV_WASM_PATH = CELLS_TARGET / "vv_curve.wasm"
LTC_WASM_PATH = CELLS_TARGET / "ltc_tap_fsm.wasm"


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------


class VoltVarState(TypedDict, total=False):
    """Per-super-step state."""

    # Per-inverter measured voltage in per-unit (e.g. 1.025 = 2.5 % over nominal).
    inverter_voltage_pu: dict[str, float]
    # Per-inverter rated reactive power in VAR (positive).
    inverter_q_max_var: dict[str, float]
    # Measured bus voltage in V (e.g. 11_000.0 for an 11 kV bus).
    bus_voltage_v: float
    # Target bus voltage in V.
    bus_voltage_target_v: float
    # Current LTC tap position (-tap_max..+tap_max).
    ltc_tap_position: int
    # Outputs.
    inverter_q_setpoint_var: Annotated[dict[str, float], or_]
    inverter_states: Annotated[dict[str, str], or_]
    bus_voltage_avg_v: float
    ltc_command: str  # "Hold" / "Raise" / "Lower"
    ltc_state: str
    # Per-cell internal bytes (resumable checkpoint).
    cell_internals: Annotated[dict[str, bytes], or_]
    cell_ecc_states: Annotated[dict[str, int], or_]


# ---------------------------------------------------------------------------
# VV_CURVE inverter nodes
# ---------------------------------------------------------------------------


_VV_ECC = {i: name for i, name in enumerate(vv_gen.ECC_STATES)}


def _make_vv_node(asset_did: str, cycle_period_ms: int):
    """One VV_CURVE cell wrapped as a LangGraph node."""
    loader = CellLoader(VV_WASM_PATH, vv_gen.CELL_SYMBOL)
    # IEEE 1547 default curve breakpoints.
    params = vv_gen.Params(
        v_dead_high_micro_pu=1_030_000,
        v_full_high_micro_pu=1_060_000,
        v_dead_low_micro_pu=970_000,
        v_full_low_micro_pu=900_000,
        cycle_period_ms=cycle_period_ms,
    )
    params_bytes = vv_gen.pack_params(params)
    initialized = {"done": False}

    def node(state: VoltVarState) -> dict[str, Any]:
        prior_internals = state.get("cell_internals") or {}
        if asset_did in prior_internals:
            if not initialized["done"]:
                loader.init(params_bytes, vv_gen.INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior_internals[asset_did])
        else:
            loader.init(params_bytes, vv_gen.INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(asset_did, 0)
        v_pu = state.get("inverter_voltage_pu", {}).get(asset_did, 1.0)
        q_max = state.get("inverter_q_max_var", {}).get(asset_did, 0.0)

        # The cell's i32 q_max scale is mVAR (= VAR × 1000). 100 kVAR maps to
        # 1e8 unit values — well inside i32 range.
        data_in = vv_gen.DataIn(
            voltage_micro_pu=int(round(v_pu * 1_000_000)),
            q_max_micro_var=int(round(q_max * 1_000)),
            voltage_quality=0,  # Good
            enable=True,
        )
        result = loader.tick(
            event_in=0,
            data_in_bytes=vv_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=vv_gen.DATA_OUT_SIZE,
        )
        out = vv_gen.unpack_data_out(result.data_out_bytes)
        # Internal mVAR → VAR.
        q_setpoint_var = out.q_setpoint_micro_var / 1_000.0
        return {
            "inverter_q_setpoint_var": {asset_did: q_setpoint_var},
            "inverter_states": {asset_did: _VV_ECC.get(result.next_ecc_state, "?")},
            "cell_internals": {
                asset_did: loader.get_internal_bytes(vv_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {asset_did: result.next_ecc_state},
        }

    return node


# ---------------------------------------------------------------------------
# Bus aggregator
# ---------------------------------------------------------------------------


def _bus_aggregator(state: VoltVarState) -> dict[str, Any]:
    """Average per-inverter voltages → cohort bus voltage estimate."""
    voltages = state.get("inverter_voltage_pu") or {}
    if not voltages:
        return {"bus_voltage_avg_v": state.get("bus_voltage_v", 0.0)}
    nominal = state.get("bus_voltage_target_v", 11_000.0)
    avg_pu = sum(voltages.values()) / len(voltages)
    return {"bus_voltage_avg_v": avg_pu * nominal}


# ---------------------------------------------------------------------------
# LTC_TAP_FSM supervisory node
# ---------------------------------------------------------------------------


_LTC_ECC = {i: name for i, name in enumerate(ltc_gen.ECC_STATES)}
_LTC_CMD_NAMES = {0: "Hold", 1: "Raise", 2: "Lower"}


def _make_ltc_node(ltc_did: str, dwell_ms: int, cycle_period_ms: int):
    """One LTC_TAP_FSM cell wrapped as a LangGraph node."""
    loader = CellLoader(LTC_WASM_PATH, ltc_gen.CELL_SYMBOL)
    params = ltc_gen.Params(
        dead_band_micro_v=150_000_000,  # ±150 V on the µV scale (for the demo)
        dwell_ms=dwell_ms,
        tap_min=-8,
        tap_max=8,
        cycle_period_ms=cycle_period_ms,
    )
    params_bytes = ltc_gen.pack_params(params)
    initialized = {"done": False}

    def node(state: VoltVarState) -> dict[str, Any]:
        prior_internals = state.get("cell_internals") or {}
        if ltc_did in prior_internals:
            if not initialized["done"]:
                loader.init(params_bytes, ltc_gen.INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior_internals[ltc_did])
        else:
            loader.init(params_bytes, ltc_gen.INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(ltc_did, 0)
        bus_v = state.get("bus_voltage_avg_v", 0.0)
        target_v = state.get("bus_voltage_target_v", 11_000.0)
        tap = state.get("ltc_tap_position", 0)

        data_in = ltc_gen.DataIn(
            voltage_meas_micro_v=int(round(bus_v * 1_000_000)),
            voltage_target_micro_v=int(round(target_v * 1_000_000)),
            tap_position=tap,
            voltage_quality=0,  # Good
            enable=True,
        )
        result = loader.tick(
            event_in=0,
            data_in_bytes=ltc_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=ltc_gen.DATA_OUT_SIZE,
        )
        out = ltc_gen.unpack_data_out(result.data_out_bytes)
        return {
            "ltc_command": _LTC_CMD_NAMES.get(out.command, "?"),
            "ltc_state": _LTC_ECC.get(result.next_ecc_state, "?"),
            "cell_internals": {
                ltc_did: loader.get_internal_bytes(ltc_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {ltc_did: result.next_ecc_state},
        }

    return node


# ---------------------------------------------------------------------------
# Graph builder
# ---------------------------------------------------------------------------


def _safe_node_id(did: str) -> str:
    return did.replace(":", "_").replace("/", "_").replace(".", "_")


def build_volt_var_graph(
    inverter_assets: list[str],
    ltc_did: str = "did:web:open-ot.etzhayyim.com:cell:ltc-substation-1",
    cycle_period_ms: int = 100,
    dwell_ms: int = 30_000,
):
    """Build the `:loop:volt-var` graph.

    `inverter_assets` is a list of DIDs (one VV_CURVE cell per asset).
    The supervisory LTC runs after the aggregator merges per-inverter
    voltages into a single bus-voltage estimate.
    """
    graph = StateGraph(VoltVarState)
    for did in inverter_assets:
        graph.add_node(_safe_node_id(did), _make_vv_node(did, cycle_period_ms))
    graph.add_node("bus_aggregator", _bus_aggregator)
    graph.add_node("ltc", _make_ltc_node(ltc_did, dwell_ms, cycle_period_ms))
    for did in inverter_assets:
        graph.add_edge(START, _safe_node_id(did))
        graph.add_edge(_safe_node_id(did), "bus_aggregator")
    graph.add_edge("bus_aggregator", "ltc")
    graph.add_edge("ltc", END)
    return graph.compile(checkpointer=MemorySaver())


# ---------------------------------------------------------------------------
# Demo
# ---------------------------------------------------------------------------


def step(
    app,
    config: dict[str, Any],
    inverter_voltage_pu: dict[str, float],
    inverter_q_max_var: dict[str, float],
    bus_voltage_target_v: float = 11_000.0,
    ltc_tap_position: int = 0,
) -> VoltVarState:
    return app.invoke(
        {
            "inverter_voltage_pu": inverter_voltage_pu,
            "inverter_q_max_var": inverter_q_max_var,
            "bus_voltage_target_v": bus_voltage_target_v,
            "ltc_tap_position": ltc_tap_position,
        },
        config=config,
    )


def demo() -> None:
    inverter_dids = [
        "did:web:open-ot.etzhayyim.com:cell:inv-1",
        "did:web:open-ot.etzhayyim.com:cell:inv-2",
        "did:web:open-ot.etzhayyim.com:cell:inv-3",
    ]
    q_max = {did: 100_000.0 for did in inverter_dids}  # 100 kVAR each
    app = build_volt_var_graph(inverter_dids, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "volt-var-demo"}}

    # Sweep voltage from 0.95 pu → 1.05 pu across 7 super-steps.
    schedule = [0.95, 0.98, 1.00, 1.02, 1.04, 1.05, 1.03]
    for i, v in enumerate(schedule, start=1):
        voltages = {did: v for did in inverter_dids}
        state = step(app, config, voltages, q_max)
        q_sum = sum(state.get("inverter_q_setpoint_var", {}).values())
        print(
            f"[step {i:>2}] V_pu={v:.3f}  "
            f"Σ Q_inv={q_sum:+10.1f} VAR  "
            f"LTC={state.get('ltc_command'):>5} ({state.get('ltc_state')})"
        )


if __name__ == "__main__":
    demo()
