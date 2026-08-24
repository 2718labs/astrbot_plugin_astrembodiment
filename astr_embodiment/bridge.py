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


class RebirthConfirmationRequired(NativeCoreError):
    pass


class RebirthFenceStale(NativeCoreError):
    pass


class RebirthNonceConflict(NativeCoreError):
    pass


class ContextProjectionIntegrity(NativeCoreError):
    pass


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
    "REBIRTH_CONFIRMATION_REQUIRED": RebirthConfirmationRequired,
    "REBIRTH_FENCE_STALE": RebirthFenceStale,
    "REBIRTH_NONCE_CONFLICT": RebirthNonceConflict,
    "CONTEXT_RECEIPT_INVALID": ContextProjectionIntegrity,
    "CONTEXT_PROJECTION": ContextProjectionIntegrity,
    "CONTEXT_COMMIT_MISSING": ContextProjectionIntegrity,
    "CONTEXT_COMMIT_INTEGRITY": ContextProjectionIntegrity,
}

_CONTEXT_SUMMARY_SCHEMA = "astrembodiment.context-summary.v1"
_CONTEXT_SUMMARY_KEYS = frozenset(
    {
        "schema",
        "summary_revision",
        "source_continuum_revision",
        "dimensions_ema_fxp6",
        "unresolved_boundary",
        "unresolved_repair",
        "repetition_count",
        "delivery_outcome",
        "summary_digest",
    }
)
_CONTEXT_DELIVERY_OUTCOMES = frozenset({"pending", "delivered", "failed"})
_CONTEXT_DIMENSION_COUNT = 15
_CONTEXT_DIMENSION_MAX = 1_000_000

_REBIRTH_PREPARE_SCHEMA = "astrembodiment.rebirth-prepare.v1"
_REBIRTH_RESPONSE_SCHEMA = "astrembodiment.rebirth-response.v1"
_REBIRTH_SCOPE_KEYS = frozenset(
    {"bot_token", "persona_token", "relation_token", "session_token"}
)
_REBIRTH_PREPARE_REQUEST_KEYS = frozenset(
    {"scope", "expected_incarnation_id", "expected_revision", "action"}
)
_REBIRTH_CONFIRM_REQUEST_REQUIRED_KEYS = frozenset(
    {
        "scope",
        "expected_incarnation_id",
        "expected_revision",
        "request_nonce",
        "action",
    }
)
_REBIRTH_CONFIRM_REQUEST_ALLOWED_KEYS = (
    _REBIRTH_CONFIRM_REQUEST_REQUIRED_KEYS | {"confirmed"}
)
_REBIRTH_PREPARE_RESPONSE_KEYS = frozenset(
    {"schema", "state", "request_nonce", "request_nonce_digest", "binding_digest"}
)
_REBIRTH_RESPONSE_KEYS = frozenset({"schema", "state", "receipt"})
_REBIRTH_RECEIPT_KEYS = frozenset(
    {
        "receipt_id",
        "action",
        "scope_token_short",
        "request_nonce_digest",
        "parent_incarnation_short",
        "child_incarnation_short",
        "before_revision",
        "after_revision",
        "outcome",
        "audit_time_ms",
    }
)
_REBIRTH_ACTIONS = frozenset({"REBIRTH", "CLEAR_ACTIVE_STATE"})


def _rebirth_integrity_error(detail: str) -> NativeCoreError:
    return NativeCoreError("REBIRTH_RESPONSE_INVALID", detail)


def _is_digest_hex(value: Any) -> bool:
    if not isinstance(value, str) or len(value) != 64:
        return False
    try:
        return len(bytes.fromhex(value)) == 32
    except ValueError:
        return False


def _is_token_hex(value: Any) -> bool:
    if not isinstance(value, str) or len(value) != 32:
        return False
    try:
        return len(bytes.fromhex(value)) == 16
    except ValueError:
        return False


def _validate_rebirth_scope(scope: Any) -> None:
    if not isinstance(scope, dict) or set(scope) != _REBIRTH_SCOPE_KEYS:
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth scope is not closed")
    if not _is_token_hex(scope["bot_token"]) or not _is_token_hex(
        scope["persona_token"]
    ) or not _is_token_hex(scope["session_token"]):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth scope token is invalid")
    relation_token = scope["relation_token"]
    if relation_token is not None and not _is_token_hex(relation_token):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth relation token is invalid")


def _validate_rebirth_prepare_request(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _REBIRTH_PREPARE_REQUEST_KEYS:
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth prepare request is not closed")
    _validate_rebirth_scope(payload["scope"])
    if not _is_digest_hex(payload["expected_incarnation_id"]):
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth expected incarnation is invalid"
        )
    if not _positive_int_or_zero(payload["expected_revision"]):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth expected revision is invalid")
    if payload["action"] not in _REBIRTH_ACTIONS:
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth action is invalid")
    return dict(payload)


def _validate_rebirth_confirm_request(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth confirmation is invalid")
    keys = set(payload)
    if not _REBIRTH_CONFIRM_REQUEST_REQUIRED_KEYS <= keys or not keys <= (
        _REBIRTH_CONFIRM_REQUEST_ALLOWED_KEYS
    ):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth confirmation is not closed")
    _validate_rebirth_scope(payload["scope"])
    if not _is_digest_hex(payload["expected_incarnation_id"]) or not _is_digest_hex(
        payload["request_nonce"]
    ):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth digest is invalid")
    if not _positive_int_or_zero(payload["expected_revision"]):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth expected revision is invalid")
    if payload["action"] not in _REBIRTH_ACTIONS:
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth action is invalid")
    # ``confirmed`` deliberately remains optional at this layer.  When it is
    # absent, Rust must issue REBIRTH_CONFIRMATION_REQUIRED rather than Python
    # manufacturing consent or converting the request to a schema error.
    if "confirmed" in payload and type(payload["confirmed"]) is not bool:
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth confirmation flag is invalid")
    return dict(payload)


def _positive_int_or_zero(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value >= 0


def _validate_rebirth_prepare_response(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _REBIRTH_PREPARE_RESPONSE_KEYS:
        raise _rebirth_integrity_error("rebirth prepare response is not closed")
    if payload["schema"] != _REBIRTH_PREPARE_SCHEMA:
        raise _rebirth_integrity_error("rebirth prepare schema is invalid")
    if payload["state"] != "CONFIRMATION_PENDING":
        raise _rebirth_integrity_error("rebirth prepare state is invalid")
    if not all(
        _is_digest_hex(payload[field])
        for field in ("request_nonce", "request_nonce_digest", "binding_digest")
    ):
        raise _rebirth_integrity_error("rebirth prepare digest is invalid")
    return dict(payload)


def _validate_rebirth_response(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _REBIRTH_RESPONSE_KEYS:
        raise _rebirth_integrity_error("rebirth response is not closed")
    if payload["schema"] != _REBIRTH_RESPONSE_SCHEMA:
        raise _rebirth_integrity_error("rebirth response schema is invalid")
    if payload["state"] not in {"COMMITTED", "REPLAYED"}:
        raise _rebirth_integrity_error("rebirth response state is invalid")
    receipt = payload["receipt"]
    if not isinstance(receipt, dict) or set(receipt) != _REBIRTH_RECEIPT_KEYS:
        raise _rebirth_integrity_error("rebirth receipt is not closed")
    if not _is_digest_hex(receipt["receipt_id"]) or not _is_digest_hex(
        receipt["request_nonce_digest"]
    ):
        raise _rebirth_integrity_error("rebirth receipt digest is invalid")
    if receipt["action"] not in _REBIRTH_ACTIONS or receipt["outcome"] != "COMMITTED":
        raise _rebirth_integrity_error("rebirth receipt enum is invalid")
    if not all(
        isinstance(receipt[field], str) and receipt[field]
        for field in (
            "scope_token_short",
            "parent_incarnation_short",
            "child_incarnation_short",
        )
    ):
        raise _rebirth_integrity_error("rebirth receipt short identifier is invalid")
    if not _positive_int_or_zero(receipt["before_revision"]) or receipt[
        "after_revision"
    ] != 0:
        raise _rebirth_integrity_error("rebirth receipt revision is invalid")
    if not _positive_int(receipt["audit_time_ms"]):
        raise _rebirth_integrity_error("rebirth audit time is invalid")
    return dict(payload)


def _context_integrity_error(detail: str) -> ContextProjectionIntegrity:
    return ContextProjectionIntegrity("CONTEXT_PROJECTION", detail)


def _positive_int(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and value > 0


def validate_context_summary_payload(payload: Any) -> dict[str, Any]:
    """Accept the native aggregate-only context schema and nothing else.

    The bridge is deliberately a second privacy boundary: an unexpected field
    could carry raw dialogue or host identifiers, so it is rejected before a
    result is cached or reaches the host prompt.
    """
    if not isinstance(payload, dict) or set(payload) != _CONTEXT_SUMMARY_KEYS:
        raise _context_integrity_error(
            "context summary must use the closed aggregate schema"
        )
    if payload["schema"] != _CONTEXT_SUMMARY_SCHEMA:
        raise _context_integrity_error("context summary schema is invalid")
    if not _positive_int(payload["summary_revision"]):
        raise _context_integrity_error("context summary revision is invalid")
    if not _positive_int(payload["source_continuum_revision"]):
        raise _context_integrity_error("context source revision is invalid")
    if not _positive_int(payload["repetition_count"]):
        raise _context_integrity_error("context repetition count is invalid")
    dimensions = payload["dimensions_ema_fxp6"]
    if (
        not isinstance(dimensions, list)
        or len(dimensions) != _CONTEXT_DIMENSION_COUNT
        or any(
            not isinstance(value, int)
            or isinstance(value, bool)
            or value < 0
            or value > _CONTEXT_DIMENSION_MAX
            for value in dimensions
        )
    ):
        raise _context_integrity_error("context dimensions are invalid")
    if not isinstance(payload["unresolved_boundary"], bool) or not isinstance(
        payload["unresolved_repair"], bool
    ):
        raise _context_integrity_error("context unresolved flags are invalid")
    if payload["delivery_outcome"] not in _CONTEXT_DELIVERY_OUTCOMES:
        raise _context_integrity_error("context delivery outcome is invalid")
    digest = payload["summary_digest"]
    if not isinstance(digest, str) or len(digest) != 64:
        raise _context_integrity_error("context summary digest is invalid")
    try:
        if len(bytes.fromhex(digest)) != 32:
            raise ValueError("digest length")
    except ValueError as exc:
        raise _context_integrity_error("context summary digest is invalid") from exc
    return dict(payload)


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

    def prepare_rebirth_v1(self, closed_request: dict[str, Any]) -> dict[str, Any]:
        """Create exactly one durable, explicitly requested rebirth challenge.

        The bridge only validates and forwards the closed request.  Nonce
        creation, challenge persistence and all authority decisions remain in
        the Rust lifecycle owner.
        """
        request = _validate_rebirth_prepare_request(closed_request)
        native = self._require()
        try:
            result = native.prepare_rebirth_v1(
                json.dumps(request, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _validate_rebirth_prepare_response(_parse_payload(result))

    def confirm_rebirth_v1(self, closed_request: dict[str, Any]) -> dict[str, Any]:
        """Forward the second, explicit rebirth action without altering consent.

        Missing ``confirmed`` deliberately reaches Rust unchanged so it maps
        to the fixed ``REBIRTH_CONFIRMATION_REQUIRED`` rejection.  Python
        never supplies a nonce, identity, receipt, audit time or replay state.
        """
        request = _validate_rebirth_confirm_request(closed_request)
        native = self._require()
        try:
            result = native.confirm_rebirth_v1(
                json.dumps(request, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _validate_rebirth_response(_parse_payload(result))

    def apply_event(
        self, scope: dict[str, Any], event: dict[str, Any]
    ) -> dict[str, Any]:
        native = self._require()
        try:
            result = native.apply_event(
                json.dumps(scope, ensure_ascii=False, sort_keys=True),
                json.dumps(event, ensure_ascii=False, sort_keys=True),
            )
        except Exception as exc:
            raise _classify(exc) from exc
        payload = _parse_payload(result)
        if payload.get("schema") == "astrembodiment.decision.v1":
            summary = payload.get("context_summary")
            if summary is None:
                raise _context_integrity_error(
                    "native decision is missing its committed context summary"
                )
            payload["context_summary"] = validate_context_summary_payload(summary)
        return payload

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
