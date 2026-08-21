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

from .contracts import ScopeTokens
from .semantic_estimator import (
    SemanticProposalError,
    proposal_to_json,
    validate_perception_proposal,
)

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


class InvalidPerceptionProposal(NativeCoreError):
    pass


class InvalidPerceptionScope(NativeCoreError):
    pass


class SemanticIdentityConflict(NativeCoreError):
    pass


class SemanticRevisionOverflow(NativeCoreError):
    pass


class SemanticStateUnchanged(NativeCoreError):
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
    "INVALID_PERCEPTION_PROPOSAL": InvalidPerceptionProposal,
    "INVALID_PERCEPTION_SCOPE": InvalidPerceptionScope,
    "SEMANTIC_IDENTITY_CONFLICT": SemanticIdentityConflict,
    "SEMANTIC_REVISION_OVERFLOW": SemanticRevisionOverflow,
    "SEMANTIC_STATE_UNCHANGED": SemanticStateUnchanged,
}

_SEMANTIC_CURSOR_SCHEMA = "astrembodiment.semantic-revision.v1"
_SEMANTIC_RESULT_SCHEMA = "astrembodiment.semantic-perception-closure.v1"
_SEMANTIC_RESULT_FIELDS = frozenset(
    {"schema", "receipt", "revision", "deduplicated"}
)
SEMANTIC_RECEIPT_FIELDS = frozenset(
    {
        "schema_version",
        "formula_digest",
        "scope_digest",
        "event_digest",
        "authority_digest",
        "base_revision",
        "next_revision",
        "state_before",
        "state_after",
        "graph_after",
        "active_nodes",
        "active_edges",
        "residuals",
        "status",
    }
)
_RESIDUAL_FIELDS = frozenset(
    {"authority", "continuity", "energy", "renormalization", "capacity"}
)
_SEMANTIC_ERROR_CODES = frozenset(
    {
        "CLOSED",
        "CLOSED_SCHEMA",
        "ENCODING",
        "GENESIS_REQUIRED",
        "INVALID_PERCEPTION_PROPOSAL",
        "INVALID_PERCEPTION_SCOPE",
        "INVALID_NEURAL_STATE",
        "LEASE_CONFLICT",
        "LEASE_IN_FLIGHT",
        "NATIVE_SYMBOL_UNAVAILABLE",
        "NATIVE_UNAVAILABLE",
        "SEMANTIC_IDENTITY_CONFLICT",
        "SEMANTIC_REVISION_OVERFLOW",
        "SEMANTIC_STATE_UNCHANGED",
        "STALE_REVISION",
        "STALE_CAUSAL_BASE",
        "STORAGE",
    }
)
_FORBIDDEN_RECEIPT_KEYS = frozenset(
    {
        "action",
        "action_contract",
        "context",
        "history",
        "input",
        "provider",
        "raw_text",
        "text",
        "tool",
        "tools",
        "xml",
    }
)
_RECEIPT_FIELDS = SEMANTIC_RECEIPT_FIELDS


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


def _degraded(code: str) -> dict[str, str]:
    """Return the only semantic failure shape exposed to the plugin."""

    return {"status": "DEGRADED", "code": code}


def _semantic_error_code(error: BaseException) -> str:
    """Extract only a fixed code; never expose native detail text."""

    code = getattr(error, "code", None)
    if not isinstance(code, str):
        message = str(error)
        code = message.partition("::")[0]
    if code in _SEMANTIC_ERROR_CODES:
        return code
    if isinstance(error, NativeCoreUnavailable):
        return "NATIVE_UNAVAILABLE"
    return "NATIVE_ERROR"


def _scope_payload(scope: ScopeTokens | str | dict[str, Any]) -> dict[str, Any]:
    if isinstance(scope, ScopeTokens):
        payload = scope.scope_json()
    elif isinstance(scope, str):
        try:
            payload = json.loads(
                scope, object_pairs_hook=_scope_pairs_without_duplicates
            )
        except (TypeError, ValueError, json.JSONDecodeError) as exc:
            raise ValueError("scope") from exc
        if not isinstance(payload, dict):
            raise ValueError("scope")
    elif isinstance(scope, dict):
        payload = dict(scope)
    else:
        raise ValueError("scope")
    if set(payload) != {
        "bot_token",
        "persona_token",
        "relation_token",
        "session_token",
    }:
        raise ValueError("scope")
    for name in ("bot_token", "persona_token", "session_token"):
        value = payload.get(name)
        if (
            not isinstance(value, str)
            or len(value) != 32
            or not _looks_hex(value)
            or not any(bytes.fromhex(value))
        ):
            raise ValueError("scope")
    relation = payload.get("relation_token")
    if relation is not None and (
        not isinstance(relation, str)
        or len(relation) != 32
        or not _looks_hex(relation)
        or not any(bytes.fromhex(relation))
    ):
        raise ValueError("scope")
    return payload


def _scope_pairs_without_duplicates(
    pairs: list[tuple[str, Any]],
) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("scope")
        result[key] = value
    return result


def _looks_hex(value: str) -> bool:
    try:
        decoded = bytes.fromhex(value)
    except (TypeError, ValueError):
        return False
    return len(decoded) == len(value) // 2


def _closed_json(value: dict[str, Any]) -> str:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )


def _native_json(value: Any) -> dict[str, Any]:
    if isinstance(value, str):
        payload = json.loads(value)
    else:
        payload = value
    if not isinstance(payload, dict):
        raise ValueError("native payload")
    return payload


def _validate_cursor_payload(value: Any) -> dict[str, Any]:
    payload = _native_json(value)
    if set(payload) != {"schema", "revision"}:
        raise ValueError("cursor payload")
    if payload.get("schema") != _SEMANTIC_CURSOR_SCHEMA:
        raise ValueError("cursor schema")
    revision = payload.get("revision")
    if type(revision) is not int or revision < 0:
        raise ValueError("cursor revision")
    return {"schema": _SEMANTIC_CURSOR_SCHEMA, "revision": revision}


def _validate_semantic_result(value: Any) -> dict[str, Any]:
    payload = _native_json(value)
    if set(payload) != _SEMANTIC_RESULT_FIELDS:
        raise ValueError("semantic payload")
    if payload.get("schema") != _SEMANTIC_RESULT_SCHEMA:
        raise ValueError("semantic schema")
    revision = payload.get("revision")
    if type(revision) is not int or revision < 0:
        raise ValueError("semantic revision")
    deduplicated = payload.get("deduplicated")
    if type(deduplicated) is not bool:
        raise ValueError("semantic deduplication")
    receipt = payload.get("receipt")
    if not isinstance(receipt, dict):
        raise ValueError("semantic receipt")
    if set(receipt) - _RECEIPT_FIELDS:
        raise ValueError("semantic receipt fields")
    if _FORBIDDEN_RECEIPT_KEYS.intersection(receipt):
        raise ValueError("semantic receipt fields")
    integer_fields = {
        "schema_version",
        "base_revision",
        "next_revision",
        "active_nodes",
        "active_edges",
    }
    for field in integer_fields.intersection(receipt):
        if type(receipt[field]) is not int or receipt[field] < 0:
            raise ValueError("semantic receipt integer")
    digest_fields = {
        "formula_digest",
        "scope_digest",
        "event_digest",
        "authority_digest",
        "state_before",
        "state_after",
        "graph_after",
    }
    for field in digest_fields.intersection(receipt):
        digest = receipt[field]
        if (
            not isinstance(digest, str)
            or len(digest) != 64
            or not _looks_hex(digest)
        ):
            raise ValueError("semantic receipt digest")
    if "status" in receipt and receipt["status"] not in {
        "committed",
        "rejected",
        "superseded",
        "stale",
    }:
        raise ValueError("semantic receipt status")
    if "residuals" in receipt:
        residuals = receipt["residuals"]
        if not isinstance(residuals, dict) or set(residuals) != _RESIDUAL_FIELDS:
            raise ValueError("semantic receipt residuals")
        if any(
            type(residuals[name]) is not int
            or not -(1 << 63) <= residuals[name] <= (1 << 63) - 1
            for name in _RESIDUAL_FIELDS
        ):
            raise ValueError("semantic receipt residuals")
    # Rebuild the outer object so an extension cannot smuggle extra fields or
    # a mutable reference into coordinator state.
    return {
        "schema": _SEMANTIC_RESULT_SCHEMA,
        "receipt": json.loads(_closed_json(receipt)),
        "revision": revision,
        "deduplicated": deduplicated,
    }


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
        return _parse_payload(result)

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

    def semantic_revision_v1(
        self, scope_json: ScopeTokens | str | dict[str, Any]
    ) -> dict[str, Any]:
        """Read the content-free SPC1 semantic cursor.

        Missing symbols, malformed native output, and native exceptions are
        represented by a fixed degraded object.  The ordinary G0 bridge
        methods above retain their historical exception semantics.
        """

        try:
            scope = _scope_payload(scope_json)
        except Exception:
            return _degraded("INVALID_PERCEPTION_SCOPE")
        try:
            native = self._require()
        except Exception as exc:
            return _degraded(_semantic_error_code(exc))
        method = getattr(native, "semantic_revision_v1", None)
        if not callable(method):
            return _degraded("NATIVE_SYMBOL_UNAVAILABLE")
        try:
            result = method(_closed_json(scope))
            return _validate_cursor_payload(result)
        except Exception as exc:
            if isinstance(exc, (ValueError, TypeError, json.JSONDecodeError)):
                return _degraded("NATIVE_MALFORMED")
            return _degraded(_semantic_error_code(exc))

    def apply_perception_proposal_v1(
        self,
        scope_json: ScopeTokens | str | dict[str, Any],
        proposal_json: str | dict[str, Any],
    ) -> dict[str, Any]:
        """Submit one closed SPC1 proposal to the native semantic lane."""

        try:
            scope = _scope_payload(scope_json)
        except Exception:
            return _degraded("INVALID_PERCEPTION_SCOPE")
        try:
            native = self._require()
        except Exception as exc:
            return _degraded(_semantic_error_code(exc))
        method = getattr(native, "apply_perception_proposal_v1", None)
        if not callable(method):
            return _degraded("NATIVE_SYMBOL_UNAVAILABLE")
        try:
            proposal = validate_perception_proposal(proposal_json)
            encoded_proposal = proposal_to_json(proposal)
        except SemanticProposalError:
            return _degraded("INVALID_PERCEPTION_PROPOSAL")
        except Exception:
            return _degraded("INVALID_PERCEPTION_PROPOSAL")
        try:
            result = method(_closed_json(scope), encoded_proposal)
            return _validate_semantic_result(result)
        except Exception as exc:
            if isinstance(exc, (ValueError, TypeError, json.JSONDecodeError)):
                return _degraded("NATIVE_MALFORMED")
            return _degraded(_semantic_error_code(exc))

    def commit_perception_proposal_v1(
        self,
        scope_json: ScopeTokens | str | dict[str, Any],
        proposal: dict[str, Any],
    ) -> dict[str, Any]:
        """Read the cursor then submit a proposal, preserving native order.

        This helper intentionally does not rewrite ``base_revision``.  The
        coordinator freezes that field after the cursor read; a mismatch is a
        fixed degraded stale-base outcome rather than an implicit rebind.
        """

        cursor = self.semantic_revision_v1(scope_json)
        if cursor.get("status") == "DEGRADED":
            return cursor
        try:
            if proposal.get("base_revision") != cursor.get("revision"):
                return _degraded("STALE_REVISION")
        except AttributeError:
            return _degraded("INVALID_PERCEPTION_PROPOSAL")
        return self.apply_perception_proposal_v1(scope_json, proposal)

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
