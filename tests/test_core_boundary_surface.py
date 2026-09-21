"""Closed Task 12A surface: source registration and fresh Native import agree."""
import ast
import hashlib
import importlib
import inspect
from pathlib import Path
import re
import struct

import pytest

ROOT = Path(__file__).resolve().parents[1]
SURFACE = (ROOT / "crates/ae-contracts/src/core_surface.rs").read_text(encoding="utf-8")


def _section(name):
    return SURFACE.split(f"pub const {name}:", 1)[1].split("];", 1)[0]


PUBLIC = re.findall(r'\("(\w+)",\s*1\)', _section("CORE_PUBLIC_METHOD_MANIFEST_V1"))
RETIRED = re.findall(r'"([^"]+)"', _section("RETIRED_OPERATION_NAME_MANIFEST_V1"))
HOST = re.findall(r'"([^"]+)"', _section("RETIRED_HOST_IDENTIFIER_MANIFEST_V1"))


def test_manifest_goldens_and_counts():
    def lp(value):
        return struct.pack("<H", len(value.encode())) + value.encode()
    public = struct.pack("<H", len(PUBLIC)) + b"".join(lp(n) + struct.pack("<H", 1) for n in PUBLIC)
    kinds = re.findall(r'\((\d+),\s*"([^"]+)",\s*(\d+)\)', _section("RETIRED_EVENT_KIND_MANIFEST_V1"))
    retired = struct.pack("<H", len(RETIRED)) + b"".join(lp(n) for n in RETIRED)
    retired += struct.pack("<H", len(kinds)) + b"".join(bytes([int(k)]) + lp(n) + bytes([int(r)]) for k,n,r in kinds)
    retired += struct.pack("<H", len(HOST)) + b"".join(lp(n) for n in HOST)
    assert (len(PUBLIC), len(RETIRED), len(kinds), len(HOST)) == (19,35,11,14)
    assert hashlib.sha256(public).hexdigest() == "800e4dccb2a29b6edbaa2ac7cc46c34f466ece682e0ccf1a272577aacb3790c4"
    assert hashlib.sha256(retired).hexdigest() == "6a28d0e925be34a108b636c47b4ea8ff1a69b763d7de14ba91d4a44127dc291a"


def test_source_surface_is_exact_and_host_has_no_retired_callables():
    native = (ROOT / "crates/ae-pyo3/src/lib.rs").read_text(encoding="utf-8")
    assert sorted(re.findall(r"wrap_pyfunction!\s*\(\s*(\w+)\s*,", native)) == sorted(PUBLIC)
    wrapper = ast.parse((ROOT / "python/astrembodiment_core/__init__.py").read_text(encoding="utf-8"))
    exports = next(ast.literal_eval(n.value) for n in wrapper.body if isinstance(n,ast.Assign) and any(isinstance(t,ast.Name) and t.id == "__all__" for t in n.targets))
    assert set(exports) == set(PUBLIC) | {"NativeCoreError"}
    from astr_embodiment.bridge import NativeBridge
    assert {n for n,v in vars(NativeBridge).items() if callable(v) and not n.startswith("_")} <= set(PUBLIC) | {"loaded", "close"}
    for path in [ROOT / "main.py", *sorted((ROOT / "astr_embodiment").glob("*.py"))]:
        tree = ast.parse(path.read_text(encoding="utf-8"))
        assert not {n.name for n in ast.walk(tree) if isinstance(n,(ast.FunctionDef,ast.AsyncFunctionDef))} & (set(RETIRED) | set(HOST))
    for module in ["autonomy", "proactive", "proactive_settings", "secret_store"]:
        assert not (ROOT / "astr_embodiment" / f"{module}.py").exists()


def test_fresh_native_direct_surface_and_predecode_rejection():
    core = importlib.import_module("astrembodiment_core")
    native = core._native_module
    assert {n for n in dir(native) if inspect.isbuiltin(getattr(native,n))} == set(PUBLIC)
    for name in RETIRED:
        assert not hasattr(native,name)
        with pytest.raises(core.NativeCoreError, match="^UNSUPPORTED_CORE_BOUNDARY$"):
            core.compile_core_host_request_v1(name, "{" * 100_000)
    for name in ["", "APPLY_EVENT", "apply_event\0", "x" * 65, "未知"]:
        with pytest.raises(core.NativeCoreError, match="^UNKNOWN_OPERATION$"):
            core.compile_core_host_request_v1(name, "{" * 100_000)
