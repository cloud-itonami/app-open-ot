"""Microgrid `:loop:bess-charge-discharge` — SOC_KALMAN → DROOP_P_F chain.

Per PROTOTYPE-MICROGRID.md §13.2. Each BESS asset runs a 2-stage cell
chain in the orchestrator:

  1. **SOC_KALMAN** estimates State-of-Charge from voltage/current
     measurements (Coulomb counter + OCV correction).
  2. **DROOP_P_F** issues a power setpoint based on grid frequency, with
     **SOC-aware clamps** — the orchestrator narrows `p_min` / `p_max`
     based on the SOC estimate to prevent over-discharge / over-charge.

Graph layout per BESS asset:

```text
   START → soc_kalman_bess_n ─→ soc_router (clamps droop limits)
                               └─→ droop_p_f_bess_n ─→ aggregator → END
```

Multi-asset version composes the per-asset chains in parallel.

Run: `uv run python -m open_ot_orchestrator.microgrid_bess_langgraph`
"""

from __future__ import annotations

from operator import or_
from pathlib import Path
from typing import Annotated, Any, TypedDict

from langgraph.checkpoint.memory import MemorySaver
from langgraph.graph import END, START, StateGraph

from ._generated import droop_p_f as droop_gen
from ._generated import soc_kalman as soc_gen
from .cell_loader import CellLoader

REPO_ROOT = Path(__file__).resolve().parents[5]
CELLS_TARGET = (
    REPO_ROOT
    / "60-apps/etzhayyim-project-open-ot/cells/target/wasm32-unknown-unknown/release"
)
SOC_WASM_PATH = CELLS_TARGET / "soc_kalman.wasm"
DROOP_WASM_PATH = CELLS_TARGET / "droop_p_f.wasm"


# ---------------------------------------------------------------------------
# State
# ---------------------------------------------------------------------------


class BessState(TypedDict, total=False):
    """Per-super-step state for a multi-asset BESS loop."""

    grid_freq_hz: float
    freq_nominal_hz: float
    # Per-asset measurements (input).
    asset_voltage_v: dict[str, float]
    asset_current_a: dict[str, float]   # +ve = discharging
    asset_temp_c: dict[str, float]
    asset_capacity_ah: dict[str, float]
    # Per-asset outputs.
    asset_soc_pct: Annotated[dict[str, float], or_]
    asset_soc_state: Annotated[dict[str, str], or_]
    asset_p_setpoint_kw: Annotated[dict[str, float], or_]
    asset_droop_state: Annotated[dict[str, str], or_]
    # Per-cell checkpoint.
    cell_internals: Annotated[dict[str, bytes], or_]
    cell_ecc_states: Annotated[dict[str, int], or_]
    # Aggregate.
    cohort_total_delta_kw: float


_SOC_ECC = {i: n for i, n in enumerate(soc_gen.ECC_STATES)}
_DROOP_ECC = {i: n for i, n in enumerate(droop_gen.ECC_STATES)}


def _soc_node(asset_did: str, capacity_ah: float, cycle_period_ms: int):
    """SOC_KALMAN per-asset node."""
    loader = CellLoader(SOC_WASM_PATH, soc_gen.CELL_SYMBOL)
    # LFP 16-cell pack defaults — overridable per asset in a follow-up.
    params = soc_gen.Params(
        capacity_micro_c=int(capacity_ah * 3600 * 1_000_000),
        internal_resistance_micro_ohm=1_000,
        ocv_at_0_pct_micro_v=40_000_000,
        ocv_at_100_pct_micro_v=57_600_000,
        correction_gain_milli=100,  # 10 % blend
        cycle_period_ms=cycle_period_ms,
    )
    params_bytes = soc_gen.pack_params(params)
    initialized = {"done": False}
    soc_key = f"{asset_did}#soc"

    def node(state: BessState) -> dict[str, Any]:
        prior = state.get("cell_internals") or {}
        if soc_key in prior:
            if not initialized["done"]:
                loader.init(params_bytes, soc_gen.INTERNAL_SIZE)
                initialized["done"] = True
            loader.set_internal_bytes(prior[soc_key])
        else:
            loader.init(params_bytes, soc_gen.INTERNAL_SIZE)
            initialized["done"] = True

        ecc_in = (state.get("cell_ecc_states") or {}).get(soc_key, 0)
        v = state.get("asset_voltage_v", {}).get(asset_did, 0.0)
        i = state.get("asset_current_a", {}).get(asset_did, 0.0)
        t = state.get("asset_temp_c", {}).get(asset_did, 25.0)

        data_in = soc_gen.DataIn(
            voltage_micro_v=int(round(v * 1_000_000)),
            current_micro_a=int(round(i * 1_000_000)),
            temp_milli_c=int(round(t * 1_000)),
            voltage_quality=0,
            current_quality=0,
            enable=True,
        )
        result = loader.tick(
            event_in=0,
            data_in_bytes=soc_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=soc_gen.DATA_OUT_SIZE,
        )
        out = soc_gen.unpack_data_out(result.data_out_bytes)
        soc_pct = out.soc_milli_pct / 1_000.0
        return {
            "asset_soc_pct": {asset_did: soc_pct},
            "asset_soc_state": {
                asset_did: _SOC_ECC.get(result.next_ecc_state, "?")
            },
            "cell_internals": {
                soc_key: loader.get_internal_bytes(soc_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {soc_key: result.next_ecc_state},
        }

    return node


def _droop_node(
    asset_did: str,
    p_rated_kw: float,
    cycle_period_ms: int,
    soc_low_pct: float = 10.0,
    soc_high_pct: float = 90.0,
):
    """DROOP_P_F per-asset node with SOC-aware clamps.

    When SOC < `soc_low_pct`, the orchestrator narrows `p_max_micro_kw`
    to 0 (no further discharge). When SOC > `soc_high_pct`, narrows
    `p_min_micro_kw` to 0 (no further charge). This is the "SOC-aware"
    extension over the bare droop loop — runs in the orchestrator (Python)
    rather than as cell logic because it requires SOC input from a peer
    cell, which is a Pregel super-step boundary per ADR §4.1.
    """
    loader = CellLoader(DROOP_WASM_PATH, droop_gen.CELL_SYMBOL)
    initialized = {"done": False}
    droop_key = f"{asset_did}#droop"

    def node(state: BessState) -> dict[str, Any]:
        soc_pct = state.get("asset_soc_pct", {}).get(asset_did, 50.0)

        # SOC-aware clamp: narrow active-power envelope when at SOC extremes.
        p_max_kw = p_rated_kw if soc_pct > soc_low_pct else 0.0
        p_min_kw = -p_rated_kw if soc_pct < soc_high_pct else 0.0

        params = droop_gen.Params(
            p_rated_micro_kw=int(round(p_rated_kw * 1_000_000)),
            p_min_micro_kw=int(round(p_min_kw * 1_000_000)),
            p_max_micro_kw=int(round(p_max_kw * 1_000_000)),
            droop_permille=50,  # 5 %
            dead_band_micro_hz=200_000,  # 0.2 Hz
            cycle_period_ms=cycle_period_ms,
        )
        params_bytes = droop_gen.pack_params(params)
        # Re-init each tick because params change with SOC. The cell's
        # init() resets the integrator-equivalent state, which is fine for
        # DROOP_P_F (pure proportional; no accumulator). For cells with
        # accumulator state, the orchestrator must preserve internal bytes
        # across the param-change boundary.
        prior = state.get("cell_internals") or {}
        loader.init(params_bytes, droop_gen.INTERNAL_SIZE)
        initialized["done"] = True
        if droop_key in prior:
            loader.set_internal_bytes(prior[droop_key])

        ecc_in = (state.get("cell_ecc_states") or {}).get(droop_key, 0)
        v_meas = state.get("asset_voltage_v", {}).get(asset_did, 0.0)
        # Re-derive current power: P = V × I (in kW). Discharging = +.
        i = state.get("asset_current_a", {}).get(asset_did, 0.0)
        current_p_kw = v_meas * i / 1000.0
        grid_f = state.get("grid_freq_hz", 50.0)
        f_nom = state.get("freq_nominal_hz", 50.0)

        data_in = droop_gen.DataIn(
            grid_freq=int(round(grid_f * 1_000_000)),
            freq_nominal=int(round(f_nom * 1_000_000)),
            current_p=int(round(current_p_kw * 1_000_000)),
            freq_quality=0,
            enable=True,
        )
        result = loader.tick(
            event_in=0,
            data_in_bytes=droop_gen.pack_data_in(data_in),
            ecc_state=ecc_in,
            super_step=0,
            data_out_size=droop_gen.DATA_OUT_SIZE,
        )
        out = droop_gen.unpack_data_out(result.data_out_bytes)
        return {
            "asset_p_setpoint_kw": {asset_did: out.p_setpoint / 1_000_000.0},
            "asset_droop_state": {
                asset_did: _DROOP_ECC.get(result.next_ecc_state, "?")
            },
            "cell_internals": {
                droop_key: loader.get_internal_bytes(droop_gen.INTERNAL_SIZE)
            },
            "cell_ecc_states": {droop_key: result.next_ecc_state},
        }

    return node


def _aggregator(state: BessState) -> dict[str, Any]:
    setpoints = state.get("asset_p_setpoint_kw") or {}
    current = state.get("asset_current_a") or {}
    voltage = state.get("asset_voltage_v") or {}
    # ΔP_cohort = Σ (setpoint - current_p)
    total = 0.0
    for did, sp in setpoints.items():
        v = voltage.get(did, 0.0)
        i = current.get(did, 0.0)
        cur_p = v * i / 1000.0
        total += sp - cur_p
    return {"cohort_total_delta_kw": total}


def _safe_node_id(did: str, stage: str) -> str:
    return f"{stage}_{did.replace(':', '_').replace('/', '_').replace('.', '_')}"


def build_bess_graph(
    bess_assets: list[tuple[str, float, float]],  # (did, p_rated_kw, capacity_ah)
    cycle_period_ms: int = 100,
):
    """Build the :loop:bess-charge-discharge graph.

    Each BESS asset gets a SOC_KALMAN → DROOP_P_F chain. All chains run
    in parallel; the aggregator merges per-asset Δp into the cohort
    output.
    """
    graph = StateGraph(BessState)
    for did, p_rated, capacity in bess_assets:
        soc_id = _safe_node_id(did, "soc")
        droop_id = _safe_node_id(did, "droop")
        graph.add_node(soc_id, _soc_node(did, capacity, cycle_period_ms))
        graph.add_node(droop_id, _droop_node(did, p_rated, cycle_period_ms))
        graph.add_edge(START, soc_id)
        graph.add_edge(soc_id, droop_id)
        graph.add_edge(droop_id, "aggregator")
    graph.add_node("aggregator", _aggregator)
    graph.add_edge("aggregator", END)
    return graph.compile(checkpointer=MemorySaver())


def step(
    app,
    config: dict[str, Any],
    grid_freq_hz: float,
    asset_voltage_v: dict[str, float],
    asset_current_a: dict[str, float],
    asset_temp_c: dict[str, float] | None = None,
    freq_nominal_hz: float = 50.0,
) -> BessState:
    return app.invoke(
        {
            "grid_freq_hz": grid_freq_hz,
            "freq_nominal_hz": freq_nominal_hz,
            "asset_voltage_v": asset_voltage_v,
            "asset_current_a": asset_current_a,
            "asset_temp_c": asset_temp_c or {did: 25.0 for did in asset_voltage_v},
        },
        config=config,
    )


def demo() -> None:
    assets = [
        # (did, p_rated_kw, capacity_ah)
        ("did:web:open-ot.etzhayyim.com:cell:bess-1", 100.0, 100.0),
        ("did:web:open-ot.etzhayyim.com:cell:bess-2", 50.0, 50.0),
    ]
    app = build_bess_graph(assets, cycle_period_ms=100)
    config = {"configurable": {"thread_id": "bess-demo"}}

    # Slight under-frequency event — both BESS should discharge if SOC > low.
    voltage_v = {assets[0][0]: 48.8, assets[1][0]: 48.5}
    current_a = {assets[0][0]: 20.0, assets[1][0]: 10.0}

    for i, f in enumerate([50.000, 49.950, 49.700, 49.500, 49.700, 49.950, 50.000], 1):
        state = step(app, config, f, voltage_v, current_a)
        socs = state.get("asset_soc_pct") or {}
        sps = state.get("asset_p_setpoint_kw") or {}
        cohort = state.get("cohort_total_delta_kw") or 0.0
        print(
            f"[step {i:>2}] f={f:.3f} Hz  "
            + " | ".join(
                f"{did.split(':')[-1]}: SOC={socs.get(did, 0):5.1f}% "
                f"P*={sps.get(did, 0):+7.1f} kW"
                for did, _, _ in assets
            )
            + f"  Σ ΔP={cohort:+7.1f} kW"
        )


if __name__ == "__main__":
    demo()
