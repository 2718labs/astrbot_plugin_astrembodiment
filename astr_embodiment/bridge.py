"""Coarse-grained bridge to the Rust native runtime.

Python may freeze persona data, build closed JSON envelopes and call exactly
the coarse surface below. It can never read or write neural state, residuals
or identity directly; every mutation goes through the Rust single writer.
"""

from __future__ import annotations

import json
import platform
import sys
from dataclasses import dataclass
from importlib import import_module
from importlib import util as importlib_util
from pathlib import Path
from typing import Any

_STORE_FILENAME = "astrembodiment.sqlite3"

class NativeCoreUnavailable(RuntimeError):
    """Raised when the bundled platform wheel cannot be imported."""

class NativeCoreError(RuntimeError):
    """Raised by the native core with a stable machine-readable code."""

    def __init__(self, code: str, detail: str) -> None:
        super().__init__(f"{code}::{detail}")
        self.code = code
        self.detail = detail

class GenesisUnavailable(NativeCoreError):
    pass

class RetryWait(NativeCoreError):
    pass

class SeedDigestCollision(NativeCoreError):
    pass

class StaleRevision(NativeCoreError):
    pass

class ClosedSchemaViolation(NativeCoreError):
    pass

class UnsupportedEventKind(NativeCoreError):
    pass

class GenesisRequired(NativeCoreError):
    pass

class GenesisManifestMismatch(NativeCoreError):
    pass

class StaleCausalBase(NativeCoreError):
    pass

class SemanticAppraisalRetryExpiredOrUnknown(NativeCoreError):
    """A semantic-appraisal retry can no longer be resolved safely."""

class ObserveInvalidRequest(NativeCoreError):
    """The closed observation request was rejected by Native."""

class ObserveInvalidCursor(NativeCoreError):
    """The observation cursor is invalid for its requested persona scope."""

class ObserveProjectionUnavailable(NativeCoreError):
    """Native could not prove a safe committed observation projection."""

_ERROR_TYPES: dict[str, type[NativeCoreError]] = {
    "GENESIS_UNAVAILABLE": GenesisUnavailable,
    "RETRY_WAIT": RetryWait,
    "SEED_DIGEST_COLLISION": SeedDigestCollision,
    "STALE_REVISION": StaleRevision,
    "CLOSED_SCHEMA": ClosedSchemaViolation,
    "UNSUPPORTED_EVENT": UnsupportedEventKind,
    "GENESIS_REQUIRED": GenesisRequired,
    "GENESIS_MANIFEST_MISMATCH": GenesisManifestMismatch,
    "STALE_CAUSAL_BASE": StaleCausalBase,
    "SEMANTIC_APPRAISAL_RETRY_EXPIRED_OR_UNKNOWN": (
        SemanticAppraisalRetryExpiredOrUnknown
    ),
    "OBSERVE_INVALID_REQUEST": ObserveInvalidRequest,
    "OBSERVE_INVALID_CURSOR": ObserveInvalidCursor,
    "OBSERVE_PROJECTION_UNAVAILABLE": ObserveProjectionUnavailable,
}

@dataclass(frozen=True, slots=True)
class NativeHealth:
    status: str
    formula: str
    neuron_slots: int
    version: str

def _classify(error: BaseException) -> NativeCoreError:
    message = str(error)
    if "::" in message:
        code, _, detail = message.partition("::")
    else:
        code, detail = getattr(error, "code", "STORAGE"), message
    error_type = _ERROR_TYPES.get(code, NativeCoreError)
    return error_type(code, detail)

def _parse_payload(result: str) -> dict[str, Any]:
    payload = json.loads(result)
    if not isinstance(payload, dict):
        raise NativeCoreUnavailable("native core returned invalid payload")
    return payload

def _bundled_native_package_dir() -> Path:
    """Return the native package beside the plugin's Python packages."""
    return Path(__file__).resolve().parents[1] / "astrembodiment_core"

def _native_import_diagnostics(error: ImportError) -> str:
    """Keep the loader's root cause visible in the AstrBot install log."""
    return (
        "AstrEmbodiment bundled native core could not be imported: "
        f"{type(error).__name__}: {error}; "
        f"python={sys.version.split()[0]} "
        f"implementation={sys.implementation.name} "
        f"machine={platform.machine()} "
        f"system={platform.system()} "
        f"executable={sys.executable}"
    )

def _load_bundled_native() -> Any:
    """Load the bundled core when a host does not expose the plugin root.

    AstrBot normally imports a plugin as ``data.plugins.<name>.main`` and the
    relative import in :meth:`NativeBridge.open` handles that namespace. Some
    host wrappers load ``main.py`` as a top-level module instead, so the plugin
    directory is not on ``sys.path`` and a normal top-level import cannot see
    the sibling package. Loading from the archive-relative path keeps that
    fallback independent of the host's import policy.
    """
    package_dir = _bundled_native_package_dir()
    init_path = package_dir / "__init__.py"
    if not init_path.is_file():
        raise ModuleNotFoundError(
            f"No bundled native package at {package_dir}",
            name="astrembodiment_core",
        )

    module_name = "astrembodiment_core"
    existing = sys.modules.get(module_name)
    if existing is not None:
        return existing

    spec = importlib_util.spec_from_file_location(
        module_name,
        init_path,
        submodule_search_locations=[str(package_dir)],
    )
    if spec is None or spec.loader is None:
        raise ImportError(f"Unable to load bundled native package from {init_path}")

    module = importlib_util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    except BaseException:
        sys.modules.pop(module_name, None)
        raise
    return module

class NativeBridge:
    def __init__(self) -> None:
        self._native: Any | None = None

    def open(self, data_dir: str) -> NativeHealth:
        """Open the native runtime inside AstrBot's plugin data directory.

        ``StarTools.get_data_dir()`` returns a directory, while SQLite needs a
        file path. Keep that host/storage boundary explicit so an already
        existing AstrBot data directory is not passed to SQLite as a database
        file.
        """
        store_path = Path(data_dir).expanduser() / _STORE_FILENAME
        try:
            native = None
            package_name = __package__ or ""
            if "." in package_name:
                try:
                    # AstrBot normally imports this module inside the plugin
                    # namespace, where the sibling package is addressable.
                    native = import_module("..astrembodiment_core", package_name)
                except ModuleNotFoundError as exc:
                    expected_name = (
                        f"{package_name.rsplit('.', 1)[0]}.astrembodiment_core"
                    )
                    if exc.name not in {expected_name, "astrembodiment_core"}:
                        raise

            if native is None:
                try:
                    native = import_module("astrembodiment_core")
                except ModuleNotFoundError as exc:
                    if exc.name != "astrembodiment_core":
                        raise
                    native = _load_bundled_native()
        except ImportError as exc:
            raise NativeCoreUnavailable(_native_import_diagnostics(exc)) from exc
        native.open(str(store_path))
        self._native = native
        payload = json.loads(native.health())
        return NativeHealth(
            status=str(payload.get("status", "unknown")),
            formula=str(payload.get("formula", "unknown")),
            neuron_slots=int(payload.get("neuron_slots", 0)),
            version=str(native.version()),
        )

    def _require(self) -> Any:
        if self._native is None:
            raise NativeCoreUnavailable("native core is not open")
        return self._native

    def ensure_genesis(self, closed_request: dict[str, Any]) -> dict[str, Any]:
        """Submit one closed PersonaGenesisRequest to the native single-writer lane.

        Python may freeze/compile a proposal, but only Rust can project the
        validated Manifest into numerical priors, commit the GenesisManifest
        and IncarnationRecord, and issue SeedCode.
        """
        native = self._require()
        try:
            result = native.ensure_genesis(
                json.dumps(closed_request, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _parse_payload(result)

    def commit_core_inbound_v1(self, request: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.commit_core_inbound_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def compile_core_host_request_v1(self, operation: str, request: dict[str, Any]) -> dict[str, Any]:
        try:
            return _parse_payload(self._require().compile_core_host_request_v1(
                operation, json.dumps(request, ensure_ascii=False, sort_keys=True)
            ))
        except Exception as exc:
            raise _classify(exc) from exc

    def commit_core_delivery_outcome_v1(self, request: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.commit_core_delivery_outcome_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def list_embodiment_personas_v1(self, request: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.list_embodiment_personas_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def read_embodiment_profile_v1(self, scope: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.read_embodiment_profile_v1(json.dumps(scope, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def get_embodiment_persona_v1(self, scope: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.get_embodiment_persona_v1(json.dumps(scope, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def embodiment_clock_status_v1(self, request: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.embodiment_clock_status_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def compare_and_swap_embodiment_profile_v1(self, request: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.compare_and_swap_embodiment_profile_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def advance_embodiment_time_v1(self, request_bytes: bytes | list[int]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.advance_embodiment_time_v1(bytes(request_bytes)))
        except Exception as exc:
            raise _classify(exc) from exc

    def create_embodiment_persona_if_missing_v1(self, request: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.create_embodiment_persona_if_missing_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def settle_semantic_appraisal_v1(
        self, request: dict[str, Any]
    ) -> dict[str, Any]:
        native = self._require()
        try:
            return _parse_payload(native.settle_semantic_appraisal_v1(json.dumps(request, sort_keys=True, separators=(",", ":"))))
        except Exception as exc:
            raise _classify(exc) from exc

    def inspect(self, scope: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            result = native.inspect(
                json.dumps(scope, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _parse_payload(result)

    def verify_replay(self, scope: dict[str, Any]) -> dict[str, Any]:
        native = self._require()
        try:
            result = native.verify_replay(
                json.dumps(scope, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _parse_payload(result)

    @property
    def loaded(self) -> bool:
        return self._native is not None

    def health(self) -> NativeHealth:
        native = self._require()
        payload = json.loads(native.health())
        return NativeHealth(
            status=str(payload.get("status", "unknown")),
            formula=str(payload.get("formula", "unknown")),
            neuron_slots=int(payload.get("neuron_slots", 0)),
            version=str(native.version()),
        )

    def close(self) -> None:
        if self._native is not None:
            self._native.flush_and_close()
        self._native = None
