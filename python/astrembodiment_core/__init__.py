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
    native_path = _bundled_native_path()
    # A new content-addressed path prevents CPython from reusing an old handle.
    sys.modules.pop(native_name, None)
    spec = importlib.util.spec_from_file_location(native_name, native_path)
    if spec is None or spec.loader is None:
        raise ImportError(f"unable to create native loader for {native_path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[native_name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        sys.modules.pop(native_name, None)
        raise
    return module


try:
    _native_module = _load_native()

    apply_event = _native_module.apply_event
    contract_info = _native_module.contract_info
    ensure_genesis = _native_module.ensure_genesis
    flush_and_close = _native_module.flush_and_close
    health = _native_module.health
    inspect = _native_module.inspect
    open = _native_module.open
    prepare_rebirth_v1 = _native_module.prepare_rebirth_v1
    confirm_rebirth_v1 = _native_module.confirm_rebirth_v1
    reconcile_seed_config_v1 = _native_module.reconcile_seed_config_v1
    ack_seed_config_writeback_v1 = _native_module.ack_seed_config_writeback_v1
    semantic_outbox_crypto_status_v1 = _native_module.semantic_outbox_crypto_status_v1
    semantic_outbox_seal_v1 = _native_module.semantic_outbox_seal_v1
    semantic_outbox_open_v1 = _native_module.semantic_outbox_open_v1
    semantic_revision_v1 = _native_module.semantic_revision_v1
    apply_perception_proposal_v1 = _native_module.apply_perception_proposal_v1
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
    "NativeCoreError",
    "ack_seed_config_writeback_v1",
    "apply_event",
    "apply_perception_proposal_v1",
    "confirm_rebirth_v1",
    "contract_info",
    "ensure_genesis",
    "flush_and_close",
    "health",
    "inspect",
    "open",
    "prepare_rebirth_v1",
    "reconcile_seed_config_v1",
    "semantic_revision_v1",
    "semantic_outbox_crypto_status_v1",
    "semantic_outbox_open_v1",
    "semantic_outbox_seal_v1",
    "verify_replay",
    "version",
]
