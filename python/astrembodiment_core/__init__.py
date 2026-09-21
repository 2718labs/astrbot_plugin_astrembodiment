"""Python package wrapper for the Rust ASTER-CCN extension."""

from __future__ import annotations

import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path
from types import ModuleType

_BUILD_ID_PATTERN = re.compile(r"^[0-9a-f]{64}$")
_BUNDLED_ROOT = Path(__file__).resolve().parent / "_bundled"
_MANIFEST_SCHEMA = "astrembodiment-native-bundle-v1"

def _native_suffixes() -> tuple[str, ...]:
    if sys.platform == "win32":
        return (".pyd",)
    if sys.platform.startswith("linux"):
        return (".abi3.so", ".so")
    return ()

def _platform_key() -> str:
    if sys.platform == "win32":
        return "win32"
    if sys.platform.startswith("linux"):
        return "linux"
    raise ImportError(f"unsupported native platform: {sys.platform}")

def _bundled_native_path() -> Path:
    suffixes = _native_suffixes()
    if not suffixes:
        raise ImportError(f"unsupported native platform: {sys.platform}")
    if not _BUNDLED_ROOT.is_dir():
        raise ImportError(f"bundled native directory does not exist: {_BUNDLED_ROOT}")

    manifest_path = _BUNDLED_ROOT / "manifest.json"
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ImportError(f"invalid bundled native manifest: {manifest_path}") from exc
    if not isinstance(manifest, dict) or manifest.get("schema") != _MANIFEST_SCHEMA:
        raise ImportError(f"unsupported bundled native manifest: {manifest_path}")
    platforms = manifest.get("platforms")
    platform_key = _platform_key()
    entry = platforms.get(platform_key) if isinstance(platforms, dict) else None
    if not isinstance(entry, dict):
        raise ImportError(  # noqa: TRY004 - this is an import capability failure.
            f"native platform is not bundled: {platform_key}"
        )

    expected_build_id = entry.get("build_id")
    filename = entry.get("filename")
    if (
        not isinstance(expected_build_id, str)
        or not _BUILD_ID_PATTERN.fullmatch(expected_build_id)
        or not isinstance(filename, str)
        or Path(filename).name != filename
        or not filename.startswith("_native")
        or not any(filename.endswith(suffix) for suffix in suffixes)
    ):
        raise ImportError(f"invalid native manifest entry for {platform_key}")

    native_path = _BUNDLED_ROOT / expected_build_id / filename
    if not native_path.is_file():
        raise ImportError(f"bundled native extension does not exist: {native_path}")
    actual_build_id = hashlib.sha256(native_path.read_bytes()).hexdigest()
    if actual_build_id != expected_build_id:
        raise ImportError(
            "bundled native extension build id mismatch: "
            f"expected {expected_build_id}, got {actual_build_id}"
        )
    return native_path

def _load_native() -> ModuleType:
    native_name = f"{__name__}._native"
    if _BUNDLED_ROOT.exists():
        # A bundled installation always wins over any stale root extension.
        native_path = _bundled_native_path()
        identity = json.loads((_BUNDLED_ROOT / "manifest.json").read_text(encoding="utf-8")).get("build_info")
    else:
        package_root = Path(__file__).resolve().parent
        candidates = [p for p in package_root.glob("_native*") if p.is_file() and p.name.endswith(_native_suffixes())]
        if len(candidates) != 1:
            raise ImportError("wheel must contain exactly one native extension")
        native_path = candidates[0]
        try:
            identity = json.loads((package_root / "build_identity.json").read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            raise ImportError("wheel build identity unavailable") from exc
    if (not isinstance(identity, dict) or not isinstance(identity.get("source_sha"), str)
        or not re.fullmatch(r"[0-9a-f]{40}", identity["source_sha"])):
        raise ImportError("clean-source build identity required")
    # A new content-addressed path prevents CPython from reusing an old handle.
    sys.modules.pop(native_name, None)
    spec = importlib.util.spec_from_file_location(native_name, native_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"unable to create native loader for {native_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[native_name] = module
    try:
        spec.loader.exec_module(module)
        if json.loads(module.build_info_v1()) != identity:
            raise ImportError("native build identity mismatch")
        methods = sorted(name for name in dir(module) if not name.startswith("__")
                         and callable(getattr(module, name)) and name != "NativeCoreError")
        if methods != sorted(identity.get("methods", [])) or len(methods) != 19:
            raise ImportError("native callable manifest mismatch")
        if not hasattr(module, "NativeCoreError"):
            raise ImportError("native error type unavailable")
    except BaseException:
        sys.modules.pop(native_name, None)
        raise
    return module

try:
    _native_module = _load_native()
    build_info_v1 = _native_module.build_info_v1

    commit_core_inbound_v1 = _native_module.commit_core_inbound_v1
    compile_core_host_request_v1 = _native_module.compile_core_host_request_v1
    commit_core_delivery_outcome_v1 = _native_module.commit_core_delivery_outcome_v1
    list_embodiment_personas_v1 = _native_module.list_embodiment_personas_v1
    read_embodiment_profile_v1 = _native_module.read_embodiment_profile_v1
    get_embodiment_persona_v1 = _native_module.get_embodiment_persona_v1
    embodiment_clock_status_v1 = _native_module.embodiment_clock_status_v1
    advance_embodiment_time_v1 = _native_module.advance_embodiment_time_v1
    compare_and_swap_embodiment_profile_v1 = _native_module.compare_and_swap_embodiment_profile_v1
    create_embodiment_persona_if_missing_v1 = _native_module.create_embodiment_persona_if_missing_v1
    ensure_genesis = _native_module.ensure_genesis
    flush_and_close = _native_module.flush_and_close
    health = _native_module.health
    inspect = _native_module.inspect
    open = _native_module.open
    settle_semantic_appraisal_v1 = _native_module.settle_semantic_appraisal_v1
    verify_replay = _native_module.verify_replay
    version = _native_module.version
except (AttributeError, ImportError) as exc:  # pragma: no cover - install failure
    raise ImportError(
        f"AstrEmbodiment native extension import failed: {type(exc).__name__}: {exc}"
    ) from exc

try:
    NativeCoreError = _native_module.NativeCoreError
except AttributeError:  # pragma: no cover - compatibility with older wheels

    class NativeCoreError(RuntimeError):
        """Compatibility marker for native builds without the exported type."""

__all__ = [
    "build_info_v1",
    "NativeCoreError",
    "commit_core_inbound_v1",
    "compile_core_host_request_v1",
    "commit_core_delivery_outcome_v1",
    "list_embodiment_personas_v1",
    "read_embodiment_profile_v1",
    "get_embodiment_persona_v1",
    "embodiment_clock_status_v1",
    "advance_embodiment_time_v1",
    "compare_and_swap_embodiment_profile_v1",
    "create_embodiment_persona_if_missing_v1",
    "ensure_genesis",
    "flush_and_close",
    "health",
    "inspect",
    "open",
    "settle_semantic_appraisal_v1",
    "verify_replay",
    "version",
]
