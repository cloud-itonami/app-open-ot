"""SPEC §6 `vertex_open_ot_loop_checkpoint` writer (sqlite stand-in for RW).

Implements the open-ot persistence shape per ADR-2605151200 §6 — schema
mirrors what the production orchestrator will write into RisingWave via
asyncpg. SQLAlchemy Core (per ADR-2605080300), so the only thing that
changes between sqlite and RisingWave is the dialect / DSN.

Schema:

    vertex_open_ot_loop_checkpoint
      loop_did             text PRIMARY KEY part 1
      super_step           integer PRIMARY KEY part 2
      ts                   text (ISO 8601, UTC)
      ecc_states_json      text (JSON: dict[cell_did, int])
      internals_json       text (JSON: dict[cell_did, base64-encoded bytes])
      in_flight_msgs_json  text (JSON: list[message_dict])
      params_rev           text (hash of all-cells params for invalidation)

    vertex_open_ot_signal_change
      change_id            integer PRIMARY KEY autoincrement
      signal_did           text
      ts                   text (ISO 8601, UTC)
      value_micro_unit     integer (signed 64-bit)
      quality              text
      loop_did_affected_json  text (JSON: list[loop_did])

This module is **not** a LangGraph BaseCheckpointSaver subclass — it is
the SPEC §6 audit / resume layer that runs alongside whatever LangGraph
checkpointer the orchestrator is using. The same writer plugs into the
minimal Pregel runner (`pregel_runner.LoopRunner`) by passing checkpoints
through `OpenOtCheckpointer.write_checkpoint(...)` after each step.

Production: replace `sqlite:///...` DSN with the RisingWave Hyperdrive
connection string per `nixos/atama/modules/checkpointer-client.nix`.
Schema and code stay byte-identical.
"""

from __future__ import annotations

import base64
import hashlib
import json
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timezone
from typing import Iterator

from sqlalchemy import (
    Column,
    Integer,
    MetaData,
    String,
    Table,
    create_engine,
    func,
    insert,
    select,
)
from sqlalchemy.engine import Engine

from .pregel_runner import Checkpoint as PregelCheckpoint
from .pregel_runner import LoopRunner

metadata = MetaData()

LoopCheckpointTable = Table(
    "vertex_open_ot_loop_checkpoint",
    metadata,
    Column("loop_did", String, primary_key=True),
    Column("super_step", Integer, primary_key=True),
    Column("ts", String, nullable=False),
    Column("ecc_states_json", String, nullable=False),
    Column("internals_json", String, nullable=False),
    Column("in_flight_msgs_json", String, nullable=False, default="[]"),
    Column("params_rev", String, nullable=False),
)

SignalChangeTable = Table(
    "vertex_open_ot_signal_change",
    metadata,
    Column("change_id", Integer, primary_key=True, autoincrement=True),
    Column("signal_did", String, nullable=False),
    Column("ts", String, nullable=False),
    Column("value_micro_unit", Integer, nullable=False),
    Column("quality", String, nullable=False),
    Column("loop_did_affected_json", String, nullable=False, default="[]"),
)


@dataclass
class CheckpointRow:
    loop_did: str
    super_step: int
    ts: datetime
    ecc_states: dict[str, int]
    internals: dict[str, bytes]
    in_flight_msgs: list[dict]
    params_rev: str


def _utc_iso_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _params_rev(runner: LoopRunner) -> str:
    """Stable hash of all cell params, for resume-validity checking."""
    h = hashlib.sha256()
    for did in sorted(runner.cells.keys()):
        spec = runner.cells[did]
        h.update(did.encode())
        h.update(b":")
        h.update(spec.params_bytes)
        h.update(b";")
    return h.hexdigest()


def _internals_to_json(internals: dict[str, bytes]) -> str:
    return json.dumps(
        {did: base64.b64encode(b).decode("ascii") for did, b in internals.items()}
    )


def _internals_from_json(s: str) -> dict[str, bytes]:
    raw = json.loads(s)
    return {did: base64.b64decode(b) for did, b in raw.items()}


class OpenOtCheckpointer:
    """SPEC §6 writer / reader. Initialise once per orchestrator process."""

    def __init__(self, dsn: str = "sqlite:///:memory:") -> None:
        # `future=True` is the SQLAlchemy 2.x default — included for clarity.
        self.engine: Engine = create_engine(dsn, future=True)
        metadata.create_all(self.engine)

    @contextmanager
    def _conn(self) -> Iterator:
        with self.engine.begin() as c:
            yield c

    # -- writes ----------------------------------------------------------

    def write_checkpoint(
        self,
        loop_did: str,
        cp: PregelCheckpoint,
        params_rev: str,
        in_flight_msgs: list[dict] | None = None,
    ) -> None:
        with self._conn() as c:
            c.execute(
                insert(LoopCheckpointTable).values(
                    loop_did=loop_did,
                    super_step=cp.super_step,
                    ts=_utc_iso_now(),
                    ecc_states_json=json.dumps(cp.ecc_states),
                    internals_json=_internals_to_json(cp.internals),
                    in_flight_msgs_json=json.dumps(in_flight_msgs or []),
                    params_rev=params_rev,
                )
            )

    def record_signal_change(
        self,
        signal_did: str,
        value_micro_unit: int,
        quality: str = "good",
        loop_dids_affected: list[str] | None = None,
    ) -> int:
        with self._conn() as c:
            r = c.execute(
                insert(SignalChangeTable).values(
                    signal_did=signal_did,
                    ts=_utc_iso_now(),
                    value_micro_unit=value_micro_unit,
                    quality=quality,
                    loop_did_affected_json=json.dumps(loop_dids_affected or []),
                )
            )
            return int(r.inserted_primary_key[0])

    # -- reads -----------------------------------------------------------

    def latest_checkpoint(self, loop_did: str) -> CheckpointRow | None:
        stmt = (
            select(LoopCheckpointTable)
            .where(LoopCheckpointTable.c.loop_did == loop_did)
            .order_by(LoopCheckpointTable.c.super_step.desc())
            .limit(1)
        )
        with self._conn() as c:
            row = c.execute(stmt).mappings().first()
        if row is None:
            return None
        return CheckpointRow(
            loop_did=row["loop_did"],
            super_step=row["super_step"],
            ts=datetime.fromisoformat(row["ts"]),
            ecc_states=json.loads(row["ecc_states_json"]),
            internals=_internals_from_json(row["internals_json"]),
            in_flight_msgs=json.loads(row["in_flight_msgs_json"]),
            params_rev=row["params_rev"],
        )

    def list_checkpoints(self, loop_did: str) -> list[CheckpointRow]:
        stmt = (
            select(LoopCheckpointTable)
            .where(LoopCheckpointTable.c.loop_did == loop_did)
            .order_by(LoopCheckpointTable.c.super_step.asc())
        )
        with self._conn() as c:
            rows = c.execute(stmt).mappings().all()
        return [
            CheckpointRow(
                loop_did=r["loop_did"],
                super_step=r["super_step"],
                ts=datetime.fromisoformat(r["ts"]),
                ecc_states=json.loads(r["ecc_states_json"]),
                internals=_internals_from_json(r["internals_json"]),
                in_flight_msgs=json.loads(r["in_flight_msgs_json"]),
                params_rev=r["params_rev"],
            )
            for r in rows
        ]

    def count_checkpoints(self, loop_did: str | None = None) -> int:
        stmt = select(func.count()).select_from(LoopCheckpointTable)
        if loop_did is not None:
            stmt = stmt.where(LoopCheckpointTable.c.loop_did == loop_did)
        with self._conn() as c:
            return int(c.execute(stmt).scalar_one())


# -- runner integration helper ---------------------------------------------


def write_runner_checkpoint(
    cp_writer: OpenOtCheckpointer,
    loop_did: str,
    runner: LoopRunner,
    cp: PregelCheckpoint,
) -> None:
    """Convenience: compute params_rev + write."""
    cp_writer.write_checkpoint(loop_did, cp, params_rev=_params_rev(runner))


def restore_runner_from_checkpointer(
    cp_writer: OpenOtCheckpointer,
    loop_did: str,
    runner: LoopRunner,
) -> PregelCheckpoint | None:
    """Load latest checkpoint for `loop_did` and apply it to the runner.

    Validates `params_rev`: a cell-params change between checkpoint write
    and resume invalidates the resume (would replay against a different
    program). Raises `ValueError` on mismatch.
    """
    row = cp_writer.latest_checkpoint(loop_did)
    if row is None:
        return None
    expected_rev = _params_rev(runner)
    if row.params_rev != expected_rev:
        raise ValueError(
            f"params_rev mismatch for {loop_did}: "
            f"checkpoint={row.params_rev} runner={expected_rev}"
        )
    pcp = PregelCheckpoint(
        super_step=row.super_step,
        ecc_states=row.ecc_states,
        internals=row.internals,
        emissions=[],
    )
    runner.restore_from_checkpoint(pcp)
    return pcp
