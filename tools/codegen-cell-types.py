#!/usr/bin/env python3
"""open-ot codegen: cell `manifest.json` → typed Python pack/unpack module.

For each `60-apps/etzhayyim-project-open-ot/cells/<cell>/manifest.json`,
emits `orchestrator/src/open_ot_orchestrator/_generated/<cell>.py` with:

  - `struct.pack` format strings derived from Rust `#[repr(C)]` layout
    rules (alignment + tail padding to struct alignment).
  - `@dataclass`-flavoured DataIn / DataOut / Params / Internal types.
  - `pack_*` / `unpack_*` functions.
  - Static metadata (FBTYPE, ECC states, ABI export names).

Eliminates the hand-rolled struct-format strings currently in
`microgrid_pregel.py` and `microgrid_islanding_langgraph.py`. Once cells
are migrated to the generated wrappers, manifest ⇄ Rust drift is
impossible by construction.

Usage:
    python3 70-tools/scripts/open-ot/codegen-cell-types.py
    # Reads default cells/ + writes default _generated/ paths.

    python3 70-tools/scripts/open-ot/codegen-cell-types.py --check
    # Dry-run — fail with nonzero exit if any generated file is stale.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
from dataclasses import dataclass
from pathlib import Path


def _short(p: Path) -> str:
    """Display a path relative to cwd if possible, else absolute."""
    try:
        return os.path.relpath(p)
    except ValueError:
        return str(p)

REPO_ROOT = Path(__file__).resolve().parent.parent.parent.parent
DEFAULT_CELLS_DIR = REPO_ROOT / "60-apps/etzhayyim-project-open-ot/cells"
DEFAULT_OUTPUT_DIR = (
    REPO_ROOT
    / "60-apps/etzhayyim-project-open-ot/orchestrator/src/open_ot_orchestrator/_generated"
)


# Rust type → (size_bytes, alignment_bytes, struct_format_code, python_type_hint).
# Bool packs as one byte (the same as u8) by Rust convention.
TYPE_INFO: dict[str, tuple[int, int, str, str]] = {
    "i8":   (1, 1, "b", "int"),
    "u8":   (1, 1, "B", "int"),
    "i16":  (2, 2, "h", "int"),
    "u16":  (2, 2, "H", "int"),
    "i32":  (4, 4, "i", "int"),
    "u32":  (4, 4, "I", "int"),
    "i64":  (8, 8, "q", "int"),
    "u64":  (8, 8, "Q", "int"),
    "f32":  (4, 4, "f", "float"),
    "f64":  (8, 8, "d", "float"),
    "bool": (1, 1, "B", "bool"),
}


@dataclass
class Layout:
    fmt: str
    size: int
    fields: list[tuple[str, str, int]]  # (name, rust_type, offset)


def compute_layout(fields: list[dict]) -> Layout:
    """Compute Rust `#[repr(C)]` layout for an ordered field list.

    Each field is `{"name": str, "rust_type": str}`. Returns the
    `struct.pack` format string, total padded size, and per-field offsets.
    """
    fmt = "<"
    offset = 0
    field_offsets: list[tuple[str, str, int]] = []
    struct_align = 1
    for f in fields:
        name = f["name"]
        rust = f["rust_type"]
        if rust not in TYPE_INFO:
            raise ValueError(f"unknown rust_type {rust!r} on field {name!r}")
        size, align, code, _ = TYPE_INFO[rust]
        # Pad up to alignment.
        pad = (align - (offset % align)) % align
        if pad:
            fmt += f"{pad}x"
            offset += pad
        field_offsets.append((name, rust, offset))
        fmt += code
        offset += size
        struct_align = max(struct_align, align)
    # Tail pad to struct alignment.
    tail = (struct_align - (offset % struct_align)) % struct_align
    if tail:
        fmt += f"{tail}x"
        offset += tail
    return Layout(fmt=fmt, size=offset, fields=field_offsets)


# --------------------------------------------------------------------------
# Codegen
# --------------------------------------------------------------------------


def _python_field_decl(name: str, rust_type: str, wire: str | None) -> str:
    py_hint = TYPE_INFO[rust_type][3]
    wire_note = f"  # {rust_type} (wire: {wire})" if wire else f"  # {rust_type}"
    return f"    {name}: {py_hint}{wire_note}"


def _emit_section(
    section_name: str,
    fields: list[dict],
    class_name: str,
) -> str:
    """Emit dataclass + pack + unpack + size constant for one struct."""
    if not fields:
        return f"# {section_name}: empty — no struct emitted.\n\n"

    layout = compute_layout(
        [{"name": f["name"], "rust_type": f["rust_type"]} for f in fields]
    )
    field_decls = "\n".join(
        _python_field_decl(f["name"], f["rust_type"], f.get("wire")) for f in fields
    )
    pack_args = ", ".join(
        ("(1 if d." + f["name"] + " else 0)") if f["rust_type"] == "bool" else "d." + f["name"]
        for f in fields
    )
    unpack_kwargs = ", ".join(
        (f["name"] + "=bool(_t[" + str(i) + "])") if f["rust_type"] == "bool"
        else (f["name"] + "=_t[" + str(i) + "]")
        for i, f in enumerate(fields)
    )
    upper = section_name.upper()
    return f"""\
{upper}_FMT = {layout.fmt!r}
{upper}_SIZE = {layout.size}
{upper}_OFFSETS: dict[str, int] = {{
{chr(10).join(f"    {n!r}: {o}," for n, _r, o in layout.fields)}
}}


@dataclass
class {class_name}:
{field_decls}


def pack_{section_name}(d: {class_name}) -> bytes:
    return struct.pack({upper}_FMT, {pack_args})


def unpack_{section_name}(buf: bytes) -> {class_name}:
    _t = struct.unpack({upper}_FMT, buf)
    return {class_name}({unpack_kwargs})


"""


def emit_module(manifest: dict, source_path: Path) -> str:
    cell_dir_name = source_path.parent.name
    cell_symbol = cell_dir_name.replace("-", "_")
    fbtype = manifest.get("iec61499_fbtype", "")
    abi = manifest.get("abi", {}) or {}
    init_export = abi.get("init_export", f"{cell_symbol}_init")
    tick_export = abi.get("tick_export", f"{cell_symbol}_tick")
    ecc = manifest.get("ecc", {}) or {}
    ecc_states = ecc.get("states", [])
    ecc_initial = ecc.get("initial", "")
    tick_max_emitted = manifest.get("tick_max_emitted", 0)
    tick_max_neighbor = manifest.get("tick_max_neighbor_msgs", 0)

    sections: list[str] = []
    sections.append(
        _emit_section("data_in", manifest.get("data_in_schema", []) or [], "DataIn")
    )
    sections.append(
        _emit_section("data_out", manifest.get("data_out_schema", []) or [], "DataOut")
    )
    sections.append(
        _emit_section("params", manifest.get("params_schema", []) or [], "Params")
    )
    sections.append(
        _emit_section("internal", manifest.get("internal_schema", []) or [], "Internal")
    )

    header = f'''\
"""Generated from `cells/{cell_dir_name}/manifest.json` by
`70-tools/scripts/open-ot/codegen-cell-types.py`. **Do not edit by hand**;
re-run the codegen and check the diff. CI gate via the same script's
`--check` flag.

FBType   : {fbtype}
ECC      : {ecc_states} (initial={ecc_initial})
ABI      : init={init_export}  tick={tick_export}
Tick caps: max_emitted={tick_max_emitted}  max_neighbor_msgs={tick_max_neighbor}
"""

from __future__ import annotations

import struct
from dataclasses import dataclass

CELL_DIR     = {cell_dir_name!r}
CELL_SYMBOL  = {cell_symbol!r}
FBTYPE       = {fbtype!r}
INIT_EXPORT  = {init_export!r}
TICK_EXPORT  = {tick_export!r}
ECC_STATES   = {ecc_states!r}
ECC_INITIAL  = {ecc_initial!r}
TICK_MAX_EMITTED        = {tick_max_emitted}
TICK_MAX_NEIGHBOR_MSGS  = {tick_max_neighbor}


'''
    return header + "\n".join(sections)


# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cells-dir", type=Path, default=DEFAULT_CELLS_DIR)
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT_DIR)
    parser.add_argument(
        "--check",
        action="store_true",
        help="Dry-run: exit non-zero if any generated file is stale.",
    )
    args = parser.parse_args(argv)

    args.output_dir.mkdir(parents=True, exist_ok=True)

    # Always (re)write __init__.py with a list of generated modules so that
    # `from open_ot_orchestrator._generated import <cell>` works cleanly.
    cell_modules = []
    for manifest in sorted(args.cells_dir.glob("*/manifest.json")):
        cell_dir = manifest.parent.name
        cell_symbol = cell_dir.replace("-", "_")
        cell_modules.append(cell_symbol)

    init_body = (
        '"""Auto-generated package — do not edit individual files."""\n\n'
        + "\n".join(f"from . import {m}  # noqa: F401" for m in cell_modules)
        + "\n\n__all__ = "
        + repr(cell_modules)
        + "\n"
    )

    diffs: list[str] = []
    new_files: list[Path] = []

    init_path = args.output_dir / "__init__.py"
    if not args.check:
        init_path.write_text(init_body)
        new_files.append(init_path)
    else:
        existing = init_path.read_text() if init_path.exists() else ""
        if existing != init_body:
            diffs.append(_short(init_path))

    for manifest_path in sorted(args.cells_dir.glob("*/manifest.json")):
        try:
            manifest = json.loads(manifest_path.read_text())
        except json.JSONDecodeError as exc:
            print(
                f"FAIL: {_short(manifest_path)}: invalid JSON: {exc}",
                file=sys.stderr,
            )
            return 2

        body = emit_module(manifest, manifest_path)
        out_path = args.output_dir / f"{manifest_path.parent.name.replace('-', '_')}.py"

        if args.check:
            existing = out_path.read_text() if out_path.exists() else ""
            if existing != body:
                diffs.append(_short(out_path))
        else:
            out_path.write_text(body)
            new_files.append(out_path)

    if args.check:
        if diffs:
            print(
                "FAIL: generated files stale — re-run codegen-cell-types.py:",
                file=sys.stderr,
            )
            for d in diffs:
                print(f"  - {d}", file=sys.stderr)
            return 1
        print("OK: all generated files up to date.")
        return 0

    print(f"OK: wrote {len(new_files)} files to {_short(args.output_dir)}")
    for f in new_files:
        print(f"  - {_short(f)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
