from __future__ import annotations

import pytest

from astr_embodiment.bridge import NativeCoreError
from astr_embodiment.contracts import ScopeTokens
from astr_embodiment.coordinator import GenesisCoordinator
from main import AstrEmbodimentPlugin


class RebirthBridge:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, object]]] = []
        self.nonce = "ab" * 32
        self.receipt = {
            "receipt_id": "cd" * 32,
            "action": "REBIRTH",
            "scope_token_short": "scope-short",
            "request_nonce_digest": "ef" * 32,
            "parent_incarnation_short": "parent-short",
            "child_incarnation_short": "child-short",
            "before_revision": 14,
            "after_revision": 0,
            "outcome": "COMMITTED",
            "audit_time_ms": 1_700_000_000_000,
        }

    def prepare_rebirth_v1(self, payload: dict[str, object]) -> dict[str, object]:
        self.calls.append(("prepare_rebirth_v1", dict(payload)))
        return {
            "schema": "astrembodiment.rebirth-prepare.v1",
            "state": "CONFIRMATION_PENDING",
            "request_nonce": self.nonce,
            "request_nonce_digest": "ef" * 32,
            "binding_digest": "01" * 32,
        }

    def confirm_rebirth_v1(self, payload: dict[str, object]) -> dict[str, object]:
        self.calls.append(("confirm_rebirth_v1", dict(payload)))
        if payload.get("confirmed") is not True:
            raise NativeCoreError(
                "REBIRTH_CONFIRMATION_REQUIRED", "native confirmation required"
            )
        if payload.get("request_nonce") != self.nonce:
            raise NativeCoreError("REBIRTH_NONCE_CONFLICT", "nonce mismatch")
        if payload.get("expected_revision") != 14:
            raise NativeCoreError("REBIRTH_FENCE_STALE", "revision mismatch")
        state = "COMMITTED" if len(self.calls) == 4 else "REPLAYED"
        return {
            "schema": "astrembodiment.rebirth-response.v1",
            "state": state,
            "receipt": dict(self.receipt),
        }


def test_rebirth_is_only_a_closed_two_step_d1_5_forwarding_gate() -> None:
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )
    bridge = RebirthBridge()
    plugin = AstrEmbodimentPlugin(None, {})
    plugin._bridge = bridge  # type: ignore[assignment]
    plugin._coordinator = GenesisCoordinator(bridge)  # type: ignore[arg-type]
    plugin._revisions[scope.persona_token] = 14

    challenge = plugin.request_rebirth(
        scope=scope,
        expected_incarnation_id="40" * 32,
        expected_revision=14,
        action="REBIRTH",
    )
    assert challenge["state"] == "CONFIRMATION_PENDING"
    assert bridge.calls == [
        (
            "prepare_rebirth_v1",
            {
                "scope": scope.scope_json(),
                "expected_incarnation_id": "40" * 32,
                "expected_revision": 14,
                "action": "REBIRTH",
            },
        )
    ]

    with pytest.raises(NativeCoreError, match="^REBIRTH_CONFIRMATION_REQUIRED"):
        plugin.confirm_rebirth_payload(
            scope=scope,
            expected_incarnation_id="40" * 32,
            expected_revision=14,
            request_nonce=challenge["request_nonce"],
            action="REBIRTH",
        )
    assert "confirmed" not in bridge.calls[-1][1]

    with pytest.raises(NativeCoreError, match="^REBIRTH_CONFIRMATION_REQUIRED"):
        plugin.confirm_rebirth_payload(
            scope=scope,
            expected_incarnation_id="40" * 32,
            expected_revision=14,
            request_nonce=challenge["request_nonce"],
            action="REBIRTH",
            confirmed=False,
        )
    assert bridge.calls[-1][1]["confirmed"] is False

    committed = plugin.confirm_rebirth_payload(
        scope=scope,
        expected_incarnation_id="40" * 32,
        expected_revision=14,
        request_nonce=challenge["request_nonce"],
        action="REBIRTH",
        confirmed=True,
    )
    assert committed["state"] == "COMMITTED"
    assert committed["receipt"] == bridge.receipt
    assert bridge.calls[-1][1] == {
        "scope": scope.scope_json(),
        "expected_incarnation_id": "40" * 32,
        "expected_revision": 14,
        "request_nonce": challenge["request_nonce"],
        "action": "REBIRTH",
        "confirmed": True,
    }
    assert scope.persona_token not in plugin._revisions
    local_state = {
        key: value
        for key, value in plugin.__dict__.items()
        if key not in {"_bridge", "_coordinator"}
    }
    assert bridge.nonce not in repr(local_state)

    replayed = plugin.confirm_rebirth_payload(
        scope=scope,
        expected_incarnation_id="40" * 32,
        expected_revision=14,
        request_nonce=challenge["request_nonce"],
        action="REBIRTH",
        confirmed=True,
    )
    assert replayed["state"] == "REPLAYED"
    assert replayed["receipt"] == committed["receipt"]
