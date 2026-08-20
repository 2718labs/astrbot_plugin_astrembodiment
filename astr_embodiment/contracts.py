"""Python-side request-local DTOs and closed FFI payload builders.

Production authority and state contracts live in ae-contracts. These DTOs must
never become a second mutable brain: they only freeze platform facts and build
closed JSON envelopes for the Rust boundary.
"""

from __future__ import annotations

from dataclasses import dataclass


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


def build_user_stimulus_json(
    *,
    scope: ScopeTokens,
    event_id: str,
    turn_id: str,
    base_revision: int,
    observed_at_ms: int,
) -> dict:
    """Closed CanonicalEvent JSON for the first-turn barrier path.

    G0 has no semantic estimator yet (G1): the estimate is an explicit
    zero-confidence placeholder so the wire digest stays deterministic and
    the event carries no raw text at all.
    """
    return {
        "kind": "user_stimulus",
        "payload": {
            "event_id": event_id,
            "scope": scope.scope_json(),
            "causal": _causal_json(turn_id, base_revision),
            "observed_at_ms": observed_at_ms,
            "evidence": {
                "schema_version": 1,
                "dimensions": {
                    "positive": 0,
                    "affiliation": 0,
                    "harm": 0,
                    "boundary": 0,
                    "repair": 0,
                    "repetition": 0,
                    "new_information": 0,
                    "constraint_instability": 0,
                    "epistemic_conflict": 0,
                    "self_responsibility": 0,
                    "other_responsibility": 0,
                    "hostility": 0,
                    "publicness": 0,
                    "engagement": 0,
                    "rejection": 0,
                },
                "estimator_confidence": 0,
                "estimator_digest": "00" * 32,
            },
        },
    }


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
    elapsed_ms: int,
) -> dict:
    return {
        "kind": "time_advance",
        "payload": {
            "event_id": event_id,
            "scope": scope.scope_json(),
            "elapsed_ms": elapsed_ms,
        },
    }
