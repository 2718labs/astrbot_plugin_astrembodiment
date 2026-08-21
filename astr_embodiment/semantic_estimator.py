"""Request-local, closed SPC1 semantic estimation.

The estimator is deliberately a small adapter around one provider call.  It
accepts only the current request text, validates a fixed raw-fxp6 JSON shape,
and discards the text before a proposal can cross the native boundary.  No
history, tools, provider transcript, action contract, or free-form text is
represented by any object in this module.
"""

from __future__ import annotations

import hashlib
import inspect
import json
import secrets
from collections.abc import Awaitable, Callable, Mapping
from dataclasses import dataclass
from typing import Any, TypeAlias

from .contracts import FrozenTurn, ScopeTokens

FXP6_SCALE = 1_000_000

DIMENSION_NAMES = (
    "positive",
    "affiliation",
    "harm",
    "boundary",
    "repair",
    "repetition",
    "new_information",
    "constraint_instability",
    "epistemic_conflict",
    "self_responsibility",
    "other_responsibility",
    "hostility",
    "publicness",
    "engagement",
    "rejection",
)

LOAD_DIMENSIONS = ("positive", "harm", "boundary", "epistemic_conflict")

# Protocol-oriented aliases kept as immutable tuples.
SEMANTIC_DIMENSIONS = DIMENSION_NAMES
LOAD_DIMENSION_NAMES = LOAD_DIMENSIONS

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

_ESTIMATE_NESTED_FIELDS = frozenset({"dimensions", "estimator_confidence"})
_ESTIMATE_FLAT_FIELDS = frozenset((*DIMENSION_NAMES, "estimator_confidence"))
_NONCE_DOMAIN = b"astr-embodiment/spc1-request-nonce-v1"


class SemanticEstimateError(ValueError):
    """Fixed, non-echoing estimator validation failure."""

    def __init__(self, code: str = "INVALID_ESTIMATE") -> None:
        # Never interpolate provider output, request text, or exception text.
        super().__init__(code)
        self.code = code


class SemanticProposalError(ValueError):
    """Fixed, non-echoing proposal validation failure."""

    def __init__(self, code: str = "INVALID_PROPOSAL") -> None:
        super().__init__(code)
        self.code = code


@dataclass(frozen=True, slots=True)
class SemanticEstimate:
    """A closed fifteen-coordinate raw fixed-point estimate."""

    dimensions: dict[str, int]
    estimator_confidence: int

    @property
    def is_load_noop(self) -> bool:
        return all(self.dimensions[name] == 0 for name in LOAD_DIMENSIONS)

    @property
    def confidence(self) -> int:
        return self.estimator_confidence

    def as_json(self) -> dict[str, Any]:
        return {
            "dimensions": {
                name: self.dimensions[name] for name in DIMENSION_NAMES
            },
            "estimator_confidence": self.estimator_confidence,
        }


# A short alias makes the DTO discoverable without introducing a second shape.
ClosedSemanticEstimate = SemanticEstimate


EstimatorProvider: TypeAlias = Callable[[str], Any | Awaitable[Any]]


def _invalid_estimate() -> SemanticEstimateError:
    return SemanticEstimateError()


def _invalid_proposal() -> SemanticProposalError:
    return SemanticProposalError()


def _reject_json_constant(_value: str) -> None:
    raise ValueError("non-finite number")


def _pairs_without_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate key")
        result[key] = value
    return result


def _decode_json_object(value: Any) -> Mapping[str, Any]:
    if isinstance(value, str):
        try:
            decoded = json.loads(
                value,
                parse_constant=_reject_json_constant,
                object_pairs_hook=_pairs_without_duplicates,
            )
        except (TypeError, ValueError, json.JSONDecodeError):
            raise _invalid_estimate() from None
    else:
        decoded = value
    if not isinstance(decoded, Mapping):
        raise _invalid_estimate()
    return decoded


def _is_raw_integer(value: Any) -> bool:
    # ``bool`` is an ``int`` subclass; exact type is intentional.
    return type(value) is int


def _validate_dimension_map(value: Any) -> dict[str, int]:
    if not isinstance(value, Mapping):
        raise _invalid_estimate()
    if set(value) != set(DIMENSION_NAMES):
        raise _invalid_estimate()
    dimensions: dict[str, int] = {}
    for name in DIMENSION_NAMES:
        raw = value.get(name)
        if not _is_raw_integer(raw) or not 0 <= raw <= FXP6_SCALE:
            raise _invalid_estimate()
        dimensions[name] = raw
    if all(raw == 0 for raw in dimensions.values()):
        raise _invalid_estimate()
    return dimensions


def _validate_confidence(value: Any) -> int:
    if not _is_raw_integer(value) or not 1 <= value <= FXP6_SCALE:
        raise _invalid_estimate()
    return value


def _parse_estimator_output(value: Any) -> SemanticEstimate:
    """Parse one provider result using the closed SPC1 estimate schema.

    Both the nested wire-shaped form (``dimensions`` plus confidence) and the
    equivalent flat sixteen-field form are accepted at this local adapter
    boundary.  The returned representation is always nested and canonical.
    Every key and number is checked before any value is copied.
    """

    if isinstance(value, SemanticEstimate):
        value = value.as_json()
    payload = _decode_json_object(value)
    keys = set(payload)
    if keys == _ESTIMATE_NESTED_FIELDS:
        dimensions_payload = payload.get("dimensions")
        confidence_payload = payload.get("estimator_confidence")
    elif keys == _ESTIMATE_FLAT_FIELDS:
        dimensions_payload = {name: payload.get(name) for name in DIMENSION_NAMES}
        confidence_payload = payload.get("estimator_confidence")
    else:
        raise _invalid_estimate()
    dimensions = _validate_dimension_map(dimensions_payload)
    confidence = _validate_confidence(confidence_payload)
    return SemanticEstimate(dimensions=dimensions, estimator_confidence=confidence)


def parse_estimator_output(value: Any) -> SemanticEstimate:
    """Fail closed for hostile mapping implementations as well as bad JSON."""

    try:
        return _parse_estimator_output(value)
    except SemanticEstimateError:
        raise
    except Exception:
        raise SemanticEstimateError() from None


# Explicit aliases used by callers that prefer validation-oriented naming.
validate_estimator_output = parse_estimator_output
parse_semantic_estimate = parse_estimator_output


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


def _is_hex_token(value: Any, byte_length: int) -> bool:
    if not isinstance(value, str) or len(value) != byte_length * 2:
        return False
    try:
        decoded = bytes.fromhex(value)
    except ValueError:
        return False
    return len(decoded) == byte_length and any(decoded)


def _validate_scope(scope: ScopeTokens) -> None:
    if not isinstance(scope, ScopeTokens):
        raise _invalid_proposal()
    for token in (scope.bot_token, scope.persona_token, scope.session_token):
        if not _is_hex_token(token, 16):
            raise _invalid_proposal()
    if scope.relation_token is not None and not _is_hex_token(scope.relation_token, 16):
        raise _invalid_proposal()


def _validate_turn(scope: ScopeTokens, turn: FrozenTurn) -> None:
    if not isinstance(turn, FrozenTurn) or turn.scope != scope:
        raise _invalid_proposal()
    if not _is_hex_token(turn.turn_id, 16) or not _is_hex_token(turn.event_id, 16):
        raise _invalid_proposal()
    if not _is_raw_integer(turn.base_revision) or turn.base_revision < 0:
        raise _invalid_proposal()
    if not _is_raw_integer(turn.observed_at_ms) or turn.observed_at_ms <= 0:
        raise _invalid_proposal()


def make_request_nonce_digest(
    scope: ScopeTokens,
    turn: FrozenTurn,
    *,
    entropy: bytes | None = None,
) -> str:
    """Create a nonzero opaque digest bound to this request's frozen facts."""

    _validate_scope(scope)
    _validate_turn(scope, turn)
    if entropy is None:
        entropy = secrets.token_bytes(32)
    if not isinstance(entropy, bytes) or not entropy:
        raise _invalid_proposal()
    binding = {
        "scope": scope.scope_json(),
        "event_id": turn.event_id,
        "turn_id": turn.turn_id,
        "base_revision": turn.base_revision,
        "observed_at_ms": turn.observed_at_ms,
    }
    digest = hashlib.sha256(_NONCE_DOMAIN + b"\x00" + entropy + _canonical_json(binding)).hexdigest()
    if digest == "00" * 32:
        # Cryptographically unreachable for SHA-256, but preserve the
        # nonzero contract even if a test replaces the hash implementation.
        digest = hashlib.sha256(_NONCE_DOMAIN + b"\x01" + entropy).hexdigest()
    return digest


request_nonce_digest = make_request_nonce_digest


def _normalise_nonce(value: Any) -> str:
    if not _is_hex_token(value, 32):
        raise _invalid_proposal()
    return value.lower()


def _validate_perception_proposal(value: Any) -> dict[str, Any]:
    """Validate and canonicalize the exact native ``PerceptionProposalV1``."""

    if isinstance(value, str):
        try:
            payload = json.loads(
                value,
                parse_constant=_reject_json_constant,
                object_pairs_hook=_pairs_without_duplicates,
            )
        except (TypeError, ValueError, json.JSONDecodeError):
            raise _invalid_proposal() from None
    else:
        payload = value
    if not isinstance(payload, Mapping) or set(payload) != set(PROPOSAL_FIELDS):
        raise _invalid_proposal()

    schema = payload.get("schema_version")
    protocol = payload.get("protocol_version")
    if not _is_raw_integer(schema) or schema != 1:
        raise _invalid_proposal()
    if not _is_raw_integer(protocol) or protocol != 1:
        raise _invalid_proposal()

    event_id = payload.get("event_id")
    turn_id = payload.get("turn_id")
    if not _is_hex_token(event_id, 16) or not _is_hex_token(turn_id, 16):
        raise _invalid_proposal()

    observed_at_ms = payload.get("observed_at_ms")
    base_revision = payload.get("base_revision")
    if not _is_raw_integer(observed_at_ms) or observed_at_ms <= 0:
        raise _invalid_proposal()
    if not _is_raw_integer(base_revision) or base_revision < 0:
        raise _invalid_proposal()

    try:
        dimensions = _validate_dimension_map(payload.get("dimensions"))
    except SemanticEstimateError:
        raise _invalid_proposal() from None
    try:
        confidence = _validate_confidence(payload.get("estimator_confidence"))
    except SemanticEstimateError:
        raise _invalid_proposal() from None
    nonce = _normalise_nonce(payload.get("request_nonce_digest"))
    return {
        "schema_version": 1,
        "event_id": event_id.lower(),
        "turn_id": turn_id.lower(),
        "observed_at_ms": observed_at_ms,
        "base_revision": base_revision,
        "dimensions": {name: dimensions[name] for name in DIMENSION_NAMES},
        "estimator_confidence": confidence,
        "protocol_version": 1,
        "request_nonce_digest": nonce,
    }


def validate_perception_proposal(value: Any) -> dict[str, Any]:
    """Fail closed for malformed or adversarial mapping implementations."""

    try:
        return _validate_perception_proposal(value)
    except SemanticProposalError:
        raise
    except Exception:
        raise SemanticProposalError() from None


validate_proposal = validate_perception_proposal


def build_perception_proposal(
    *,
    scope: ScopeTokens,
    turn: FrozenTurn,
    estimate: SemanticEstimate | Mapping[str, Any],
    base_revision: int,
    nonce_digest: str,
) -> dict[str, Any]:
    """Bind a validated local estimate to opaque turn facts for native use."""

    _validate_scope(scope)
    _validate_turn(scope, turn)
    if not _is_raw_integer(base_revision) or base_revision < 0:
        raise _invalid_proposal()
    if isinstance(estimate, SemanticEstimate):
        canonical_estimate = estimate
    else:
        try:
            canonical_estimate = parse_estimator_output(estimate)
        except SemanticEstimateError:
            raise _invalid_proposal() from None
    proposal = {
        "schema_version": 1,
        "event_id": turn.event_id,
        "turn_id": turn.turn_id,
        "observed_at_ms": turn.observed_at_ms,
        "base_revision": base_revision,
        "dimensions": {
            name: canonical_estimate.dimensions[name] for name in DIMENSION_NAMES
        },
        "estimator_confidence": canonical_estimate.estimator_confidence,
        "protocol_version": 1,
        "request_nonce_digest": nonce_digest,
    }
    return validate_perception_proposal(proposal)


def proposal_to_json(proposal: Mapping[str, Any]) -> str:
    """Return canonical closed JSON suitable for the PyO3 boundary."""

    canonical = validate_perception_proposal(proposal)
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


class SemanticEstimator:
    """One-call adapter for a request-local provider/estimator function."""

    def __init__(self, provider: EstimatorProvider) -> None:
        self._provider = provider

    async def estimate(self, request_text: str) -> SemanticEstimate:
        if not isinstance(request_text, str):
            raise SemanticEstimateError("ESTIMATOR_MALFORMED")
        provider = self._provider
        if not callable(provider):
            candidate = getattr(provider, "estimate", None)
            if not callable(candidate):
                raise SemanticEstimateError("ESTIMATOR_UNAVAILABLE")
            provider = candidate
        try:
            # Deliberately pass only the current request text.  No kwargs for
            # tools/history/context can accidentally cross this seam.
            result = provider(request_text)
            if inspect.isawaitable(result):
                result = await result
        except Exception:
            raise SemanticEstimateError("ESTIMATOR_UNAVAILABLE") from None
        try:
            return parse_estimator_output(result)
        except SemanticEstimateError as exc:
            if exc.code != "INVALID_ESTIMATE":
                raise
            raise SemanticEstimateError("ESTIMATOR_MALFORMED") from None

    async def __call__(self, request_text: str) -> SemanticEstimate:
        return await self.estimate(request_text)


async def estimate_request(
    provider: EstimatorProvider,
    request_text: str,
) -> SemanticEstimate:
    """Convenience wrapper preserving the one-argument provider boundary."""

    return await SemanticEstimator(provider).estimate(request_text)
