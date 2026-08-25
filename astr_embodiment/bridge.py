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
    _canonical_hex,
    _canonical_nonzero_hex,
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


class RebirthConfirmationRequired(NativeCoreError):
    pass


class RebirthFenceStale(NativeCoreError):
    pass


class RebirthNonceConflict(NativeCoreError):
    pass


class ContextProjectionIntegrity(NativeCoreError):
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
    "REBIRTH_CONFIRMATION_REQUIRED": RebirthConfirmationRequired,
    "REBIRTH_FENCE_STALE": RebirthFenceStale,
    "REBIRTH_NONCE_CONFLICT": RebirthNonceConflict,
    "CONTEXT_RECEIPT_INVALID": ContextProjectionIntegrity,
    "CONTEXT_PROJECTION": ContextProjectionIntegrity,
    "CONTEXT_COMMIT_MISSING": ContextProjectionIntegrity,
    "CONTEXT_COMMIT_INTEGRITY": ContextProjectionIntegrity,
    "INVALID_PERCEPTION_PROPOSAL": InvalidPerceptionProposal,
    "INVALID_PERCEPTION_SCOPE": InvalidPerceptionScope,
    "SEMANTIC_IDENTITY_CONFLICT": SemanticIdentityConflict,
    "SEMANTIC_REVISION_OVERFLOW": SemanticRevisionOverflow,
    "SEMANTIC_STATE_UNCHANGED": SemanticStateUnchanged,
}

_SEMANTIC_CURSOR_SCHEMA = "astrembodiment.semantic-revision.v1"
_SEMANTIC_RESULT_SCHEMA_V1 = "astrembodiment.semantic-perception-closure.v1"
_SEMANTIC_RESULT_SCHEMA_V2 = "astrembodiment.semantic-perception-closure.v2"
_SEMANTIC_RESULT_BASE_FIELDS = frozenset(
    {
        "schema",
        "receipt",
        "semantic_vector_receipt",
        "node_observability",
        "revision",
        "deduplicated",
    }
)
_SEMANTIC_RESULT_WITH_EXPRESSION_FIELDS = frozenset(
    {*_SEMANTIC_RESULT_BASE_FIELDS, "expression_projection"}
)
_SEMANTIC_RESULT_V2_FIELDS = frozenset(
    {
        "schema",
        "availability",
        "receipt",
        "telemetry_receipt",
        "semantic_vector_receipt",
        "node_observability",
        "revision",
        "deduplicated",
        "expression_projection",
    }
)
_SEMANTIC_CLOSURE_AVAILABILITY = frozenset({"AVAILABLE", "UNAVAILABLE_LEGACY"})
_EXPRESSION_PROJECTION_SCHEMA = "astr-embodiment.expression-projection.v1"
_EXPRESSION_PROFILE_FIELDS = (
    "warmth",
    "sensitivity",
    "guardedness",
    "repair_orientation",
    "engagement",
    "epistemic_caution",
)
_SEMANTIC_VECTOR_RECEIPT_SCHEMA = "astr-embodiment.semantic-vector-receipt.v2"
_SEMANTIC_VECTOR_FORMULA = "full-vector-route-neutral-relaxation-v1"
_NODE_OBSERVABILITY_SCHEMA = "astr-embodiment.node-observability.v1"
_NODE_OBSERVABILITY_FORMULA = "spc1-node-observability-v1"
_NODE_REGION_LAYOUT = (
    ("interoception_allostasis", 2_048),
    ("affective_valuation", 2_048),
    ("salience", 1_024),
    ("epistemic_fallibility", 2_048),
    ("social_boundary", 2_048),
    ("temper_inhibitory", 1_024),
    ("world_model_imagination", 4_096),
    ("global_workspace", 1_024),
    ("action_expression", 1_024),
)
_NODE_CAPACITY = 16_384
_EDGE_CAPACITY = 524_288
_RESIDUAL_FIELDS = frozenset(
    {"authority", "continuity", "energy", "renormalization", "capacity"}
)
_NATIVE_TELEMETRY_RECEIPT_SCHEMA = "native-telemetry-receipt.v1"
_NATIVE_TELEMETRY_FORMULA = "phase0-native-propagation-fxp6-v1"
_NATIVE_TELEMETRY_FIELDS = frozenset(
    {
        "schema",
        "formula",
        "formula_digest",
        "scope_digest",
        "event_digest",
        "source_digest",
        "base_revision",
        "next_revision",
        "phase",
        "state_before",
        "state_after",
        "graph_before",
        "graph_after",
        "local_digest",
        "compensation_digest",
        "effective_digest",
        "energy",
        "capacity",
        "residuals",
        "residual_health",
        "native_gate",
        "checkpoint_digest",
        "telemetry_digest",
    }
)
_NATIVE_TELEMETRY_DIGEST_FIELDS = (
    "formula_digest",
    "scope_digest",
    "event_digest",
    "source_digest",
    "state_before",
    "state_after",
    "graph_before",
    "graph_after",
    "local_digest",
    "compensation_digest",
    "effective_digest",
    "checkpoint_digest",
    "telemetry_digest",
)
_NATIVE_TELEMETRY_ENERGY_FIELDS = (
    "reserve_before",
    "reserve_after",
    "recovered",
    "spent",
    "headroom",
    "residual",
)
_NATIVE_TELEMETRY_CAPACITY_FIELDS = (
    "upper_saturated_nodes",
    "node_limit",
    "node_headroom",
    "edge_used",
    "edge_limit",
    "edge_headroom",
    "headroom",
    "residual",
)
_FXP6_ONE = 1_000_000
_SEMANTIC_ERROR_CODES = frozenset(
    {
        "INVALID_PERCEPTION_PROPOSAL",
        "INVALID_PERCEPTION_SCOPE",
        "NATIVE_SYMBOL_UNAVAILABLE",
        "SEMANTIC_IDENTITY_CONFLICT",
        "SEMANTIC_REVISION_OVERFLOW",
        "SEMANTIC_STATE_UNCHANGED",
        "STALE_REVISION",
        "STALE_CAUSAL_BASE",
        "CLOSED_SCHEMA",
        "GENESIS_REQUIRED",
    }
)

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
_REBIRTH_CONFIRM_REQUEST_ALLOWED_KEYS = _REBIRTH_CONFIRM_REQUEST_REQUIRED_KEYS | {
    "confirmed"
}
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
    if (
        not _is_token_hex(scope["bot_token"])
        or not _is_token_hex(scope["persona_token"])
        or not _is_token_hex(scope["session_token"])
    ):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth scope token is invalid")
    relation_token = scope["relation_token"]
    if relation_token is not None and not _is_token_hex(relation_token):
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth relation token is invalid"
        )


def _validate_rebirth_prepare_request(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _REBIRTH_PREPARE_REQUEST_KEYS:
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth prepare request is not closed"
        )
    _validate_rebirth_scope(payload["scope"])
    if not _is_digest_hex(payload["expected_incarnation_id"]):
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth expected incarnation is invalid"
        )
    if not _positive_int_or_zero(payload["expected_revision"]):
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth expected revision is invalid"
        )
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
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth confirmation is not closed"
        )
    _validate_rebirth_scope(payload["scope"])
    if not _is_digest_hex(payload["expected_incarnation_id"]) or not _is_digest_hex(
        payload["request_nonce"]
    ):
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth digest is invalid")
    if not _positive_int_or_zero(payload["expected_revision"]):
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth expected revision is invalid"
        )
    if payload["action"] not in _REBIRTH_ACTIONS:
        raise ClosedSchemaViolation("CLOSED_SCHEMA", "rebirth action is invalid")
    # ``confirmed`` deliberately remains optional at this layer.  When it is
    # absent, Rust must issue REBIRTH_CONFIRMATION_REQUIRED rather than Python
    # manufacturing consent or converting the request to a schema error.
    if "confirmed" in payload and type(payload["confirmed"]) is not bool:
        raise ClosedSchemaViolation(
            "CLOSED_SCHEMA", "rebirth confirmation flag is invalid"
        )
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
    if (
        not _positive_int_or_zero(receipt["before_revision"])
        or receipt["after_revision"] != 0
    ):
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


def _semantic_degraded(code: str) -> dict[str, str]:
    return {"status": "DEGRADED", "code": code}


def _semantic_error_code(error: BaseException) -> str:
    code = getattr(error, "code", None)
    if not isinstance(code, str):
        code = str(error).partition("::")[0]
    if code in _SEMANTIC_ERROR_CODES:
        return code
    if isinstance(error, NativeCoreUnavailable):
        return "NATIVE_SYMBOL_UNAVAILABLE"
    return "NATIVE_ERROR"


def _semantic_pairs_without_duplicates(
    pairs: list[tuple[str, Any]],
) -> dict[str, Any]:
    payload: dict[str, Any] = {}
    for key, value in pairs:
        if key in payload:
            raise ValueError("semantic payload")
        payload[key] = value
    return payload


def _semantic_json(value: Any) -> dict[str, Any]:
    if type(value) is str:
        payload = json.loads(value, object_pairs_hook=_semantic_pairs_without_duplicates)
    else:
        payload = value
    if type(payload) is not dict or any(type(key) is not str for key in payload):
        raise ValueError("semantic payload")
    return payload


def _semantic_closed_json(value: dict[str, Any]) -> str:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )


def _semantic_scope_payload(scope: ScopeTokens | str | dict[str, Any]) -> dict[str, Any]:
    if type(scope) is ScopeTokens:
        payload: Any = scope.scope_json()
    elif type(scope) is str:
        payload = json.loads(scope, object_pairs_hook=_semantic_pairs_without_duplicates)
    elif type(scope) is dict:
        payload = scope
    else:
        raise ValueError("scope")
    if type(payload) is not dict or set(payload) != {
        "bot_token",
        "persona_token",
        "relation_token",
        "session_token",
    }:
        raise ValueError("scope")
    relation = payload["relation_token"]
    if relation is not None and type(relation) is not str:
        raise ValueError("scope")
    return {
        "bot_token": _canonical_nonzero_hex(payload["bot_token"], 16),
        "persona_token": _canonical_nonzero_hex(payload["persona_token"], 16),
        "relation_token": (
            _canonical_nonzero_hex(relation, 16) if relation is not None else None
        ),
        "session_token": _canonical_nonzero_hex(payload["session_token"], 16),
    }


def _validate_cursor_payload(value: Any) -> dict[str, Any]:
    payload = _semantic_json(value)
    if set(payload) != {"schema", "revision"}:
        raise ValueError("semantic cursor")
    revision = payload["revision"]
    if payload["schema"] != _SEMANTIC_CURSOR_SCHEMA or type(revision) is not int or revision < 0:
        raise ValueError("semantic cursor")
    return {"schema": _SEMANTIC_CURSOR_SCHEMA, "revision": revision}


def _validate_receipt(
    value: Any, *, revision: int, expected_base_revision: int | None
) -> tuple[dict[str, Any], bool]:
    fields = {
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
    if type(value) is not dict or set(value) != fields:
        raise ValueError("semantic receipt")
    if value["schema_version"] != 1 or type(value["schema_version"]) is not int:
        raise ValueError("semantic receipt")
    for field in ("base_revision", "next_revision", "active_nodes", "active_edges"):
        if type(value[field]) is not int or value[field] < 0:
            raise ValueError("semantic receipt")
    if value["active_nodes"] > _NODE_CAPACITY:
        raise ValueError("semantic receipt")
    digests: dict[str, str] = {}
    for field in (
        "formula_digest",
        "scope_digest",
        "event_digest",
        "authority_digest",
        "state_before",
        "state_after",
        "graph_after",
    ):
        digests[field] = _canonical_hex(value[field], 32)
    if value["status"] != "committed":
        raise ValueError("semantic receipt")
    if value["next_revision"] != revision or value["base_revision"] + 1 != revision:
        raise ValueError("semantic receipt")
    if expected_base_revision is not None and value["base_revision"] != expected_base_revision:
        raise ValueError("semantic receipt")
    residuals = value["residuals"]
    if type(residuals) is not dict or set(residuals) != _RESIDUAL_FIELDS:
        raise ValueError("semantic receipt")
    if any(type(residuals[name]) is not int for name in _RESIDUAL_FIELDS):
        raise ValueError("semantic receipt")
    return (
        {
            "schema_version": 1,
            **digests,
            "base_revision": value["base_revision"],
            "next_revision": revision,
            "active_nodes": value["active_nodes"],
            "active_edges": value["active_edges"],
            "residuals": {name: residuals[name] for name in sorted(_RESIDUAL_FIELDS)},
            "status": "committed",
        },
        digests["state_before"] != digests["state_after"],
    )


def _validate_semantic_vector_receipt(
    value: Any, *, expected_state_changed: bool
) -> dict[str, Any]:
    fields = {
        "schema",
        "formula",
        "dimension_slot_count",
        "evaluated_dimension_count",
        "injected_dimension_count",
        "nonzero_evidence_dimension_count",
        "neutral_baseline_dimension_count",
        "unavailable_dimension_count",
        "state_changed",
    }
    if type(value) is not dict or set(value) != fields:
        raise ValueError("semantic vector receipt")
    if (
        value["schema"] != _SEMANTIC_VECTOR_RECEIPT_SCHEMA
        or value["formula"] != _SEMANTIC_VECTOR_FORMULA
    ):
        raise ValueError("semantic vector receipt")
    count_fields = (
        "dimension_slot_count",
        "evaluated_dimension_count",
        "injected_dimension_count",
        "nonzero_evidence_dimension_count",
        "neutral_baseline_dimension_count",
        "unavailable_dimension_count",
    )
    if any(type(value[field]) is not int or not 0 <= value[field] <= 15 for field in count_fields):
        raise ValueError("semantic vector receipt")
    if (
        value["dimension_slot_count"] != 15
        or value["evaluated_dimension_count"] != 15
        or value["injected_dimension_count"] != 15
        or value["unavailable_dimension_count"] != 0
        or value["nonzero_evidence_dimension_count"] + value["neutral_baseline_dimension_count"] != 15
        or type(value["state_changed"]) is not bool
        or value["state_changed"] is not expected_state_changed
    ):
        raise ValueError("semantic vector receipt")
    return {
        "schema": _SEMANTIC_VECTOR_RECEIPT_SCHEMA,
        "formula": _SEMANTIC_VECTOR_FORMULA,
        **{field: value[field] for field in count_fields},
        "state_changed": expected_state_changed,
    }


def _validate_node_component(value: Any, *, capacity: int) -> dict[str, int]:
    fields = {
        "before_mean_fxp6",
        "after_mean_fxp6",
        "delta_mean_fxp6",
        "changed_node_count",
        "nonzero_after_count",
    }
    if type(value) is not dict or set(value) != fields:
        raise ValueError("node component")
    for field in ("before_mean_fxp6", "after_mean_fxp6", "delta_mean_fxp6"):
        if type(value[field]) is not int:
            raise ValueError("node component")
    for field in ("changed_node_count", "nonzero_after_count"):
        if type(value[field]) is not int or not 0 <= value[field] <= capacity:
            raise ValueError("node component")
    return {field: value[field] for field in fields}


def _validate_node_observability(
    value: Any,
    *,
    expected_revision: int,
    expected_selected_node_count: int,
    expected_state_changed: bool,
) -> dict[str, Any]:
    fields = {
        "schema",
        "formula",
        "revision",
        "field_node_capacity",
        "region_layout",
        "counts",
        "residuals",
        "regions",
    }
    if type(value) is not dict or set(value) != fields:
        raise ValueError("node observability")
    if (
        value["schema"] != _NODE_OBSERVABILITY_SCHEMA
        or value["formula"] != _NODE_OBSERVABILITY_FORMULA
        or value["revision"] != expected_revision
        or value["field_node_capacity"] != _NODE_CAPACITY
        or value["region_layout"] != "regions-v1"
    ):
        raise ValueError("node observability")
    counts = value["counts"]
    count_fields = {
        "selected_node_count",
        "activated_node_count",
        "changed_node_count",
        "potential_nonzero_after_count",
        "excitation_nonzero_after_count",
        "signal_nonzero_after_count",
    }
    if type(counts) is not dict or set(counts) != count_fields:
        raise ValueError("node observability")
    if any(type(counts[field]) is not int or not 0 <= counts[field] <= _NODE_CAPACITY for field in count_fields):
        raise ValueError("node observability")
    if (
        counts["selected_node_count"] != expected_selected_node_count
        or counts["changed_node_count"] > counts["activated_node_count"]
        or counts["activated_node_count"] > counts["selected_node_count"]
        or counts["signal_nonzero_after_count"] < max(
            counts["potential_nonzero_after_count"], counts["excitation_nonzero_after_count"]
        )
        or counts["signal_nonzero_after_count"] > counts["potential_nonzero_after_count"] + counts["excitation_nonzero_after_count"]
        or (expected_state_changed and counts["changed_node_count"] == 0)
        or (not expected_state_changed and counts["changed_node_count"] != 0)
    ):
        raise ValueError("node observability")
    if value["residuals"] != {
        "state": "NOT_COMPUTED",
        "formula": None,
        "values_fxp6": None,
    }:
        raise ValueError("node observability")
    regions = value["regions"]
    if type(regions) is not list or len(regions) != len(_NODE_REGION_LAYOUT):
        raise ValueError("node observability")
    canonical_regions: list[dict[str, Any]] = []
    totals = {"selected": 0, "activated": 0, "changed": 0, "potential": 0, "excitation": 0}
    region_fields = {
        "region_id",
        "region_name",
        "node_capacity",
        "selected_node_count",
        "activated_node_count",
        "changed_node_count",
        "potential",
        "excitation",
    }
    for region_id, (region_name, capacity) in enumerate(_NODE_REGION_LAYOUT):
        region = regions[region_id]
        if type(region) is not dict or set(region) != region_fields:
            raise ValueError("node observability")
        if (
            region["region_id"] != region_id
            or region["region_name"] != region_name
            or region["node_capacity"] != capacity
        ):
            raise ValueError("node observability")
        for field in ("selected_node_count", "activated_node_count", "changed_node_count"):
            if type(region[field]) is not int or not 0 <= region[field] <= capacity:
                raise ValueError("node observability")
        if region["changed_node_count"] > region["activated_node_count"] or region["activated_node_count"] > region["selected_node_count"]:
            raise ValueError("node observability")
        potential = _validate_node_component(region["potential"], capacity=capacity)
        excitation = _validate_node_component(region["excitation"], capacity=capacity)
        if (
            region["changed_node_count"] < max(potential["changed_node_count"], excitation["changed_node_count"])
            or region["changed_node_count"] > potential["changed_node_count"] + excitation["changed_node_count"]
        ):
            raise ValueError("node observability")
        totals["selected"] += region["selected_node_count"]
        totals["activated"] += region["activated_node_count"]
        totals["changed"] += region["changed_node_count"]
        totals["potential"] += potential["nonzero_after_count"]
        totals["excitation"] += excitation["nonzero_after_count"]
        canonical_regions.append(
            {
                "region_id": region_id,
                "region_name": region_name,
                "node_capacity": capacity,
                "selected_node_count": region["selected_node_count"],
                "activated_node_count": region["activated_node_count"],
                "changed_node_count": region["changed_node_count"],
                "potential": potential,
                "excitation": excitation,
            }
        )
    if (
        totals["selected"] != counts["selected_node_count"]
        or totals["activated"] != counts["activated_node_count"]
        or totals["changed"] != counts["changed_node_count"]
        or totals["potential"] != counts["potential_nonzero_after_count"]
        or totals["excitation"] != counts["excitation_nonzero_after_count"]
    ):
        raise ValueError("node observability")
    return {
        "schema": _NODE_OBSERVABILITY_SCHEMA,
        "formula": _NODE_OBSERVABILITY_FORMULA,
        "revision": expected_revision,
        "field_node_capacity": _NODE_CAPACITY,
        "region_layout": "regions-v1",
        "counts": {field: counts[field] for field in count_fields},
        "residuals": {"state": "NOT_COMPUTED", "formula": None, "values_fxp6": None},
        "regions": canonical_regions,
    }


def _validate_expression_projection(value: Any, *, expected_revision: int) -> dict[str, Any]:
    if type(value) is not dict or set(value) != {"schema", "revision", "profile_fxp6"}:
        raise ValueError("expression projection")
    if value["schema"] != _EXPRESSION_PROJECTION_SCHEMA or value["revision"] != expected_revision:
        raise ValueError("expression projection")
    profile = value["profile_fxp6"]
    if type(profile) is not dict or set(profile) != set(_EXPRESSION_PROFILE_FIELDS):
        raise ValueError("expression projection")
    if any(type(profile[name]) is not int or not 0 <= profile[name] <= 1_000_000 for name in _EXPRESSION_PROFILE_FIELDS):
        raise ValueError("expression projection")
    return {
        "schema": _EXPRESSION_PROJECTION_SCHEMA,
        "revision": expected_revision,
        "profile_fxp6": {name: profile[name] for name in _EXPRESSION_PROFILE_FIELDS},
    }


def _validate_native_telemetry_receipt(
    value: Any, *, receipt: dict[str, Any], revision: int
) -> dict[str, Any]:
    if type(value) is not dict or set(value) != _NATIVE_TELEMETRY_FIELDS:
        raise ValueError("native telemetry receipt")
    if (
        value["schema"] != _NATIVE_TELEMETRY_RECEIPT_SCHEMA
        or value["formula"] != _NATIVE_TELEMETRY_FORMULA
        or value["phase"] != "PREPARE"
    ):
        raise ValueError("native telemetry receipt")
    digests = {
        field: _canonical_hex(value[field], 32)
        for field in _NATIVE_TELEMETRY_DIGEST_FIELDS
    }
    for field in ("base_revision", "next_revision"):
        if type(value[field]) is not int or value[field] < 0:
            raise ValueError("native telemetry receipt")
    if (
        value["base_revision"] != receipt["base_revision"]
        or value["next_revision"] != revision
        or value["next_revision"] != receipt["next_revision"]
        or any(
            digests[field] != receipt[field]
            for field in (
                "formula_digest",
                "scope_digest",
                "event_digest",
                "state_before",
                "state_after",
                "graph_after",
            )
        )
    ):
        raise ValueError("native telemetry receipt")
    energy = value["energy"]
    if (
        type(energy) is not dict
        or set(energy) != set(_NATIVE_TELEMETRY_ENERGY_FIELDS)
    ):
        raise ValueError("native telemetry receipt")
    if any(
        type(energy[field]) is not int or not 0 <= energy[field] <= _FXP6_ONE
        for field in _NATIVE_TELEMETRY_ENERGY_FIELDS
    ):
        raise ValueError("native telemetry receipt")
    capacity = value["capacity"]
    if (
        type(capacity) is not dict
        or set(capacity) != set(_NATIVE_TELEMETRY_CAPACITY_FIELDS)
    ):
        raise ValueError("native telemetry receipt")
    if any(
        type(capacity[field]) is not int or capacity[field] < 0
        for field in _NATIVE_TELEMETRY_CAPACITY_FIELDS
    ):
        raise ValueError("native telemetry receipt")
    if (
        capacity["node_limit"] != _NODE_CAPACITY
        or capacity["edge_limit"] != _EDGE_CAPACITY
        or capacity["upper_saturated_nodes"] > capacity["node_limit"]
        or capacity["edge_used"] > capacity["edge_limit"]
        or any(
            capacity[field] > _FXP6_ONE
            for field in (
                "node_headroom",
                "edge_headroom",
                "headroom",
                "residual",
            )
        )
    ):
        raise ValueError("native telemetry receipt")

    def headroom(*, used: int, limit: int) -> int:
        return _FXP6_ONE - ((used * _FXP6_ONE + limit // 2) // limit)

    if (
        capacity["node_headroom"]
        != headroom(
            used=capacity["upper_saturated_nodes"], limit=capacity["node_limit"]
        )
        or capacity["edge_headroom"]
        != headroom(used=capacity["edge_used"], limit=capacity["edge_limit"])
        or capacity["headroom"]
        != min(capacity["node_headroom"], capacity["edge_headroom"])
    ):
        raise ValueError("native telemetry receipt")
    residuals = value["residuals"]
    if type(residuals) is not dict or set(residuals) != _RESIDUAL_FIELDS:
        raise ValueError("native telemetry receipt")
    if any(
        type(residuals[field]) is not int
        or not 0 <= residuals[field] <= _FXP6_ONE
        for field in _RESIDUAL_FIELDS
    ):
        raise ValueError("native telemetry receipt")
    if any(
        type(value[field]) is not int or not 0 <= value[field] <= _FXP6_ONE
        for field in ("residual_health", "native_gate")
    ):
        raise ValueError("native telemetry receipt")
    if (
        residuals != receipt["residuals"]
        or energy["headroom"] != energy["reserve_after"]
        or energy["residual"] != residuals["energy"]
        or capacity["residual"] != residuals["capacity"]
        or capacity["residual"] != 0
        or value["residual_health"] != _FXP6_ONE - max(residuals.values())
        or value["native_gate"]
        != min(energy["headroom"], capacity["headroom"], value["residual_health"])
    ):
        raise ValueError("native telemetry receipt")
    return {
        "schema": _NATIVE_TELEMETRY_RECEIPT_SCHEMA,
        "formula": _NATIVE_TELEMETRY_FORMULA,
        **{field: digests[field] for field in _NATIVE_TELEMETRY_DIGEST_FIELDS},
        "base_revision": value["base_revision"],
        "next_revision": revision,
        "phase": "PREPARE",
        "energy": {
            field: energy[field] for field in _NATIVE_TELEMETRY_ENERGY_FIELDS
        },
        "capacity": {
            field: capacity[field] for field in _NATIVE_TELEMETRY_CAPACITY_FIELDS
        },
        "residuals": {field: residuals[field] for field in sorted(_RESIDUAL_FIELDS)},
        "residual_health": value["residual_health"],
        "native_gate": value["native_gate"],
    }


def _validate_semantic_result(
    value: Any, *, expected_base_revision: int | None = None
) -> dict[str, Any]:
    payload = _semantic_json(value)
    schema = payload.get("schema")
    availability: str | None = None
    if schema == _SEMANTIC_RESULT_SCHEMA_V1:
        if set(payload) not in {
            _SEMANTIC_RESULT_BASE_FIELDS,
            _SEMANTIC_RESULT_WITH_EXPRESSION_FIELDS,
        }:
            raise ValueError("semantic result")
    elif schema == _SEMANTIC_RESULT_SCHEMA_V2:
        if set(payload) != _SEMANTIC_RESULT_V2_FIELDS:
            raise ValueError("semantic result")
        availability = payload["availability"]
        if (
            type(availability) is not str
            or availability not in _SEMANTIC_CLOSURE_AVAILABILITY
        ):
            raise ValueError("semantic result")
    else:
        raise ValueError("semantic result")
    revision = payload["revision"]
    if type(revision) is not int or revision < 0:
        raise ValueError("semantic result")
    if type(payload["deduplicated"]) is not bool:
        raise ValueError("semantic result")
    receipt, state_changed = _validate_receipt(
        payload["receipt"],
        revision=revision,
        expected_base_revision=expected_base_revision,
    )
    if availability == "UNAVAILABLE_LEGACY":
        if any(
            payload[field] is not None
            for field in (
                "telemetry_receipt",
                "semantic_vector_receipt",
                "node_observability",
                "expression_projection",
            )
        ):
            raise ValueError("semantic result")
        return {
            "schema": _SEMANTIC_RESULT_SCHEMA_V2,
            "availability": "UNAVAILABLE_LEGACY",
            "receipt": receipt,
            "telemetry_receipt": None,
            "semantic_vector_receipt": None,
            "node_observability": None,
            "full_vector_state": "UNAVAILABLE_LEGACY",
            "node_observability_state": "UNAVAILABLE_LEGACY",
            "revision": revision,
            "deduplicated": payload["deduplicated"],
            "expression_projection": None,
        }
    vector = _validate_semantic_vector_receipt(
        payload["semantic_vector_receipt"], expected_state_changed=state_changed
    )
    nodes = _validate_node_observability(
        payload["node_observability"],
        expected_revision=revision,
        expected_selected_node_count=receipt["active_nodes"],
        expected_state_changed=state_changed,
    )
    expression = None
    if "expression_projection" in payload and payload["expression_projection"] is not None:
        expression = _validate_expression_projection(
            payload["expression_projection"], expected_revision=revision
        )
    if schema == _SEMANTIC_RESULT_SCHEMA_V2 and expression is None:
        raise ValueError("semantic result")
    result = {
        "schema": schema,
        "receipt": receipt,
        "semantic_vector_receipt": vector,
        "node_observability": nodes,
        "full_vector_state": "FULL_VECTOR_CONFIRMED",
        "node_observability_state": "CONFIRMED",
        "revision": revision,
        "deduplicated": payload["deduplicated"],
        "expression_projection": expression,
    }
    if schema == _SEMANTIC_RESULT_SCHEMA_V2:
        result["availability"] = "AVAILABLE"
        result["telemetry_receipt"] = _validate_native_telemetry_receipt(
            payload["telemetry_receipt"], receipt=receipt, revision=revision
        )
    return result


def validate_semantic_result(
    value: Any, *, expected_base_revision: int | None = None
) -> dict[str, Any]:
    return _validate_semantic_result(value, expected_base_revision=expected_base_revision)


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

    def semantic_revision_v1(
        self, scope_json: ScopeTokens | str | dict[str, Any]
    ) -> dict[str, Any]:
        """Read the independent semantic cursor through a closed failure ABI."""

        try:
            scope = _semantic_scope_payload(scope_json)
        except BaseException:
            return _semantic_degraded("INVALID_PERCEPTION_SCOPE")
        try:
            native = self._require()
            method = getattr(native, "semantic_revision_v1", None)
            if not callable(method):
                return _semantic_degraded("NATIVE_SYMBOL_UNAVAILABLE")
            return _validate_cursor_payload(method(_semantic_closed_json(scope)))
        except BaseException as exc:
            if isinstance(exc, (TypeError, ValueError, json.JSONDecodeError)):
                return _semantic_degraded("NATIVE_MALFORMED")
            return _semantic_degraded(_semantic_error_code(exc))

    def apply_perception_proposal_v1(
        self,
        scope_json: ScopeTokens | str | dict[str, Any],
        proposal_json: str | dict[str, Any],
    ) -> dict[str, Any]:
        """Submit one V3-derived, closed 15D proposal to native exactly once."""

        try:
            scope = _semantic_scope_payload(scope_json)
        except BaseException:
            return _semantic_degraded("INVALID_PERCEPTION_SCOPE")
        try:
            proposal = validate_perception_proposal(proposal_json, scope=scope)
            encoded_proposal = proposal_to_json(proposal, scope=scope)
        except (SemanticProposalError, TypeError, ValueError, json.JSONDecodeError):
            return _semantic_degraded("INVALID_PERCEPTION_PROPOSAL")
        except BaseException:
            return _semantic_degraded("INVALID_PERCEPTION_PROPOSAL")
        try:
            native = self._require()
            method = getattr(native, "apply_perception_proposal_v1", None)
            if not callable(method):
                return _semantic_degraded("NATIVE_SYMBOL_UNAVAILABLE")
            return _validate_semantic_result(
                method(_semantic_closed_json(scope), encoded_proposal),
                expected_base_revision=proposal["base_revision"],
            )
        except BaseException as exc:
            if isinstance(exc, (TypeError, ValueError, json.JSONDecodeError)):
                return _semantic_degraded("NATIVE_MALFORMED")
            return _semantic_degraded(_semantic_error_code(exc))

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
