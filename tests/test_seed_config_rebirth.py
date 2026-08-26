from __future__ import annotations

import asyncio

from astr_embodiment.contracts import ScopeTokens
from main import AstrEmbodimentPlugin


class SavingConfig(dict[str, object]):
    def __init__(self, **values: object) -> None:
        super().__init__(**values)
        self.save_calls = 0

    def save_config(self) -> None:
        self.save_calls += 1


class SeedConfigBridge:
    loaded = True

    def __init__(self) -> None:
        self.requests: list[dict[str, object]] = []
        self.acks: list[dict[str, object]] = []

    def reconcile_seed_config_v1(self, request: dict[str, object]) -> dict[str, object]:
        self.requests.append(dict(request))
        return {
            "schema": "astrembodiment.seed-config-result.v1",
            "state": "REBIRTH_COMMITTED",
            "writeback": {
                "seed_code": "AE-S1-NATIVE-REPLACEMENT",
                "mirror_guard": "b" * 64,
                "writeback_token": "c" * 64,
            },
            "before_revision": 7,
            "after_revision": 0,
            "reason": "SEED_CLEAR_REBIRTH_COMMITTED",
        }

    def ack_seed_config_writeback_v1(
        self, request: dict[str, object]
    ) -> dict[str, object]:
        self.acks.append(dict(request))
        return {
            "schema": "astrembodiment.seed-config-ack.v1",
            "state": "MIRROR_ACTIVE",
        }

    def confirm_rebirth_v1(self, _request: object) -> None:
        raise AssertionError("seed clear must not use the manual confirmation ABI")


def test_startup_empty_seed_uses_only_native_seed_clear_reconciliation() -> None:
    config = SavingConfig(seed_code="", seed_mirror_guard_v1="a" * 64)
    bridge = SeedConfigBridge()
    plugin = AstrEmbodimentPlugin(None, config)
    plugin._bridge = bridge  # type: ignore[assignment]
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )

    result = asyncio.run(
        plugin._reconcile_seed_config_v1(scope, origin="STARTUP_READ")
    )

    assert result["state"] == "REBIRTH_COMMITTED"
    assert bridge.requests == [
        {
            "schema": "astrembodiment.seed-config-observation.v1",
            "scope": {
                "bot_token": "10" * 16,
                "persona_token": "20" * 16,
                "relation_token": None,
            },
            "observation": "PRESENT_EMPTY",
            "origin": "STARTUP_READ",
            "mirror_guard": "a" * 64,
            "previous_observation": None,
            "package_epoch": plugin._seed_config_package_epoch_v1(),
            "config_schema_version": 1,
            "host_config_revision": 0,
        }
    ]
    assert config["seed_code"] == "AE-S1-NATIVE-REPLACEMENT"
    assert config["seed_mirror_guard_v1"] == "b" * 64
    assert config.save_calls == 1
    assert bridge.acks == [
        {
            "schema": "astrembodiment.seed-config-writeback-ack.v1",
            "scope": {
                "bot_token": "10" * 16,
                "persona_token": "20" * 16,
                "relation_token": None,
            },
            "writeback_token": "c" * 64,
            "write_succeeded": True,
            "host_config_revision": 0,
        }
    ]
