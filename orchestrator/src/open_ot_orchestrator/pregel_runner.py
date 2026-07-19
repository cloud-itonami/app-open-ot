"""Minimal Python Pregel BSP runner for open-ot loops.

No LangGraph dependency — keeps the IEC 61499 ↔ Pregel super-step contract
isolated so it can be tested by itself. ADR-2605151200 §LangGraph + Pregel
binding §4.1 mapping table is the spec; this is the executable shape.

Per super-step:

  1. For each cell, look up its `event_in_code` + `data_in_bytes` for this
     step (from the orchestrator's signal-change feed, or from the previous
     step's emitted neighbor messages — none of the current cells emit
     neighbor messages, so this is purely external feed today).
  2. Call `cell.tick(event_in, data_in, ecc_state, super_step, ...)`.
  3. Update `ecc_states` and `internals` snapshots in place.
  4. Append a `Checkpoint` row: (super_step, ecc_states, internals,
     emitted_per_cell). This row IS the audit trail.

Replay from checkpoint = construct a fresh `LoopRunner`, restore each
cell's `internal` bytes from the checkpoint, replay subsequent inputs.
The determinism contract (SPEC §4.2) requires byte-identical outputs.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Callable

from .cell_loader import CellLoader, TickResult


@dataclass
class CellSpec:
    """Static description of one cell instance in a loop."""

    did: str
    loader: CellLoader
    params_bytes: bytes
    internal_size: int
    data_in_size: int
    data_out_size: int
    initial_ecc: int = 0


@dataclass
class StepInput:
    """Per-cell input for one super-step."""

    event_in_code: int
    data_in_bytes: bytes


@dataclass
class StepEmission:
    """Per-cell output of one super-step."""

    cell_did: str
    next_ecc_state: int
    out_event_raw: int
    data_out_bytes: bytes


@dataclass
class Checkpoint:
    """One row of the audit trail / resume source."""

    super_step: int
    ecc_states: dict[str, int]
    internals: dict[str, bytes]
    emissions: list[StepEmission] = field(default_factory=list)


class LoopRunner:
    """Run one open-ot loop as a sequence of Pregel super-steps."""

    def __init__(self, cells: list[CellSpec]) -> None:
        if not cells:
            raise ValueError("LoopRunner requires at least one cell")
        seen_dids = set()
        for c in cells:
            if c.did in seen_dids:
                raise ValueError(f"duplicate cell DID: {c.did}")
            seen_dids.add(c.did)
        self.cells: dict[str, CellSpec] = {c.did: c for c in cells}
        self._ecc_states: dict[str, int] = {c.did: c.initial_ecc for c in cells}
        self._super_step: int = 0
        self.checkpoints: list[Checkpoint] = []

    def initialize(self) -> None:
        """Call `<cell>_init` for each cell. Must be called before `run_step`."""
        for cell in self.cells.values():
            cell.loader.init(cell.params_bytes, cell.internal_size)
        # Snapshot the post-init state as super_step = 0.
        self._super_step = 0
        self._snapshot()

    def run_step(self, inputs: dict[str, StepInput]) -> Checkpoint:
        """Run one super-step. `inputs` keyed by cell DID.

        Cells without an entry in `inputs` are skipped this step (they
        retain their previous ECC state and internal). This implements
        the single-task / row-driven trigger pattern (per ADR-2605082200):
        only cells affected by the triggering signal change participate.
        """
        self._super_step += 1
        emissions: list[StepEmission] = []
        for did, cell in self.cells.items():
            if did not in inputs:
                continue
            inp = inputs[did]
            result: TickResult = cell.loader.tick(
                event_in=inp.event_in_code,
                data_in_bytes=inp.data_in_bytes,
                ecc_state=self._ecc_states[did],
                super_step=self._super_step,
                data_out_size=cell.data_out_size,
            )
            self._ecc_states[did] = result.next_ecc_state
            emissions.append(
                StepEmission(
                    cell_did=did,
                    next_ecc_state=result.next_ecc_state,
                    out_event_raw=result.out_event_raw,
                    data_out_bytes=result.data_out_bytes,
                )
            )
        return self._snapshot(emissions=emissions)

    def _snapshot(self, emissions: list[StepEmission] | None = None) -> Checkpoint:
        cp = Checkpoint(
            super_step=self._super_step,
            ecc_states=dict(self._ecc_states),
            internals={
                did: cell.loader.get_internal_bytes(cell.internal_size)
                for did, cell in self.cells.items()
            },
            emissions=list(emissions) if emissions else [],
        )
        self.checkpoints.append(cp)
        return cp

    # -- resume / replay --------------------------------------------------

    def restore_from_checkpoint(self, checkpoint: Checkpoint) -> None:
        """Set each cell's internal bytes + ECC state from a checkpoint."""
        if set(checkpoint.internals.keys()) != set(self.cells.keys()):
            raise ValueError("checkpoint cell DIDs do not match runner cells")
        for did, cell in self.cells.items():
            cell.loader.set_internal_bytes(checkpoint.internals[did])
            self._ecc_states[did] = checkpoint.ecc_states[did]
        self._super_step = checkpoint.super_step
        # New checkpoint stream starts fresh from the resumed point.
        self.checkpoints = [checkpoint]


# -- replay helper ---------------------------------------------------------


def replay(
    runner_factory: Callable[[], LoopRunner],
    inputs_per_step: list[dict[str, StepInput]],
    resume_from: Checkpoint | None = None,
    resume_inputs_offset: int = 0,
) -> list[Checkpoint]:
    """Run inputs through a fresh runner. If `resume_from` is set, restore
    that checkpoint first and start from `resume_inputs_offset` in the
    `inputs_per_step` list.

    Returns the runner's full checkpoint stream after the run.
    """
    runner = runner_factory()
    if resume_from is None:
        runner.initialize()
        for inp in inputs_per_step:
            runner.run_step(inp)
    else:
        runner.initialize()  # init still needed to allocate internal bytes
        runner.restore_from_checkpoint(resume_from)
        for inp in inputs_per_step[resume_inputs_offset:]:
            runner.run_step(inp)
    return runner.checkpoints
