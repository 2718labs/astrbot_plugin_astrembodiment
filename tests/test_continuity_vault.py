from __future__ import annotations

from pathlib import Path

from astr_embodiment.bridge import NativeBridge
from astr_embodiment.contracts import ScopeTokens, build_user_stimulus_json
from astr_embodiment.persona_genesis import (
    PERSONA_COMPILER_SCHEMA,
    PersonaSourceSnapshot,
    _ALLOSTATIC_NAMES,
    _EPISTEMIC_NAMES,
    _EXPRESSION_NAMES,
    _SOCIAL_NAMES,
    _TRAIT_NAMES,
    build_closed_request,
    validate_proposal,
)


def _scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="66" * 16,
        persona_token="77" * 16,
        relation_token=None,
        session_token="88" * 16,
    )


def _request(scope: ScopeTokens) -> dict[str, object]:
    source = PersonaSourceSnapshot.freeze(
        persona_id="d1-vault-persona",
        persona={"prompt": "aggregate-only continuity fixture"},
        selection="conversation",
    )
    proposal = validate_proposal(
        {
            "schema": PERSONA_COMPILER_SCHEMA,
            "traits": {
                name: {"value": 0.5, "confidence": 0.5} for name in _TRAIT_NAMES
            },
            "expression": {name: 0.5 for name in _EXPRESSION_NAMES},
            "allostasis": {name: 0.5 for name in _ALLOSTATIC_NAMES},
            "epistemic": {name: 0.5 for name in _EPISTEMIC_NAMES},
            "social": {name: 0.5 for name in _SOCIAL_NAMES},
        }
    )
    return build_closed_request(
        scope=scope,
        source=source,
        proposal=proposal,
        selection="conversation",
        compiler_protocol_digest="00" * 32,
        compiler_model_digest="00" * 32,
        formula_digest="00" * 32,
        incarnation_nonce="b2" * 32,
        observed_at_ms=1_700_000_000_000,
    )


def test_real_continuity_vault_reopens_and_deduplicates_without_new_genesis(
    tmp_path: Path,
) -> None:
    scope = _scope()
    event = build_user_stimulus_json(
        scope=scope,
        event_id="99" * 16,
        turn_id="aa" * 16,
        base_revision=0,
        observed_at_ms=1_700_000_000_001,
    )
    first = NativeBridge()
    first.open(str(tmp_path))
    try:
        first.ensure_genesis(_request(scope))
        committed = first.apply_event(scope.scope_json(), event)
        assert committed["revision"] == 1
        assert committed["deduplicated"] is False
        digest = committed["context_summary"]["summary_digest"]
    finally:
        first.close()

    reopened = NativeBridge()
    reopened.open(str(tmp_path))
    try:
        duplicate = reopened.apply_event(scope.scope_json(), event)
        replay = reopened.verify_replay(scope.scope_json())
        assert duplicate["deduplicated"] is True
        assert duplicate["revision"] == 1
        assert duplicate["context_summary"]["summary_digest"] == digest
        assert replay["ok"] is True
        assert replay["checked"] == 1
    finally:
        reopened.close()


def test_real_vault_rebirth_challenge_keeps_the_raw_nonce_out_of_durable_files(
    tmp_path: Path,
) -> None:
    scope = _scope()
    bridge = NativeBridge()
    bridge.open(str(tmp_path))
    try:
        genesis = bridge.ensure_genesis(_request(scope))
        receipt = genesis["receipt"]
        assert isinstance(receipt, dict)
        incarnation_id = receipt["incarnation_id"]
        assert isinstance(incarnation_id, str) and len(incarnation_id) == 64
        challenge = bridge.prepare_rebirth_v1(
            {
                "scope": scope.scope_json(),
                "expected_incarnation_id": incarnation_id,
                "expected_revision": 0,
                "action": "REBIRTH",
            }
        )
        assert challenge["state"] == "CONFIRMATION_PENDING"
        raw_nonce = challenge["request_nonce"]
        assert isinstance(raw_nonce, str) and len(raw_nonce) == 64
    finally:
        bridge.close()

    durable_bytes = b"".join(
        path.read_bytes()
        for path in tmp_path.rglob("*")
        if path.is_file()
    )
    assert raw_nonce.encode() not in durable_bytes
