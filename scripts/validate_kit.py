#!/usr/bin/env python3
"""Static validation for the development kit.

This does not replace cargo check. It validates formats, Python syntax, links,
and the critical formula/authority separation.
"""

from __future__ import annotations

import ast
import hashlib
import json
import sys
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def validate_python() -> None:
    for path in list(ROOT.rglob("*.py")):
        ast.parse(path.read_text(encoding="utf-8"), filename=str(path))


def validate_toml() -> None:
    for path in ROOT.rglob("*.toml"):
        tomllib.loads(path.read_text(encoding="utf-8"))


def validate_json() -> None:
    for path in ROOT.rglob("*.json"):
        # AstrBot-generated JSON files may carry a UTF-8 BOM on Windows.
        json.loads(path.read_text(encoding="utf-8-sig"))


def validate_markdown_links() -> None:
    import re

    pattern = re.compile(r"\[[^\]]+\]\(([^)]+)\)")
    errors: list[str] = []
    for path in ROOT.rglob("*.md"):
        text = path.read_text(encoding="utf-8")
        for target in pattern.findall(text):
            if "://" in target or target.startswith("#"):
                continue
            clean = target.split("#", 1)[0]
            if clean and not (path.parent / clean).resolve().exists():
                errors.append(f"{path.relative_to(ROOT)} -> {target}")
    if errors:
        raise SystemExit("Broken Markdown links:\n" + "\n".join(errors))


def validate_invariants() -> None:
    formula = tomllib.loads(
        (ROOT / "model/formula-v1.toml").read_text(encoding="utf-8")
    )
    one = tomllib.loads((ROOT / "model/runtime-1c1g.toml").read_text(encoding="utf-8"))
    two = tomllib.loads((ROOT / "model/runtime-2c2g.toml").read_text(encoding="utf-8"))
    authority = tomllib.loads(
        (ROOT / "model/authority-matrix-v1.toml").read_text(encoding="utf-8")
    )
    assert formula["neuron_slots"] == 16_384
    assert formula["edge_capacity"] == 524_288
    assert one["worker_threads"] == 1 and two["worker_threads"] == 2
    assert "neuron_slots" not in one and "neuron_slots" not in two
    assert authority["self_action"]["allow"] == []
    assert authority["self_critique"]["allow"] == []
    assert authority["platform_observed"]["allow"] == []


def write_manifest() -> None:
    rows = []
    for path in sorted(
        p for p in ROOT.rglob("*") if p.is_file() and p.name != "FILE_MANIFEST.json"
    ):
        data = path.read_bytes()
        rows.append(
            {
                "path": str(path.relative_to(ROOT)).replace("\\", "/"),
                "bytes": len(data),
                "sha256": hashlib.sha256(data).hexdigest(),
            }
        )
    (ROOT / "FILE_MANIFEST.json").write_text(
        json.dumps(
            {"schema": "astrembodiment-devkit-manifest-v1", "files": rows}, indent=2
        ),
        encoding="utf-8",
    )


def main() -> int:
    validate_python()
    validate_toml()
    validate_json()
    validate_markdown_links()
    validate_invariants()
    write_manifest()
    print("AstrEmbodiment development kit static validation: OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
