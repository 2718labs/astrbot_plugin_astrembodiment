"""Closed, content-free context binding for the V3 semantic estimator.

Only the current request text is ever sent to the provider.  This module
adapts the already-validated native D1 aggregate into a separate host-only
summary and binds it to one opaque request nonce.
"""

from __future__ import annotations

import hashlib
import hmac
import json
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any

from .contracts import ScopeTokens

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
CONTEXT_SUMMARY_V1_SCHEMA = "astr-embodiment.context-summary.v1"

_BINDING_FIELDS = frozenset(
    {
        "relation_scope_token",
        "base_continuum_revision",
        "summary_revision",
        "summary_digest",
        "estimator_formula_digest",
        "request_nonce_digest",
    }
)
_SUMMARY_FIELDS = frozenset({"schema", "binding", "dimensions"})
_SUMMARY_DIMENSION_FIELDS = frozenset({"state", "intensity_fxp6"})
_SUMMARY_STATES = frozenset({"PRESENT", "ABSENT", "UNAVAILABLE"})


class ContextBindingError(ValueError):
    """Fixed, non-echoing validation error at the host-only boundary."""

    def __init__(self, code: str = "INVALID_CONTEXT_BINDING") -> None:
        super().__init__(code)
        self.code = code


def _invalid_binding() -> ContextBindingError:
    return ContextBindingError()


def _binding_mismatch() -> ContextBindingError:
    return ContextBindingError("CONTEXT_BINDING_MISMATCH")


def _canonical_hex(value: Any, byte_length: int, *, nonzero: bool) -> str:
    if type(value) is not str or len(value) != byte_length * 2:
        raise _invalid_binding()
    try:
        decoded = bytes.fromhex(value)
    except (TypeError, ValueError):
        raise _invalid_binding() from None
    if len(decoded) != byte_length or (nonzero and not any(decoded)):
        raise _invalid_binding()
    return decoded.hex()


def _canonical_revision(value: Any) -> int:
    if type(value) is not int or value < 0:
        raise _invalid_binding()
    return value


@dataclass(frozen=True, slots=True)
class ContextBindingV1:
    """Opaque facts that bind one historical summary to one estimator call."""

    relation_scope_token: str
    base_continuum_revision: int
    summary_revision: int
    summary_digest: str
    estimator_formula_digest: str
    request_nonce_digest: str

    def __post_init__(self) -> None:
        object.__setattr__(
            self,
            "relation_scope_token",
            _canonical_hex(self.relation_scope_token, 16, nonzero=True),
        )
        object.__setattr__(
            self,
            "base_continuum_revision",
            _canonical_revision(self.base_continuum_revision),
        )
        object.__setattr__(self, "summary_revision", _canonical_revision(self.summary_revision))
        object.__setattr__(
            self,
            "summary_digest",
            _canonical_hex(self.summary_digest, 32, nonzero=True),
        )
        object.__setattr__(
            self,
            "estimator_formula_digest",
            _canonical_hex(self.estimator_formula_digest, 32, nonzero=False),
        )
        object.__setattr__(
            self,
            "request_nonce_digest",
            _canonical_hex(self.request_nonce_digest, 32, nonzero=True),
        )

    def as_json(self) -> dict[str, int | str]:
        return {
            "relation_scope_token": self.relation_scope_token,
            "base_continuum_revision": self.base_continuum_revision,
            "summary_revision": self.summary_revision,
            "summary_digest": self.summary_digest,
            "estimator_formula_digest": self.estimator_formula_digest,
            "request_nonce_digest": self.request_nonce_digest,
        }

    @classmethod
    def from_json(cls, value: Any) -> ContextBindingV1:
        if type(value) is cls:
            return value
        if type(value) is not dict or set(value) != _BINDING_FIELDS:
            raise _invalid_binding()
        try:
            return cls(
                relation_scope_token=value["relation_scope_token"],
                base_continuum_revision=value["base_continuum_revision"],
                summary_revision=value["summary_revision"],
                summary_digest=value["summary_digest"],
                estimator_formula_digest=value["estimator_formula_digest"],
                request_nonce_digest=value["request_nonce_digest"],
            )
        except ContextBindingError:
            raise
        except BaseException:
            raise _invalid_binding() from None


def _canonical_summary_dimensions(value: Any) -> dict[str, dict[str, int | str | None]]:
    if type(value) is not dict or set(value) != set(DIMENSION_NAMES):
        raise _invalid_binding()
    dimensions: dict[str, dict[str, int | str | None]] = {}
    for name in DIMENSION_NAMES:
        slot = value[name]
        if type(slot) is not dict or set(slot) != _SUMMARY_DIMENSION_FIELDS:
            raise _invalid_binding()
        state = slot["state"]
        intensity = slot["intensity_fxp6"]
        if type(state) is not str or state not in _SUMMARY_STATES:
            raise _invalid_binding()
        if state == "PRESENT":
            if type(intensity) is not int or not 1 <= intensity <= FXP6_SCALE:
                raise _invalid_binding()
        elif state == "ABSENT":
            if type(intensity) is not int or intensity != 0:
                raise _invalid_binding()
        elif intensity is not None:
            raise _invalid_binding()
        dimensions[name] = {"state": state, "intensity_fxp6": intensity}
    return dimensions


def _canonical_summary_json(dimensions: Mapping[str, Any]) -> bytes:
    try:
        canonical = _canonical_summary_dimensions(dimensions)
        return json.dumps(
            {"schema": CONTEXT_SUMMARY_V1_SCHEMA, "dimensions": canonical},
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
    except ContextBindingError:
        raise
    except BaseException:
        raise _invalid_binding() from None


def context_summary_digest(dimensions: Mapping[str, Any]) -> str:
    """Return the digest of an adapted, content-free V3 summary only."""

    return hashlib.sha256(_canonical_summary_json(dimensions)).hexdigest()


def _bindings_match(left: ContextBindingV1, right: ContextBindingV1) -> bool:
    return (
        hmac.compare_digest(left.relation_scope_token, right.relation_scope_token)
        and left.base_continuum_revision == right.base_continuum_revision
        and left.summary_revision == right.summary_revision
        and hmac.compare_digest(left.summary_digest, right.summary_digest)
        and hmac.compare_digest(left.estimator_formula_digest, right.estimator_formula_digest)
        and hmac.compare_digest(left.request_nonce_digest, right.request_nonce_digest)
    )


def build_context_summary(
    *,
    binding: ContextBindingV1 | Mapping[str, Any],
    dimensions: Mapping[str, Any],
) -> dict[str, Any]:
    """Build one closed host-only historical summary."""

    canonical_binding = ContextBindingV1.from_json(binding)
    canonical_dimensions = _canonical_summary_dimensions(dimensions)
    if not hmac.compare_digest(
        canonical_binding.summary_digest, context_summary_digest(canonical_dimensions)
    ):
        raise _binding_mismatch()
    return {
        "schema": CONTEXT_SUMMARY_V1_SCHEMA,
        "binding": canonical_binding.as_json(),
        "dimensions": canonical_dimensions,
    }


def validate_context_summary(
    value: Any,
    *,
    expected_binding: ContextBindingV1 | Mapping[str, Any],
) -> dict[str, Any]:
    """Fail closed unless all summary content and opaque bindings match."""

    try:
        expected = ContextBindingV1.from_json(expected_binding)
        if type(value) is not dict or set(value) != _SUMMARY_FIELDS:
            raise _invalid_binding()
        if value["schema"] != CONTEXT_SUMMARY_V1_SCHEMA:
            raise _invalid_binding()
        candidate = ContextBindingV1.from_json(value["binding"])
        dimensions = _canonical_summary_dimensions(value["dimensions"])
        if not _bindings_match(candidate, expected):
            raise _binding_mismatch()
        if not hmac.compare_digest(candidate.summary_digest, context_summary_digest(dimensions)):
            raise _binding_mismatch()
        return {
            "schema": CONTEXT_SUMMARY_V1_SCHEMA,
            "binding": candidate.as_json(),
            "dimensions": dimensions,
        }
    except ContextBindingError:
        raise
    except BaseException:
        raise _invalid_binding() from None


def adapt_native_context_summary_v1(
    summary: Any,
    *,
    scope: ScopeTokens,
    nonce_digest: str,
    estimator_formula_digest: str,
) -> dict[str, Any]:
    """Adapt the validated native D1 aggregate into the V3 host-only summary.

    The source validator remains in :mod:`bridge`; importing it lazily avoids a
    bridge → estimator → context-binding import cycle.  No native digest or
    native metadata is forwarded: V3 receives only the fixed 15-value summary.
    """

    try:
        from .bridge import validate_context_summary_payload

        native_summary = validate_context_summary_payload(summary)
        if type(scope) is not ScopeTokens:
            raise _invalid_binding()
        values = native_summary["dimensions_ema_fxp6"]
        if type(values) is not list or len(values) != len(DIMENSION_NAMES):
            raise _invalid_binding()
        dimensions = {
            name: (
                {"state": "PRESENT", "intensity_fxp6": value}
                if type(value) is int and value > 0
                else {"state": "ABSENT", "intensity_fxp6": 0}
            )
            for name, value in zip(DIMENSION_NAMES, values, strict=True)
        }
        canonical_dimensions = _canonical_summary_dimensions(dimensions)
        relation_scope_token = scope.relation_token or scope.persona_token
        binding = ContextBindingV1(
            relation_scope_token=relation_scope_token,
            base_continuum_revision=native_summary["source_continuum_revision"],
            summary_revision=native_summary["summary_revision"],
            summary_digest=context_summary_digest(canonical_dimensions),
            estimator_formula_digest=estimator_formula_digest,
            request_nonce_digest=nonce_digest,
        )
        return build_context_summary(binding=binding, dimensions=canonical_dimensions)
    except ContextBindingError:
        raise
    except BaseException:
        raise _invalid_binding() from None
