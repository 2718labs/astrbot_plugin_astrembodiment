"""Python-side request-local DTOs and closed FFI payload builders.

Production authority and state contracts live in ae-contracts. These DTOs must
never become a second mutable brain: they only freeze platform facts and build
closed JSON envelopes for the Rust boundary.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence


@dataclass(frozen=True, slots=True)
class ScopeTokens:
    """Opaque tokens for one commit lane. Relation may be None for 1:1 chat."""

    bot_token: str
    persona_token: str
    session_token: str
    relation_token: str | None = None

    def scope_json(self) -> dict:
        payload: dict = {
            "bot_token": self.bot_token,
            "persona_token": self.persona_token,
            # Rust's closed serde schema requires optional fields to be
            # present explicitly; ``null`` represents an absent relation.
            "relation_token": self.relation_token,
            "session_token": self.session_token,
        }
        return payload


@dataclass(frozen=True, slots=True)
class FrozenTurn:
    """One frozen platform turn: opaque ids only, no raw text."""

    scope: ScopeTokens
    turn_id: str
    event_id: str
    base_revision: int
    observed_at_ms: int


def scope_json(
    *,
    bot_token: str,
    persona_token: str,
    session_token: str,
    relation_token: str | None = None,
) -> dict:
    payload: dict = {
        "bot_token": bot_token,
        "persona_token": persona_token,
        "relation_token": relation_token,
        "session_token": session_token,
    }
    return payload


def _causal_json(
    turn_id: str,
    base_revision: int,
    action_id: str | None = None,
    delivery_id: str | None = None,
    claim_id: str | None = None,
) -> dict:
    return {
        "turn_id": turn_id,
        "action_id": action_id,
        "delivery_id": delivery_id,
        "claim_id": claim_id,
        "base_revision": base_revision,
    }


def build_alpha3_request(operation: str, request: Mapping[str, Any]) -> dict[str, Any]:
    """Build the single closed alpha3 dispatcher envelope.

    The operation string is selected by a typed bridge wrapper, never copied
    from a plugin-supplied payload.
    """
    if not operation or not isinstance(request, Mapping):
        raise ValueError("alpha3 operation and request are required")
    return {"operation": operation, "request": dict(request)}


def build_readiness_items_v1(
    *,
    global_enabled: bool,
    target_available: bool,
    secret_available: bool,
    provider_available: bool,
    send_available: bool,
) -> list[dict[str, object]]:
    """Freeze every typed Host-readiness item exactly once.

    Native recomputes consent, timezone, budget, sleep and policy readiness;
    Host marks those inputs ready only to request that authoritative check.
    """
    host_status = {
        "global_switch": global_enabled,
        "target_envelope": target_available,
        "secret_store": secret_available,
        "provider": provider_available,
        "astrbot_send": send_available,
    }
    kinds: Sequence[str] = (
        "global_switch",
        "relation_consent",
        "trusted_timezone",
        "target_envelope",
        "secret_store",
        "provider",
        "astrbot_send",
        "budget",
        "sleep_quiet_hours",
        "policy_revision",
    )
    return [
        {
            "kind": kind,
            "status": (
                "ready"
                if kind not in host_status or host_status[kind]
                else "unavailable_on_host"
            ),
            "witness_revision": 0,
        }
        for kind in kinds
    ]


def build_delivery_outcome_json(
    *,
    scope: ScopeTokens,
    event_id: str,
    turn_id: str,
    base_revision: int,
    delivered: bool,
    visible_action_digest: str,
    delivered_at_ms: int,
) -> dict:
    """Platform delivery fact: settles action facts, never a residual."""
    return {
        "kind": "delivery_outcome",
        "payload": {
            "event_id": event_id,
            "scope": scope.scope_json(),
            "causal": _causal_json(turn_id, base_revision),
            "delivered": delivered,
            "visible_action_digest": visible_action_digest,
            "delivered_at_ms": delivered_at_ms,
        },
    }


def build_time_advance_json(
    *,
    scope: ScopeTokens,
    event_id: str,
    expected_generation: int,
    frozen: Mapping[str, object],
) -> dict:
    frozen_body = dict(frozen)
    try:
        frozen_input_digest = str(frozen_body.pop("frozen_input_digest"))
    except KeyError as exc:
        raise ValueError("frozen input digest is required") from exc
    return {
        "kind": "time_advance",
        "payload": {
            "event_id": event_id,
            "scope": scope.scope_json(),
            "expected_generation": expected_generation,
            "frozen": frozen_body,
            "frozen_input_digest": frozen_input_digest,
        },
    }
