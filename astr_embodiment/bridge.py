"""Coarse-grained bridge to the Rust native runtime.

Python may freeze persona data, build closed JSON envelopes and call exactly
the coarse surface below. It can never read or write neural state, residuals
or identity directly; every mutation goes through the Rust single writer.
"""

from __future__ import annotations

import json
import platform
import re
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
INVALID_NEURAL_STATE_SUBCODES = frozenset(
    {
        "BASELINE_STATE_INVALID",
        "FIELD_STATE_INVALID",
        "GRAPH_STATE_INVALID",
        "DYNAMICS_INVALID",
        "SEMANTIC_CLOSURE_INVALID",
        "SNAPSHOT_WIRE_INVALID",
        "SNAPSHOT_ATTESTATION_MISMATCH",
        "AESEM3_RETIRED_COMPENSATION_NONZERO",
        "RELATION_SCOPE_MISSING",
        "UNKNOWN_INVALID_NEURAL_STATE",
    }
)
_UNKNOWN_INVALID_NEURAL_STATE = "UNKNOWN_INVALID_NEURAL_STATE"
_INVALID_NEURAL_STATE_MESSAGE = re.compile(r"\AINVALID_NEURAL_STATE::([A-Z0-9_]+)\Z")
_STATE_SUBCODE_MISSING = object()
FIELD_MIGRATION_SUBCODES = frozenset(
    {
        "FIELD_MIGRATION_APPLIED",
        "FIELD_MIGRATION_REPLAYED",
        "FIELD_MIGRATION_REFUSED_SOURCE",
        "FIELD_MIGRATION_REFUSED_STRUCTURE",
        "FIELD_MIGRATION_REFUSED_RANGE",
        "FIELD_MIGRATION_TRANSFORM_INVALID",
        "FIELD_MIGRATION_CONCURRENT_STALE",
        "FIELD_MIGRATION_BACKUP_FAILED",
        "FIELD_MIGRATION_STORAGE_FAILED",
        "FIELD_MIGRATION_UNKNOWN",
    }
)
_FIELD_MIGRATION_UNKNOWN = "FIELD_MIGRATION_UNKNOWN"
_MIGRATION_SUBCODE_MISSING = object()


class _InvalidSemanticMigrationSubcode(ValueError):
    """Raised for a non-null migration telemetry value outside the frozen enum."""


def normalize_invalid_neural_state_subcode(value: object) -> str:
    if type(value) is str and value in INVALID_NEURAL_STATE_SUBCODES:
        return value
    return _UNKNOWN_INVALID_NEURAL_STATE


def normalize_field_migration_subcode(value: object) -> str:
    """Reduce native migration telemetry to its frozen, privacy-safe enum."""

    if type(value) is str and value in FIELD_MIGRATION_SUBCODES:
        return value
    return _FIELD_MIGRATION_UNKNOWN


def _migration_subcode(error: BaseException) -> str:
    return normalize_field_migration_subcode(
        getattr(error, "migration_subcode", _MIGRATION_SUBCODE_MISSING)
    )


def _invalid_neural_state_subcode(error: BaseException) -> str:
    state_subcode = getattr(error, "state_subcode", _STATE_SUBCODE_MISSING)
    if state_subcode is not _STATE_SUBCODE_MISSING:
        if getattr(error, "code", None) != "INVALID_NEURAL_STATE":
            return _UNKNOWN_INVALID_NEURAL_STATE
        return normalize_invalid_neural_state_subcode(state_subcode)
    match = _INVALID_NEURAL_STATE_MESSAGE.fullmatch(str(error))
    if match is None:
        return _UNKNOWN_INVALID_NEURAL_STATE
    return normalize_invalid_neural_state_subcode(match.group(1))


class NativeCoreUnavailable(RuntimeError):
    """Raised when the bundled platform wheel cannot be imported."""


class NativeCoreError(RuntimeError):
    """Raised by the native core with a stable machine-readable code."""

    def __init__(self, code: str, detail: str) -> None:
        super().__init__(f"{code}::{detail}")
        self.code = code
        self.detail = detail


class InvalidNeuralState(NativeCoreError):
    """A closed native-state rejection that never retains raw exception detail."""

    def __init__(self, state_subcode: object) -> None:
        self.state_subcode = normalize_invalid_neural_state_subcode(state_subcode)
        super().__init__("INVALID_NEURAL_STATE", self.state_subcode)


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


class SeedConfigLifecycleFailure(NativeCoreError):
    """Closed failure emitted by the dedicated native seed-config lifecycle."""


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
    "SEED_CONFIG_SCHEMA_INVALID": SeedConfigLifecycleFailure,
    "SEED_CONFIG_OBSERVATION_UNCERTAIN": SeedConfigLifecycleFailure,
    "SEED_CONFIG_MIRROR_STALE": SeedConfigLifecycleFailure,
    "SEED_CLEAR_FENCE_STALE": SeedConfigLifecycleFailure,
    "SEED_CLEAR_IN_FLIGHT": SeedConfigLifecycleFailure,
    "SEED_CLEAR_STORAGE_FAILED": SeedConfigLifecycleFailure,
    "SEED_CLEAR_LOCATOR_INVALID": SeedConfigLifecycleFailure,
    "SEED_CLEAR_UNKNOWN": SeedConfigLifecycleFailure,
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
        "migration_subcode",
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
_NODE_OBSERVABILITY_SCHEMA = "astr-embodiment.node-observability.v2"
_NODE_OBSERVABILITY_CONTRACT_INFO_SCHEMA = (
    "astr-embodiment.node-observability-contract-info.v1"
)
_NODE_OBSERVABILITY_CONTRACT_IDS = frozenset(
    {"astr-embodiment.node-observability-contract.v2"}
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
SEMANTIC_NATIVE_ERROR_CODES = frozenset(
    {
        "CLOSED",
        "CLOSED_SCHEMA",
        "CONTEXT_COMMIT_INTEGRITY",
        "CONTEXT_COMMIT_MISSING",
        "CONTEXT_PROJECTION",
        "CONTEXT_RECEIPT_INVALID",
        "DUPLICATE_EVENT",
        "ENCODING",
        "GENESIS_MANIFEST_MISMATCH",
        "GENESIS_REQUIRED",
        "GENESIS_UNAVAILABLE",
        "IDENTITY_MISMATCH",
        "INVALID_NEURAL_STATE",
        "INVALID_PERCEPTION_PROPOSAL",
        "INVALID_PERCEPTION_SCOPE",
        "LEASE_CONFLICT",
        "LEASE_IN_FLIGHT",
        "LEGACY_UNATTESTED",
        "NATIVE_ERROR",
        "POISONED",
        "RETRY_WAIT",
        "SEED_DIGEST_COLLISION",
        "SEMANTIC_IDENTITY_CONFLICT",
        "SEMANTIC_REVISION_OVERFLOW",
        "STALE_CAUSAL_BASE",
        "STALE_REVISION",
        "STORAGE",
        "UNSUPPORTED_EVENT",
    }
)
SEMANTIC_NATIVE_FAILURE_STAGES = frozenset({"CURSOR", "NATIVE_APPLY"})
_SEMANTIC_ERROR_CODES = (
    frozenset(
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
    | SEMANTIC_NATIVE_ERROR_CODES
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


_SEED_CONFIG_OBSERVATION_SCHEMA = "astrembodiment.seed-config-observation.v1"
_SEED_CONFIG_RESULT_SCHEMA = "astrembodiment.seed-config-result.v1"
_SEED_CONFIG_ACK_SCHEMA = "astrembodiment.seed-config-ack.v1"
_SEED_CONFIG_WRITEBACK_ACK_SCHEMA = "astrembodiment.seed-config-writeback-ack.v1"
_SEED_CONFIG_SCOPE_KEYS = frozenset({"bot_token", "persona_token", "relation_token"})
_SEED_CONFIG_RECONCILE_REQUIRED_KEYS = frozenset(
    {
        "schema",
        "scope",
        "observation",
        "origin",
        "previous_observation",
        "package_epoch",
        "config_schema_version",
        "host_config_revision",
    }
)
_SEED_CONFIG_RECONCILE_OPTIONAL_KEYS = frozenset({"seed_code", "mirror_guard"})
_SEED_CONFIG_RESULT_KEYS = frozenset(
    {"schema", "state", "writeback", "before_revision", "after_revision", "reason"}
)
_SEED_CONFIG_WRITEBACK_KEYS = frozenset(
    {"seed_code", "mirror_guard", "writeback_token"}
)
_SEED_CONFIG_ACK_REQUEST_KEYS = frozenset(
    {"schema", "scope", "writeback_token", "write_succeeded", "host_config_revision"}
)
_SEED_CONFIG_ACK_RESULT_KEYS = frozenset({"schema", "state"})
_SEED_CONFIG_OBSERVATIONS = frozenset(
    {"PRESENT_NONEMPTY", "PRESENT_EMPTY", "MISSING", "READ_FAILED"}
)
_SEED_CONFIG_ORIGINS = frozenset(
    {
        "USER_SAVE_EVENT",
        "STARTUP_READ",
        "PLUGIN_WRITEBACK",
        "LEGACY_CONFIG_MIGRATION",
    }
)
_SEED_CONFIG_RESULT_STATES = frozenset(
    {
        "UNCHANGED",
        "WRITE_MIRROR",
        "DEFERRED",
        "REBIRTH_COMMITTED",
        "REBIRTH_REPLAYED",
    }
)
_SEED_CONFIG_REASONS = frozenset(
    {
        "SEED_CONFIG_NATIVE_MATCH",
        "SEED_CONFIG_REPAIR_REQUIRED",
        "SEED_CONFIG_OBSERVATION_DEFERRED",
        "SEED_CLEAR_REBIRTH_COMMITTED",
        "SEED_CLEAR_REBIRTH_REPLAYED",
    }
)


def _seed_config_schema_error(detail: str) -> SeedConfigLifecycleFailure:
    return SeedConfigLifecycleFailure("SEED_CONFIG_SCHEMA_INVALID", detail)


def _seed_config_response_error(detail: str) -> SeedConfigLifecycleFailure:
    return SeedConfigLifecycleFailure("SEED_CLEAR_UNKNOWN", detail)


def _is_seed_config_capability(value: Any) -> bool:
    return _is_digest_hex(value) and value == value.lower()


def _validate_seed_config_scope(scope: Any) -> None:
    if not isinstance(scope, dict) or set(scope) != _SEED_CONFIG_SCOPE_KEYS:
        raise _seed_config_schema_error("seed config scope is not closed")
    if not _is_token_hex(scope["bot_token"]) or not _is_token_hex(
        scope["persona_token"]
    ):
        raise _seed_config_schema_error("seed config scope token is invalid")
    relation_token = scope["relation_token"]
    if relation_token is not None and not _is_token_hex(relation_token):
        raise _seed_config_schema_error("seed config relation token is invalid")


def _validate_seed_config_reconcile_request(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict):
        raise _seed_config_schema_error("seed config request is invalid")
    keys = set(payload)
    if not _SEED_CONFIG_RECONCILE_REQUIRED_KEYS <= keys or not keys <= (
        _SEED_CONFIG_RECONCILE_REQUIRED_KEYS | _SEED_CONFIG_RECONCILE_OPTIONAL_KEYS
    ):
        raise _seed_config_schema_error("seed config request is not closed")
    if payload["schema"] != _SEED_CONFIG_OBSERVATION_SCHEMA:
        raise _seed_config_schema_error("seed config schema is invalid")
    _validate_seed_config_scope(payload["scope"])
    observation = payload["observation"]
    origin = payload["origin"]
    if (
        observation not in _SEED_CONFIG_OBSERVATIONS
        or origin not in _SEED_CONFIG_ORIGINS
    ):
        raise _seed_config_schema_error("seed config enum is invalid")
    previous = payload["previous_observation"]
    if previous is not None and previous != "PRESENT_NONEMPTY":
        raise _seed_config_schema_error("seed config previous observation is invalid")
    if observation == "PRESENT_NONEMPTY":
        seed_code = payload.get("seed_code")
        if type(seed_code) is not str or not seed_code or len(seed_code) > 256:
            raise _seed_config_schema_error("seed config seed code is invalid")
    elif "seed_code" in payload:
        raise _seed_config_schema_error("seed config empty observation carries seed")
    if "mirror_guard" in payload and not _is_seed_config_capability(
        payload["mirror_guard"]
    ):
        raise _seed_config_schema_error("seed config mirror guard is invalid")
    package_epoch = payload["package_epoch"]
    if (
        type(package_epoch) is not str
        or not package_epoch
        or len(package_epoch) > 128
        or not all(
            char.isascii() and (char.isalnum() or char in "._-")
            for char in package_epoch
        )
    ):
        raise _seed_config_schema_error("seed config package epoch is invalid")
    if payload["config_schema_version"] != 1:
        raise _seed_config_schema_error("seed config schema version is invalid")
    if not _positive_int_or_zero(payload["host_config_revision"]):
        raise _seed_config_schema_error("seed config revision is invalid")
    return dict(payload)


def _validate_seed_config_writeback(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _SEED_CONFIG_WRITEBACK_KEYS:
        raise _seed_config_response_error("seed config writeback is not closed")
    if (
        type(payload["seed_code"]) is not str
        or not payload["seed_code"]
        or len(payload["seed_code"]) > 256
        or not _is_seed_config_capability(payload["mirror_guard"])
        or not _is_seed_config_capability(payload["writeback_token"])
    ):
        raise _seed_config_response_error("seed config writeback is invalid")
    return dict(payload)


def _validate_seed_config_result(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _SEED_CONFIG_RESULT_KEYS:
        raise _seed_config_response_error("seed config result is not closed")
    if payload["schema"] != _SEED_CONFIG_RESULT_SCHEMA:
        raise _seed_config_response_error("seed config result schema is invalid")
    state = payload["state"]
    if (
        state not in _SEED_CONFIG_RESULT_STATES
        or payload["reason"] not in _SEED_CONFIG_REASONS
    ):
        raise _seed_config_response_error("seed config result enum is invalid")
    writeback = payload["writeback"]
    if state in {"WRITE_MIRROR", "REBIRTH_COMMITTED", "REBIRTH_REPLAYED"}:
        payload = dict(payload)
        payload["writeback"] = _validate_seed_config_writeback(writeback)
    elif writeback is not None:
        raise _seed_config_response_error("seed config unexpected writeback")
    if state in {"REBIRTH_COMMITTED", "REBIRTH_REPLAYED"}:
        if (
            not _positive_int_or_zero(payload["before_revision"])
            or payload["after_revision"] != 0
        ):
            raise _seed_config_response_error("seed config rebirth revision is invalid")
    elif (
        payload["before_revision"] is not None or payload["after_revision"] is not None
    ):
        raise _seed_config_response_error("seed config nonrebirth revision is invalid")
    return dict(payload)


def _validate_seed_config_ack_request(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _SEED_CONFIG_ACK_REQUEST_KEYS:
        raise _seed_config_schema_error("seed config acknowledgement is not closed")
    if payload["schema"] != _SEED_CONFIG_WRITEBACK_ACK_SCHEMA:
        raise _seed_config_schema_error("seed config acknowledgement schema is invalid")
    _validate_seed_config_scope(payload["scope"])
    if (
        payload["write_succeeded"] is not True
        or not _is_seed_config_capability(payload["writeback_token"])
        or not _positive_int_or_zero(payload["host_config_revision"])
    ):
        raise _seed_config_schema_error("seed config acknowledgement is invalid")
    return dict(payload)


def _validate_seed_config_ack_result(payload: Any) -> dict[str, Any]:
    if not isinstance(payload, dict) or set(payload) != _SEED_CONFIG_ACK_RESULT_KEYS:
        raise _seed_config_response_error(
            "seed config acknowledgement result is not closed"
        )
    if payload["schema"] != _SEED_CONFIG_ACK_SCHEMA or payload["state"] not in {
        "MIRROR_ACTIVE",
        "REPLAYED",
        "STALE",
    }:
        raise _seed_config_response_error(
            "seed config acknowledgement result is invalid"
        )
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


def _semantic_degraded(
    code: str,
    *,
    state_subcode: object = _STATE_SUBCODE_MISSING,
    migration_subcode: object = _MIGRATION_SUBCODE_MISSING,
) -> dict[str, Any]:
    result = {"status": "DEGRADED", "code": code}
    if code == "INVALID_NEURAL_STATE" and state_subcode is not _STATE_SUBCODE_MISSING:
        result["state_subcode"] = normalize_invalid_neural_state_subcode(state_subcode)
    if migration_subcode is not _MIGRATION_SUBCODE_MISSING:
        result["migration_subcode"] = normalize_field_migration_subcode(
            migration_subcode
        )
    return result


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
        payload = json.loads(
            value, object_pairs_hook=_semantic_pairs_without_duplicates
        )
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


def _semantic_scope_payload(
    scope: ScopeTokens | str | dict[str, Any],
) -> dict[str, Any]:
    if type(scope) is ScopeTokens:
        payload: Any = scope.scope_json()
    elif type(scope) is str:
        payload = json.loads(
            scope, object_pairs_hook=_semantic_pairs_without_duplicates
        )
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
    if (
        payload["schema"] != _SEMANTIC_CURSOR_SCHEMA
        or type(revision) is not int
        or revision < 0
    ):
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
    if (
        expected_base_revision is not None
        and value["base_revision"] != expected_base_revision
    ):
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
    if any(
        type(value[field]) is not int or not 0 <= value[field] <= 15
        for field in count_fields
    ):
        raise ValueError("semantic vector receipt")
    if (
        value["dimension_slot_count"] != 15
        or value["evaluated_dimension_count"] != 15
        or value["injected_dimension_count"] != 15
        or value["unavailable_dimension_count"] != 0
        or value["nonzero_evidence_dimension_count"]
        + value["neutral_baseline_dimension_count"]
        != 15
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


def _validate_node_observability_contract_info(value: Any) -> str:
    payload = _semantic_json(value)
    fields = {"schema", "contract_id", "node_observability_schema"}
    if type(payload) is not dict or set(payload) != fields:
        raise ValueError("node observability contract info")
    if (
        payload["schema"] != _NODE_OBSERVABILITY_CONTRACT_INFO_SCHEMA
        or type(payload["contract_id"]) is not str
        or payload["contract_id"] not in _NODE_OBSERVABILITY_CONTRACT_IDS
        or payload["node_observability_schema"] != _NODE_OBSERVABILITY_SCHEMA
    ):
        raise ValueError("node observability contract info")
    return payload["contract_id"]


def _validate_node_observability(
    value: Any,
) -> dict[str, Any]:
    fields = {
        "schema",
        "contract_id",
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
        or type(value["contract_id"]) is not str
        or value["contract_id"] not in _NODE_OBSERVABILITY_CONTRACT_IDS
        or type(value["formula"]) is not str
        or type(value["revision"]) is not int
        or value["revision"] < 0
        or type(value["field_node_capacity"]) is not int
        or value["field_node_capacity"] < 0
        or type(value["region_layout"]) is not str
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
    if any(
        type(counts[field]) is not int
        or not 0 <= counts[field] <= value["field_node_capacity"]
        for field in count_fields
    ):
        raise ValueError("node observability")
    if value["residuals"] != {
        "state": "NOT_COMPUTED",
        "formula": None,
        "values_fxp6": None,
    }:
        raise ValueError("node observability")
    regions = value["regions"]
    if type(regions) is not list:
        raise ValueError("node observability")
    canonical_regions: list[dict[str, Any]] = []
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
    for region in regions:
        if type(region) is not dict or set(region) != region_fields:
            raise ValueError("node observability")
        if (
            type(region["region_id"]) is not int
            or not 0 <= region["region_id"] <= 255
            or type(region["region_name"]) is not str
            or type(region["node_capacity"]) is not int
            or region["node_capacity"] < 0
        ):
            raise ValueError("node observability")
        capacity = region["node_capacity"]
        for field in (
            "selected_node_count",
            "activated_node_count",
            "changed_node_count",
        ):
            if type(region[field]) is not int or not 0 <= region[field] <= capacity:
                raise ValueError("node observability")
        potential = _validate_node_component(region["potential"], capacity=capacity)
        excitation = _validate_node_component(region["excitation"], capacity=capacity)
        canonical_regions.append(
            {
                "region_id": region["region_id"],
                "region_name": region["region_name"],
                "node_capacity": capacity,
                "selected_node_count": region["selected_node_count"],
                "activated_node_count": region["activated_node_count"],
                "changed_node_count": region["changed_node_count"],
                "potential": potential,
                "excitation": excitation,
            }
        )
    return {
        "schema": _NODE_OBSERVABILITY_SCHEMA,
        "contract_id": value["contract_id"],
        "formula": value["formula"],
        "revision": value["revision"],
        "field_node_capacity": value["field_node_capacity"],
        "region_layout": value["region_layout"],
        "counts": {field: counts[field] for field in count_fields},
        "residuals": {"state": "NOT_COMPUTED", "formula": None, "values_fxp6": None},
        "regions": canonical_regions,
    }


def _validate_expression_projection(
    value: Any, *, expected_revision: int
) -> dict[str, Any]:
    if type(value) is not dict or set(value) != {"schema", "revision", "profile_fxp6"}:
        raise ValueError("expression projection")
    if (
        value["schema"] != _EXPRESSION_PROJECTION_SCHEMA
        or value["revision"] != expected_revision
    ):
        raise ValueError("expression projection")
    profile = value["profile_fxp6"]
    if type(profile) is not dict or set(profile) != set(_EXPRESSION_PROFILE_FIELDS):
        raise ValueError("expression projection")
    if any(
        type(profile[name]) is not int or not 0 <= profile[name] <= 1_000_000
        for name in _EXPRESSION_PROFILE_FIELDS
    ):
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
    if type(energy) is not dict or set(energy) != set(_NATIVE_TELEMETRY_ENERGY_FIELDS):
        raise ValueError("native telemetry receipt")
    if any(
        type(energy[field]) is not int or not 0 <= energy[field] <= _FXP6_ONE
        for field in _NATIVE_TELEMETRY_ENERGY_FIELDS
    ):
        raise ValueError("native telemetry receipt")
    capacity = value["capacity"]
    if type(capacity) is not dict or set(capacity) != set(
        _NATIVE_TELEMETRY_CAPACITY_FIELDS
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
        type(residuals[field]) is not int or not 0 <= residuals[field] <= _FXP6_ONE
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
        "energy": {field: energy[field] for field in _NATIVE_TELEMETRY_ENERGY_FIELDS},
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
    migration_subcode: str | None = None
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
        raw_migration_subcode = payload["migration_subcode"]
        if raw_migration_subcode is not None:
            if (
                type(raw_migration_subcode) is not str
                or raw_migration_subcode not in FIELD_MIGRATION_SUBCODES
            ):
                raise _InvalidSemanticMigrationSubcode("semantic migration subcode")
            migration_subcode = raw_migration_subcode
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
            "migration_subcode": migration_subcode,
        }
    vector = _validate_semantic_vector_receipt(
        payload["semantic_vector_receipt"], expected_state_changed=state_changed
    )
    nodes = _validate_node_observability(payload["node_observability"])
    expression = None
    if (
        "expression_projection" in payload
        and payload["expression_projection"] is not None
    ):
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
        result["migration_subcode"] = migration_subcode
        result["telemetry_receipt"] = _validate_native_telemetry_receipt(
            payload["telemetry_receipt"], receipt=receipt, revision=revision
        )
    return result


def validate_semantic_result(
    value: Any, *, expected_base_revision: int | None = None
) -> dict[str, Any]:
    return _validate_semantic_result(
        value, expected_base_revision=expected_base_revision
    )


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
    if code == "INVALID_NEURAL_STATE":
        return InvalidNeuralState(_invalid_neural_state_subcode(error))
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

    def reconcile_seed_config_v1(
        self, closed_request: dict[str, Any]
    ) -> dict[str, Any]:
        """Reconcile a closed tri-state SeedCode observation in native code.

        The bridge validates only the observation envelope. It never calls
        ``inspect``, supplies a manual confirmation flag, or caches a native
        authority fence for this lifecycle.
        """
        request = _validate_seed_config_reconcile_request(closed_request)
        native = self._require()
        try:
            result = native.reconcile_seed_config_v1(
                json.dumps(request, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _validate_seed_config_result(_parse_payload(result))

    def ack_seed_config_writeback_v1(
        self, closed_request: dict[str, Any]
    ) -> dict[str, Any]:
        """Activate a native mirror after the exact host writeback succeeds."""
        request = _validate_seed_config_ack_request(closed_request)
        native = self._require()
        try:
            result = native.ack_seed_config_writeback_v1(
                json.dumps(request, ensure_ascii=False, sort_keys=True)
            )
        except Exception as exc:
            raise _classify(exc) from exc
        return _validate_seed_config_ack_result(_parse_payload(result))

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
            result = _validate_semantic_result(
                method(_semantic_closed_json(scope), encoded_proposal),
                expected_base_revision=proposal["base_revision"],
            )
            nodes = result["node_observability"]
            if nodes is None:
                return result
            contract_info = getattr(native, "contract_info", None)
            if not callable(contract_info):
                return _semantic_degraded("NATIVE_SYMBOL_UNAVAILABLE")
            native_contract_id = _validate_node_observability_contract_info(
                contract_info()
            )
            if nodes["contract_id"] != native_contract_id:
                raise ValueError("node observability contract mismatch")
            return result
        except _InvalidSemanticMigrationSubcode:
            return _semantic_degraded(
                "NATIVE_ERROR", migration_subcode=_FIELD_MIGRATION_UNKNOWN
            )
        except BaseException as exc:
            if isinstance(exc, (TypeError, ValueError, json.JSONDecodeError)):
                return _semantic_degraded("NATIVE_MALFORMED")
            code = _semantic_error_code(exc)
            return _semantic_degraded(
                code,
                state_subcode=(
                    _invalid_neural_state_subcode(exc)
                    if code == "INVALID_NEURAL_STATE"
                    else _STATE_SUBCODE_MISSING
                ),
                migration_subcode=_migration_subcode(exc),
            )

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
