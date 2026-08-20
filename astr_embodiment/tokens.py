"""Opaque token derivation for the Rust boundary.

Raw platform ids (bot id, persona id, session keys) never enter Rust state.
Stable, hash-derived opaque tokens are computed here instead: the same
platform id always maps to the same token, so bindings and commit lanes
survive restarts.
"""

from __future__ import annotations

import hashlib

BOT_TOKEN_DOMAIN = b"ae.bot-token.v1"
PERSONA_TOKEN_DOMAIN = b"ae.persona-token.v1"
SESSION_TOKEN_DOMAIN = b"ae.session-token.v1"
RELATION_TOKEN_DOMAIN = b"ae.relation-token.v1"
TURN_ID_DOMAIN = b"ae.turn-id.v1"
EVENT_ID_DOMAIN = b"ae.event-id.v1"


def derive_token(domain: bytes, seed: str) -> str:
    """32 lowercase hex chars = 16 opaque bytes."""
    digest = hashlib.sha256(domain + b"\x00" + seed.encode("utf-8")).digest()
    return digest[:16].hex()


def bot_token(bot_id: str) -> str:
    return derive_token(BOT_TOKEN_DOMAIN, bot_id)


def persona_token(persona_id: str) -> str:
    return derive_token(PERSONA_TOKEN_DOMAIN, persona_id)


def session_token(session_key: str) -> str:
    return derive_token(SESSION_TOKEN_DOMAIN, session_key)


def relation_token(relation_key: str) -> str:
    return derive_token(RELATION_TOKEN_DOMAIN, relation_key)


def turn_id(session_key: str, seq: int) -> str:
    return derive_token(TURN_ID_DOMAIN, f"{session_key}#{seq}")


def event_id(seed: str) -> str:
    return derive_token(EVENT_ID_DOMAIN, seed)
