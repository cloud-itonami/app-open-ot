"""Async microgrid `:loop:freq-droop` (#2 of the carry-over candidates).

Same graph as `microgrid_langgraph.py`, but driven via `app.astream(...)`
to demonstrate the production-shape async runtime. The synchronous
`invoke()` form is the one used in the Pregel-equivalence proof; this
module shows that the same graph runs unchanged under the async event
loop, which is what a Granian / FastAPI worker on the Atama edge
controller will use (per ADR-2605080600 §LangGraph + Granian L3 Runtime).

Key differences from the sync demo:

  - Per-super-step iteration uses `async for chunk in app.astream(...)`.
  - State updates are streamed (`stream_mode="updates"`) so the
    orchestrator sees each LangGraph node's contribution incrementally —
    useful for live HMI updates and for the per-tick checkpointer write
    that production will do.
  - Multiple loops (multiple `thread_id`s) can run concurrently via
    `asyncio.gather`, which is the per-loop parallelism we need for
    multi-tenant edge controllers.

Run: `uv run python -m open_ot_orchestrator.microgrid_async_langgraph`
"""

from __future__ import annotations

import asyncio
from typing import Any

from .microgrid_langgraph import build_freq_droop_graph


async def step_async(
    app,
    config: dict[str, Any],
    grid_freq_hz: float,
    current_p_kw: dict[str, float],
    freq_nominal_hz: float = 50.0,
) -> dict[str, Any]:
    """Run one async super-step.

    Streams the per-node updates and returns the final consolidated state
    (after the aggregator node). Caller can also iterate updates live by
    using `astream` directly.
    """
    inputs = {
        "grid_freq_hz": grid_freq_hz,
        "freq_nominal_hz": freq_nominal_hz,
        "current_p_kw": current_p_kw,
    }
    final_state: dict[str, Any] = {}
    async for chunk in app.astream(inputs, config=config, stream_mode="updates"):
        # `chunk` is `{node_id: state_delta}`; merge into final_state.
        for delta in chunk.values():
            for k, v in delta.items():
                if isinstance(v, dict) and isinstance(final_state.get(k), dict):
                    final_state[k] = {**final_state[k], **v}
                else:
                    final_state[k] = v
    return final_state


async def run_schedule(
    app,
    thread_id: str,
    schedule: list[float],
    current_p_kw: dict[str, float],
) -> list[dict[str, Any]]:
    """Run a frequency schedule on one thread; return per-step final states."""
    config = {"configurable": {"thread_id": thread_id}}
    states: list[dict[str, Any]] = []
    for f in schedule:
        s = await step_async(app, config, f, current_p_kw)
        states.append(s)
    return states


async def run_concurrent_loops(
    app,
    loops: list[tuple[str, list[float], dict[str, float]]],
) -> dict[str, list[dict[str, Any]]]:
    """Run N independent loops concurrently via asyncio.gather.

    Each (thread_id, schedule, current_p_kw) entry is one loop; LangGraph's
    per-thread checkpointer keeps their states isolated.
    """
    tasks = [run_schedule(app, tid, sched, cp) for tid, sched, cp in loops]
    results = await asyncio.gather(*tasks)
    return {tid: r for (tid, _, _), r in zip(loops, results)}


# ---------------------------------------------------------------------------
# Demo
# ---------------------------------------------------------------------------


async def _demo() -> None:
    print("[microgrid-async] building :loop:freq-droop with 2 BESS assets")
    bess = [
        ("did:web:open-ot.etzhayyim.com:cell:droop-bess-1", 1000.0),
        ("did:web:open-ot.etzhayyim.com:cell:droop-bess-2", 500.0),
    ]
    app = build_freq_droop_graph(bess, cycle_period_ms=100)
    schedule = [50.000, 50.050, 50.300, 50.500, 50.300, 50.050, 50.000]
    current_p = {bess[0][0]: 800.0, bess[1][0]: 400.0}

    # Single-loop async run.
    print("\n--- single async thread ---")
    states = await run_schedule(app, "async-demo-1", schedule, current_p)
    for i, s in enumerate(states, start=1):
        print(
            f"[step {i:>2}] f={schedule[i - 1]:.3f} Hz  "
            f"cohort ΔP = {s['cohort_total_delta_kw']:+8.2f} kW"
        )

    # Two threads concurrently — different schedules, demonstrating
    # asyncio.gather + per-thread isolation.
    print("\n--- two concurrent threads ---")
    results = await run_concurrent_loops(
        app,
        loops=[
            ("async-demo-thread-A", schedule, current_p),
            ("async-demo-thread-B", [50.0, 49.7, 49.5, 49.7, 50.0], current_p),
        ],
    )
    for tid, states in results.items():
        last = states[-1]
        print(f"[{tid}] final cohort ΔP = {last['cohort_total_delta_kw']:+8.2f} kW")


def demo() -> None:
    asyncio.run(_demo())


if __name__ == "__main__":
    demo()
