"""Request-local semantic estimation with a closed, non-authoritative ABI."""

from __future__ import annotations

import json
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass
from types import MappingProxyType
from typing import Any

from .semantic_contract import (
    ABSENT,
    DIMENSION_NAMES,
    FXP6_SCALE,
    PRESENT,
    UNAVAILABLE,
    build_semantic_estimate_schema_v3,
    validate_state_intensity,
)

SEMANTIC_ESTIMATE_V3_SCHEMA = "astr-embodiment.semantic-estimate.v3"
_DIMENSION_LIST = ",".join(DIMENSION_NAMES)
SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT = (
    "Evaluate only the current user message. Do not infer facts from hidden history. "
    f"The exact dimension keys are: {_DIMENSION_LIST}. "
    "Return exactly the root keys schema and dimensions, and exactly the slot keys "
    "state, intensity_fxp6, and confidence_fxp6. For every dimension return PRESENT "
    "with integer intensity 1..1000000, "
    "ABSENT with integer 0, or UNAVAILABLE with null. confidence_fxp6 must be an "
    "integer 0..1000000. schema must equal astr-embodiment.semantic-estimate.v3. "
    "Return only one JSON object and no Markdown fences."
)
_ROOT_FIELDS = frozenset({"schema", "dimensions"})
_SLOT_FIELDS = frozenset({"state", "intensity_fxp6", "confidence_fxp6"})
_JSON_WHITESPACE = " \t\r\n"


class SemanticEstimateError(ValueError):
    """Fixed, non-echoing estimator contract failure."""

    def __init__(self, code: str = "ESTIMATOR_MALFORMED") -> None:
        super().__init__(code)
        self.code = code


@dataclass(frozen=True, slots=True)
class DimensionEstimateV3:
    state: str
    intensity_fxp6: int | None
    confidence_fxp6: int


@dataclass(frozen=True, slots=True)
class SemanticEstimateV3:
    dimensions: Mapping[str, DimensionEstimateV3]


@dataclass(frozen=True, slots=True)
class SemanticEstimatorRequestV3:
    system_prompt: str
    request_json: str
    structured_schema: Mapping[str, Any]

    @property
    def reserved_tokens(self) -> int:
        return len(self.system_prompt.encode("utf-8")) + len(
            self.request_json.encode("utf-8")
        ) + 512 + 256


def _reject_constant(_value: str) -> None:
    raise ValueError("non-finite json number")


def _reject_duplicate_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate json key")
        result[key] = value
    return result


def _decode_closed_object(value: Any) -> dict[str, Any]:
    if type(value) is str:
        try:
            value = json.loads(
                value.strip(_JSON_WHITESPACE),
                parse_constant=_reject_constant,
                object_pairs_hook=_reject_duplicate_pairs,
            )
        except (TypeError, ValueError, json.JSONDecodeError):
            raise SemanticEstimateError() from None
    if type(value) is not dict or any(type(key) is not str for key in value):
        raise SemanticEstimateError()
    return value


def parse_estimator_output_v3(value: Any) -> SemanticEstimateV3:
    """Parse exactly one complete fifteen-dimension JSON estimate."""

    payload = _decode_closed_object(value)
    if set(payload) != _ROOT_FIELDS or payload.get("schema") != SEMANTIC_ESTIMATE_V3_SCHEMA:
        raise SemanticEstimateError()
    raw_dimensions = payload.get("dimensions")
    if type(raw_dimensions) is not dict or set(raw_dimensions) != set(DIMENSION_NAMES):
        raise SemanticEstimateError()
    dimensions: dict[str, DimensionEstimateV3] = {}
    for name in DIMENSION_NAMES:
        slot = raw_dimensions[name]
        if type(slot) is not dict or set(slot) != _SLOT_FIELDS:
            raise SemanticEstimateError()
        state = slot["state"]
        intensity = slot["intensity_fxp6"]
        confidence = slot["confidence_fxp6"]
        if (
            type(state) is not str
            or not validate_state_intensity(state, intensity)
            or type(confidence) is not int
            or not 0 <= confidence <= FXP6_SCALE
        ):
            raise SemanticEstimateError()
        dimensions[name] = DimensionEstimateV3(state, intensity, confidence)
    return SemanticEstimateV3(MappingProxyType(dimensions))


def _nonzero_hex(value: Any, byte_length: int) -> str:
    if type(value) is not str or len(value) != byte_length * 2:
        raise SemanticEstimateError("INVALID_PERCEPTION_PROPOSAL")
    try:
        decoded = bytes.fromhex(value)
    except ValueError:
        raise SemanticEstimateError("INVALID_PERCEPTION_PROPOSAL") from None
    if len(decoded) != byte_length or not any(decoded):
        raise SemanticEstimateError("INVALID_PERCEPTION_PROPOSAL")
    return decoded.hex()


def build_perception_proposal_v3(
    *,
    estimate: SemanticEstimateV3 | Mapping[str, Any] | str,
    origin_digest: str,
    request_nonce_digest: str,
) -> dict[str, Any]:
    """Reduce a complete estimate to Native's exact six-key proposal."""

    parsed = estimate if type(estimate) is SemanticEstimateV3 else parse_estimator_output_v3(estimate)
    dimensions: dict[str, int] = {}
    confidences: list[int] = []
    for name in DIMENSION_NAMES:
        slot = parsed.dimensions[name]
        if slot.state == UNAVAILABLE:
            raise SemanticEstimateError("SEMANTIC_VECTOR_UNAVAILABLE")
        dimensions[name] = slot.intensity_fxp6 if slot.state == PRESENT else 0
        confidences.append(slot.confidence_fxp6)
    confidence = min(confidences)
    if confidence <= 0:
        raise SemanticEstimateError("ESTIMATOR_UNCERTAIN")
    return {
        "schema_version": 1,
        "origin_digest": _nonzero_hex(origin_digest, 32),
        "dimensions": dimensions,
        "estimator_confidence": confidence,
        "protocol_version": 1,
        "request_nonce_digest": _nonzero_hex(request_nonce_digest, 32),
    }


def build_estimator_request_v3(message: str) -> SemanticEstimatorRequestV3:
    """Freeze the sole Provider input and compute its exact reservation bytes."""

    if type(message) is not str:
        raise SemanticEstimateError("INVALID_CURRENT_MESSAGE")
    normalized = unicodedata.normalize("NFC", message)
    encoded = normalized.encode("utf-8")
    if not encoded or len(encoded) > 8_192:
        raise SemanticEstimateError("INVALID_CURRENT_MESSAGE")
    request_json = json.dumps(
        {"current_turn_text": normalized},
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )
    return SemanticEstimatorRequestV3(
        system_prompt=SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
        request_json=request_json,
        structured_schema=MappingProxyType(build_semantic_estimate_schema_v3()),
    )
