from __future__ import annotations

import asyncio
import hashlib
import importlib.util
import inspect
import json
import shutil
import sys
import zipfile
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import astr_embodiment.bridge as bridge_module  # noqa: E402
from astr_embodiment.contracts import FrozenTurn, ScopeTokens  # noqa: E402
from astr_embodiment.persona_genesis import PersonaGenesisError  # noqa: E402
from astr_embodiment.semantic_estimator import (  # noqa: E402
    DIMENSION_NAMES,
    FXP6_SCALE,
    parse_estimator_output,
)
from astr_embodiment.tokens import event_id, turn_id  # noqa: E402
import main as main_module  # noqa: E402
from main import AstrEmbodimentPlugin  # noqa: E402


class FakeConfig(dict):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.save_calls = 0

    def save_config(self):
        self.save_calls += 1


class FailingConfig(FakeConfig):
    def save_config(self):
        self.save_calls += 1
        raise OSError("configuration storage is unavailable")


class SyncAstrBotConfig(dict):
    """Synchronous host config with a trap for accidental ``await``."""

    def __await__(self):
        raise AssertionError("AstrBotConfig from get_config must not be awaited")


class FakeContext:
    def __init__(
        self, *, configured_provider: str = "helper", current_provider: str = "chat"
    ):
        self.configured_provider = configured_provider
        self.current_provider = current_provider
        self.current_calls = 0
        self.provider_calls: list[str] = []
        self.generate_calls: list[dict] = []
        self.config_calls: list[str | None] = []
        self.session_config: dict = {"provider_settings": {}}

    def get_config(self, *, umo: str | None = None):
        """Match AstrBot v4.26.7: get_config is synchronous."""
        self.config_calls.append(umo)
        return self.session_config

    def get_provider_by_id(self, provider_id: str):
        self.provider_calls.append(provider_id)
        if provider_id == self.configured_provider:
            return object()
        return None

    async def get_current_chat_provider_id(self, *, umo: str):
        self.current_calls += 1
        return self.current_provider

    async def llm_generate(self, **kwargs):
        self.generate_calls.append(kwargs)
        return SimpleNamespace(completion_text='{"ok": true}')


class FakeEvent:
    unified_msg_origin = "test:private:1"

    def __init__(self):
        self.stopped = False
        self.sent: list[str] = []
        self.extra: dict[str, str] = {}

    def plain_result(self, text: str) -> str:
        return text

    def stop_event(self) -> None:
        self.stopped = True

    async def send(self, result: str) -> None:
        self.sent.append(result)

    def set_extra(self, key: str, value: str) -> None:
        self.extra[key] = value

    def get_extra(self, key: str, default=None):
        return self.extra.get(key, default)


class FakeRequest:
    def __init__(self):
        self.prompt = "用户原始问题"
        self.system_prompt = "原有系统提示"
        self.contexts = [{"role": "user", "content": "历史"}]


class FakeConversationManager:
    async def get_curr_conversation_id(self, _umo: str):
        return "conversation-1"

    async def get_conversation(self, _umo: str, _conversation_id: str):
        return SimpleNamespace(persona_id="persona-from-conversation")


class FakePersonaManager:
    def __init__(self):
        self.calls: list[dict] = []

    async def resolve_selected_persona(self, **kwargs):
        self.calls.append(kwargs)
        persona_id = kwargs.get("conversation_persona_id")
        if persona_id == "persona-from-conversation":
            return persona_id, {"prompt": "会话人格"}, None, False
        return None, None, None, False


class DefaultPersonaManager(FakePersonaManager):
    def __init__(self):
        super().__init__()
        self.default_calls: list[str] = []

    async def get_default_persona_v3(self, umo: str):
        self.default_calls.append(umo)
        return SimpleNamespace(name="default", prompt="AstrBot 默认人格")


def plugin(config=None, context=None):
    return AstrEmbodimentPlugin(context or FakeContext(), config or FakeConfig())


def test_explicit_assistant_provider_is_used_without_fallback():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        instance = plugin(FakeConfig(assistant_provider_id="helper"), context)

        response = await instance._llm_generate(
            FakeEvent(), prompt="compile", system_prompt="compiler"
        )

        return context, response

    context, response = asyncio.run(run())

    assert response.completion_text == '{"ok": true}'
    assert context.current_calls == 0
    assert context.generate_calls[0]["chat_provider_id"] == "helper"
    assert context.generate_calls[0]["contexts"] is None
    assert context.generate_calls[0]["tools"] is None


def test_empty_assistant_provider_uses_current_chat_provider():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        instance = plugin(FakeConfig(assistant_provider_id="   "), context)

        await instance._llm_generate(
            FakeEvent(), prompt="compile", system_prompt="compiler"
        )
        return context

    context = asyncio.run(run())

    assert context.current_calls == 1
    assert context.generate_calls[0]["chat_provider_id"] == "chat"


def test_nested_assistant_provider_is_used():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        instance = plugin(
            FakeConfig(model_settings={"assistant_provider_id": "helper"}), context
        )

        await instance._llm_generate(
            FakeEvent(), prompt="compile", system_prompt="compiler"
        )
        return context

    context = asyncio.run(run())

    assert context.current_calls == 0
    assert context.generate_calls[0]["chat_provider_id"] == "helper"


def test_persona_resolution_reads_current_conversation_for_agent_request():
    async def run():
        context = FakeContext()
        context.persona_manager = FakePersonaManager()
        context.conversation_manager = FakeConversationManager()
        instance = plugin(FakeConfig(), context)
        result = await instance.resolve_effective_persona(FakeEvent(), FakeRequest())
        return result, context.persona_manager

    result, persona_manager = asyncio.run(run())

    assert result == (
        "persona-from-conversation",
        {"prompt": "会话人格"},
        "conversation",
    )
    assert persona_manager.calls[0]["conversation_persona_id"] == (
        "persona-from-conversation"
    )


def test_persona_resolution_accepts_synchronous_astrbot_config():
    """AstrBot v4.26.7 get_config returns AstrBotConfig without await."""

    async def run():
        context = FakeContext()
        context.session_config = SyncAstrBotConfig(
            {"provider_settings": {"default_personality": "configured-default"}}
        )
        context.persona_manager = FakePersonaManager()
        context.conversation_manager = FakeConversationManager()
        instance = plugin(FakeConfig(), context)
        result = await instance.resolve_effective_persona(FakeEvent(), FakeRequest())
        return result, context

    result, context = asyncio.run(run())

    assert result == (
        "persona-from-conversation",
        {"prompt": "会话人格"},
        "conversation",
    )
    assert context.config_calls == ["test:private:1"]


def test_persona_resolution_falls_back_to_default_persona_v3_without_explicit_persona():
    async def run():
        context = FakeContext()
        context.persona_manager = DefaultPersonaManager()
        context.conversation_manager = FakeConversationManager()
        instance = plugin(FakeConfig(), context)
        # No conversation persona is available; the manager's default must win.
        context.conversation_manager.get_conversation = lambda _umo, _conversation_id: (
            SimpleNamespace(persona_id=None)
        )
        result = await instance.resolve_effective_persona(FakeEvent(), FakeRequest())
        return result, context.persona_manager

    result, persona_manager = asyncio.run(run())

    assert result == (
        "default",
        SimpleNamespace(name="default", prompt="AstrBot 默认人格"),
        "explicit_default",
    )
    assert persona_manager.default_calls == ["test:private:1"]


def test_empty_assistant_provider_uses_current_chat_provider_when_unconfigured():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="main")
        instance = plugin(FakeConfig(), context)

        await instance._llm_generate(
            FakeEvent(), prompt="compile", system_prompt="compiler"
        )
        return context

    context = asyncio.run(run())

    assert context.current_calls == 1
    assert context.generate_calls[0]["chat_provider_id"] == "main"


def test_invalid_explicit_assistant_provider_does_not_fallback():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        instance = plugin(FakeConfig(assistant_provider_id="missing"), context)

        with pytest.raises(ValueError, match="missing"):
            await instance._llm_generate(
                FakeEvent(), prompt="compile", system_prompt="compiler"
            )
        return context

    context = asyncio.run(run())

    assert context.current_calls == 0
    assert context.generate_calls == []


def test_seed_is_saved_and_is_visible_to_a_new_plugin_instance(tmp_path: Path):
    config_path = tmp_path / "plugin.json"
    first = FakeConfig(seed_code="")
    instance = plugin(first, FakeContext())

    asyncio.run(instance._persist_seed("AE-S1-0123456789ABCDEF"))

    assert first["seed_code"] == "AE-S1-0123456789ABCDEF"
    assert first.save_calls == 1

    second = FakeConfig(seed_code=first["seed_code"])
    assert plugin(second, FakeContext()).config["seed_code"] == first["seed_code"]
    assert (
        not config_path.exists()
    )  # persistence belongs to AstrBotConfig, not plugin files


def test_seed_persistence_rolls_back_when_astrbot_config_save_fails():
    config = FailingConfig(seed_code="AE-S1-PREVIOUS")
    instance = plugin(config, FakeContext())

    with pytest.raises(OSError, match="storage is unavailable"):
        asyncio.run(instance._persist_seed("AE-S1-NOT-PERSISTED"))

    assert config["seed_code"] == "AE-S1-PREVIOUS"
    assert instance._config_values["seed_code"] == "AE-S1-PREVIOUS"
    assert config.save_calls == 1


def test_request_injection_preserves_original_prompt_and_is_idempotent():
    request = FakeRequest()
    instance = plugin(FakeConfig(seed_code="AE-S1-0123456789ABCDEF"), FakeContext())
    contract = {
        "continuous": {
            "directness": 450000,
            "verbosity": 500000,
            "confidence_ceiling": 700000,
        },
        "must_verify": True,
        "may_set_boundary": True,
    }

    instance._inject_request(request, "AE-S1-0123456789ABCDEF", contract)
    first_prompt = request.system_prompt
    instance._inject_request(request, "AE-S1-0123456789ABCDEF", contract)

    assert request.prompt == "用户原始问题"
    assert request.contexts == [{"role": "user", "content": "历史"}]
    assert request.system_prompt == first_prompt
    assert request.system_prompt.startswith("原有系统提示")
    assert request.system_prompt.count("AstrEmbodiment Runtime Context") == 1
    assert "directness=0.450" in request.system_prompt


def test_fixed_runtime_values_always_decode_the_native_fxp6_wire_format():
    instance = plugin(FakeConfig(seed_code="seed"), FakeContext())

    assert instance._fixed_value(0) == 0.0
    assert instance._fixed_value(1) == 0.000001
    assert instance._fixed_value(-1) == -0.000001
    assert instance._fixed_value(1_000_000) == 1.0


def test_on_llm_request_mutates_the_provider_request_with_native_decision():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        seed = "AE-S1-0123456789ABCDEF"

        async def resolve(_event, _request=None):
            return "persona-1", {"prompt": "你是一个测试人格"}, "conversation"

        async def first_turn(**_kwargs):
            return {
                "genesis": {
                    "seed_code": seed,
                    "incarnation_id": "AE-I1-0123456789ABCDEF",
                },
                "seed_code": seed,
                "incarnation_id": "AE-I1-0123456789ABCDEF",
                "revision": 1,
                "contract": {
                    "continuous": {"directness": 450000},
                    "must_verify": True,
                },
            }

        instance.resolve_effective_persona = resolve
        instance._coordinator.first_turn = first_turn
        request = FakeRequest()
        await instance.on_llm_request(FakeEvent(), request)
        return instance, request

    instance, request = asyncio.run(run())

    assert request.prompt == "用户原始问题"
    assert request.contexts == [{"role": "user", "content": "历史"}]
    assert request.system_prompt.startswith("原有系统提示")
    assert "AstrEmbodiment Runtime Context" in request.system_prompt
    assert "seed_code=AE-S1-0123456789ABCDEF" in request.system_prompt
    assert "directness=0.450" in request.system_prompt
    assert instance.config["seed_code"] == "AE-S1-0123456789ABCDEF"


def test_delivery_revision_synchronizes_native_result():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        event = FakeEvent()
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        event.turn_token = "turn-1"
        instance._turn_seq[scope.session_token] = 1
        instance._pending[event.turn_token] = {
            "scope": scope,
            "turn_id": event.turn_token,
            "base_revision": 4,
            "contract": {},
        }

        async def apply_delivery(**kwargs):
            assert kwargs["base_revision"] == 4
            return {"revision": 5}

        instance._coordinator.apply_delivery = apply_delivery
        await instance.after_message_sent(event)
        return instance, scope

    instance, scope = asyncio.run(run())

    assert instance._revisions[scope.persona_token] == 5


def test_reload_hydrates_revision_and_turn_id_without_reuse():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        event = FakeEvent()
        request = FakeRequest()
        calls: list[str] = []
        stimulus: dict = {}

        async def resolve(_event, _request=None):
            return "persona-1", {"prompt": "测试人格"}, "conversation"

        async def ensure_genesis(**_kwargs):
            calls.append("genesis")
            return {
                "seed_code": "AE-S1-RELOAD",
                "incarnation_id": "AE-I1-RELOAD",
            }

        def inspect(_scope):
            assert calls == ["genesis"]
            calls.append("inspect")
            return {"bound": True, "revision": 7}

        async def first_turn(**kwargs):
            calls.append("stimulus")
            stimulus.update(kwargs)
            return {
                "genesis": {
                    "seed_code": "AE-S1-RELOAD",
                    "incarnation_id": "AE-I1-RELOAD",
                },
                "seed_code": "AE-S1-RELOAD",
                "incarnation_id": "AE-I1-RELOAD",
                "revision": 8,
                "contract": {},
            }

        instance.resolve_effective_persona = resolve
        instance._coordinator.ensure_genesis = ensure_genesis
        instance._bridge._native = object()
        instance._bridge.inspect = inspect
        instance._coordinator.first_turn = first_turn
        await instance.on_llm_request(event, request)
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        return instance, event, scope, calls, stimulus

    instance, event, scope, calls, stimulus = asyncio.run(run())

    assert calls == ["genesis", "inspect", "stimulus"]
    assert stimulus["base_revision"] == 7
    assert stimulus["turn_id"] == event.turn_token
    assert instance._turn_seq[scope.session_token] == 8


def test_first_genesis_decision_persists_seed_to_astrbot_config():
    async def run():
        context = FakeContext()
        instance = plugin(FakeConfig(), context)

        async def resolve_default(*_args, **_kwargs):
            return "default", {"prompt": "默认人格"}, "provider_default"

        instance.resolve_effective_persona = resolve_default

        async def first_turn(**_kwargs):
            return {
                "genesis": {
                    "seed_code": "AE-S1-FIRST-GENESIS",
                    "incarnation_id": "AE-I1-FIRST-GENESIS",
                },
                "seed_code": "AE-S1-FIRST-GENESIS",
                "incarnation_id": "AE-I1-FIRST-GENESIS",
                "revision": 1,
                "contract": {"continuous": {"directness": 500000}},
            }

        instance._coordinator.first_turn = first_turn
        request = FakeRequest()
        await instance.on_llm_request(FakeEvent(), request)
        return instance

    instance = asyncio.run(run())

    assert instance.config["seed_code"] == "AE-S1-FIRST-GENESIS"
    assert instance.config.save_calls == 1


def test_seed_command_uses_main_chat_provider_through_full_genesis_compiler():
    async def run():
        context = FakeContext(current_provider="main-dialogue")
        context.persona_manager = DefaultPersonaManager()
        context.conversation_manager = FakeConversationManager()
        context.conversation_manager.get_conversation = lambda _umo, _conversation_id: (
            SimpleNamespace(persona_id=None)
        )

        async def llm_generate(**kwargs):
            context.generate_calls.append(kwargs)
            prompt = kwargs["prompt"]
            template = prompt.split("Target template:\n", 1)[1].split(
                "\nPersona source data", 1
            )[0]
            return SimpleNamespace(completion_text=template)

        context.llm_generate = llm_generate
        instance = plugin(FakeConfig(), context)

        async def ensure_genesis(**kwargs):
            proposal = await kwargs["compiler"](kwargs["source"])
            assert proposal["schema"].endswith("genesis-manifest-proposal.v1")
            assert kwargs["selection"] == "explicit_default"
            return {
                "seed_code": "AE-S1-MAIN-PROVIDER",
                "incarnation_id": "AE-I1-MAIN-PROVIDER",
            }

        instance._coordinator.ensure_genesis = ensure_genesis
        event = FakeEvent()
        results = [item async for item in instance.seed_command(event)]
        return results, context, instance

    results, context, instance = asyncio.run(run())

    assert results == ["SeedCode: AE-S1-MAIN-PROVIDER"]
    assert context.current_calls == 1
    assert context.generate_calls[0]["chat_provider_id"] == "main-dialogue"
    assert instance.config["seed_code"] == "AE-S1-MAIN-PROVIDER"
    assert instance.config.save_calls == 1


def test_on_llm_request_stops_and_reports_when_genesis_fails():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())

        async def fail(*_args, **_kwargs):
            raise PersonaGenesisError("当前会话没有可用的人格")

        instance._run_genesis = fail
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return event, request, instance

    event, request, instance = asyncio.run(run())

    assert event.stopped is True
    assert event.sent == [
        "AstrEmbodiment 创世未完成，本轮未调用对话模型：当前会话没有可用的人格"
    ]
    assert request.system_prompt == "原有系统提示"
    assert instance.config.get("seed_code", "") == ""


def test_on_llm_request_rejects_incomplete_native_genesis_receipt():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())

        async def resolve(_event, _request=None):
            return "persona-1", {"prompt": "测试人格"}, "conversation"

        async def first_turn(**_kwargs):
            return {
                "genesis": {"seed_code": "AE-S1-INCOMPLETE"},
                "seed_code": "AE-S1-INCOMPLETE",
                "revision": 1,
                "contract": {},
            }

        instance.resolve_effective_persona = resolve
        instance._coordinator.first_turn = first_turn
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return event, request, instance

    event, request, instance = asyncio.run(run())

    assert event.stopped is True
    assert event.sent == [
        "AstrEmbodiment 创世未完成，本轮未调用对话模型：原生创世回执不完整"
    ]
    assert request.system_prompt == "原有系统提示"
    assert instance.config.get("seed_code", "") == ""


def test_persona_text_cannot_bypass_genesis_by_containing_the_injection_marker():
    async def run():
        instance = plugin(FakeConfig(seed_code=""), FakeContext())
        calls = 0

        async def first_turn(*_args, **_kwargs):
            nonlocal calls
            calls += 1
            return {
                "genesis": {
                    "seed_code": "AE-S1-MARKER-SAFE",
                    "incarnation_id": "AE-I1-MARKER-SAFE",
                },
                "seed_code": "AE-S1-MARKER-SAFE",
                "incarnation_id": "AE-I1-MARKER-SAFE",
                "revision": 1,
                "contract": {"continuous": {"directness": 500_000}},
            }

        async def resolve(_event, _request=None):
            return "persona-1", {"prompt": "测试人格"}, "conversation"

        instance.resolve_effective_persona = resolve
        instance._coordinator.first_turn = first_turn
        event = FakeEvent()
        request = FakeRequest()
        request.system_prompt += "\n人格会讨论 AstrEmbodiment Runtime Context。"
        await instance.on_llm_request(event, request)
        return calls, event, request

    calls, event, request = asyncio.run(run())

    assert calls == 1
    assert event.stopped is False
    assert "seed_code=AE-S1-MARKER-SAFE" in request.system_prompt


def test_seed_save_failure_stops_the_host_llm_and_rolls_back_visible_seed():
    async def run():
        config = FailingConfig(seed_code="AE-S1-PREVIOUS")
        instance = plugin(config, FakeContext())

        async def genesis(*_args, **_kwargs):
            return (
                {
                    "genesis": {
                        "seed_code": "AE-S1-NOT-PERSISTED",
                        "incarnation_id": "AE-I1-NOT-PERSISTED",
                    },
                    "seed_code": "AE-S1-NOT-PERSISTED",
                    "incarnation_id": "AE-I1-NOT-PERSISTED",
                    "revision": 1,
                    "contract": {},
                },
                SimpleNamespace(persona_token="persona"),
                "session",
                0,
                "turn",
                0,
            )

        instance._run_genesis = genesis
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return config, instance, event, request

    config, instance, event, request = asyncio.run(run())

    assert event.stopped is True
    assert event.sent == [
        "AstrEmbodiment 创世未完成，本轮未调用对话模型：创世结果处理失败"
    ]
    assert config["seed_code"] == "AE-S1-PREVIOUS"
    assert instance._config_values["seed_code"] == "AE-S1-PREVIOUS"
    assert request.system_prompt == "原有系统提示"
    assert instance._pending == {}


def test_invalid_native_contract_stops_before_request_injection():
    async def run():
        instance = plugin(FakeConfig(seed_code=""), FakeContext())

        async def genesis(*_args, **_kwargs):
            return (
                {
                    "genesis": {
                        "seed_code": "AE-S1-VALID",
                        "incarnation_id": "AE-I1-VALID",
                    },
                    "seed_code": "AE-S1-VALID",
                    "incarnation_id": "AE-I1-VALID",
                    "revision": 1,
                    "contract": "not-a-mapping",
                },
                SimpleNamespace(persona_token="persona"),
                "session",
                0,
                "turn",
                0,
            )

        instance._run_genesis = genesis
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return instance, event, request

    instance, event, request = asyncio.run(run())

    assert event.stopped is True
    assert request.system_prompt == "原有系统提示"
    assert instance._pending == {}


def test_native_genesis_identity_must_be_present_in_the_nested_receipt():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())

        async def genesis(*_args, **_kwargs):
            return (
                {
                    "seed_code": "AE-S1-TOP-LEVEL-ONLY",
                    "incarnation_id": "AE-I1-TOP-LEVEL-ONLY",
                    "revision": 1,
                    "contract": {},
                },
                SimpleNamespace(persona_token="persona"),
                "session",
                0,
                "turn",
                0,
            )

        instance._run_genesis = genesis
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return instance, event, request

    instance, event, request = asyncio.run(run())

    assert event.stopped is True
    assert event.sent == [
        "AstrEmbodiment 创世未完成，本轮未调用对话模型：原生创世回执不完整"
    ]
    assert instance.config.get("seed_code", "") == ""
    assert request.system_prompt == "原有系统提示"


def test_native_genesis_identity_mirror_must_match_the_nested_receipt():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())

        async def genesis(*_args, **_kwargs):
            return (
                {
                    "genesis": {
                        "seed_code": "AE-S1-NESTED",
                        "incarnation_id": "AE-I1-NESTED",
                    },
                    "seed_code": "AE-S1-CONFLICT",
                    "incarnation_id": "AE-I1-NESTED",
                    "revision": 1,
                    "contract": {},
                },
                SimpleNamespace(persona_token="persona"),
                "session",
                0,
                "turn",
                0,
            )

        instance._run_genesis = genesis
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return instance, event, request

    instance, event, request = asyncio.run(run())

    assert event.stopped is True
    assert event.sent == [
        "AstrEmbodiment 创世未完成，本轮未调用对话模型：原生创世回执身份不一致"
    ]
    assert instance.config.get("seed_code", "") == ""
    assert request.system_prompt == "原有系统提示"


def test_seed_command_echoes_saved_seed_without_regenerating():
    async def run():
        context = FakeContext()
        instance = plugin(FakeConfig(seed_code="AE-S1-0123456789ABCDEF"), context)
        event = FakeEvent()
        return [item async for item in instance.seed_command(event)], context

    results, context = asyncio.run(run())

    assert results == ["SeedCode: AE-S1-0123456789ABCDEF"]
    assert context.generate_calls == []


def test_native_bridge_finds_sibling_package_for_top_level_loader(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A file-based/top-level host loader must still see the bundled core."""
    plugin_root = tmp_path / "plugin"
    native_root = plugin_root / "astrembodiment_core"
    native_root.mkdir(parents=True)
    (native_root / "__init__.py").write_text(
        """
import json

def open(_data_dir):
    pass

def health():
    return json.dumps({"status": "test", "formula": "test", "neuron_slots": 1})

def version():
    return "test"
""",
        encoding="utf-8",
    )
    monkeypatch.setattr(
        bridge_module, "__file__", str(plugin_root / "astr_embodiment" / "bridge.py")
    )
    monkeypatch.setattr(
        sys, "path", [entry for entry in sys.path if entry != str(plugin_root)]
    )
    monkeypatch.delitem(sys.modules, "astrembodiment_core", raising=False)

    health = bridge_module.NativeBridge().open(str(tmp_path / "runtime"))

    assert health.status == "test"
    assert health.version == "test"


def test_native_bridge_error_keeps_import_diagnostics(
    monkeypatch: pytest.MonkeyPatch,
):
    def fail_import(_name: str, _package: str | None = None):
        raise ImportError("DLL load failed: incompatible ABI")

    monkeypatch.setattr(bridge_module, "import_module", fail_import)

    with pytest.raises(
        bridge_module.NativeCoreUnavailable,
        match=r"ImportError: DLL load failed: incompatible ABI",
    ):
        bridge_module.NativeBridge().open("runtime")


def test_native_initializer_accepts_core_without_optional_exception_export(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """Older Linux builds may omit NativeCoreError but expose the core API."""
    package_dir = tmp_path / "astrembodiment_core"
    package_dir.mkdir()
    source_init = (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_text(
        encoding="utf-8"
    )
    (package_dir / "__init__.py").write_text(source_init, encoding="utf-8")
    native_filename = "_native.pyd" if sys.platform == "win32" else "_native.abi3.so"
    native_payload = b"compat-native"
    build_id = hashlib.sha256(native_payload).hexdigest()
    bundled_dir = package_dir / "_bundled" / build_id
    bundled_dir.mkdir(parents=True)
    (bundled_dir / native_filename).write_bytes(native_payload)
    (package_dir / "_bundled" / "manifest.json").write_text(
        json.dumps(
            {
                "schema": "astrembodiment-native-bundle-v1",
                "platforms": {
                    "win32" if sys.platform == "win32" else "linux": {
                        "build_id": build_id,
                        "filename": native_filename,
                    }
                },
            }
        ),
        encoding="utf-8",
    )

    class FakeLoader:
        def create_module(self, _spec):
            return None

        def exec_module(self, native):
            native.apply_event = lambda *_args: "{}"
            native.apply_perception_proposal_v1 = lambda *_args: "{}"
            native.ensure_genesis = lambda *_args: "{}"
            native.flush_and_close = lambda: None
            native.health = lambda: "{}"
            native.inspect = lambda *_args: "{}"
            native.open = lambda *_args: None
            native.semantic_revision_v1 = lambda *_args: "{}"
            native.verify_replay = lambda *_args: "{}"
            native.version = lambda: "compat"

    original_spec_from_file_location = importlib.util.spec_from_file_location

    def fake_spec_from_file_location(name, location, **kwargs):
        if name.endswith("._native"):
            return importlib.util.spec_from_loader(
                name, FakeLoader(), origin=str(location)
            )
        return original_spec_from_file_location(name, location, **kwargs)

    monkeypatch.setattr(
        importlib.util, "spec_from_file_location", fake_spec_from_file_location
    )
    module_name = "_astrembodiment_core_compat_test"
    spec = importlib.util.spec_from_file_location(
        module_name,
        package_dir / "__init__.py",
        submodule_search_locations=[str(package_dir)],
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
    finally:
        sys.modules.pop(module_name, None)
        sys.modules.pop(f"{module_name}._native", None)
    assert module.version() == "compat"
    assert issubclass(module.NativeCoreError, RuntimeError)


@pytest.mark.skipif(sys.platform != "win32", reason="requires the Windows fresh wheel")
def test_native_initializer_ignores_stale_root_module_for_bundled_extension(
    tmp_path: Path,
):
    """The package must not let a stale root _native.py shadow the bundled wheel."""
    wheel_path = next(
        (
            ROOT.parents[1] / ".codex-task-temp" / "rebuild-native-win-current" / "dist"
        ).glob("*.whl"),
        None,
    )
    assert wheel_path is not None
    package_dir = tmp_path / "astrembodiment_core"
    with zipfile.ZipFile(wheel_path) as wheel:
        bundled_payload = wheel.read("astrembodiment_core/_native.pyd")
    build_id = hashlib.sha256(bundled_payload).hexdigest()
    bundled_dir = package_dir / "_bundled" / build_id
    bundled_dir.mkdir(parents=True)
    (bundled_dir / "_native.pyd").write_bytes(bundled_payload)
    (package_dir / "_bundled" / "manifest.json").write_text(
        json.dumps(
            {
                "schema": "astrembodiment-native-bundle-v1",
                "platforms": {
                    "win32": {"build_id": build_id, "filename": "_native.pyd"}
                },
            }
        ),
        encoding="utf-8",
    )
    (package_dir / "_native.py").write_text(
        "version=lambda: 'stale-root'\nhealth=lambda: '{}'\n",
        encoding="utf-8",
    )
    (package_dir / "__init__.py").write_text(
        (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_text(
            encoding="utf-8"
        ),
        encoding="utf-8",
    )

    module_name = "_astrembodiment_core_bundled_regression"
    spec = importlib.util.spec_from_file_location(
        module_name,
        package_dir / "__init__.py",
        submodule_search_locations=[str(package_dir)],
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
        assert module.version() == "1.0.0-rc1"
        assert callable(module.apply_event)
        assert callable(module.apply_perception_proposal_v1)
        assert callable(module.semantic_revision_v1)
        assert Path(sys.modules[f"{module_name}._native"].__file__).parts[-3:] == (
            "_bundled",
            build_id,
            "_native.pyd",
        )
    finally:
        sys.modules.pop(module_name, None)
        sys.modules.pop(f"{module_name}._native", None)


def test_native_loader_uses_new_physical_build_after_same_process_reload(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
):
    """A purged package name must load a replacement from a different build path."""
    package_dir = tmp_path / "astrembodiment_core"
    old_payload = b"old-native"
    old_build_id = hashlib.sha256(old_payload).hexdigest()
    new_payload = b"new-native"
    new_build_id = hashlib.sha256(new_payload).hexdigest()
    native_filename = "_native.pyd" if sys.platform == "win32" else "_native.abi3.so"
    old_dir = package_dir / "_bundled" / old_build_id
    old_dir.mkdir(parents=True)
    (old_dir / native_filename).write_bytes(old_payload)
    (package_dir / "_bundled" / "manifest.json").write_text(
        json.dumps(
            {
                "schema": "astrembodiment-native-bundle-v1",
                "platforms": {
                    "win32" if sys.platform == "win32" else "linux": {
                        "build_id": old_build_id,
                        "filename": native_filename,
                    }
                },
            }
        ),
        encoding="utf-8",
    )
    (package_dir / "_native.py").write_text(
        "version=lambda: 'stale-root'\n", encoding="utf-8"
    )
    source_init = (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_text(
        encoding="utf-8"
    )
    (package_dir / "__init__.py").write_text(source_init, encoding="utf-8")

    class FakeLoader:
        def create_module(self, _spec):
            return None

        def exec_module(self, module):
            module.version = lambda: Path(module.__spec__.origin).parent.name
            module.health = lambda: "{}"
            module.open = lambda _data_dir: None
            module.ensure_genesis = lambda *_args: "{}"
            module.apply_event = lambda *_args: "{}"
            module.apply_perception_proposal_v1 = lambda *_args: "{}"
            module.inspect = lambda *_args: "{}"
            module.semantic_revision_v1 = lambda *_args: "{}"
            module.verify_replay = lambda *_args: "{}"
            module.flush_and_close = lambda: None
            module.NativeCoreError = RuntimeError

    original_spec_from_file_location = importlib.util.spec_from_file_location

    def fake_spec_from_file_location(name, location, **kwargs):
        if name.endswith("._native"):
            return importlib.util.spec_from_loader(
                name, FakeLoader(), origin=str(location)
            )
        return original_spec_from_file_location(name, location, **kwargs)

    monkeypatch.setattr(
        importlib.util, "spec_from_file_location", fake_spec_from_file_location
    )

    module_name = "_astrembodiment_core_reload_regression"

    def load_package():
        spec = original_spec_from_file_location(
            module_name,
            package_dir / "__init__.py",
            submodule_search_locations=[str(package_dir)],
        )
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        sys.modules[module_name] = module
        spec.loader.exec_module(module)
        return module

    try:
        first = load_package()
        old_native = sys.modules[f"{module_name}._native"]
        assert first.version() == old_build_id

        shutil.rmtree(old_dir)
        new_dir = package_dir / "_bundled" / new_build_id
        new_dir.mkdir(parents=True)
        (new_dir / native_filename).write_bytes(new_payload)
        (package_dir / "_bundled" / "manifest.json").write_text(
            json.dumps(
                {
                    "schema": "astrembodiment-native-bundle-v1",
                    "platforms": {
                        "win32" if sys.platform == "win32" else "linux": {
                            "build_id": new_build_id,
                            "filename": native_filename,
                        }
                    },
                }
            ),
            encoding="utf-8",
        )
        sys.modules.pop(module_name, None)
        sys.modules.pop(f"{module_name}._native", None)

        second = load_package()
        assert second.version() == new_build_id
        assert second.apply_event is not old_native.apply_event
    finally:
        sys.modules.pop(module_name, None)
        sys.modules.pop(f"{module_name}._native", None)


def test_schema_exposes_chinese_provider_and_seed_fields():
    schema = json.loads((ROOT / "_conf_schema.json").read_text(encoding="utf-8"))

    provider = schema["model_settings"]["items"]["assistant_provider_id"]
    seed = schema["seed_code"]
    assert provider["_special"] == "select_provider"
    assert provider["default"] == ""
    assert (
        provider["description"]
        != provider["description"].encode("ascii", "ignore").decode()
    )
    assert seed["type"] in {"string", "text"}
    assert seed["default"] == ""
    assert seed["readonly"] is True
    assert "种子" in seed["description"]


def test_runtime_commands_and_hooks_expose_chinese_descriptions_without_webui():
    source = inspect.getsource(AstrEmbodimentPlugin)

    assert '@filter.command("ae", desc="查看 AstrEmbodiment 运行状态")' in source
    assert '@filter.command("ae_seed", desc="查看或生成当前人格的 SeedCode")' in source
    assert (
        '@filter.on_llm_request(desc="LLM 请求前：生成并注入 AstrEmbodiment 运行契约")'
        in source
    )
    assert (
        '@filter.on_llm_response(desc="LLM 响应后：登记候选行动（当前仅观察）")'
        in source
    )
    assert (
        '@filter.after_message_sent(desc="消息发送后：提交投递事实并同步原生修订号")'
        in source
    )
    assert "无需 WebUI" in inspect.getdoc(AstrEmbodimentPlugin.seed_command)


def _spc1_scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="11" * 16,
        persona_token="22" * 16,
        session_token="33" * 16,
    )


def _spc1_genesis_result(scope: ScopeTokens, *, seq: int = 0) -> tuple:
    session_key = scope.session_token
    return (
        {
            "genesis": {
                "seed_code": "AE-S1-SPC1",
                "incarnation_id": "AE-I1-SPC1",
            },
            "seed_code": "AE-S1-SPC1",
            "incarnation_id": "AE-I1-SPC1",
            "revision": 8,
            "contract": {"continuous": {"directness": 500_000}},
        },
        scope,
        session_key,
        seq,
        turn_id(session_key, seq),
        7,
    )


class RecordingLogger:
    def __init__(self) -> None:
        self.info_messages: list[str] = []
        self.warning_messages: list[str] = []

    def info(self, template: str, *args: object) -> None:
        self.info_messages.append(template % args)

    def warning(self, template: str, *args: object) -> None:
        self.warning_messages.append(template % args)


class RaisingLogger:
    def info(self, _template: str, *_args: object) -> None:
        raise RuntimeError("LOGGER_RAW_SENTINEL")

    def warning(self, _template: str, *_args: object) -> None:
        raise RuntimeError("LOGGER_RAW_SENTINEL")


def _spc1_dimensions() -> dict[str, int]:
    names = (
        "positive",
        "affiliation",
        "harm",
        "boundary",
        "repair",
        "repetition",
        "new_information",
        "constraint_instability",
        "epistemic_conflict",
        "self_responsibility",
        "other_responsibility",
        "hostility",
        "publicness",
        "engagement",
        "rejection",
    )
    return {name: index * 10_000 for index, name in enumerate(names, start=1)}


def _spc1_zero_load_dimensions() -> dict[str, int]:
    dimensions = _spc1_dimensions()
    for name in ("positive", "harm", "boundary", "epistemic_conflict"):
        dimensions[name] = 0
    return dimensions


def _spc1_calculation() -> dict:
    return {
        "state_changed": True,
        "active_nodes": 17,
        "active_edges": 23,
        "residuals_fxp6": {
            "authority": 0,
            "continuity": 0,
            "energy": 0,
            "renormalization": 0,
            "capacity": 0,
        },
    }


def _spc1_diagnostic(
    *,
    stage: str,
    commit_state: str,
    values_state: str,
    dimensions_fxp6: dict[str, int] | None = None,
    estimator_confidence_fxp6: int | None = None,
    base_revision: int | None = None,
    revision: int | None = None,
    deduplicated: bool | None = None,
    receipt_status: str | None = None,
    calculation_state: str | None = None,
    native_calculation: dict | None = None,
) -> dict:
    if calculation_state is None:
        calculation_state = (
            "CONFIRMED"
            if commit_state in {"CONFIRMED_NEW", "CONFIRMED_EXISTING"}
            else "UNCONFIRMED" if commit_state == "UNKNOWN" else "NOT_ATTEMPTED"
        )
    if native_calculation is None and calculation_state == "CONFIRMED":
        native_calculation = _spc1_calculation()
    return {
        "stage": stage,
        "commit_state": commit_state,
        "values_state": values_state,
        "dimensions_fxp6": dimensions_fxp6,
        "estimator_confidence_fxp6": estimator_confidence_fxp6,
        "base_revision": base_revision,
        "revision": revision,
        "deduplicated": deduplicated,
        "receipt_status": receipt_status,
        "calculation_state": calculation_state,
        "native_calculation": native_calculation,
    }


def _spc1_outcome(status: str, code: str, diagnostic: dict) -> dict:
    return {"status": status, "code": code, "diagnostic": diagnostic}


def _spc1_success_outcome() -> dict:
    return _spc1_outcome(
        "SUCCESS",
        "SEMANTIC_COMMITTED",
        _spc1_diagnostic(
            stage="RECEIPT",
            commit_state="CONFIRMED_NEW",
            values_state="COMMITTED",
            dimensions_fxp6=_spc1_dimensions(),
            estimator_confidence_fxp6=900_000,
            base_revision=7,
            revision=8,
            deduplicated=False,
            receipt_status="committed",
        ),
    )


def test_spc1_observatory_success_emits_all_dimensions_at_info(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    instance._emit_semantic_observatory(
        _spc1_success_outcome(),
        {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
    )

    assert recorder.warning_messages == []
    assert len(recorder.info_messages) == 1
    prefix = "AstrEmbodiment SPC1 observatory: "
    assert recorder.info_messages[0].startswith(prefix)
    record = json.loads(recorder.info_messages[0][len(prefix) :])
    assert record["schema"] == "astr-embodiment.observatory.semantic-injection.v1"
    assert record["status"] == "SUCCESS"
    assert record["code"] == "SEMANTIC_COMMITTED"
    assert record["fxp_scale"] == 1_000_000
    assert record["dimensions_fxp6"] == _spc1_dimensions()
    assert len(record["dimensions_fxp6"]) == 15
    assert record["estimator_confidence_fxp6"] == 900_000
    assert record["revision"] == 8
    assert record["calculation_state"] == "CONFIRMED"
    assert record["native_calculation"] == _spc1_calculation()


def test_spc1_observatory_degraded_warns_without_echoing_raw_fields(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())
    raw = {
        "status": "DEGRADED",
        "code": "NATIVE_ERROR",
        "diagnostic": {
            "stage": "NATIVE_APPLY",
            "commit_state": "UNKNOWN",
            "values_state": "ESTIMATED_NOT_CONFIRMED",
            "dimensions_fxp6": _spc1_dimensions(),
            "estimator_confidence_fxp6": 900_000,
            "base_revision": 7,
            "revision": None,
            "deduplicated": None,
            "receipt_status": None,
            "calculation_state": "UNCONFIRMED",
            "native_calculation": None,
        },
        "request": "USER_RAW_SENTINEL",
        "exception": "EXCEPTION_RAW_SENTINEL",
        "scope_digest": "DIGEST_RAW_SENTINEL",
    }

    instance._emit_semantic_observatory(
        raw, {"status": "DEGRADED", "code": "NATIVE_ERROR"}
    )

    assert recorder.info_messages == []
    assert len(recorder.warning_messages) == 1
    encoded = recorder.warning_messages[0]
    assert "USER_RAW_SENTINEL" not in encoded
    assert "EXCEPTION_RAW_SENTINEL" not in encoded
    assert "DIGEST_RAW_SENTINEL" not in encoded
    record = json.loads(encoded.split(": ", 1)[1])
    assert record["status"] == "DEGRADED"
    assert record["stage"] == "NATIVE_APPLY"
    assert record["commit_state"] == "UNKNOWN"
    assert record["dimensions_fxp6"] == _spc1_dimensions()
    assert record["calculation_state"] == "UNCONFIRMED"
    assert record["native_calculation"] is None


@pytest.mark.parametrize(
    ("status", "code", "diagnostic"),
    [
        pytest.param(
            "SUCCESS",
            "SEMANTIC_COMMITTED",
            _spc1_diagnostic(
                stage="RECEIPT",
                commit_state="CONFIRMED_EXISTING",
                values_state="COMMITTED",
                dimensions_fxp6=_spc1_dimensions(),
                estimator_confidence_fxp6=900_000,
                base_revision=7,
                revision=8,
                deduplicated=True,
                receipt_status="committed",
            ),
            id="deduplicated-success",
        ),
        pytest.param(
            "NOOP",
            "EMPTY_REQUEST",
            _spc1_diagnostic(
                stage="INPUT",
                commit_state="NOT_ATTEMPTED",
                values_state="UNAVAILABLE",
            ),
            id="empty-request",
        ),
        pytest.param(
            "NOOP",
            "ZERO_LOAD",
            _spc1_diagnostic(
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
                values_state="ESTIMATED_NOT_COMMITTED",
                dimensions_fxp6=_spc1_zero_load_dimensions(),
                estimator_confidence_fxp6=900_000,
            ),
            id="zero-load",
        ),
        pytest.param(
            "DEGRADED",
            "INVALID_TURN",
            _spc1_diagnostic(
                stage="INPUT",
                commit_state="NOT_ATTEMPTED",
                values_state="UNAVAILABLE",
            ),
            id="degraded-input",
        ),
        pytest.param(
            "DEGRADED",
            "ESTIMATOR_MALFORMED",
            _spc1_diagnostic(
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
                values_state="UNAVAILABLE",
            ),
            id="degraded-estimator",
        ),
        pytest.param(
            "DEGRADED",
            "NATIVE_ERROR",
            _spc1_diagnostic(
                stage="CURSOR",
                commit_state="NOT_ATTEMPTED",
                values_state="ESTIMATED_NOT_CONFIRMED",
                dimensions_fxp6=_spc1_dimensions(),
                estimator_confidence_fxp6=900_000,
            ),
            id="degraded-cursor",
        ),
        pytest.param(
            "DEGRADED",
            "INVALID_PROPOSAL",
            _spc1_diagnostic(
                stage="PROPOSAL",
                commit_state="NOT_ATTEMPTED",
                values_state="ESTIMATED_NOT_CONFIRMED",
                dimensions_fxp6=_spc1_dimensions(),
                estimator_confidence_fxp6=900_000,
            ),
            id="degraded-proposal",
        ),
        pytest.param(
            "DEGRADED",
            "NATIVE_ERROR",
            _spc1_diagnostic(
                stage="NATIVE_APPLY",
                commit_state="UNKNOWN",
                values_state="ESTIMATED_NOT_CONFIRMED",
                dimensions_fxp6=_spc1_dimensions(),
                estimator_confidence_fxp6=900_000,
                base_revision=7,
            ),
            id="degraded-native-apply",
        ),
        pytest.param(
            "DEGRADED",
            "NATIVE_MALFORMED",
            _spc1_diagnostic(
                stage="RECEIPT",
                commit_state="UNKNOWN",
                values_state="ESTIMATED_NOT_CONFIRMED",
                dimensions_fxp6=_spc1_dimensions(),
                estimator_confidence_fxp6=900_000,
                base_revision=7,
            ),
            id="degraded-receipt",
        ),
        pytest.param(
            "DEGRADED",
            "NATIVE_ERROR",
            _spc1_diagnostic(
                stage="INTERNAL",
                commit_state="UNKNOWN",
                values_state="UNAVAILABLE",
            ),
            id="degraded-internal",
        ),
    ],
)
def test_spc1_observatory_accepts_only_valid_outcome_semantics(
    status: str, code: str, diagnostic: dict
) -> None:
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())
    raw = _spc1_outcome(status, code, diagnostic)

    record = instance._semantic_observatory_record(
        raw, {"status": status, "code": code}
    )

    assert record["status"] == status
    assert record["code"] == code
    assert record["stage"] == diagnostic["stage"]
    assert record["commit_state"] == diagnostic["commit_state"]
    assert record["values_state"] == diagnostic["values_state"]
    assert record["dimensions_fxp6"] == diagnostic["dimensions_fxp6"]
    assert record["base_revision"] == diagnostic["base_revision"]
    assert record["revision"] == diagnostic["revision"]
    assert record["deduplicated"] == diagnostic["deduplicated"]
    assert record["receipt_status"] == diagnostic["receipt_status"]


@pytest.mark.parametrize(
    ("raw", "closed"),
    [
        pytest.param(
            _spc1_outcome(
                "SUCCESS",
                "SEMANTIC_COMMITTED",
                _spc1_diagnostic(
                    stage="INPUT",
                    commit_state="NOT_ATTEMPTED",
                    values_state="UNAVAILABLE",
                ),
            ),
            {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
            id="success-before-commit",
        ),
        pytest.param(
            _spc1_outcome(
                "SUCCESS",
                "SEMANTIC_COMMITTED",
                _spc1_diagnostic(
                    stage="RECEIPT",
                    commit_state="CONFIRMED_NEW",
                    values_state="COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                    revision=8,
                    deduplicated=True,
                    receipt_status="committed",
                ),
            ),
            {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
            id="confirmed-new-deduplicated",
        ),
        pytest.param(
            _spc1_outcome(
                "SUCCESS",
                "SEMANTIC_COMMITTED",
                _spc1_diagnostic(
                    stage="RECEIPT",
                    commit_state="CONFIRMED_EXISTING",
                    values_state="COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                    revision=8,
                    deduplicated=False,
                    receipt_status="committed",
                ),
            ),
            {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
            id="confirmed-existing-new",
        ),
        pytest.param(
            _spc1_outcome(
                "SUCCESS",
                "SEMANTIC_COMMITTED",
                _spc1_diagnostic(
                    stage="RECEIPT",
                    commit_state="CONFIRMED_NEW",
                    values_state="COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    revision=8,
                    deduplicated=False,
                    receipt_status="committed",
                ),
            ),
            {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
            id="success-missing-base-revision",
        ),
        pytest.param(
            _spc1_outcome(
                "SUCCESS",
                "SEMANTIC_COMMITTED",
                _spc1_diagnostic(
                    stage="RECEIPT",
                    commit_state="CONFIRMED_NEW",
                    values_state="COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                    deduplicated=False,
                    receipt_status="committed",
                ),
            ),
            {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
            id="success-missing-result-revision",
        ),
        pytest.param(
            _spc1_outcome(
                "SUCCESS",
                "SEMANTIC_COMMITTED",
                _spc1_diagnostic(
                    stage="RECEIPT",
                    commit_state="CONFIRMED_NEW",
                    values_state="COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                    revision=8,
                    deduplicated=False,
                ),
            ),
            {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
            id="success-missing-receipt-status",
        ),
        pytest.param(
            _spc1_outcome(
                "NOOP",
                "EMPTY_REQUEST",
                _spc1_diagnostic(
                    stage="INPUT",
                    commit_state="NOT_ATTEMPTED",
                    values_state="ESTIMATED_NOT_COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                ),
            ),
            {"status": "NOOP", "code": "EMPTY_REQUEST"},
            id="empty-request-with-estimate",
        ),
        pytest.param(
            _spc1_outcome(
                "NOOP",
                "ZERO_LOAD",
                _spc1_diagnostic(
                    stage="ESTIMATOR",
                    commit_state="NOT_ATTEMPTED",
                    values_state="ESTIMATED_NOT_COMMITTED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                ),
            ),
            {"status": "NOOP", "code": "ZERO_LOAD"},
            id="zero-load-with-positive-load",
        ),
        pytest.param(
            _spc1_outcome(
                "NOOP",
                "ZERO_LOAD",
                _spc1_diagnostic(
                    stage="ESTIMATOR",
                    commit_state="NOT_ATTEMPTED",
                    values_state="ESTIMATED_NOT_COMMITTED",
                    dimensions_fxp6=_spc1_zero_load_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                ),
            ),
            {"status": "NOOP", "code": "ZERO_LOAD"},
            id="zero-load-with-native-base",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "INVALID_TURN",
                _spc1_diagnostic(
                    stage="INPUT",
                    commit_state="NOT_ATTEMPTED",
                    values_state="ESTIMATED_NOT_CONFIRMED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                ),
            ),
            {"status": "DEGRADED", "code": "INVALID_TURN"},
            id="degraded-input-with-estimate",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "NATIVE_ERROR",
                _spc1_diagnostic(
                    stage="CURSOR",
                    commit_state="NOT_ATTEMPTED",
                    values_state="UNAVAILABLE",
                ),
            ),
            {"status": "DEGRADED", "code": "NATIVE_ERROR"},
            id="degraded-cursor-without-estimate",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "NATIVE_ERROR",
                _spc1_diagnostic(
                    stage="CURSOR",
                    commit_state="UNKNOWN",
                    values_state="ESTIMATED_NOT_CONFIRMED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                ),
            ),
            {"status": "DEGRADED", "code": "NATIVE_ERROR"},
            id="degraded-cursor-unknown-commit",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "NATIVE_ERROR",
                _spc1_diagnostic(
                    stage="NATIVE_APPLY",
                    commit_state="NOT_ATTEMPTED",
                    values_state="ESTIMATED_NOT_CONFIRMED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                ),
            ),
            {"status": "DEGRADED", "code": "NATIVE_ERROR"},
            id="degraded-native-not-attempted",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "NATIVE_MALFORMED",
                _spc1_diagnostic(
                    stage="RECEIPT",
                    commit_state="UNKNOWN",
                    values_state="ESTIMATED_NOT_CONFIRMED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                ),
            ),
            {"status": "DEGRADED", "code": "NATIVE_MALFORMED"},
            id="degraded-receipt-missing-base",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "ESTIMATOR_MALFORMED",
                _spc1_diagnostic(
                    stage="ESTIMATOR",
                    commit_state="NOT_ATTEMPTED",
                    values_state="UNAVAILABLE",
                    base_revision=7,
                ),
            ),
            {"status": "DEGRADED", "code": "ESTIMATOR_MALFORMED"},
            id="degraded-estimator-with-base",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "NATIVE_ERROR",
                _spc1_diagnostic(
                    stage="INTERNAL",
                    commit_state="UNKNOWN",
                    values_state="ESTIMATED_NOT_CONFIRMED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                ),
            ),
            {"status": "DEGRADED", "code": "NATIVE_ERROR"},
            id="degraded-internal-with-estimate",
        ),
        pytest.param(
            _spc1_outcome(
                "DEGRADED",
                "NATIVE_ERROR",
                _spc1_diagnostic(
                    stage="NATIVE_APPLY",
                    commit_state="UNKNOWN",
                    values_state="ESTIMATED_NOT_CONFIRMED",
                    dimensions_fxp6=_spc1_dimensions(),
                    estimator_confidence_fxp6=900_000,
                    base_revision=7,
                    revision=8,
                ),
            ),
            {"status": "DEGRADED", "code": "NATIVE_ERROR"},
            id="degraded-with-native-result",
        ),
        pytest.param(
            _spc1_success_outcome(),
            {"status": "DEGRADED", "code": "NATIVE_ERROR"},
            id="raw-and-closed-mismatch",
        ),
    ],
)
def test_spc1_observatory_invalid_outcome_semantics_use_fixed_fallback(
    raw: dict, closed: dict[str, str]
) -> None:
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    record = instance._semantic_observatory_record(raw, closed)

    assert record == {
        "schema": "astr-embodiment.observatory.semantic-injection.v1",
        "status": "DEGRADED",
        "code": "NATIVE_MALFORMED",
        "stage": "INTERNAL",
        "commit_state": "UNKNOWN",
        "values_state": "UNAVAILABLE",
        "fxp_scale": 1_000_000,
        "dimensions_fxp6": None,
        "estimator_confidence_fxp6": None,
        "base_revision": None,
        "revision": None,
        "deduplicated": None,
        "receipt_status": None,
        "calculation_state": "UNCONFIRMED",
        "native_calculation": None,
    }


@pytest.mark.parametrize("code", ["INVALID_TURN", "NATIVE_ERROR"])
def test_spc1_observatory_local_degraded_preserves_closed_code(
    monkeypatch: pytest.MonkeyPatch, code: str
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    instance._emit_semantic_observatory(
        None, {"status": "DEGRADED", "code": code}
    )

    assert recorder.info_messages == []
    assert len(recorder.warning_messages) == 1
    assert json.loads(recorder.warning_messages[0].split(": ", 1)[1]) == {
        "schema": "astr-embodiment.observatory.semantic-injection.v1",
        "status": "DEGRADED",
        "code": code,
        "stage": "INTERNAL",
        "commit_state": "UNKNOWN",
        "values_state": "UNAVAILABLE",
        "fxp_scale": 1_000_000,
        "dimensions_fxp6": None,
        "estimator_confidence_fxp6": None,
        "base_revision": None,
        "revision": None,
        "deduplicated": None,
        "receipt_status": None,
        "calculation_state": "UNCONFIRMED",
        "native_calculation": None,
    }


def test_spc1_observatory_empty_raw_mapping_still_uses_fixed_fallback() -> None:
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    record = instance._semantic_observatory_record(
        {}, {"status": "DEGRADED", "code": "INVALID_TURN"}
    )

    assert record["status"] == "DEGRADED"
    assert record["code"] == "NATIVE_MALFORMED"
    assert record["stage"] == "INTERNAL"
    assert record["dimensions_fxp6"] is None


@pytest.mark.parametrize("configured", [False, "true", 1, None])
def test_spc1_observatory_disabled_or_malformed_config_emits_nothing(
    monkeypatch: pytest.MonkeyPatch, configured: object
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=configured), FakeContext())

    instance._emit_semantic_observatory(
        _spc1_success_outcome(),
        {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
    )

    assert recorder.info_messages == []
    assert recorder.warning_messages == []


def test_spc1_observatory_malformed_diagnostic_falls_back_without_raw_fields(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    instance._emit_semantic_observatory(
        {
            "diagnostic": {
                "stage": "INTERNAL_RAW_SENTINEL",
                "dimensions_fxp6": {"raw": "DIMENSION_RAW_SENTINEL"},
            }
        },
        {"status": "DEGRADED", "code": "NATIVE_ERROR"},
    )

    assert recorder.info_messages == []
    assert len(recorder.warning_messages) == 1
    encoded = recorder.warning_messages[0]
    assert "INTERNAL_RAW_SENTINEL" not in encoded
    assert "DIMENSION_RAW_SENTINEL" not in encoded
    assert json.loads(encoded.split(": ", 1)[1]) == {
        "schema": "astr-embodiment.observatory.semantic-injection.v1",
        "status": "DEGRADED",
        "code": "NATIVE_MALFORMED",
        "stage": "INTERNAL",
        "commit_state": "UNKNOWN",
        "values_state": "UNAVAILABLE",
        "fxp_scale": 1_000_000,
        "dimensions_fxp6": None,
        "estimator_confidence_fxp6": None,
        "base_revision": None,
        "revision": None,
        "deduplicated": None,
        "receipt_status": None,
        "calculation_state": "UNCONFIRMED",
        "native_calculation": None,
    }


def test_spc1_observatory_logger_failure_never_raises(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    monkeypatch.setattr(main_module, "logger", RaisingLogger())
    instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())

    instance._emit_semantic_observatory(
        _spc1_success_outcome(),
        {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"},
    )


def test_spc1_hook_runs_after_g0_injection_and_passes_only_current_prompt():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        scope = _spc1_scope()
        async def run_genesis(*_args, **_kwargs):
            return _spc1_genesis_result(scope)

        instance._run_genesis = run_genesis
        calls: list[tuple] = []

        async def preflight(scope_arg, frozen_turn, request_text, estimator):
            calls.append((scope_arg, frozen_turn, request_text, estimator))
            return {"status": "DEGRADED", "code": "NATIVE_SYMBOL_UNAVAILABLE"}

        instance._coordinator.preflight_stimulus = preflight
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return instance, event, request, calls

    instance, event, request, calls = asyncio.run(run())

    assert event.stopped is False
    assert len(calls) == 1
    scope, frozen_turn, request_text, estimator = calls[0]
    assert scope == _spc1_scope()
    assert isinstance(frozen_turn, FrozenTurn)
    assert frozen_turn.scope == scope
    assert frozen_turn.turn_id == event.turn_token
    assert frozen_turn.event_id == event_id(f"{scope.session_token}#0")
    assert frozen_turn.base_revision == 7
    assert frozen_turn.observed_at_ms > 0
    assert request_text == request.prompt
    assert callable(estimator)
    assert "seed_code=AE-S1-SPC1" in request.system_prompt
    assert instance._pending[event.turn_token]["base_revision"] == 8


def test_spc1_estimator_boundary_uses_one_prompt_argument_and_keeps_g0_contract():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        scope = _spc1_scope()
        async def run_genesis(*_args, **_kwargs):
            return _spc1_genesis_result(scope)

        instance._run_genesis = run_genesis
        observed: dict[str, object] = {}

        async def preflight(scope_arg, frozen_turn, request_text, estimator):
            observed["scope"] = scope_arg
            observed["turn"] = frozen_turn
            observed["text"] = request_text
            result = estimator(request_text)
            if inspect.isawaitable(result):
                result = await result
            observed["estimate_result"] = result
            return {"status": "SUCCESS", "code": "SEMANTIC_COMMITTED"}

        instance._coordinator.preflight_stimulus = preflight
        event = FakeEvent()
        request = FakeRequest()
        request.prompt = "SPC1_RAW_SENTINEL"
        await instance.on_llm_request(event, request)
        return instance, event, request, observed

    instance, event, request, observed = asyncio.run(run())

    assert event.stopped is False
    assert observed["text"] == "SPC1_RAW_SENTINEL"
    assert observed["estimate_result"] == '{"ok": true}'
    provider_call = instance.context.generate_calls[-1]
    assert provider_call["prompt"] == "SPC1_RAW_SENTINEL"
    assert provider_call["contexts"] is None
    assert provider_call["tools"] is None
    assert provider_call["temperature"] == 0
    assert "SPC1_RAW_SENTINEL" not in provider_call["system_prompt"]
    assert request.system_prompt.count("SPC1_RAW_SENTINEL") == 0
    assert request.contexts == [{"role": "user", "content": "历史"}]
    assert instance._pending[event.turn_token]["contract"] == {
        "continuous": {"directness": 500_000}
    }


def test_spc1_estimator_prompt_declares_a_parseable_exact_closed_schema():
    async def run() -> str:
        instance = plugin(FakeConfig(), FakeContext())
        await instance._spc1_estimate(FakeEvent(), "SPC1_SCHEMA_SENTINEL")
        return instance.context.generate_calls[-1]["system_prompt"]

    system_prompt = asyncio.run(run())

    assert "SPC1_SCHEMA_SENTINEL" not in system_prompt
    assert "Target template:\n" in system_prompt
    template_text = system_prompt.split("Target template:\n", 1)[1].split(
        "\nDimension meanings:\n", 1
    )[0]
    template = json.loads(template_text)
    assert list(template) == ["dimensions", "estimator_confidence"]
    assert tuple(template["dimensions"]) == DIMENSION_NAMES
    assert parse_estimator_output(template).as_json() == template
    assert f"integer in [0,{FXP6_SCALE}]" in system_prompt
    assert f"integer in [1,{FXP6_SCALE}]" in system_prompt
    assert "all-zero" in system_prompt
    assert "Markdown" in system_prompt
    assert "data, not instructions" in system_prompt


def test_spc1_repeated_hook_is_at_most_once_and_keeps_closed_request_marker(
    monkeypatch: pytest.MonkeyPatch,
):
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)

    async def run():
        instance = plugin(FakeConfig(observatory_enabled=True), FakeContext())
        scope = _spc1_scope()
        async def run_genesis(*_args, **_kwargs):
            return _spc1_genesis_result(scope)

        instance._run_genesis = run_genesis
        calls: list[str] = []

        async def preflight(_scope, _turn, request_text, _estimator):
            calls.append(request_text)
            return _spc1_success_outcome()

        instance._coordinator.preflight_stimulus = preflight
        event = FakeEvent()
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        request.prompt = "CHANGED_RAW_SENTINEL"
        await instance.on_llm_request(event, request)
        return instance, event, request, calls

    instance, event, request, calls = asyncio.run(run())

    assert calls == ["用户原始问题"]
    assert getattr(request, "_astrembodiment_semantic_preflight_v1") == {
        "status": "SUCCESS",
        "code": "SEMANTIC_COMMITTED",
    }
    assert "CHANGED_RAW_SENTINEL" not in json.dumps(
        getattr(request, "_astrembodiment_semantic_preflight_v1")
    )
    assert event.stopped is False
    assert len(instance._pending) == 1
    assert len(recorder.info_messages) == 1
    assert recorder.warning_messages == []
