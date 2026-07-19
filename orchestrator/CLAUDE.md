# orchestrator — Pregel demos

Python orchestrator demos for `etzhayyim-project-open-ot`. Demonstrates the central architectural claim of ADR-2605151200: **IEC 61499 event tick ≡ Pregel super-step**, with cells running as WASM modules and the orchestrator coordinating multi-cell loops.

## Layout

| Path | Purpose |
|---|---|
| `pyproject.toml` | uv-managed project, requires Python ≥ 3.11 |
| `src/open_ot_orchestrator/cell_loader.py` | wasmtime-py wrapper for any open-ot cell — `init()` / `tick()` / `get_internal_bytes()` / `set_internal_bytes()` |
| `src/open_ot_orchestrator/pregel_runner.py` | minimal Python BSP super-step runner (#3a) |
| `src/open_ot_orchestrator/microgrid_pregel.py` | concrete demo using `pregel_runner` (#3a) |
| `src/open_ot_orchestrator/microgrid_langgraph.py` | real LangGraph SDK integration (#3b) |
| `tests/test_*.py` | pytest unit + integration tests |
| `README.md` | usage |

## Build / run

```bash
# 1. Build cells as wasm32-unknown-unknown (one-time per cell change).
cd ../cells
cargo build --release --no-default-features --target wasm32-unknown-unknown -p pid-limited
cargo build --release --no-default-features --target wasm32-unknown-unknown -p droop-p-f

# 2. Set up Python venv (uv).
cd ../orchestrator
uv sync

# 3. Run tests.
uv run pytest

# 4. Run microgrid Pregel demo.
uv run python -m open_ot_orchestrator.microgrid_pregel

# 5. Run microgrid LangGraph demo.
uv run python -m open_ot_orchestrator.microgrid_langgraph
```

## Scope discipline

- **demo-only**: this is a reference orchestrator that demonstrates the binding. It is **not** production. Production-side LangGraph orchestrator runs inside the etzhayyim LangServer pod (per ADR-2605080600) on a Giemon Atama edge controller (per cad-spec).
- **No checkpointer persistence**: in-memory dict for #3a, LangGraph `MemorySaver` for #3b. Real RisingWave checkpointer integration belongs in the LangServer pod.
- **No wamrc**: this orchestrator runs cells via Wasmtime (Python). Real embedded path is WAMR AOT on Zephyr (Mimi/Te) — that's a different deployment, not this demo.

## Why Wasmtime (not WAMR) here

Python is the orchestrator language; `wasmtime-py` exists, `wamr-py` does not. WAMR is for embedded MCU deployments where the orchestrator (Atama) doesn't run cells anyway — Atama runs Wasmtime for tier-2 cells co-located on the gateway, and dispatches tier-1 cells over Zenoh to Mimi/Te (where they run on WAMR AOT). So Wasmtime here matches the Atama-side runtime, not the field-device runtime.
