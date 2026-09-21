#!/usr/bin/env python3
"""Check active source registrations; historical Rust codec strings are data."""

from __future__ import annotations
import ast
import hashlib
import json
from pathlib import Path
import re
import struct

ROOT = Path(__file__).resolve().parents[1]
RETIRED_MODULES = {
    "autonomy",
    "proactive",
    "proactive_settings",
    "secret_store",
    "observatory",
    "observatory_controls",
    "display",
    "ecosystem",
}
FORBIDDEN_MEMBERS = {f"astr_embodiment/{name}.py" for name in RETIRED_MODULES}
FORBIDDEN_SETTINGS = {
    "proactive_enabled",
    "proactive_frequency",
    "user_quiet_hours",
    "proactive_settings_revision",
    "proactive_daily_max",
    "min_proactive_cooldown_minutes",
    "quiet_hours_start",
    "quiet_hours_end",
    "quiet_hours_emergency_bypass",
    "intention_ttl_minutes",
    "unanswered_backoff_base_minutes",
    "unanswered_hard_stop",
    "emergency_threshold",
    "inner_activity_token_daily_max",
}


def manifests(root=ROOT):
    source = (root / "crates/ae-contracts/src/core_surface.rs").read_text(
        encoding="utf-8"
    )

    def section(name):
        return source.split(f"pub const {name}:", 1)[1].split("];", 1)[0]

    public = re.findall(r'\("(\w+)",\s*1\)', section("CORE_PUBLIC_METHOD_MANIFEST_V1"))
    retired = re.findall(r'"([^"]+)"', section("RETIRED_OPERATION_NAME_MANIFEST_V1"))
    host = re.findall(r'"([^"]+)"', section("RETIRED_HOST_IDENTIFIER_MANIFEST_V1"))
    kinds = re.findall(
        r'\((\d+),\s*"([^"]+)",\s*(\d+)\)', section("RETIRED_EVENT_KIND_MANIFEST_V1")
    )

    def lp(s):
        return struct.pack("<H", len(s.encode())) + s.encode()

    pub = struct.pack("<H", len(public)) + b"".join(lp(n) + b"\x01\x00" for n in public)
    ret = struct.pack("<H", len(retired)) + b"".join(lp(n) for n in retired)
    ret += struct.pack("<H", len(kinds)) + b"".join(
        bytes([int(k)]) + lp(n) + bytes([int(r)]) for k, n, r in kinds
    )
    ret += struct.pack("<H", len(host)) + b"".join(lp(n) for n in host)
    if (len(public), len(retired), len(kinds), len(host)) != (19, 35, 11, 14):
        raise ValueError("CORE_MANIFEST_COUNT")
    if (
        hashlib.sha256(pub).hexdigest()
        != "800e4dccb2a29b6edbaa2ac7cc46c34f466ece682e0ccf1a272577aacb3790c4"
        or hashlib.sha256(ret).hexdigest()
        != "6a28d0e925be34a108b636c47b4ea8ff1a69b763d7de14ba91d4a44127dc291a"
    ):
        raise ValueError("CORE_MANIFEST_GOLDEN")
    return public, set(retired) | set(host)


def scan_python(source, filename, retired):
    tree = ast.parse(source, filename=filename)
    for node in ast.walk(tree):
        if (
            isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            and node.name in retired
        ):
            raise ValueError(f"RETIRED_CALLABLE: {filename}:{node.name}")
        if isinstance(node, ast.Attribute) and node.attr in retired | {"send_message"}:
            raise ValueError(f"RETIRED_REFERENCE: {filename}:{node.attr}")
        if isinstance(node, ast.ImportFrom):
            names = (node.module or "").split(".") + [a.name for a in node.names]
            if set(names) & RETIRED_MODULES:
                raise ValueError(f"RETIRED_IMPORT: {filename}")
        if isinstance(node, ast.Import) and any(
            set(a.name.split(".")) & RETIRED_MODULES for a in node.names
        ):
            raise ValueError(f"RETIRED_IMPORT: {filename}")
        if (
            isinstance(node, ast.Constant)
            and isinstance(node.value, str)
            and node.value in retired | FORBIDDEN_SETTINGS
        ):
            raise ValueError(f"RETIRED_REGISTRATION: {filename}:{node.value}")
    return tree


def scan_source(root=ROOT):
    public, retired = manifests(root)
    native = (root / "crates/ae-pyo3/src/lib.rs").read_text(encoding="utf-8")
    registrations = re.findall(r"wrap_pyfunction!\s*\(\s*(\w+)\s*,", native)
    if sorted(registrations) != sorted(public):
        raise ValueError("NATIVE_REGISTRATION_SET")
    wrapper = ast.parse(
        (root / "python/astrembodiment_core/__init__.py").read_text(encoding="utf-8")
    )
    exports = next(
        ast.literal_eval(n.value)
        for n in wrapper.body
        if isinstance(n, ast.Assign)
        and any(isinstance(t, ast.Name) and t.id == "__all__" for t in n.targets)
    )
    if sorted(exports) != sorted(public + ["NativeCoreError"]):
        raise ValueError("WRAPPER_EXPORT_SET")
    for member in FORBIDDEN_MEMBERS:
        if (root / member).exists():
            raise ValueError(f"RETIRED_MEMBER: {member}")
    for path in [root / "main.py", *sorted((root / "astr_embodiment").rglob("*.py"))]:
        scan_python(path.read_text(encoding="utf-8"), str(path), retired)
    schema = json.loads((root / "_conf_schema.json").read_text(encoding="utf-8"))

    def keys(value):
        if isinstance(value, dict):
            for key, child in value.items():
                yield key
                yield from keys(child)
        elif isinstance(value, list):
            for child in value:
                yield from keys(child)

    if set(keys(schema)) & FORBIDDEN_SETTINGS:
        raise ValueError("RETIRED_CONFIG_KEY")
    return {
        "status": "SOURCE_SCAN_PASS",
        "methods": public,
        "scope": "active Python AST, PyO3 registration and wrapper exports; Rust execution reachability requires independent review",
    }


if __name__ == "__main__":
    print(json.dumps(scan_source(), sort_keys=True))
