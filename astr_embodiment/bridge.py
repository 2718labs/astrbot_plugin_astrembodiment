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
_SEMANTIC_RESULT_FIELDS = frozenset({"schema", "receipt", "revision", "deduplicated"})
_SEMANTIC_RESULT_WITH_EXPRESSION_FIELDS = frozenset(
    {
        "schema",
        "receipt",
        "revision",
        "deduplicated",
        "expression_projection",
    }
)
_SEMANTIC_RESULT_FULL_VECTOR_FIELDS = frozenset(
    {
        "schema",
        "receipt",
        "semantic_vector_receipt",
        "node_observability",
        "revision",
        "deduplicated",
    }
)
_SEMANTIC_RESULT_FULL_VECTOR_WITH_EXPRESSION_FIELDS = frozenset(
    {*_SEMANTIC_RESULT_FULL_VECTOR_FIELDS, "expression_projection"}
)
_SEMANTIC_RESULT_CANONICAL_STATE_FIELDS = frozenset(
    {"full_vector_state", "node_observability_state"}
)
_SEMANTIC_RESULT_LEGACY_CANONICAL_FIELDS = frozenset(
    {
        *_SEMANTIC_RESULT_FIELDS,
        "semantic_vector_receipt",
        "node_observability",
        *_SEMANTIC_RESULT_CANONICAL_STATE_FIELDS,
    }
)
_SEMANTIC_RESULT_LEGACY_CANONICAL_WITH_EXPRESSION_FIELDS = frozenset(
    {*_SEMANTIC_RESULT_LEGACY_CANONICAL_FIELDS, "expression_projection"}
)
_SEMANTIC_RESULT_FULL_VECTOR_CANONICAL_FIELDS = frozenset(
    {
        *_SEMANTIC_RESULT_FULL_VECTOR_FIELDS,
        *_SEMANTIC_RESULT_CANONICAL_STATE_FIELDS,
    }
)
_SEMANTIC_RESULT_FULL_VECTOR_CANONICAL_WITH_EXPRESSION_FIELDS = frozenset(
    {*_SEMANTIC_RESULT_FULL_VECTOR_CANONICAL_FIELDS, "expression_projection"}
)
_EXPRESSION_PROJECTION_SCHEMA = "astr-embodiment.expression-projection.v1"
_EXPRESSION_PROJECTION_FIELDS = frozenset({"schema", "revision", "profile_fxp6"})
_EXPRESSION_PROFILE_FIELD_ORDER = (
    "warmth",
    "sensitivity",
    "guardedness",
    "repair_orientation",
    "engagement",
    "epistemic_caution",
)
_EXPRESSION_PROFILE_FIELDS = frozenset(_EXPRESSION_PROFILE_FIELD_ORDER)
_EXPRESSION_FXP6_MAX = 1_000_000
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
_SEMANTIC_VECTOR_RECEIPT_SCHEMA = "astr-embodiment.semantic-vector-receipt.v2"
_SEMANTIC_VECTOR_FORMULA = "full-vector-route-neutral-relaxation-v1"
_SEMANTIC_VECTOR_RECEIPT_FIELDS = frozenset(
    {
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
)
_NODE_OBSERVABILITY_SCHEMA = "astr-embodiment.node-observability.v1"
_NODE_OBSERVABILITY_FORMULA = "spc1-node-observability-v1"
_NODE_OBSERVABILITY_FIELDS = frozenset(
    {
        "schema",
        "formula",
        "revision",
        "field_node_capacity",
        "region_layout",
        "counts",
        "residuals",
        "regions",
    }
)
_NODE_OBSERVABILITY_COUNTS_FIELDS = frozenset(
    {
        "selected_node_count",
        "activated_node_count",
        "changed_node_count",
        "potential_nonzero_after_count",
        "excitation_nonzero_after_count",
        "signal_nonzero_after_count",
    }
)
_NODE_OBSERVABILITY_RESIDUAL_FIELDS = frozenset(
    {"state", "formula", "values_fxp6"}
)
_NODE_OBSERVABILITY_REGION_FIELDS = frozenset(
    {
        "region_id",
        "region_name",
        "node_capacity",
        "selected_node_count",
        "activated_node_count",
        "changed_node_count",
        "potential",
        "excitation",
    }
)
_NODE_OBSERVABILITY_COMPONENT_FIELDS = frozenset(
    {
        "before_mean_fxp6",
        "after_mean_fxp6",
        "delta_mean_fxp6",
        "changed_node_count",
        "nonzero_after_count",
    }
)
_NODE_OBSERVABILITY_REGION_LAYOUT = (
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
_NODE_OBSERVABILITY_CAPACITY = 16_384
_I64_MIN = -(1 << 63)
_I64_MAX = (1 << 63) - 1
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
    if type(scope) is ScopeTokens:
        payload: dict[str, Any] = {
            "bot_token": scope.bot_token,
            "persona_token": scope.persona_token,
            "relation_token": scope.relation_token,
            "session_token": scope.session_token,
        }
    elif type(scope) is str:
        try:
            payload = json.loads(
                scope, object_pairs_hook=_scope_pairs_without_duplicates
            )
        except (TypeError, ValueError, json.JSONDecodeError) as exc:
            raise ValueError("scope") from exc
        if type(payload) is not dict:
            raise ValueError("scope")
    elif type(scope) is dict:
        payload = scope
    else:
        raise ValueError("scope")
    if any(type(key) is not str for key in payload):
        raise ValueError("scope")
    if set(payload) != {
        "bot_token",
        "persona_token",
        "relation_token",
        "session_token",
    }:
        raise ValueError("scope")
    try:
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
    except (TypeError, ValueError):
        raise ValueError("scope") from None


def _scope_pairs_without_duplicates(
    pairs: list[tuple[str, Any]],
) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("scope")
        result[key] = value
    return result


def _closed_json(value: dict[str, Any]) -> str:
    return json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )


def _native_json(value: Any) -> dict[str, Any]:
    if type(value) is str:
        payload = json.loads(value, object_pairs_hook=_native_pairs_without_duplicates)
    else:
        payload = value
    if type(payload) is not dict or any(type(key) is not str for key in payload):
        raise ValueError("native payload")
    return payload


def _native_pairs_without_duplicates(
    pairs: list[tuple[str, Any]],
) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("native payload")
        result[key] = value
    return result


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


def _validate_expression_projection(
    value: Any, *, expected_revision: int
) -> dict[str, Any]:
    if type(value) is not dict or any(type(key) is not str for key in value):
        raise ValueError("expression projection")
    if set(value) != _EXPRESSION_PROJECTION_FIELDS:
        raise ValueError("expression projection fields")
    if value["schema"] != _EXPRESSION_PROJECTION_SCHEMA:
        raise ValueError("expression projection schema")
    revision = value["revision"]
    if type(revision) is not int or revision != expected_revision:
        raise ValueError("expression projection revision")
    profile = value["profile_fxp6"]
    if type(profile) is not dict or any(type(key) is not str for key in profile):
        raise ValueError("expression profile")
    if set(profile) != _EXPRESSION_PROFILE_FIELDS:
        raise ValueError("expression profile fields")
    canonical_profile: dict[str, int] = {}
    for name in _EXPRESSION_PROFILE_FIELD_ORDER:
        item = profile[name]
        if type(item) is not int or not 0 <= item <= _EXPRESSION_FXP6_MAX:
            raise ValueError("expression profile value")
        canonical_profile[name] = item
    return {
        "schema": _EXPRESSION_PROJECTION_SCHEMA,
        "revision": revision,
        "profile_fxp6": canonical_profile,
    }


def _validate_receipt(
    value: Any,
    *,
    revision: int,
    expected_base_revision: int | None,
    allow_unchanged_state: bool,
) -> tuple[dict[str, Any], bool]:
    if type(value) is not dict or any(type(key) is not str for key in value):
        raise ValueError("semantic receipt")
    if set(value) != _RECEIPT_FIELDS:
        raise ValueError("semantic receipt fields")
    if _FORBIDDEN_RECEIPT_KEYS.intersection(value):
        raise ValueError("semantic receipt fields")
    if type(value["schema_version"]) is not int or value["schema_version"] != 1:
        raise ValueError("semantic receipt schema")
    for field in ("base_revision", "next_revision", "active_nodes", "active_edges"):
        if type(value[field]) is not int or value[field] < 0:
            raise ValueError("semantic receipt integer")
    if value["active_nodes"] > _NODE_OBSERVABILITY_CAPACITY:
        raise ValueError("semantic receipt active nodes")
    digest_fields = (
        "formula_digest",
        "scope_digest",
        "event_digest",
        "authority_digest",
        "state_before",
        "state_after",
        "graph_after",
    )
    canonical_digests: dict[str, str] = {}
    for field in digest_fields:
        try:
            canonical_digests[field] = _canonical_hex(value[field], 32)
        except (TypeError, ValueError):
            raise ValueError("semantic receipt digest") from None
    state_changed = canonical_digests["state_before"] != canonical_digests["state_after"]
    if not state_changed and not allow_unchanged_state:
        raise ValueError("semantic receipt transition")
    if type(value["status"]) is not str or value["status"] != "committed":
        raise ValueError("semantic receipt status")
    residuals = value["residuals"]
    if type(residuals) is not dict or any(type(key) is not str for key in residuals):
        raise ValueError("semantic receipt residuals")
    if set(residuals) != _RESIDUAL_FIELDS:
        raise ValueError("semantic receipt residuals")
    if any(
        type(residuals[name]) is not int
        or not _I64_MIN <= residuals[name] <= _I64_MAX
        for name in _RESIDUAL_FIELDS
    ):
        raise ValueError("semantic receipt residuals")
    if value["next_revision"] != revision:
        raise ValueError("semantic receipt revision")
    if value["base_revision"] + 1 != value["next_revision"]:
        raise ValueError("semantic receipt revision")
    if expected_base_revision is not None:
        if type(expected_base_revision) is not int or expected_base_revision < 0:
            raise ValueError("semantic receipt base")
        if value["base_revision"] != expected_base_revision:
            raise ValueError("semantic receipt base")
    return (
        {
            "schema_version": 1,
            "formula_digest": canonical_digests["formula_digest"],
            "scope_digest": canonical_digests["scope_digest"],
            "event_digest": canonical_digests["event_digest"],
            "authority_digest": canonical_digests["authority_digest"],
            "base_revision": value["base_revision"],
            "next_revision": value["next_revision"],
            "state_before": canonical_digests["state_before"],
            "state_after": canonical_digests["state_after"],
            "graph_after": canonical_digests["graph_after"],
            "active_nodes": value["active_nodes"],
            "active_edges": value["active_edges"],
            "residuals": {
                name: residuals[name]
                for name in (
                    "authority",
                    "continuity",
                    "energy",
                    "renormalization",
                    "capacity",
                )
            },
            "status": "committed",
        },
        state_changed,
    )


def _validate_semantic_vector_receipt(
    value: Any, *, expected_state_changed: bool
) -> dict[str, Any]:
    if type(value) is not dict or any(type(key) is not str for key in value):
        raise ValueError("semantic vector receipt")
    if set(value) != _SEMANTIC_VECTOR_RECEIPT_FIELDS:
        raise ValueError("semantic vector receipt fields")
    if value["schema"] != _SEMANTIC_VECTOR_RECEIPT_SCHEMA:
        raise ValueError("semantic vector receipt schema")
    if value["formula"] != _SEMANTIC_VECTOR_FORMULA:
        raise ValueError("semantic vector receipt formula")
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
        raise ValueError("semantic vector receipt count")
    if (
        value["dimension_slot_count"] != 15
        or value["evaluated_dimension_count"] != 15
        or value["injected_dimension_count"] != 15
        or value["unavailable_dimension_count"] != 0
        or value["nonzero_evidence_dimension_count"]
        + value["neutral_baseline_dimension_count"]
        != 15
    ):
        raise ValueError("semantic vector receipt invariant")
    if type(value["state_changed"]) is not bool:
        raise ValueError("semantic vector receipt state")
    if value["state_changed"] is not expected_state_changed:
        raise ValueError("semantic vector receipt state")
    return {
        "schema": _SEMANTIC_VECTOR_RECEIPT_SCHEMA,
        "formula": _SEMANTIC_VECTOR_FORMULA,
        "dimension_slot_count": 15,
        "evaluated_dimension_count": 15,
        "injected_dimension_count": 15,
        "nonzero_evidence_dimension_count": value[
            "nonzero_evidence_dimension_count"
        ],
        "neutral_baseline_dimension_count": value[
            "neutral_baseline_dimension_count"
        ],
        "unavailable_dimension_count": 0,
        "state_changed": expected_state_changed,
    }


def _validate_node_component(value: Any, *, capacity: int) -> dict[str, int]:
    if type(value) is not dict or any(type(key) is not str for key in value):
        raise ValueError("node component")
    if set(value) != _NODE_OBSERVABILITY_COMPONENT_FIELDS:
        raise ValueError("node component fields")
    for field in ("before_mean_fxp6", "after_mean_fxp6", "delta_mean_fxp6"):
        if type(value[field]) is not int or not _I64_MIN <= value[field] <= _I64_MAX:
            raise ValueError("node component value")
    for field in ("changed_node_count", "nonzero_after_count"):
        if type(value[field]) is not int or not 0 <= value[field] <= capacity:
            raise ValueError("node component count")
    return {
        "before_mean_fxp6": value["before_mean_fxp6"],
        "after_mean_fxp6": value["after_mean_fxp6"],
        "delta_mean_fxp6": value["delta_mean_fxp6"],
        "changed_node_count": value["changed_node_count"],
        "nonzero_after_count": value["nonzero_after_count"],
    }


def _validate_node_observability(
    value: Any,
    *,
    expected_revision: int,
    expected_selected_node_count: int,
    expected_state_changed: bool,
) -> dict[str, Any]:
    if type(value) is not dict or any(type(key) is not str for key in value):
        raise ValueError("node observability")
    if set(value) != _NODE_OBSERVABILITY_FIELDS:
        raise ValueError("node observability fields")
    if value["schema"] != _NODE_OBSERVABILITY_SCHEMA:
        raise ValueError("node observability schema")
    if value["formula"] != _NODE_OBSERVABILITY_FORMULA:
        raise ValueError("node observability formula")
    if type(value["revision"]) is not int or value["revision"] != expected_revision:
        raise ValueError("node observability revision")
    if value["field_node_capacity"] != _NODE_OBSERVABILITY_CAPACITY:
        raise ValueError("node observability capacity")
    if value["region_layout"] != "regions-v1":
        raise ValueError("node observability layout")
    counts = value["counts"]
    if type(counts) is not dict or any(type(key) is not str for key in counts):
        raise ValueError("node observability counts")
    if set(counts) != _NODE_OBSERVABILITY_COUNTS_FIELDS:
        raise ValueError("node observability counts")
    for field in _NODE_OBSERVABILITY_COUNTS_FIELDS:
        if type(counts[field]) is not int or not 0 <= counts[field] <= _NODE_OBSERVABILITY_CAPACITY:
            raise ValueError("node observability count")
    if (
        counts["selected_node_count"] != expected_selected_node_count
        or counts["changed_node_count"] > counts["activated_node_count"]
        or counts["activated_node_count"] > counts["selected_node_count"]
        or counts["signal_nonzero_after_count"]
        < max(
            counts["potential_nonzero_after_count"],
            counts["excitation_nonzero_after_count"],
        )
        or counts["signal_nonzero_after_count"]
        > counts["potential_nonzero_after_count"]
        + counts["excitation_nonzero_after_count"]
        or (expected_state_changed and counts["changed_node_count"] == 0)
        or (not expected_state_changed and counts["changed_node_count"] != 0)
    ):
        raise ValueError("node observability invariant")
    residuals = value["residuals"]
    if type(residuals) is not dict or any(type(key) is not str for key in residuals):
        raise ValueError("node observability residuals")
    if set(residuals) != _NODE_OBSERVABILITY_RESIDUAL_FIELDS or residuals != {
        "state": "NOT_COMPUTED",
        "formula": None,
        "values_fxp6": None,
    }:
        raise ValueError("node observability residuals")
    regions = value["regions"]
    if type(regions) is not list or len(regions) != len(_NODE_OBSERVABILITY_REGION_LAYOUT):
        raise ValueError("node observability regions")
    canonical_regions: list[dict[str, Any]] = []
    selected_total = activated_total = changed_total = 0
    potential_nonzero_total = excitation_nonzero_total = 0
    for region_id, (region_name, capacity) in enumerate(_NODE_OBSERVABILITY_REGION_LAYOUT):
        region = regions[region_id]
        if type(region) is not dict or any(type(key) is not str for key in region):
            raise ValueError("node observability region")
        if set(region) != _NODE_OBSERVABILITY_REGION_FIELDS:
            raise ValueError("node observability region fields")
        if (
            type(region["region_id"]) is not int
            or region["region_id"] != region_id
            or region["region_name"] != region_name
            or region["node_capacity"] != capacity
        ):
            raise ValueError("node observability region identity")
        for field in (
            "selected_node_count",
            "activated_node_count",
            "changed_node_count",
        ):
            if type(region[field]) is not int or not 0 <= region[field] <= capacity:
                raise ValueError("node observability region count")
        if (
            region["changed_node_count"] > region["activated_node_count"]
            or region["activated_node_count"] > region["selected_node_count"]
        ):
            raise ValueError("node observability region invariant")
        potential = _validate_node_component(region["potential"], capacity=capacity)
        excitation = _validate_node_component(region["excitation"], capacity=capacity)
        if (
            region["changed_node_count"]
            < max(potential["changed_node_count"], excitation["changed_node_count"])
            or region["changed_node_count"]
            > potential["changed_node_count"] + excitation["changed_node_count"]
        ):
            raise ValueError("node observability region component")
        selected_total += region["selected_node_count"]
        activated_total += region["activated_node_count"]
        changed_total += region["changed_node_count"]
        potential_nonzero_total += potential["nonzero_after_count"]
        excitation_nonzero_total += excitation["nonzero_after_count"]
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
        sum(capacity for _, capacity in _NODE_OBSERVABILITY_REGION_LAYOUT)
        != _NODE_OBSERVABILITY_CAPACITY
        or selected_total != counts["selected_node_count"]
        or activated_total != counts["activated_node_count"]
        or changed_total != counts["changed_node_count"]
        or potential_nonzero_total != counts["potential_nonzero_after_count"]
        or excitation_nonzero_total != counts["excitation_nonzero_after_count"]
    ):
        raise ValueError("node observability totals")
    canonical = {
        "schema": _NODE_OBSERVABILITY_SCHEMA,
        "formula": _NODE_OBSERVABILITY_FORMULA,
        "revision": expected_revision,
        "field_node_capacity": _NODE_OBSERVABILITY_CAPACITY,
        "region_layout": "regions-v1",
        "counts": {
            "selected_node_count": counts["selected_node_count"],
            "activated_node_count": counts["activated_node_count"],
            "changed_node_count": counts["changed_node_count"],
            "potential_nonzero_after_count": counts["potential_nonzero_after_count"],
            "excitation_nonzero_after_count": counts["excitation_nonzero_after_count"],
            "signal_nonzero_after_count": counts["signal_nonzero_after_count"],
        },
        "residuals": {
            "state": "NOT_COMPUTED",
            "formula": None,
            "values_fxp6": None,
        },
        "regions": canonical_regions,
    }
    if len(_closed_json(canonical).encode("utf-8")) > _NODE_OBSERVABILITY_CAPACITY:
        raise ValueError("node observability size")
    return canonical


def _validate_semantic_result(
    value: Any,
    *,
    expected_base_revision: int | None = None,
) -> dict[str, Any]:
    payload = _native_json(value)
    payload_fields = set(payload)
    canonicalized = False
    if payload_fields in {
        _SEMANTIC_RESULT_FIELDS,
        _SEMANTIC_RESULT_WITH_EXPRESSION_FIELDS,
    }:
        full_vector = False
    elif payload_fields in {
        _SEMANTIC_RESULT_FULL_VECTOR_FIELDS,
        _SEMANTIC_RESULT_FULL_VECTOR_WITH_EXPRESSION_FIELDS,
    }:
        # A current PyO3 closure always carries both independent v2
        # projections.  A historical v1 snapshot intentionally has both keys
        # with null values so callers can distinguish its unattested retry
        # from a malformed partial v2 payload.
        full_vector = (
            payload.get("semantic_vector_receipt") is not None
            or payload.get("node_observability") is not None
        )
    elif payload_fields in {
        _SEMANTIC_RESULT_LEGACY_CANONICAL_FIELDS,
        _SEMANTIC_RESULT_LEGACY_CANONICAL_WITH_EXPRESSION_FIELDS,
        _SEMANTIC_RESULT_FULL_VECTOR_CANONICAL_FIELDS,
        _SEMANTIC_RESULT_FULL_VECTOR_CANONICAL_WITH_EXPRESSION_FIELDS,
    }:
        canonicalized = True
        # The canonical legacy and v2 result shapes intentionally have the
        # same top-level keys.  Their closed projection values, not a lax
        # field subset, determine which strict validator applies.
        full_vector = (
            payload.get("semantic_vector_receipt") is not None
            or payload.get("node_observability") is not None
        )
    else:
        raise ValueError("semantic payload")
    if payload.get("schema") != _SEMANTIC_RESULT_SCHEMA:
        raise ValueError("semantic schema")
    revision = payload.get("revision")
    if type(revision) is not int or revision < 0:
        raise ValueError("semantic revision")
    deduplicated = payload.get("deduplicated")
    if type(deduplicated) is not bool:
        raise ValueError("semantic deduplication")
    receipt, state_changed = _validate_receipt(
        payload.get("receipt"),
        revision=revision,
        expected_base_revision=expected_base_revision,
        allow_unchanged_state=full_vector,
    )
    if full_vector:
        semantic_vector_receipt = _validate_semantic_vector_receipt(
            payload["semantic_vector_receipt"], expected_state_changed=state_changed
        )
        try:
            node_observability = _validate_node_observability(
                payload["node_observability"],
                expected_revision=revision,
                expected_selected_node_count=receipt["active_nodes"],
                expected_state_changed=state_changed,
            )
            node_observability_state = "CONFIRMED"
        except (TypeError, ValueError):
            node_observability = None
            node_observability_state = "REJECTED"
        canonical_result: dict[str, Any] = {
            "schema": _SEMANTIC_RESULT_SCHEMA,
            "receipt": receipt,
            "semantic_vector_receipt": semantic_vector_receipt,
            "node_observability": node_observability,
            "full_vector_state": "FULL_VECTOR_CONFIRMED",
            "node_observability_state": node_observability_state,
            "revision": revision,
            "deduplicated": deduplicated,
        }
    else:
        canonical_result = {
            "schema": _SEMANTIC_RESULT_SCHEMA,
            "receipt": receipt,
            "semantic_vector_receipt": None,
            "node_observability": None,
            "full_vector_state": "LEGACY_UNATTESTED",
            "node_observability_state": "UNAVAILABLE",
            "revision": revision,
            "deduplicated": deduplicated,
        }
    if "expression_projection" in payload:
        try:
            canonical_result["expression_projection"] = _validate_expression_projection(
                payload["expression_projection"], expected_revision=revision
            )
        except (TypeError, ValueError):
            canonical_result["expression_projection"] = None
    if canonicalized and (
        payload.get("full_vector_state") != canonical_result["full_vector_state"]
        or payload.get("node_observability_state")
        != canonical_result["node_observability_state"]
    ):
        raise ValueError("canonical semantic result state")
    return canonical_result


def validate_semantic_result(
    value: Any,
    *,
    expected_base_revision: int | None = None,
) -> dict[str, Any]:
    """Validate a raw PyO3 result or its exact closed canonical projection."""

    return _validate_semantic_result(
        value,
        expected_base_revision=expected_base_revision,
    )


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
            proposal = validate_perception_proposal(proposal_json, scope=scope)
            encoded_proposal = proposal_to_json(proposal, scope=scope)
        except SemanticProposalError:
            return _degraded("INVALID_PERCEPTION_PROPOSAL")
        except Exception:
            return _degraded("INVALID_PERCEPTION_PROPOSAL")
        try:
            native = self._require()
        except Exception as exc:
            return _degraded(_semantic_error_code(exc))
        method = getattr(native, "apply_perception_proposal_v1", None)
        if not callable(method):
            return _degraded("NATIVE_SYMBOL_UNAVAILABLE")
        try:
            result = method(_closed_json(scope), encoded_proposal)
            return _validate_semantic_result(
                result,
                expected_base_revision=proposal["base_revision"],
            )
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
