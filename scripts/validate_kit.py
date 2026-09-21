#!/usr/bin/env python3
"""Read-only syntax, release identity and closed-surface validation."""
from __future__ import annotations
import ast
import json
import subprocess
import tomllib
from pathlib import Path
from scan_core_boundary import scan_source

ROOT = Path(__file__).resolve().parents[1]

def source_files():
    # Git inventory excludes user installations, databases and build/cache trees.
    names = subprocess.check_output(
        ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
        cwd=ROOT,
    ).decode("utf-8").split("\0")
    return [ROOT / name for name in sorted(set(names)) if name and (ROOT / name).is_file()]

def main():
    for path in source_files():
        if path.suffix == ".py":
            ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        elif path.suffix == ".toml":
            tomllib.loads(path.read_text(encoding="utf-8"))
        elif path.suffix == ".json":
            json.loads(path.read_text(encoding="utf-8-sig"))
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    project = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    assert cargo["workspace"]["package"]["version"] == "1.1.0"
    assert project["project"]["version"] == "1.1.0"
    assert 'version: "1.1.0"' in (ROOT / "metadata.yaml").read_text(encoding="utf-8")
    scan_source(ROOT)
    print("AstrEmbodiment source static validation: OK (read-only; no artifact acceptance)")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
