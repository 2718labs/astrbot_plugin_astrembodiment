"""Closed V3, request-local semantic estimation for the preview lane.

This module accepts the frozen current request text only at the provider
boundary.  It neither accepts nor retains dialogue history, tools, system
state, provider transcripts, or action contracts.
"""

from __future__ import annotations

import asyncio
import copy
import hashlib
import hmac
import inspect
import json
import math
from collections.abc import Callable, Mapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any

from .context_binding import (
    DIMENSION_NAMES,
    ContextBindingV1,
    validate_context_summary,
)
from .contracts import FrozenTurn, ScopeTokens

FXP6_SCALE = 1_000_000
SEMANTIC_ESTIMATE_V3_SCHEMA = "astr-embodiment.semantic-estimate.v3"
ESTIMATOR_FORMULA_DIGEST = hashlib.sha256(
    b"astr-embodiment/semantic-estimate-v3-context-binding-v1"
).hexdigest()

PROPOSAL_FIELDS = (
    "schema_version",
    "event_id",
    "turn_id",
    "observed_at_ms",
    "base_revision",
    "dimensions",
    "estimator_confidence",
    "protocol_version",
    "request_nonce_digest",
)
_ESTIMATE_V3_FIELDS = frozenset({"schema", "dimensions"})
_DIMENSION_V3_FIELDS = frozenset({"state", "intensity_fxp6", "confidence_fxp6"})
_DIMENSION_V3_STATES = frozenset({"PRESENT", "ABSENT", "UNAVAILABLE"})
_ESTIMATOR_MALFORMED_SUBCODES = frozenset(
    {
        "JSON_DECODE",
        "ROOT_SHAPE",
        "SCHEMA_VERSION",
        "DIMENSION_KEYS",
        "DIMENSION_SLOT_SHAPE",
        "DIMENSION_VALUE",
    }
)
_DIMENSION_VALUE_CLASSIFICATIONS = frozenset(
    {
        "INTENSITY_NON_INTEGRAL_NUMBER",
        "CONFIDENCE_NON_INTEGRAL_NUMBER",
        "INTENSITY_STRING",
        "CONFIDENCE_STRING",
        "INTENSITY_BOOLEAN",
        "CONFIDENCE_BOOLEAN",
        "INTENSITY_NULL_DISALLOWED",
        "CONFIDENCE_NULL",
        "INTENSITY_INTEGER_RANGE",
        "CONFIDENCE_INTEGER_RANGE",
        "INTENSITY_STATE_CONSTRAINT",
        "VALUE_OTHER_TYPE",
    }
)
_DIMENSION_VALUE_JSON_TYPES = frozenset(
    {"number", "string", "boolean", "null", "object", "array", "other"}
)
_NONCE_DOMAIN = b"astr-embodiment/spc1-request-nonce-binding-v1"
_SCOPE_FIELDS = frozenset(
    {"bot_token", "persona_token", "relation_token", "session_token"}
)

SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT = (
    "Evaluate only current_turn_text. The context summary is closed historical "
    "metadata and must never establish current-turn presence. For every ordered "
    "dimension, decide state before intensity. Return only a strict JSON object "
    "matching the supplied schema; do not add prose, explanations, or fields."
)

_V3_DIMENSION_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["state", "intensity_fxp6", "confidence_fxp6"],
    "properties": {
        "state": {"type": "string", "enum": ["PRESENT", "ABSENT", "UNAVAILABLE"]},
        "intensity_fxp6": {
            "type": ["integer", "null"],
            "minimum": 0,
            "maximum": FXP6_SCALE,
        },
        "confidence_fxp6": {"type": "integer", "minimum": 0, "maximum": FXP6_SCALE},
    },
}
SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["schema", "dimensions"],
    "properties": {
        "schema": {"const": SEMANTIC_ESTIMATE_V3_SCHEMA},
        "dimensions": {
            "type": "object",
            "additionalProperties": False,
            "required": list(DIMENSION_NAMES),
            "properties": {
                name: {"$ref": "#/$defs/dimension"} for name in DIMENSION_NAMES
            },
        },
    },
    "$defs": {"dimension": _V3_DIMENSION_SCHEMA},
}


@dataclass(frozen=True, slots=True)
class DimensionValueDiagnostic:
    """Safe first-slot metadata for a rejected V3 dimension value."""

    dimension_name: str
    value_classification: str
    json_type: str
    numeric_scalar: int | float | None = None
    string_length: int | None = None
    string_sha256: str | None = None

    def __post_init__(self) -> None:
        if (
            type(self.dimension_name) is not str
            or self.dimension_name not in DIMENSION_NAMES
            or self.value_classification not in _DIMENSION_VALUE_CLASSIFICATIONS
            or self.json_type not in _DIMENSION_VALUE_JSON_TYPES
        ):
            raise ValueError("invalid dimension value diagnostic")
        has_numeric = self.numeric_scalar is not None
        has_string = self.string_length is not None or self.string_sha256 is not None
        if has_numeric and has_string:
            raise ValueError("mixed dimension value diagnostic")
        if has_numeric:
            if self.json_type != "number" or not (
                type(self.numeric_scalar) is int
                or (
                    type(self.numeric_scalar) is float
                    and math.isfinite(self.numeric_scalar)
                )
            ):
                raise ValueError("invalid numeric dimension value diagnostic")
        elif self.json_type == "number" and has_string:
            raise ValueError("invalid number dimension value diagnostic")
        if has_string:
            if (
                self.json_type != "string"
                or type(self.string_length) is not int
                or self.string_length < 0
                or type(self.string_sha256) is not str
                or len(self.string_sha256) != 64
            ):
                raise ValueError("invalid string dimension value diagnostic")
        elif self.json_type == "string":
            raise ValueError("missing string dimension value diagnostic")

    def as_json(self) -> dict[str, int | float | str]:
        result: dict[str, int | float | str] = {
            "dimension_name": self.dimension_name,
            "value_classification": self.value_classification,
            "json_type": self.json_type,
        }
        if self.numeric_scalar is not None:
            result["numeric_scalar"] = self.numeric_scalar
        elif self.string_length is not None and self.string_sha256 is not None:
            result["string_length"] = self.string_length
            result["string_sha256"] = self.string_sha256
        return result


class SemanticEstimateError(ValueError):
    """Fixed, non-echoing V3 parse/provider failure."""

    def __init__(
        self,
        code: str = "ESTIMATOR_MALFORMED",
        subcode: str | None = None,
        diagnostic: DimensionValueDiagnostic | None = None,
    ) -> None:
        if subcode is not None and subcode not in _ESTIMATOR_MALFORMED_SUBCODES:
            raise ValueError("invalid estimator malformed subcode")
        if diagnostic is not None and (
            subcode != "DIMENSION_VALUE"
            or type(diagnostic) is not DimensionValueDiagnostic
        ):
            raise ValueError("invalid estimator dimension value diagnostic")
        super().__init__(code)
        self.code = code
        self.subcode = subcode
        self.diagnostic = diagnostic

    def diagnostic_json(self) -> dict[str, int | float | str] | None:
        if self.diagnostic is None:
            return None
        return self.diagnostic.as_json()


class SemanticProposalError(ValueError):
    """Fixed, non-echoing closed proposal failure."""

    def __init__(self, code: str = "INVALID_PERCEPTION_PROPOSAL") -> None:
        super().__init__(code)
        self.code = code


def _invalid_estimate(
    subcode: str,
    diagnostic: DimensionValueDiagnostic | None = None,
) -> SemanticEstimateError:
    return SemanticEstimateError("ESTIMATOR_MALFORMED", subcode, diagnostic)


def _invalid_proposal() -> SemanticProposalError:
    return SemanticProposalError()


def _reject_json_constant(_value: str) -> None:
    raise ValueError("json constant")


def _pairs_without_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def _decode_json_object(value: Any) -> dict[str, Any]:
    if type(value) is str:
        try:
            payload = json.loads(
                value,
                parse_constant=_reject_json_constant,
                object_pairs_hook=_pairs_without_duplicates,
            )
        except (TypeError, ValueError, json.JSONDecodeError):
            raise _invalid_estimate("JSON_DECODE") from None
    else:
        payload = value
    if type(payload) is not dict or any(type(key) is not str for key in payload):
        raise _invalid_estimate("ROOT_SHAPE")
    return payload


def _is_raw_integer(value: Any) -> bool:
    return type(value) is int


def _json_value_type(value: Any) -> str:
    if value is None:
        return "null"
    if type(value) is bool:
        return "boolean"
    if type(value) is int or type(value) is float:
        return "number"
    if type(value) is str:
        return "string"
    if type(value) is dict:
        return "object"
    if type(value) is list:
        return "array"
    return "other"


def _dimension_value_diagnostic(
    dimension_name: str,
    value_classification: str,
    value: Any,
) -> DimensionValueDiagnostic:
    json_type = _json_value_type(value)
    if type(value) is int or (type(value) is float and math.isfinite(value)):
        return DimensionValueDiagnostic(
            dimension_name=dimension_name,
            value_classification=value_classification,
            json_type=json_type,
            numeric_scalar=value,
        )
    if type(value) is str:
        return DimensionValueDiagnostic(
            dimension_name=dimension_name,
            value_classification=value_classification,
            json_type=json_type,
            string_length=len(value),
            string_sha256=hashlib.sha256(value.encode("utf-8")).hexdigest(),
        )
    return DimensionValueDiagnostic(
        dimension_name=dimension_name,
        value_classification=value_classification,
        json_type=json_type,
    )


def _dimension_intensity_failure_v3(value: Any, state: str) -> str | None:
    if type(value) is bool:
        return "INTENSITY_BOOLEAN"
    if value is None:
        return None if state == "UNAVAILABLE" else "INTENSITY_NULL_DISALLOWED"
    if type(value) is str:
        return "INTENSITY_STRING"
    if type(value) is float:
        return "INTENSITY_NON_INTEGRAL_NUMBER"
    if type(value) is not int:
        return "VALUE_OTHER_TYPE"
    if not 0 <= value <= FXP6_SCALE:
        return "INTENSITY_INTEGER_RANGE"
    if state == "PRESENT":
        return None if value >= 1 else "INTENSITY_STATE_CONSTRAINT"
    if state == "ABSENT":
        return None if value == 0 else "INTENSITY_STATE_CONSTRAINT"
    return "INTENSITY_STATE_CONSTRAINT"


def _dimension_confidence_failure_v3(value: Any) -> str | None:
    if type(value) is bool:
        return "CONFIDENCE_BOOLEAN"
    if value is None:
        return "CONFIDENCE_NULL"
    if type(value) is str:
        return "CONFIDENCE_STRING"
    if type(value) is float:
        return "CONFIDENCE_NON_INTEGRAL_NUMBER"
    if type(value) is not int:
        return "VALUE_OTHER_TYPE"
    if not 0 <= value <= FXP6_SCALE:
        return "CONFIDENCE_INTEGER_RANGE"
    return None


def _diagnose_dimension_value_v3(
    dimension_name: str,
    slot: Mapping[str, Any],
) -> DimensionValueDiagnostic:
    state = slot["state"]
    if type(state) is not str or state not in _DIMENSION_V3_STATES:
        return _dimension_value_diagnostic(
            dimension_name,
            "VALUE_OTHER_TYPE",
            state,
        )
    intensity = slot["intensity_fxp6"]
    intensity_failure = _dimension_intensity_failure_v3(intensity, state)
    if intensity_failure is not None:
        return _dimension_value_diagnostic(
            dimension_name,
            intensity_failure,
            intensity,
        )
    confidence = slot["confidence_fxp6"]
    confidence_failure = _dimension_confidence_failure_v3(confidence)
    if confidence_failure is not None:
        return _dimension_value_diagnostic(
            dimension_name,
            confidence_failure,
            confidence,
        )
    return _dimension_value_diagnostic(
        dimension_name,
        "VALUE_OTHER_TYPE",
        state,
    )


def _validate_v3_confidence(value: Any) -> int:
    if not _is_raw_integer(value) or not 0 <= value <= FXP6_SCALE:
        raise _invalid_estimate("DIMENSION_VALUE")
    return value


@dataclass(frozen=True, slots=True)
class DimensionEstimateV3:
    """One explicit current-turn presence decision."""

    state: str
    intensity_fxp6: int | None
    confidence_fxp6: int

    def __post_init__(self) -> None:
        if type(self.state) is not str or self.state not in _DIMENSION_V3_STATES:
            raise _invalid_estimate("DIMENSION_VALUE")
        if self.state == "PRESENT":
            if type(self.intensity_fxp6) is not int or not 1 <= self.intensity_fxp6 <= FXP6_SCALE:
                raise _invalid_estimate("DIMENSION_VALUE")
        elif self.state == "ABSENT":
            if type(self.intensity_fxp6) is not int or self.intensity_fxp6 != 0:
                raise _invalid_estimate("DIMENSION_VALUE")
        elif self.intensity_fxp6 is not None:
            raise _invalid_estimate("DIMENSION_VALUE")
        _validate_v3_confidence(self.confidence_fxp6)

    def as_json(self) -> dict[str, int | str | None]:
        return {
            "state": self.state,
            "intensity_fxp6": self.intensity_fxp6,
            "confidence_fxp6": self.confidence_fxp6,
        }


@dataclass(frozen=True, slots=True)
class SemanticEstimateV3:
    """The complete ordered fifteen-dimension V3 current-turn estimate."""

    dimensions: Mapping[str, DimensionEstimateV3]
    schema: str = SEMANTIC_ESTIMATE_V3_SCHEMA

    def __post_init__(self) -> None:
        if self.schema != SEMANTIC_ESTIMATE_V3_SCHEMA:
            raise _invalid_estimate("SCHEMA_VERSION")
        if type(self.dimensions) is not dict or set(self.dimensions) != set(DIMENSION_NAMES):
            raise _invalid_estimate("DIMENSION_KEYS")
        canonical: dict[str, DimensionEstimateV3] = {}
        for name in DIMENSION_NAMES:
            dimension = self.dimensions[name]
            if type(dimension) is not DimensionEstimateV3:
                raise _invalid_estimate("DIMENSION_VALUE")
            canonical[name] = dimension
        object.__setattr__(self, "dimensions", MappingProxyType(canonical))

    def as_json(self) -> dict[str, Any]:
        return {
            "schema": SEMANTIC_ESTIMATE_V3_SCHEMA,
            "dimensions": {
                name: self.dimensions[name].as_json() for name in DIMENSION_NAMES
            },
        }


def _validate_dimension_slots_v3(value: Any) -> dict[str, DimensionEstimateV3]:
    if type(value) is not dict or set(value) != set(DIMENSION_NAMES):
        raise _invalid_estimate("DIMENSION_KEYS")
    dimensions: dict[str, DimensionEstimateV3] = {}
    for name in DIMENSION_NAMES:
        slot = value[name]
        if type(slot) is not dict or set(slot) != _DIMENSION_V3_FIELDS:
            raise _invalid_estimate("DIMENSION_SLOT_SHAPE")
        try:
            dimensions[name] = DimensionEstimateV3(
                state=slot["state"],
                intensity_fxp6=slot["intensity_fxp6"],
                confidence_fxp6=slot["confidence_fxp6"],
            )
        except SemanticEstimateError as exc:
            if exc.subcode != "DIMENSION_VALUE":
                raise
            raise _invalid_estimate(
                "DIMENSION_VALUE",
                _diagnose_dimension_value_v3(name, slot),
            ) from None
    return dimensions


def parse_estimator_output_v3(value: Any) -> SemanticEstimateV3:
    """Parse only the exact V3 closed JSON shape."""

    try:
        if type(value) is SemanticEstimateV3:
            value = value.as_json()
        payload = _decode_json_object(value)
        if set(payload) != _ESTIMATE_V3_FIELDS:
            raise _invalid_estimate("ROOT_SHAPE")
        if payload["schema"] != SEMANTIC_ESTIMATE_V3_SCHEMA:
            raise _invalid_estimate("SCHEMA_VERSION")
        return SemanticEstimateV3(dimensions=_validate_dimension_slots_v3(payload["dimensions"]))
    except SemanticEstimateError:
        raise
    except BaseException:
        raise _invalid_estimate("DIMENSION_VALUE") from None


def _canonical_json(value: Mapping[str, Any]) -> bytes:
    try:
        return json.dumps(
            value,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
    except (TypeError, ValueError):
        raise _invalid_proposal() from None


def _canonical_hex(value: Any, bytes_len: int) -> str:
    if type(value) is not str or len(value) != bytes_len * 2:
        raise ValueError("hex token")
    try:
        decoded = bytes.fromhex(value)
    except (TypeError, ValueError):
        raise ValueError("hex token") from None
    if len(decoded) != bytes_len:
        raise ValueError("hex token")
    return decoded.hex()


def _canonical_nonzero_hex(value: Any, bytes_len: int) -> str:
    canonical = _canonical_hex(value, bytes_len)
    if not any(bytes.fromhex(canonical)):
        raise ValueError("hex token")
    return canonical


def _canonical_scope(scope: ScopeTokens | Mapping[str, Any]) -> ScopeTokens:
    if type(scope) is ScopeTokens:
        payload: dict[str, Any] = scope.scope_json()
    elif type(scope) is dict:
        payload = scope
    else:
        raise _invalid_proposal()
    if any(type(key) is not str for key in payload) or set(payload) != _SCOPE_FIELDS:
        raise _invalid_proposal()
    relation = payload["relation_token"]
    if relation is not None and type(relation) is not str:
        raise _invalid_proposal()
    try:
        return ScopeTokens(
            bot_token=_canonical_nonzero_hex(payload["bot_token"], 16),
            persona_token=_canonical_nonzero_hex(payload["persona_token"], 16),
            session_token=_canonical_nonzero_hex(payload["session_token"], 16),
            relation_token=(
                _canonical_nonzero_hex(relation, 16) if relation is not None else None
            ),
        )
    except (TypeError, ValueError):
        raise _invalid_proposal() from None


def _canonical_turn(scope: ScopeTokens, turn: FrozenTurn) -> FrozenTurn:
    canonical_scope = _canonical_scope(scope)
    if type(turn) is not FrozenTurn or _canonical_scope(turn.scope) != canonical_scope:
        raise _invalid_proposal()
    if type(turn.base_revision) is not int or turn.base_revision < 0:
        raise _invalid_proposal()
    if type(turn.observed_at_ms) is not int or turn.observed_at_ms <= 0:
        raise _invalid_proposal()
    try:
        return FrozenTurn(
            scope=canonical_scope,
            event_id=_canonical_nonzero_hex(turn.event_id, 16),
            turn_id=_canonical_nonzero_hex(turn.turn_id, 16),
            base_revision=turn.base_revision,
            observed_at_ms=turn.observed_at_ms,
        )
    except (TypeError, ValueError):
        raise _invalid_proposal() from None


def make_request_nonce_digest(
    scope: ScopeTokens,
    turn: FrozenTurn,
    *,
    entropy: bytes | None = None,
) -> str:
    """Bind scope, event, turn, semantic base revision, and observed time."""

    canonical_scope = _canonical_scope(scope)
    canonical_turn = _canonical_turn(canonical_scope, turn)
    if entropy is not None and (type(entropy) is not bytes or not entropy):
        raise _invalid_proposal()
    binding = {
        "scope": canonical_scope.scope_json(),
        "event_id": canonical_turn.event_id,
        "turn_id": canonical_turn.turn_id,
        "base_revision": canonical_turn.base_revision,
        "observed_at_ms": canonical_turn.observed_at_ms,
    }
    digest = hashlib.sha256(_NONCE_DOMAIN + b"\x00" + _canonical_json(binding)).hexdigest()
    if digest == "00" * 32:
        digest = hashlib.sha256(_NONCE_DOMAIN + b"\x01" + _canonical_json(binding)).hexdigest()
    return digest


def _validate_dimension_map(value: Any) -> dict[str, int]:
    if type(value) is not dict or set(value) != set(DIMENSION_NAMES):
        raise _invalid_proposal()
    dimensions: dict[str, int] = {}
    for name in DIMENSION_NAMES:
        item = value[name]
        if type(item) is not int or not 0 <= item <= FXP6_SCALE:
            raise _invalid_proposal()
        dimensions[name] = item
    return dimensions


def _validate_proposal_nonce(scope: ScopeTokens, payload: Mapping[str, Any]) -> None:
    turn = FrozenTurn(
        scope=scope,
        event_id=payload["event_id"],
        turn_id=payload["turn_id"],
        base_revision=payload["base_revision"],
        observed_at_ms=payload["observed_at_ms"],
    )
    if not hmac.compare_digest(payload["request_nonce_digest"], make_request_nonce_digest(scope, turn)):
        raise _invalid_proposal()


def validate_perception_proposal(
    value: Any,
    *,
    scope: ScopeTokens | Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Validate and canonicalize the exact native PerceptionProposalV1 ABI."""

    try:
        if type(value) is str:
            payload = json.loads(
                value,
                parse_constant=_reject_json_constant,
                object_pairs_hook=_pairs_without_duplicates,
            )
        else:
            payload = value
        if type(payload) is not dict or any(type(key) is not str for key in payload):
            raise _invalid_proposal()
        if set(payload) != set(PROPOSAL_FIELDS):
            raise _invalid_proposal()
        if payload["schema_version"] != 1 or payload["protocol_version"] != 1:
            raise _invalid_proposal()
        if type(payload["schema_version"]) is not int or type(payload["protocol_version"]) is not int:
            raise _invalid_proposal()
        event_id = _canonical_nonzero_hex(payload["event_id"], 16)
        turn_id = _canonical_nonzero_hex(payload["turn_id"], 16)
        observed_at_ms = payload["observed_at_ms"]
        base_revision = payload["base_revision"]
        if type(observed_at_ms) is not int or observed_at_ms <= 0:
            raise _invalid_proposal()
        if type(base_revision) is not int or base_revision < 0:
            raise _invalid_proposal()
        confidence = payload["estimator_confidence"]
        if type(confidence) is not int or not 1 <= confidence <= FXP6_SCALE:
            raise _invalid_proposal()
        nonce = _canonical_nonzero_hex(payload["request_nonce_digest"], 32)
        canonical = {
            "schema_version": 1,
            "event_id": event_id,
            "turn_id": turn_id,
            "observed_at_ms": observed_at_ms,
            "base_revision": base_revision,
            "dimensions": _validate_dimension_map(payload["dimensions"]),
            "estimator_confidence": confidence,
            "protocol_version": 1,
            "request_nonce_digest": nonce,
        }
        if scope is not None:
            _validate_proposal_nonce(_canonical_scope(scope), canonical)
        return canonical
    except SemanticProposalError:
        raise
    except (TypeError, ValueError, json.JSONDecodeError):
        raise _invalid_proposal() from None


def proposal_to_json(
    proposal: Mapping[str, Any],
    *,
    scope: ScopeTokens | Mapping[str, Any] | None = None,
) -> str:
    """Serialize only the canonical, closed shared ABI JSON."""

    canonical = validate_perception_proposal(proposal, scope=scope)
    try:
        return json.dumps(
            canonical,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
            allow_nan=False,
        )
    except (TypeError, ValueError):
        raise _invalid_proposal() from None


def build_perception_proposal_v3(
    *,
    scope: ScopeTokens,
    turn: FrozenTurn,
    estimate: SemanticEstimateV3 | Mapping[str, Any] | str,
    base_revision: int,
    nonce_digest: str,
) -> dict[str, Any]:
    """Reduce one complete V3 estimate to the shared 15D proposal exactly."""

    try:
        canonical_scope = _canonical_scope(scope)
        canonical_turn = _canonical_turn(canonical_scope, turn)
        if type(base_revision) is not int or base_revision != canonical_turn.base_revision:
            raise _invalid_proposal()
        canonical_estimate = (
            estimate if type(estimate) is SemanticEstimateV3 else parse_estimator_output_v3(estimate)
        )
        dimensions: dict[str, int] = {}
        confidences: list[int] = []
        for name in DIMENSION_NAMES:
            slot = canonical_estimate.dimensions[name]
            if slot.state == "UNAVAILABLE":
                raise SemanticProposalError("SEMANTIC_VECTOR_UNAVAILABLE")
            dimensions[name] = slot.intensity_fxp6 if slot.state == "PRESENT" else 0
            confidences.append(slot.confidence_fxp6)
        confidence = min(confidences)
        if confidence == 0:
            raise SemanticProposalError("ESTIMATOR_UNCERTAIN")
        proposal = {
            "schema_version": 1,
            "event_id": canonical_turn.event_id,
            "turn_id": canonical_turn.turn_id,
            "observed_at_ms": canonical_turn.observed_at_ms,
            "base_revision": base_revision,
            "dimensions": dimensions,
            "estimator_confidence": confidence,
            "protocol_version": 1,
            "request_nonce_digest": nonce_digest,
        }
        return validate_perception_proposal(proposal, scope=canonical_scope)
    except SemanticProposalError:
        raise
    except SemanticEstimateError as exc:
        raise SemanticProposalError(exc.code) from None
    except BaseException:
        raise _invalid_proposal() from None


def build_contextual_estimator_request(
    *,
    request_text: str,
    binding: ContextBindingV1 | Mapping[str, Any],
    summary: Any,
) -> dict[str, Any]:
    """Build the sole provider mapping after every opaque binding validates."""

    if type(request_text) is not str:
        raise SemanticEstimateError("EMPTY_REQUEST")
    try:
        canonical_summary = validate_context_summary(summary, expected_binding=binding)
        schema = copy.deepcopy(SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA)
        return {
            "current_turn_text": request_text,
            "system_prompt": SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
            "structured_schema": schema,
            "input": {"context_summary": canonical_summary},
        }
    except SemanticEstimateError:
        raise
    except BaseException:
        raise SemanticEstimateError("ESTIMATOR_MALFORMED") from None


async def estimate_context_bound(
    provider: Callable[[Mapping[str, Any]], Any] | Any,
    request_text: str,
    *,
    binding: ContextBindingV1 | Mapping[str, Any],
    summary: Any,
) -> SemanticEstimateV3:
    """Call the provider exactly once with only the closed contextual mapping."""

    request = build_contextual_estimator_request(
        request_text=request_text,
        binding=binding,
        summary=summary,
    )
    callable_provider = provider
    if not callable(callable_provider):
        try:
            callable_provider = getattr(callable_provider, "estimate", None)
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            raise SemanticEstimateError("ESTIMATOR_UNAVAILABLE") from None
    if not callable(callable_provider):
        raise SemanticEstimateError("ESTIMATOR_UNAVAILABLE")
    try:
        result = callable_provider(request)
        if inspect.isawaitable(result):
            result = await result
    except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
        raise
    except BaseException:
        raise SemanticEstimateError("ESTIMATOR_UNAVAILABLE") from None
    try:
        return parse_estimator_output_v3(result)
    except SemanticEstimateError as exc:
        if exc.code == "ESTIMATOR_MALFORMED":
            raise
        raise SemanticEstimateError("ESTIMATOR_MALFORMED") from None
