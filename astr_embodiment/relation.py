"""Canonical, display-name-independent relation identities."""

from __future__ import annotations

import hashlib
import unicodedata
from typing import Literal

RELATION_TOKEN_DOMAIN = b"ae.relation-token.v1"


class RelationBindingError(ValueError):
    """A verified inbound target could not be bound without guessing."""


def canonical_relation_key(
    *,
    platform_id: str,
    bot_id: str,
    persona_id: str,
    target_kind: Literal["private", "group"],
    target_id: str,
) -> bytes:
    fields = ("v1", platform_id, bot_id, persona_id, target_kind, target_id)
    if target_kind not in {"private", "group"} or any(not value for value in fields):
        raise RelationBindingError("incomplete relation identity")
    out = bytearray()
    for value in fields:
        encoded = unicodedata.normalize("NFC", value).encode("utf-8")
        out.extend(len(encoded).to_bytes(4, "big"))
        out.extend(encoded)
    return bytes(out)


def relation_token_from_key(key: bytes) -> str:
    digest = hashlib.sha256(RELATION_TOKEN_DOMAIN + b"\x00" + key).digest()
    return digest[:16].hex()
