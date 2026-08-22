"""Request-local, closed SPC1 semantic estimation.

The estimator is deliberately a small adapter around one provider call.  It
accepts only the current request text, validates a fixed raw-fxp6 JSON shape,
and discards the text before a proposal can cross the native boundary.  No
history, tools, provider transcript, action contract, or free-form text is
represented by any object in this module.
"""

from __future__ import annotations

import asyncio
import hashlib
import hmac
import inspect
import json
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
_NONCE_DOMAIN = b"astr-embodiment/spc1-request-nonce-binding-v1"
_SCOPE_FIELDS = frozenset(
    {"bot_token", "persona_token", "relation_token", "session_token"}
)


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
    if type(value) is str:
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
    if type(decoded) is not dict:
        raise _invalid_estimate()
    return decoded


def _is_raw_integer(value: Any) -> bool:
    # ``bool`` is an ``int`` subclass; exact type is intentional.
    return type(value) is int


def _validate_dimension_map(value: Any) -> dict[str, int]:
    if type(value) is not dict:
        raise _invalid_estimate()
    if any(type(key) is not str for key in value) or set(value) != set(DIMENSION_NAMES):
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
    except BaseException:
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


def _canonical_hex(value: Any, bytes_len: int) -> str:
    """Decode a plain hex token and return a fresh lowercase representation."""

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


def _is_hex_token(value: Any, byte_length: int) -> bool:
    try:
        _canonical_nonzero_hex(value, byte_length)
    except (TypeError, ValueError):
        return False
    return True


def _canonical_scope(scope: ScopeTokens | Mapping[str, Any]) -> ScopeTokens:
    """Return a plain ``ScopeTokens`` with byte-canonical token strings."""

    if type(scope) is ScopeTokens:
        payload: dict[str, Any] = {
            "bot_token": scope.bot_token,
            "persona_token": scope.persona_token,
            "relation_token": scope.relation_token,
            "session_token": scope.session_token,
        }
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
    if type(turn) is not FrozenTurn:
        raise _invalid_proposal()
    try:
        turn_scope = _canonical_scope(turn.scope)
    except SemanticProposalError:
        raise
    if turn_scope != canonical_scope:
        raise _invalid_proposal()
    if type(turn.base_revision) is not int or turn.base_revision < 0:
        raise _invalid_proposal()
    if type(turn.observed_at_ms) is not int or turn.observed_at_ms <= 0:
        raise _invalid_proposal()
    try:
        event_id = _canonical_nonzero_hex(turn.event_id, 16)
        turn_id = _canonical_nonzero_hex(turn.turn_id, 16)
    except (TypeError, ValueError):
        raise _invalid_proposal() from None
    return FrozenTurn(
        scope=canonical_scope,
        turn_id=turn_id,
        event_id=event_id,
        base_revision=turn.base_revision,
        observed_at_ms=turn.observed_at_ms,
    )


def _validate_scope(scope: ScopeTokens) -> ScopeTokens:
    return _canonical_scope(scope)


def _validate_turn(scope: ScopeTokens, turn: FrozenTurn) -> FrozenTurn:
    return _canonical_turn(scope, turn)


def make_request_nonce_digest(
    scope: ScopeTokens,
    turn: FrozenTurn,
    *,
    entropy: bytes | None = None,
) -> str:
    """Create the deterministic nonce binding for one frozen request.

    ``entropy`` remains an accepted, validated keyword for source compatibility
    with the first SPC1 draft, but it is intentionally not included in the
    digest.  The bridge must be able to recompute this value from the fixed
    scope/turn facts alone; a random salt would make an otherwise closed
    proposal unverifiable at that boundary.
    """

    canonical_scope = _validate_scope(scope)
    canonical_turn = _validate_turn(canonical_scope, turn)
    if entropy is not None and (not isinstance(entropy, bytes) or not entropy):
        raise _invalid_proposal()
    # Hex tokens are byte identities; canonicalize from decoded bytes so no
    # overridable ``str.lower`` implementation can alter the bound bytes.
    scope_binding = {
        "bot_token": canonical_scope.bot_token,
        "persona_token": canonical_scope.persona_token,
        "relation_token": (
            canonical_scope.relation_token
            if canonical_scope.relation_token is not None
            else None
        ),
        "session_token": canonical_scope.session_token,
    }
    binding = {
        "scope": scope_binding,
        "event_id": canonical_turn.event_id,
        "turn_id": canonical_turn.turn_id,
        "base_revision": canonical_turn.base_revision,
        "observed_at_ms": canonical_turn.observed_at_ms,
    }
    digest = hashlib.sha256(
        _NONCE_DOMAIN + b"\x00" + _canonical_json(binding)
    ).hexdigest()
    if digest == "00" * 32:
        # Cryptographically unreachable for SHA-256, but preserve the
        # nonzero contract even if a test replaces the hash implementation.
        digest = hashlib.sha256(_NONCE_DOMAIN + b"\x01" + _canonical_json(binding)).hexdigest()
    return digest


request_nonce_digest = make_request_nonce_digest


def _normalise_nonce(value: Any) -> str:
    try:
        return _canonical_nonzero_hex(value, 32)
    except (TypeError, ValueError):
        raise _invalid_proposal() from None


def _scope_from_binding_value(value: ScopeTokens | Mapping[str, Any]) -> ScopeTokens:
    return _canonical_scope(value)


def _validate_nonce_binding(
    scope: ScopeTokens,
    payload: Mapping[str, Any],
) -> None:
    turn = FrozenTurn(
        scope=scope,
        event_id=payload["event_id"],
        turn_id=payload["turn_id"],
        base_revision=payload["base_revision"],
        observed_at_ms=payload["observed_at_ms"],
    )
    expected = make_request_nonce_digest(scope, turn)
    actual = payload["request_nonce_digest"]
    if not hmac.compare_digest(actual, expected):
        raise _invalid_proposal()


def _validate_perception_proposal(
    value: Any,
    *,
    scope: ScopeTokens | Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Validate and canonicalize the exact native ``PerceptionProposalV1``."""

    if type(value) is str:
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
    if type(payload) is not dict or any(type(key) is not str for key in payload):
        raise _invalid_proposal()
    if set(payload) != set(PROPOSAL_FIELDS):
        raise _invalid_proposal()

    schema = payload.get("schema_version")
    protocol = payload.get("protocol_version")
    if not _is_raw_integer(schema) or schema != 1:
        raise _invalid_proposal()
    if not _is_raw_integer(protocol) or protocol != 1:
        raise _invalid_proposal()

    try:
        event_id = _canonical_nonzero_hex(payload.get("event_id"), 16)
        turn_id = _canonical_nonzero_hex(payload.get("turn_id"), 16)
    except (TypeError, ValueError):
        raise _invalid_proposal() from None

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
    canonical = {
        "schema_version": 1,
        "event_id": event_id,
        "turn_id": turn_id,
        "observed_at_ms": observed_at_ms,
        "base_revision": base_revision,
        "dimensions": {name: dimensions[name] for name in DIMENSION_NAMES},
        "estimator_confidence": confidence,
        "protocol_version": 1,
        "request_nonce_digest": nonce,
    }
    if scope is not None:
        _validate_nonce_binding(_scope_from_binding_value(scope), canonical)
    return canonical


def validate_perception_proposal(
    value: Any,
    *,
    scope: ScopeTokens | Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    """Fail closed for malformed or adversarial mapping implementations."""

    try:
        return _validate_perception_proposal(value, scope=scope)
    except SemanticProposalError:
        raise
    except BaseException:
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

    canonical_scope = _validate_scope(scope)
    canonical_turn = _validate_turn(canonical_scope, turn)
    if not _is_raw_integer(base_revision) or base_revision < 0:
        raise _invalid_proposal()
    if base_revision != canonical_turn.base_revision:
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
        "event_id": canonical_turn.event_id,
        "turn_id": canonical_turn.turn_id,
        "observed_at_ms": canonical_turn.observed_at_ms,
        "base_revision": base_revision,
        "dimensions": {
            name: canonical_estimate.dimensions[name] for name in DIMENSION_NAMES
        },
        "estimator_confidence": canonical_estimate.estimator_confidence,
        "protocol_version": 1,
        "request_nonce_digest": nonce_digest,
    }
    return validate_perception_proposal(proposal, scope=canonical_scope)


def proposal_to_json(
    proposal: Mapping[str, Any],
    *,
    scope: ScopeTokens | Mapping[str, Any] | None = None,
) -> str:
    """Return canonical closed JSON suitable for the PyO3 boundary."""

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
        except asyncio.CancelledError:
            raise
        except BaseException:
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
