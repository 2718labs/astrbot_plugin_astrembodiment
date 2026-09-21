"""Typed ordinary inbound observation builder."""
from __future__ import annotations
import hashlib
import unicodedata
from .contracts import ScopeTokens

def persona_scope(scope: ScopeTokens) -> dict[str, str]:
    return {"bot_token": scope.bot_token, "persona_token": scope.persona_token}

def build_core_inbound_request(bridge, *, scope, event, message, appraisal, observed_at_ms):
    """Freeze real adapter/account/conversation/message metadata, never a revision."""
    def metadata(method, fallback=""):
        getter = getattr(event, method, None)
        return str(getter() if callable(getter) else fallback or "")

    obj = getattr(event, "message_obj", None)
    fields = (
        metadata("get_platform_id", getattr(event, "platform_id", "")),
        metadata("get_self_id", getattr(event, "bot_id", "")),
        str(getattr(event, "unified_msg_origin", "") or ""),
        metadata("get_message_id", getattr(obj, "message_id", None) or getattr(event, "message_id", "")),
    )
    if not all(fields):
        raise ValueError("CORE_EVENT_IDENTITY_UNAVAILABLE")
    identity = _framed_digest(b"ae.astrbot.event-identity.v1", *(v.encode("utf-8") for v in fields))
    encoded = unicodedata.normalize("NFC", message).encode("utf-8")
    if len(encoded) > 8192:
        raise ValueError("CORE_MESSAGE_LIMIT")
    observation = {
        "schema_version": 1, "operation_id": "00" * 16,
        "scope": persona_scope(scope), "turn_id": "00" * 16,
        "observed_at_utc_ms": observed_at_ms,
        "message_digest": _framed_digest(b"ae.core-inbound.message.v1", encoded),
        "astrbot_event_identity_digest": identity,
        "astrbot_source_digest": _framed_digest(b"ae.astrbot.source.v1", *(v.encode("utf-8") for v in fields[:3])),
        "extractor_digest": _framed_digest(b"ae.core-inbound.extractor.v1", b"astrbot-metadata-v1"),
        "confidence": 1_000_000, "relation_evidence_ref": None,
        "session_evidence_ref": None,
    }
    return bridge.compile_core_host_request_v1("inbound", {
        "schema_version": 1, "observation": observation, "appraisal": appraisal,
    })

def _framed_digest(domain: bytes, *fields: bytes) -> str:
    digest = hashlib.sha256()
    digest.update(domain)
    digest.update(b"\0")
    for field in fields:
        digest.update(len(field).to_bytes(8, "little"))
        digest.update(field)
    return digest.hexdigest()
