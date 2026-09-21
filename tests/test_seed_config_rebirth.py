from __future__ import annotations

import asyncio

import main
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


class DeferredSeedConfigBridge:
    loaded = True

    def __init__(self) -> None:
        self.requests: list[dict[str, object]] = []

    def reconcile_seed_config_v1(self, request: dict[str, object]) -> dict[str, object]:
        self.requests.append(dict(request))
        return {
            "schema": "astrembodiment.seed-config-result.v1",
            "state": "DEFERRED",
            "writeback": None,
            "before_revision": None,
            "after_revision": None,
            "reason": "SEED_CONFIG_OBSERVATION_DEFERRED",
        }


class FailingSavingConfig(SavingConfig):
    def __init__(self, secret_failure: str, **values: object) -> None:
        super().__init__(**values)
        self._secret_failure = secret_failure

    def save_config(self) -> None:
        raise RuntimeError(self._secret_failure)


class WarningRecorder:
    def __init__(self) -> None:
        self.messages: list[str] = []

    def warning(self, message: str, *args: object, **_kwargs: object) -> None:
        self.messages.append(message % args if args else message)


class RebirthOutbox:
    def __init__(self) -> None:
        self.calls: list[tuple[ScopeTokens, str | None]] = []

    async def cancel_rebirth(
        self,
        *,
        scope: ScopeTokens,
        old_incarnation_id: str | None,
    ) -> int:
        self.calls.append((scope, old_incarnation_id))
        return 1


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

    result = asyncio.run(plugin._reconcile_seed_config_v1(scope, origin="STARTUP_READ"))

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


def test_rebirth_scrubs_outbox_before_forgetting_process_local_scope() -> None:
    config = SavingConfig(seed_code="", seed_mirror_guard_v1="a" * 64)
    bridge = SeedConfigBridge()
    outbox = RebirthOutbox()
    plugin = AstrEmbodimentPlugin(None, config)
    plugin._bridge = bridge  # type: ignore[assignment]
    plugin._semantic_outbox = outbox  # type: ignore[assignment]
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )
    plugin._seed_receipts[scope.persona_token] = {"incarnation_id": "AE-I1-OLD"}

    result = asyncio.run(plugin._reconcile_seed_config_v1(scope, origin="STARTUP_READ"))

    assert result["state"] == "REBIRTH_COMMITTED"
    assert outbox.calls == [(scope, "AE-I1-OLD")]
    assert scope.persona_token not in plugin._seed_receipts


def test_startup_observation_is_captured_once_not_relabelled_live_mutation() -> None:
    config = SavingConfig(
        seed_code="AE-S1-PERSISTED",
        seed_mirror_guard_v1="a" * 64,
    )
    bridge = DeferredSeedConfigBridge()
    plugin = AstrEmbodimentPlugin(None, config)
    plugin._bridge = bridge  # type: ignore[assignment]
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )

    # A later in-process mutation is not proof of a startup read or user save.
    config["seed_code"] = ""

    asyncio.run(plugin._consume_seed_config_startup_v1(scope))
    asyncio.run(plugin._consume_seed_config_startup_v1(scope))

    assert len(bridge.requests) == 1
    assert bridge.requests[0]["origin"] == "STARTUP_READ"
    assert bridge.requests[0]["observation"] == "PRESENT_NONEMPTY"
    assert bridge.requests[0]["seed_code"] == "AE-S1-PERSISTED"


def test_unavailable_package_epoch_defers_without_calling_native(
    monkeypatch: object,
) -> None:
    monkeypatch.setattr(  # type: ignore[attr-defined]
        AstrEmbodimentPlugin,
        "_compute_seed_config_package_epoch_v1",
        staticmethod(lambda: None),
    )
    config = SavingConfig(seed_code="", seed_mirror_guard_v1="a" * 64)
    bridge = DeferredSeedConfigBridge()
    plugin = AstrEmbodimentPlugin(None, config)
    plugin._bridge = bridge  # type: ignore[assignment]
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )

    result = asyncio.run(plugin._consume_seed_config_startup_v1(scope))

    assert result is not None
    assert result["state"] == "DEFERRED"
    assert bridge.requests == []


def test_writeback_save_failure_logs_no_raw_seed_capability_or_host_error(
    monkeypatch: object,
) -> None:
    raw_seed = "AE-S1-NATIVE-REPLACEMENT"
    raw_guard = "b" * 64
    raw_token = "c" * 64
    config = FailingSavingConfig(
        f"host failure includes {raw_seed} {raw_guard} {raw_token}",
        seed_code="",
        seed_mirror_guard_v1="a" * 64,
    )
    bridge = SeedConfigBridge()
    recorder = WarningRecorder()
    monkeypatch.setattr(main, "logger", recorder)  # type: ignore[attr-defined]
    plugin = AstrEmbodimentPlugin(None, config)
    plugin._bridge = bridge  # type: ignore[assignment]
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )

    result = asyncio.run(plugin._reconcile_seed_config_v1(scope, origin="STARTUP_READ"))

    assert result["state"] == "REBIRTH_COMMITTED"
    assert bridge.acks == []
    emitted = "\n".join(recorder.messages)
    for raw in (raw_seed, raw_guard, raw_token, "host failure includes"):
        assert raw not in emitted
