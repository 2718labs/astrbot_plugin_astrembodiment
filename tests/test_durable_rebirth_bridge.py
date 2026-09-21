from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from astr_embodiment.bridge import NativeBridge, NativeCoreError
from astr_embodiment.contracts import ScopeTokens
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
        bot_token="d1" * 16,
        persona_token="d2" * 16,
        relation_token=None,
        session_token="d3" * 16,
    )


def _genesis_request(scope: ScopeTokens) -> dict[str, object]:
    source = PersonaSourceSnapshot.freeze(
        persona_id="d15-durable-rebirth-persona",
        persona={"prompt": "durable rebirth bridge fixture"},
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
        compiler_protocol_digest="10" * 32,
        compiler_model_digest="20" * 32,
        formula_digest="30" * 32,
        incarnation_nonce="40" * 32,
        observed_at_ms=1_700_000_001_000,
    )


def _prepare_payload(
    scope: ScopeTokens, genesis: dict[str, Any], *, action: str = "REBIRTH"
) -> dict[str, object]:
    receipt = genesis["receipt"]
    assert isinstance(receipt, dict)
    incarnation_id = receipt["incarnation_id"]
    assert isinstance(incarnation_id, str) and len(incarnation_id) == 64
    return {
        "scope": scope.scope_json(),
        "expected_incarnation_id": incarnation_id,
        "expected_revision": 0,
        "action": action,
    }


def _confirm_payload(
    prepare: dict[str, object], challenge: dict[str, object], *, confirmed: bool
) -> dict[str, object]:
    nonce = challenge["request_nonce"]
    assert isinstance(nonce, str) and len(nonce) == 64
    return {
        **prepare,
        "request_nonce": nonce,
        "confirmed": confirmed,
    }


def _open_seeded_native(
    tmp_path: Path,
) -> tuple[NativeBridge, ScopeTokens, dict[str, Any]]:
    scope = _scope()
    bridge = NativeBridge()
    bridge.open(str(tmp_path))
    genesis = bridge.ensure_genesis(_genesis_request(scope))
    return bridge, scope, genesis


def test_bridge_does_not_default_false_or_missing_confirmation_to_true(
    tmp_path: Path,
) -> None:
    bridge, scope, genesis = _open_seeded_native(tmp_path)
    try:
        prepare = _prepare_payload(scope, genesis)
        challenge = bridge.prepare_rebirth_v1(prepare)
        assert challenge["state"] == "CONFIRMATION_PENDING"
        for invalid in (
            _confirm_payload(prepare, challenge, confirmed=False),
            {
                key: value
                for key, value in _confirm_payload(
                    prepare, challenge, confirmed=True
                ).items()
                if key != "confirmed"
            },
        ):
            with pytest.raises(NativeCoreError, match="^REBIRTH_CONFIRMATION_REQUIRED"):
                bridge.confirm_rebirth_v1(invalid)
    finally:
        bridge.close()


def test_bridge_returns_committed_then_replayed_envelopes_after_reopen(
    tmp_path: Path,
) -> None:
    bridge, scope, genesis = _open_seeded_native(tmp_path)
    prepare = _prepare_payload(scope, genesis)
    try:
        challenge = bridge.prepare_rebirth_v1(prepare)
        committed = bridge.confirm_rebirth_v1(
            _confirm_payload(prepare, challenge, confirmed=True)
        )
        assert committed["state"] == "COMMITTED"
        receipt = committed["receipt"]
        assert receipt["after_revision"] == 0
        assert receipt["audit_time_ms"] > 0
        assert "request_nonce" not in committed
        assert "request_nonce" not in receipt
    finally:
        bridge.close()

    reopened = NativeBridge()
    reopened.open(str(tmp_path))
    try:
        replayed = reopened.confirm_rebirth_v1(
            _confirm_payload(prepare, challenge, confirmed=True)
        )
        assert replayed["state"] == "REPLAYED"
        assert replayed["receipt"] == committed["receipt"]
        assert "request_nonce" not in replayed
        assert "request_nonce" not in replayed["receipt"]
        assert (
            json.dumps(replayed, sort_keys=True).count(challenge["request_nonce"]) == 0
        )
    finally:
        reopened.close()
