from __future__ import annotations

import asyncio
import hashlib
import importlib.util
import inspect
import json
import shutil
import sys
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import astr_embodiment.bridge as bridge_module  # noqa: E402
from astr_embodiment.auxiliary_transport import AuxiliaryTransportError  # noqa: E402
import main as main_module  # noqa: E402
from astr_embodiment.contracts import ScopeTokens  # noqa: E402
from astr_embodiment.persona_genesis import PersonaGenesisError  # noqa: E402
from astr_embodiment.semantic_estimator import (  # noqa: E402
    SemanticEstimateError,
    parse_estimator_output_v3,
)
from astr_embodiment.coordinator import GenesisCoordinator  # noqa: E402
from main import AstrEmbodimentPlugin  # noqa: E402


def _unreachable_mandatory_native_abi(*_args: object, **_kwargs: object) -> str:
    raise AssertionError("loader fixture must not invoke mandatory native ABI stubs")


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
        if provider_id in {self.configured_provider, self.current_provider}:
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


def test_bound_scope_reuses_durable_identity_without_calling_genesis():
    # format_incarnation_id(&[0; 32]) from ae-genesis: 13 Crockford groups.
    durable_incarnation_id = (
        "AE-I1-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000"
    )

    class BoundBridge:
        loaded = True

        def __init__(self) -> None:
            self.genesis_calls = 0
            self.apply_calls = 0

        def inspect(self, _scope):
            return {
                "bound": True,
                "seed_code": "AE-S1-0123456789ABCDEF",
                "incarnation_id": durable_incarnation_id,
                "revision": 14,
            }

        def ensure_genesis(self, _request):
            self.genesis_calls += 1
            raise AssertionError("ordinary reopen must not call ensure_genesis")

        def apply_event(self, _scope, _event):
            self.apply_calls += 1
            return {
                "schema": "astrembodiment.decision.v1",
                "revision": 15,
                "deduplicated": False,
                "context_summary": {
                    "schema": "astrembodiment.context-summary.v1",
                    "summary_revision": 1,
                    "source_continuum_revision": 15,
                    "dimensions_ema_fxp6": [0] * 15,
                    "unresolved_boundary": False,
                    "unresolved_repair": False,
                    "repetition_count": 1,
                    "delivery_outcome": "pending",
                    "summary_digest": "cd" * 32,
                },
            }

    async def run():
        instance = plugin()
        bridge = BoundBridge()
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)  # type: ignore[arg-type]

        async def resolve(*_args, **_kwargs):
            return "persona-a", {"prompt": "already durable"}, "conversation"

        instance.resolve_effective_persona = resolve
        result = await instance._run_genesis(
            FakeEvent(), FakeRequest(), apply_stimulus=True
        )
        return bridge, result

    bridge, result = asyncio.run(run())
    decision, _scope, _session, _seq, _turn, base_revision = result
    assert bridge.genesis_calls == 0
    assert bridge.apply_calls == 1
    assert base_revision == 14
    assert decision["incarnation_id"] == durable_incarnation_id
    assert decision["seed_code"] == "AE-S1-0123456789ABCDEF"


def test_explicit_assistant_provider_is_used_without_fallback():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        instance = plugin(FakeConfig(assistant_provider_id="helper"), context)

        response = await instance._llm_generate(
            FakeEvent(), prompt="compile", system_prompt="compiler"
        )

        return context, response

    context, response = asyncio.run(run())

    assert response == '{"ok": true}'
    assert context.current_calls == 0
    assert context.generate_calls[0] == {
        "chat_provider_id": "helper",
        "prompt": "compile",
        "system_prompt": "compiler",
        "tools": None,
    }


@pytest.mark.parametrize(
    (
        "assistant_provider_id",
        "legacy_provider_id",
        "configured_provider",
        "expected_provider",
        "expected_current_calls",
    ),
    [
        ("assistant", "legacy", "assistant", "assistant", 0),
        ("   ", "legacy", "legacy", "legacy", 0),
        ("   ", "   ", "chat", "chat", 2),
    ],
)
def test_unified_auxiliary_provider_selection_is_shared_by_compiler_and_v3(
    assistant_provider_id: str,
    legacy_provider_id: str,
    configured_provider: str,
    expected_provider: str,
    expected_current_calls: int,
):
    async def run():
        context = FakeContext(
            configured_provider=configured_provider,
            current_provider="chat",
        )

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text=json.dumps(_v3_test_estimate()))

        context.llm_generate = generate
        instance = plugin(
            FakeConfig(
                model_settings={
                    "assistant_provider_id": assistant_provider_id,
                    "semantic_estimator_provider_id": legacy_provider_id,
                },
                observatory_enabled=False,
            ),
            context,
        )
        await instance._llm_generate(
            FakeEvent(), prompt="compile", system_prompt="compiler"
        )
        estimate = await instance._semantic_estimate_v3(
            FakeEvent(),
            {
                "current_turn_text": "current turn",
                "system_prompt": main_module.SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
                "structured_schema": main_module.SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA,
                "input": {"context_summary": {}},
            },
        )
        return context, estimate

    context, estimate = asyncio.run(run())

    assert estimate.as_json() == _v3_test_estimate()
    assert [call["chat_provider_id"] for call in context.generate_calls] == [
        expected_provider,
        expected_provider,
    ]
    assert context.current_calls == expected_current_calls


@pytest.mark.parametrize(
    ("model_settings", "lookup_raises"),
    [
        (
            {
                "assistant_provider_id": "assistant-provider-private-id",
                "semantic_estimator_provider_id": "legacy",
            },
            False,
        ),
        (
            {
                "assistant_provider_id": "   ",
                "semantic_estimator_provider_id": "legacy-provider-private-id",
            },
            True,
        ),
    ],
)
def test_unified_auxiliary_provider_unavailable_is_fail_closed_for_both_consumers(
    model_settings: dict[str, str],
    lookup_raises: bool,
    monkeypatch: pytest.MonkeyPatch,
):
    class RecordingLogger:
        def __init__(self) -> None:
            self.warning_messages: list[str] = []

        def warning(self, template: str, *args: object) -> None:
            self.warning_messages.append(template % args if args else template)

    raw_provider_id = next(
        value for value in model_settings.values() if value not in {"legacy", "   "}
    )
    recorder = RecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)

    async def run():
        context = FakeContext(configured_provider="legacy", current_provider="chat")
        if lookup_raises:

            def get_provider_by_id(provider_id: str):
                context.provider_calls.append(provider_id)
                raise RuntimeError(f"provider lookup failed: {provider_id}")

            context.get_provider_by_id = get_provider_by_id
        instance = plugin(FakeConfig(model_settings=model_settings), context)
        event = FakeEvent()
        request_mapping = {
            "current_turn_text": "current turn",
            "system_prompt": main_module.SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
            "structured_schema": main_module.SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA,
            "input": {"context_summary": {}},
        }
        with pytest.raises(AuxiliaryTransportError) as compiler_error:
            await instance._llm_generate(
                event, prompt="compile", system_prompt="compiler"
            )
        with pytest.raises(SemanticEstimateError) as semantic_error:
            await instance._semantic_estimate_v3(event, request_mapping)
        await instance._stop_genesis_turn(event, str(compiler_error.value))
        return context, event, compiler_error.value, semantic_error.value

    context, event, compiler_error, semantic_error = asyncio.run(run())

    assert context.current_calls == 0
    assert context.generate_calls == []
    assert str(compiler_error) == "ESTIMATOR_UNAVAILABLE"
    assert semantic_error.code == "ESTIMATOR_UNAVAILABLE"
    assert semantic_error.transport_meta is not None
    expected_subcode = (
        "PROVIDER_RESOLUTION_FAILED" if lookup_raises else "PROVIDER_NOT_FOUND"
    )
    assert compiler_error.meta.transport_subcode == expected_subcode
    assert semantic_error.transport_meta.transport_subcode == expected_subcode
    assert compiler_error.meta.attempted is False
    assert compiler_error.meta.attempt_count == 0
    assert raw_provider_id not in "\n".join(recorder.warning_messages)
    assert raw_provider_id not in "\n".join(event.sent)


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


def test_invalid_explicit_assistant_provider_does_not_fallback_or_expose_raw_id():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        raw_provider_id = "missing-provider-id"
        instance = plugin(FakeConfig(assistant_provider_id=raw_provider_id), context)

        with pytest.raises(ValueError, match="辅助模型 Provider 不存在") as exc_info:
            await instance._llm_generate(
                FakeEvent(), prompt="compile", system_prompt="compiler"
            )
        return context, raw_provider_id, exc_info.value

    context, raw_provider_id, error = asyncio.run(run())

    assert context.current_calls == 0
    assert context.generate_calls == []
    assert raw_provider_id not in str(error)


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
        event.turn_token = "turn-before-plugin-reload"
        request = FakeRequest()
        calls: list[str] = []
        stimulus: dict = {}

        async def resolve(_event, _request=None):
            return "persona-1", {"prompt": "测试人格"}, "conversation"

        async def ensure_genesis(**_kwargs):
            calls.append("genesis")

        def inspect(_scope):
            calls.append("inspect")
            return {
                "bound": True,
                "seed_code": "AE-S1-RELOAD",
                "seed_code_short": "AE-S1-RELOAD",
                "incarnation_id": (
                    "AE-I1-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000-0000"
                ),
                "revision": 7,
            }

        async def apply_stimulus(**kwargs):
            calls.append("stimulus")
            stimulus.update(kwargs)
            return {
                "revision": 8,
                "contract": {},
            }

        instance.resolve_effective_persona = resolve
        instance._coordinator.ensure_genesis = ensure_genesis
        instance._bridge._native = object()
        instance._bridge.inspect = inspect
        instance._coordinator.apply_stimulus = apply_stimulus
        await instance.on_llm_request(event, request)
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        return instance, event, scope, calls, stimulus

    instance, event, scope, calls, stimulus = asyncio.run(run())

    assert calls == ["inspect", "stimulus"]
    assert "genesis" not in calls
    assert stimulus["base_revision"] == 7
    assert stimulus["turn_id"] == event.turn_token
    assert stimulus["turn_id"] != "turn-before-plugin-reload"
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

    original_import_module = bridge_module.import_module

    def missing_top_level_core(name: str, package: str | None = None):
        if name == "astrembodiment_core" and package is None:
            raise ModuleNotFoundError(
                "No module named 'astrembodiment_core'", name="astrembodiment_core"
            )
        return original_import_module(name, package)

    monkeypatch.setattr(bridge_module, "import_module", missing_top_level_core)

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
            native.ensure_genesis = lambda *_args: "{}"
            native.flush_and_close = lambda: None
            native.health = lambda: "{}"
            native.inspect = lambda *_args: "{}"
            native.open = lambda *_args: None
            native.prepare_rebirth_v1 = _unreachable_mandatory_native_abi
            native.confirm_rebirth_v1 = _unreachable_mandatory_native_abi
            native.semantic_revision_v1 = _unreachable_mandatory_native_abi
            native.apply_perception_proposal_v1 = _unreachable_mandatory_native_abi
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
    """The package must not let a stale root module shadow the fresh stage."""
    staged_package = ROOT / "astrembodiment_core"
    manifest_path = staged_package / "_bundled" / "manifest.json"
    if not manifest_path.is_file():
        pytest.fail(
            "current fresh Windows native stage is required at "
            f"{manifest_path}; do not substitute a historical wheel"
        )
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    assert isinstance(manifest, dict), "fresh stage manifest must be an object"
    assert manifest.get("schema") == "astrembodiment-native-bundle-v1", (
        "fresh stage must use the content-addressed native manifest"
    )
    platforms = manifest.get("platforms") if isinstance(manifest, dict) else None
    entry = platforms.get("win32") if isinstance(platforms, dict) else None
    assert isinstance(entry, dict), "fresh stage must contain the win32 entry"
    build_id = entry.get("build_id")
    native_filename = entry.get("filename")
    assert isinstance(build_id, str) and len(build_id) == 64
    assert isinstance(native_filename, str) and native_filename.endswith(".pyd")
    staged_native = staged_package / "_bundled" / build_id / native_filename
    assert staged_native.is_file(), "fresh stage must contain its manifest binary"
    bundled_payload = staged_native.read_bytes()
    assert hashlib.sha256(bundled_payload).hexdigest() == build_id

    package_dir = tmp_path / "astrembodiment_core"
    bundled_dir = package_dir / "_bundled" / build_id
    bundled_dir.mkdir(parents=True)
    copied_native = bundled_dir / native_filename
    copied_native.write_bytes(bundled_payload)
    assert hashlib.sha256(copied_native.read_bytes()).hexdigest() == build_id
    shutil.copyfile(manifest_path, package_dir / "_bundled" / "manifest.json")
    (package_dir / "_native.py").write_text(
        "version=lambda: 'stale-root'\nhealth=lambda: '{}'\n",
        encoding="utf-8",
    )
    staged_initializer = staged_package / "__init__.py"
    assert staged_initializer.is_file(), "fresh stage must contain its initializer"
    shutil.copyfile(staged_initializer, package_dir / "__init__.py")

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
        assert module.version() == "1.0.0"
        assert callable(module.apply_event)
        assert Path(sys.modules[f"{module_name}._native"].__file__).parts[-3:] == (
            "_bundled",
            build_id,
            native_filename,
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
            module.prepare_rebirth_v1 = _unreachable_mandatory_native_abi
            module.confirm_rebirth_v1 = _unreachable_mandatory_native_abi
            module.semantic_revision_v1 = _unreachable_mandatory_native_abi
            module.apply_perception_proposal_v1 = _unreachable_mandatory_native_abi
            module.inspect = lambda *_args: "{}"
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


def test_schema_exposes_one_unified_chinese_provider_selector_and_seed_fields():
    schema = json.loads((ROOT / "_conf_schema.json").read_text(encoding="utf-8"))

    model_settings = schema["model_settings"]["items"]
    provider = model_settings["assistant_provider_id"]
    legacy_provider = model_settings["semantic_estimator_provider_id"]
    seed = schema["seed_code"]
    assert provider["_special"] == "select_provider"
    assert provider["default"] == ""
    assert (
        provider["hint"]
        == "统一用于辅助能力与当前请求的闭合的十五维语义估计；留空时使用当前会话 Provider。"
    )
    assert legacy_provider["type"] == "string"
    assert legacy_provider["default"] == ""
    assert legacy_provider["invisible"] is True
    visible_provider_selectors = [
        key
        for key, metadata in model_settings.items()
        if metadata.get("_special") == "select_provider"
        and not metadata.get("invisible", False)
    ]
    assert visible_provider_selectors == ["assistant_provider_id"]
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


_V3_TEST_DIMENSIONS = (
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
_V3_TEST_REGIONS = (
    ("interoception_allostasis", 2_048),
    ("affective_valuation", 2_048),
    ("salience", 1_024),
    ("epistemic_fallibility", 2_048),
    ("social_boundary", 2_048),
    ("temper_inhibitory", 1_024),
    ("world_model_imagination", 4_096),
    ("global_workspace", 1_024),
    ("action_expression", 1_024),
)


class _SemanticRecordingLogger:
    def __init__(self) -> None:
        self.warning_messages: list[str] = []
        self.info_messages: list[str] = []

    def warning(self, template: str, *args: object) -> None:
        self.warning_messages.append(template % args if args else template)

    def info(self, template: str, *args: object) -> None:
        self.info_messages.append(template % args if args else template)

    def error(self, _template: str, *_args: object) -> None:
        return None

    def debug(self, _template: str, *_args: object) -> None:
        return None

    def exception(self, _template: str, *_args: object) -> None:
        return None


def _v3_test_scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="11" * 16,
        persona_token="22" * 16,
        session_token="33" * 16,
    )


def _v3_test_context_summary() -> dict:
    return {
        "schema": "astrembodiment.context-summary.v1",
        "summary_revision": 1,
        "source_continuum_revision": 8,
        "dimensions_ema_fxp6": [0] * 15,
        "unresolved_boundary": False,
        "unresolved_repair": False,
        "repetition_count": 1,
        "delivery_outcome": "pending",
        "summary_digest": "ab" * 32,
    }


def _v3_test_genesis_result(scope: ScopeTokens) -> tuple:
    return (
        {
            "genesis": {
                "seed_code": "AE-S1-SEMANTIC",
                "incarnation_id": "AE-I1-SEMANTIC",
            },
            "seed_code": "AE-S1-SEMANTIC",
            "incarnation_id": "AE-I1-SEMANTIC",
            "revision": 8,
            "contract": {"continuous": {"directness": 500_000}},
            "context_summary": _v3_test_context_summary(),
        },
        scope,
        scope.session_token,
        0,
        "44" * 16,
        7,
    )


def _v3_test_estimate() -> dict:
    dimensions = {
        name: {
            "state": "ABSENT",
            "intensity_fxp6": 0,
            "confidence_fxp6": 900_000,
        }
        for name in _V3_TEST_DIMENSIONS
    }
    dimensions["rejection"] = {
        "state": "PRESENT",
        "intensity_fxp6": 900_000,
        "confidence_fxp6": 900_000,
    }
    return {"schema": "astr-embodiment.semantic-estimate.v3", "dimensions": dimensions}


def test_v3_estimator_accepts_only_one_exact_json_fence():
    expected = _v3_test_estimate()
    bare_completion = json.dumps(expected)
    fenced_completion = f"\t\r\n```json\r\n{bare_completion}\r\n```\n \t"

    assert parse_estimator_output_v3(bare_completion).as_json() == expected
    assert parse_estimator_output_v3(fenced_completion).as_json() == expected

    for rejected_completion in (
        f"{bare_completion}\n{bare_completion}",
        f"explanation:\n{bare_completion}",
    ):
        with pytest.raises(SemanticEstimateError) as exc_info:
            parse_estimator_output_v3(rejected_completion)
        assert exc_info.value.code == "ESTIMATOR_MALFORMED"
        assert exc_info.value.subcode == "JSON_DECODE"


def _v3_test_native_closure() -> dict:
    regions = []
    for region_id, (region_name, capacity) in enumerate(_V3_TEST_REGIONS):
        selected = 1 if region_name == "affective_valuation" else 0
        component = {
            "before_mean_fxp6": 0,
            "after_mean_fxp6": selected,
            "delta_mean_fxp6": selected,
            "changed_node_count": selected,
            "nonzero_after_count": selected,
        }
        regions.append(
            {
                "region_id": region_id,
                "region_name": region_name,
                "node_capacity": capacity,
                "selected_node_count": selected,
                "activated_node_count": selected,
                "changed_node_count": selected,
                "potential": dict(component),
                "excitation": dict(component),
            }
        )
    return {
        "schema": "astrembodiment.semantic-perception-closure.v1",
        "receipt": {
            "schema_version": 1,
            "formula_digest": "00" * 32,
            "scope_digest": "11" * 32,
            "event_digest": "22" * 32,
            "authority_digest": "33" * 32,
            "base_revision": 0,
            "next_revision": 1,
            "state_before": "44" * 32,
            "state_after": "55" * 32,
            "graph_after": "66" * 32,
            "active_nodes": 1,
            "active_edges": 0,
            "residuals": {
                "authority": 0,
                "continuity": 0,
                "energy": 0,
                "renormalization": 0,
                "capacity": 0,
            },
            "status": "committed",
        },
        "semantic_vector_receipt": {
            "schema": "astr-embodiment.semantic-vector-receipt.v2",
            "formula": "full-vector-route-neutral-relaxation-v1",
            "dimension_slot_count": 15,
            "evaluated_dimension_count": 15,
            "injected_dimension_count": 15,
            "nonzero_evidence_dimension_count": 1,
            "neutral_baseline_dimension_count": 14,
            "unavailable_dimension_count": 0,
            "state_changed": True,
        },
        "node_observability": {
            "schema": "astr-embodiment.node-observability.v1",
            "formula": "spc1-node-observability-v1",
            "revision": 1,
            "field_node_capacity": 16_384,
            "region_layout": "regions-v1",
            "counts": {
                "selected_node_count": 1,
                "activated_node_count": 1,
                "changed_node_count": 1,
                "potential_nonzero_after_count": 1,
                "excitation_nonzero_after_count": 1,
                "signal_nonzero_after_count": 1,
            },
            "residuals": {
                "state": "NOT_COMPUTED",
                "formula": None,
                "values_fxp6": None,
            },
            "regions": regions,
        },
        "revision": 1,
        "deduplicated": False,
        "expression_projection": {
            "schema": "astr-embodiment.expression-projection.v1",
            "revision": 1,
            "profile_fxp6": {
                "warmth": 100_000,
                "sensitivity": 200_000,
                "guardedness": 800_000,
                "repair_orientation": 0,
                "engagement": 100_000,
                "epistemic_caution": 300_000,
            },
        },
    }


def _v3_test_native_closure_v2() -> dict:
    closure = _v3_test_native_closure()
    closure["schema"] = "astrembodiment.semantic-perception-closure.v2"
    closure["availability"] = "AVAILABLE"
    closure["migration_subcode"] = None
    closure["telemetry_receipt"] = {
        "schema": "native-telemetry-receipt.v1",
        "formula": "phase0-native-propagation-fxp6-v1",
        "formula_digest": closure["receipt"]["formula_digest"],
        "scope_digest": closure["receipt"]["scope_digest"],
        "event_digest": closure["receipt"]["event_digest"],
        "source_digest": "77" * 32,
        "base_revision": closure["receipt"]["base_revision"],
        "next_revision": closure["receipt"]["next_revision"],
        "phase": "PREPARE",
        "state_before": closure["receipt"]["state_before"],
        "state_after": closure["receipt"]["state_after"],
        "graph_before": "88" * 32,
        "graph_after": closure["receipt"]["graph_after"],
        "local_digest": "99" * 32,
        "compensation_digest": "aa" * 32,
        "effective_digest": "bb" * 32,
        "energy": {
            "reserve_before": 0,
            "reserve_after": 0,
            "recovered": 0,
            "spent": 0,
            "headroom": 0,
            "residual": 0,
        },
        "capacity": {
            "upper_saturated_nodes": 0,
            "node_limit": 16_384,
            "node_headroom": 1_000_000,
            "edge_used": 0,
            "edge_limit": 524_288,
            "edge_headroom": 1_000_000,
            "headroom": 1_000_000,
            "residual": 0,
        },
        "residuals": closure["receipt"]["residuals"],
        "residual_health": 1_000_000,
        "native_gate": 0,
        "checkpoint_digest": "cc" * 32,
        "telemetry_digest": "dd" * 32,
    }
    return closure


@pytest.mark.parametrize(
    ("wire_subcode", "expected_subcode"),
    (
        (None, None),
        ("FIELD_MIGRATION_APPLIED", "FIELD_MIGRATION_APPLIED"),
    ),
)
def test_v2_semantic_closure_accepts_exact_migration_subcodes(
    wire_subcode: str | None, expected_subcode: str | None
) -> None:
    """Native v2 accepts JSON null or one exact frozen migration code."""

    closure = _v3_test_native_closure_v2()
    closure["migration_subcode"] = wire_subcode

    result = bridge_module.validate_semantic_result(closure)

    assert result["migration_subcode"] == expected_subcode


@pytest.mark.parametrize("wire_subcode", ("untrusted-migration-subcode", 17))
def test_v2_semantic_closure_rejects_nonnull_unknown_migration_subcodes(
    wire_subcode: object,
) -> None:
    """Unknown non-null native success telemetry must never become a closure."""

    closure = _v3_test_native_closure_v2()
    closure["migration_subcode"] = wire_subcode

    with pytest.raises(ValueError):
        bridge_module.validate_semantic_result(closure)


def test_v3_estimator_delivers_closed_schema_and_logs_malformed_metadata(
    monkeypatch: pytest.MonkeyPatch,
):
    class HostResponse:
        def __init__(self, completion_text: str) -> None:
            self.completion_text = completion_text

    current_turn_text = "private current turn must remain the sole user prompt"
    canonical_completion = json.dumps(_v3_test_estimate())
    malformed_estimate = _v3_test_estimate()
    malformed_estimate["dimensions"]["unexpected_dimension"] = {
        "private_completion_secret": "must not be logged"
    }
    malformed_completion = json.dumps(malformed_estimate)

    def request_mapping() -> dict:
        return {
            "current_turn_text": current_turn_text,
            "system_prompt": main_module.SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
            "structured_schema": main_module.SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA,
            "input": {"context_summary": {}},
        }

    def instance_for(completion_text: str):
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return HostResponse(completion_text)

        context.llm_generate = generate
        return (
            plugin(
                FakeConfig(
                    model_settings={"assistant_provider_id": "semantic"},
                    observatory_enabled=False,
                ),
                context,
            ),
            context,
        )

    instance, context = instance_for(canonical_completion)
    canonical_result = asyncio.run(
        instance._semantic_estimate_v3(FakeEvent(), request_mapping())
    )

    assert len(context.generate_calls) == 1
    provider_call = context.generate_calls[0]
    canonical_schema = json.dumps(
        main_module.SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )
    assert provider_call["prompt"] == current_turn_text
    assert set(provider_call) == {
        "chat_provider_id",
        "prompt",
        "system_prompt",
        "tools",
    }
    assert provider_call["tools"] is None
    assert current_turn_text not in provider_call["system_prompt"]
    assert canonical_schema in provider_call["system_prompt"]
    assert all(
        dimension_name in provider_call["system_prompt"]
        for dimension_name in _V3_TEST_DIMENSIONS
    )
    assert canonical_result.as_json() == _v3_test_estimate()

    recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    malformed_instance, malformed_context = instance_for(malformed_completion)
    with pytest.raises(SemanticEstimateError) as exc_info:
        asyncio.run(
            malformed_instance._semantic_estimate_v3(FakeEvent(), request_mapping())
        )

    assert exc_info.value.code == "ESTIMATOR_MALFORMED"
    assert exc_info.value.subcode == "DIMENSION_KEYS"
    assert len(malformed_context.generate_calls) == 1
    assert len(recorder.warning_messages) == 1
    warning_payload = json.loads(recorder.warning_messages[0])
    assert warning_payload == {
        "return_type": "canonical_completion_text",
        "extraction_path": "canonical_text",
        "character_length": len(malformed_completion),
        "sha256": hashlib.sha256(malformed_completion.encode("utf-8")).hexdigest(),
        "subcode": "DIMENSION_KEYS",
        "transport_subcode": "NONE",
        "attempted": True,
        "attempt_count": 1,
    }
    assert current_turn_text not in recorder.warning_messages[0]
    assert malformed_completion not in recorder.warning_messages[0]


def test_v3_rejection_text_commits_nonzero_semantics_and_injects_same_turn_expression(
    monkeypatch: pytest.MonkeyPatch,
):
    class NativeAbi:
        def __init__(self) -> None:
            self.cursor_calls = 0
            self.proposals: list[dict] = []

        def semantic_revision_v1(self, _scope_json: str) -> str:
            self.cursor_calls += 1
            return json.dumps(
                {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
            )

        def apply_perception_proposal_v1(
            self, _scope_json: str, proposal_json: str
        ) -> str:
            self.proposals.append(json.loads(proposal_json))
            return json.dumps(_v3_test_native_closure_v2())

    async def run():
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text=json.dumps(_v3_test_estimate()))

        context.llm_generate = generate
        instance = plugin(
            FakeConfig(
                model_settings={"assistant_provider_id": "semantic"},
                observatory_enabled=False,
            ),
            context,
        )
        native = NativeAbi()
        bridge = bridge_module.NativeBridge()
        bridge._native = native
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)
        scope = _v3_test_scope()

        async def run_genesis(*_args, **_kwargs):
            return _v3_test_genesis_result(scope)

        instance._run_genesis = run_genesis
        request = FakeRequest()
        request.prompt = "请不要再联系我，我明确拒绝。"
        event = FakeEvent()
        observed_outcomes: list[dict[str, object]] = []
        original_preflight = instance._coordinator.preflight_semantic_v3

        async def capture_preflight(**kwargs):
            result = await original_preflight(**kwargs)
            observed_outcomes.append(result)
            return result

        instance._coordinator.preflight_semantic_v3 = capture_preflight
        await instance.on_llm_request(event, request)
        return instance, context, native, event, request, observed_outcomes

    recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    _instance, context, native, event, request, observed_outcomes = asyncio.run(run())

    assert event.stopped is False
    assert native.cursor_calls == 1
    assert len(native.proposals) == 1
    assert native.proposals[0]["dimensions"]["rejection"] == 900_000
    assert native.proposals[0]["dimensions"] != {
        name: 0 for name in _V3_TEST_DIMENSIONS
    }
    assert native.proposals[0]["estimator_confidence"] == 900_000
    assert observed_outcomes[0]["migration_subcode"] is None
    assert len(context.generate_calls) == 1
    provider_call = context.generate_calls[0]
    assert provider_call["chat_provider_id"] == "semantic"
    assert provider_call["prompt"] == "请不要再联系我，我明确拒绝。"
    assert provider_call["tools"] is None
    assert provider_call["prompt"] not in provider_call["system_prompt"]
    assert request.prompt == "请不要再联系我，我明确拒绝。"
    assert request.contexts == [{"role": "user", "content": "历史"}]
    assert request.system_prompt.count("AE Affect Expression Context") == 1
    semantic_record = getattr(
        request, "_astrembodiment_semantic_observatory_record_v1", {}
    )
    assert {
        key: semantic_record.get(key)
        for key in (
            "status",
            "code",
            "calibration_state",
            "expression_state",
            "migration_subcode",
        )
    } == {
        "status": "DEGRADED",
        "code": "HUMAN_GOLD_UNVERIFIED",
        "calibration_state": "UNVERIFIED_HUMAN_GOLD",
        "expression_state": "APPLIED",
        "migration_subcode": None,
    }
    assert len(recorder.warning_messages) == 1
    assert recorder.info_messages == []


def test_v3_unknown_success_migration_subcode_fails_closed_before_expression(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    raw_migration_subcode = "untrusted-migration-subcode private-provider-id"

    class NativeAbi:
        def semantic_revision_v1(self, _scope_json: str) -> str:
            return json.dumps(
                {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
            )

        def apply_perception_proposal_v1(
            self, _scope_json: str, _proposal_json: str
        ) -> str:
            closure = _v3_test_native_closure_v2()
            closure["migration_subcode"] = raw_migration_subcode
            return json.dumps(closure)

    async def run() -> tuple[dict[str, object], FakeRequest]:
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text=json.dumps(_v3_test_estimate()))

        context.llm_generate = generate
        instance = plugin(
            FakeConfig(
                model_settings={"assistant_provider_id": "semantic"},
                observatory_enabled=False,
            ),
            context,
        )
        bridge = bridge_module.NativeBridge()
        bridge._native = NativeAbi()
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)
        scope = _v3_test_scope()

        async def run_genesis(*_args, **_kwargs):
            return _v3_test_genesis_result(scope)

        instance._run_genesis = run_genesis
        observed_outcome: dict[str, object] = {}
        original_preflight = instance._coordinator.preflight_semantic_v3

        async def capture_preflight(**kwargs):
            result = await original_preflight(**kwargs)
            observed_outcome.update(result)
            return result

        instance._coordinator.preflight_semantic_v3 = capture_preflight
        request = FakeRequest()
        request.prompt = "请停止并保持边界。"
        await instance.on_llm_request(FakeEvent(), request)
        return observed_outcome, request

    recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    observed_outcome, request = asyncio.run(run())

    assert observed_outcome == {
        "status": "DEGRADED",
        "code": "NATIVE_ERROR",
        "cause_code": "NATIVE_ERROR",
        "native_stage": "NATIVE_APPLY",
        "migration_subcode": "FIELD_MIGRATION_UNKNOWN",
    }
    semantic_record = getattr(
        request, "_astrembodiment_semantic_observatory_record_v1", {}
    )
    assert semantic_record["expression_state"] == "NOT_ATTEMPTED"
    assert semantic_record["cause_code"] == "NATIVE_ERROR"
    assert semantic_record["migration_subcode"] == "FIELD_MIGRATION_UNKNOWN"
    assert raw_migration_subcode not in "\n".join(recorder.warning_messages)


def test_expression_not_attempted_is_warn_with_explicit_code_and_reason(
    monkeypatch: pytest.MonkeyPatch,
):
    class NativeAbi:
        def semantic_revision_v1(self, _scope_json: str) -> str:
            return json.dumps(
                {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
            )

    async def run():
        context = FakeContext(configured_provider="semantic")

        async def unavailable_generate(**_kwargs):
            raise RuntimeError("provider unavailable")

        context.llm_generate = unavailable_generate
        instance = plugin(
            FakeConfig(
                model_settings={"assistant_provider_id": "semantic"},
                observatory_enabled=False,
            ),
            context,
        )
        bridge = bridge_module.NativeBridge()
        bridge._native = NativeAbi()
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)
        scope = _v3_test_scope()

        async def run_genesis(*_args, **_kwargs):
            return _v3_test_genesis_result(scope)

        instance._run_genesis = run_genesis
        request = FakeRequest()
        request.prompt = "请停止。"
        await instance.on_llm_request(FakeEvent(), request)
        return request

    recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    request = asyncio.run(run())

    semantic_record = getattr(
        request, "_astrembodiment_semantic_observatory_record_v1", {}
    )
    assert {
        key: semantic_record.get(key)
        for key in ("expression_state", "code", "reason", "cause_code")
    } == {
        "expression_state": "NOT_ATTEMPTED",
        "code": "EXPRESSION_NOT_ATTEMPTED",
        "reason": "EXPRESSION_NOT_ATTEMPTED",
        "cause_code": "ESTIMATOR_UNAVAILABLE",
    }
    assert recorder.warning_messages[0] == (
        "AstrEmbodiment semantic transport failure: "
        "code=ESTIMATOR_UNAVAILABLE transport_subcode=PROVIDER_CALL_FAILED "
        "attempted=True attempt_count=2"
    )
    assert len(recorder.warning_messages) == 2
    assert recorder.info_messages == []


@pytest.mark.parametrize(
    ("message", "expected_state_subcode"),
    (
        (
            "INVALID_NEURAL_STATE::GRAPH_STATE_INVALID",
            "GRAPH_STATE_INVALID",
        ),
        (
            "INVALID_NEURAL_STATE::UNMAPPED_FUTURE_REJECTION",
            "UNKNOWN_INVALID_NEURAL_STATE",
        ),
        (
            "INVALID_NEURAL_STATE::GRAPH_STATE_INVALID::private-native-detail",
            "UNKNOWN_INVALID_NEURAL_STATE",
        ),
    ),
)
def test_invalid_neural_state_classification_requires_exact_closed_subcode(
    message: str, expected_state_subcode: str
) -> None:
    classified = bridge_module._classify(RuntimeError(message))

    assert isinstance(classified, bridge_module.InvalidNeuralState)
    assert classified.code == "INVALID_NEURAL_STATE"
    assert classified.state_subcode == expected_state_subcode
    assert classified.detail == expected_state_subcode
    assert "private-native-detail" not in str(classified)


@pytest.mark.parametrize(
    (
        "native_code",
        "native_state_subcode",
        "native_migration_subcode",
        "expected_state_subcode",
        "expected_migration_subcode",
        "expected_stage",
    ),
    (
        (
            "LEGACY_UNATTESTED",
            None,
            "FIELD_MIGRATION_REFUSED_STRUCTURE",
            None,
            "FIELD_MIGRATION_REFUSED_STRUCTURE",
            "NATIVE_APPLY",
        ),
        (
            "INVALID_NEURAL_STATE",
            "GRAPH_STATE_INVALID",
            "FIELD_MIGRATION_REFUSED_RANGE",
            "GRAPH_STATE_INVALID",
            "FIELD_MIGRATION_REFUSED_RANGE",
            "NATIVE_APPLY",
        ),
        (
            "INVALID_NEURAL_STATE",
            "GRAPH_STATE_INVALID::raw-native-detail provider-id=private-provider-id",
            "FIELD_MIGRATION_REFUSED_RANGE::raw-native-detail provider-id=private-provider-id",
            "UNKNOWN_INVALID_NEURAL_STATE",
            "FIELD_MIGRATION_UNKNOWN",
            "NATIVE_APPLY",
        ),
        (
            "STORAGE",
            None,
            None,
            None,
            "FIELD_MIGRATION_UNKNOWN",
            "NATIVE_APPLY",
        ),
    ),
)
def test_semantic_native_failures_preserve_exact_safe_code_and_stage(
    monkeypatch: pytest.MonkeyPatch,
    native_code: str,
    native_state_subcode: str | None,
    native_migration_subcode: str | None,
    expected_state_subcode: str | None,
    expected_migration_subcode: str,
    expected_stage: str,
):
    """PyO3 codes survive the Python preview path without exposing detail."""

    native_detail = "private native detail provider-id=private-provider-id"

    class NativeAbi:
        def semantic_revision_v1(self, _scope_json: str) -> str:
            return json.dumps(
                {"schema": "astrembodiment.semantic-revision.v1", "revision": 2}
            )

        def apply_perception_proposal_v1(
            self, _scope_json: str, _proposal_json: str
        ) -> str:
            native_suffix = native_state_subcode or native_detail
            error = RuntimeError(f"{native_code}::{native_suffix}")
            if native_migration_subcode is not None:
                error.migration_subcode = native_migration_subcode
            raise error

    async def run() -> tuple[dict[str, object], FakeRequest]:
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text=json.dumps(_v3_test_estimate()))

        context.llm_generate = generate
        instance = plugin(
            FakeConfig(
                model_settings={"assistant_provider_id": "semantic"},
                observatory_enabled=False,
            ),
            context,
        )
        bridge = bridge_module.NativeBridge()
        bridge._native = NativeAbi()
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)
        scope = _v3_test_scope()

        async def run_genesis(*_args, **_kwargs):
            return _v3_test_genesis_result(scope)

        instance._run_genesis = run_genesis
        observed_outcome: dict[str, object] = {}
        original_preflight = instance._coordinator.preflight_semantic_v3

        async def capture_preflight(**kwargs):
            result = await original_preflight(**kwargs)
            observed_outcome.update(result)
            return result

        instance._coordinator.preflight_semantic_v3 = capture_preflight
        request = FakeRequest()
        request.prompt = "请停止并保持边界。"
        await instance.on_llm_request(FakeEvent(), request)
        return observed_outcome, request

    recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    observed_outcome, request = asyncio.run(run())

    expected_outcome: dict[str, object] = {
        "status": "DEGRADED",
        "code": native_code,
        "cause_code": native_code,
        "native_stage": expected_stage,
        "transport_subcode": "NONE",
        "attempted": True,
        "attempt_count": 1,
    }
    if expected_state_subcode is not None:
        expected_outcome["state_subcode"] = expected_state_subcode
    expected_outcome["migration_subcode"] = expected_migration_subcode
    assert observed_outcome == expected_outcome
    semantic_record = getattr(
        request, "_astrembodiment_semantic_observatory_record_v1", {}
    )
    assert {
        key: semantic_record.get(key)
        for key in (
            "expression_state",
            "code",
            "reason",
            "cause_code",
            "state_subcode",
            "migration_subcode",
            "dimensions_fxp6",
            "revision",
        )
    } == {
        "expression_state": "NOT_ATTEMPTED",
        "code": "EXPRESSION_NOT_ATTEMPTED",
        "reason": "EXPRESSION_NOT_ATTEMPTED",
        "cause_code": native_code,
        "state_subcode": expected_state_subcode,
        "migration_subcode": expected_migration_subcode,
        "dimensions_fxp6": None,
        "revision": None,
    }
    expected_warning = (
        "AstrEmbodiment semantic native failure: "
        f"code={native_code} stage={expected_stage}"
    )
    if expected_state_subcode is not None:
        expected_warning += f" state_subcode={expected_state_subcode}"
    expected_warning += f" migration_subcode={expected_migration_subcode}"
    assert recorder.warning_messages[0] == expected_warning
    assert len(recorder.warning_messages) == 2
    assert (
        json.loads(
            recorder.warning_messages[1].removeprefix(main_module._OBSERVATORY_PREFIX)
        )
        == semantic_record
    )
    assert native_detail not in "\n".join(recorder.warning_messages)
    assert "raw-native-detail" not in "\n".join(recorder.warning_messages)
    assert "private-provider-id" not in "\n".join(recorder.warning_messages)


def test_v3_dimension_value_provider_warn_includes_first_safe_diagnostic(
    monkeypatch: pytest.MonkeyPatch,
):
    class NativeAbi:
        def __init__(self) -> None:
            self.cursor_calls = 0
            self.proposal_calls = 0

        def semantic_revision_v1(self, _scope_json: str) -> str:
            self.cursor_calls += 1
            return json.dumps(
                {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
            )

        def apply_perception_proposal_v1(
            self, _scope_json: str, _proposal_json: str
        ) -> str:
            self.proposal_calls += 1
            raise AssertionError("malformed estimate must not reach native")

    malformed_estimate = _v3_test_estimate()
    malformed_estimate["dimensions"]["rejection"]["confidence_fxp6"] = 0.75
    malformed_completion = json.dumps(malformed_estimate)
    expected_diagnostic = {
        "dimension_name": "rejection",
        "value_classification": "CONFIDENCE_NON_INTEGRAL_NUMBER",
        "json_type": "number",
        "numeric_scalar": 0.75,
    }

    async def run():
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text=malformed_completion)

        context.llm_generate = generate
        instance = plugin(
            FakeConfig(
                model_settings={"assistant_provider_id": "semantic"},
                observatory_enabled=False,
            ),
            context,
        )
        native = NativeAbi()
        bridge = bridge_module.NativeBridge()
        bridge._native = native
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)
        scope = _v3_test_scope()

        async def run_genesis(*_args, **_kwargs):
            return _v3_test_genesis_result(scope)

        instance._run_genesis = run_genesis
        request = FakeRequest()
        request.prompt = "只验证安全诊断传播。"
        await instance.on_llm_request(FakeEvent(), request)
        return context, native, request

    recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", recorder)
    context, native, request = asyncio.run(run())

    assert len(context.generate_calls) == 1
    assert native.cursor_calls == 1
    assert native.proposal_calls == 0
    assert len(recorder.warning_messages) == 2
    provider_warning = json.loads(recorder.warning_messages[0])
    assert provider_warning.get("subcode") == "DIMENSION_VALUE"
    assert provider_warning.get("dimension_diagnostic") == expected_diagnostic
    assert malformed_completion not in "\n".join(recorder.warning_messages)


def test_v3_positive_null_schema_contract_and_e2e_cause_preservation(
    monkeypatch: pytest.MonkeyPatch,
):
    """Keep schema, prompt, parser, proposal admission, and observatory aligned."""

    class NativeAbi:
        def __init__(self, *, closure: dict | None) -> None:
            self.closure = closure
            self.cursor_calls = 0
            self.proposal_calls = 0

        def semantic_revision_v1(self, _scope_json: str) -> str:
            self.cursor_calls += 1
            return json.dumps(
                {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
            )

        def apply_perception_proposal_v1(
            self, _scope_json: str, _proposal_json: str
        ) -> str:
            self.proposal_calls += 1
            if self.closure is None:
                raise AssertionError(
                    "this estimate must not reach native proposal apply"
                )
            return json.dumps(self.closure)

    async def run(completion_text: str, native: NativeAbi):
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text=completion_text)

        context.llm_generate = generate
        instance = plugin(
            FakeConfig(
                model_settings={"assistant_provider_id": "semantic"},
                observatory_enabled=False,
            ),
            context,
        )
        bridge = bridge_module.NativeBridge()
        bridge._native = native
        instance._bridge = bridge
        instance._coordinator = GenesisCoordinator(bridge)
        scope = _v3_test_scope()

        async def run_genesis(*_args, **_kwargs):
            return _v3_test_genesis_result(scope)

        instance._run_genesis = run_genesis
        request = FakeRequest()
        request.prompt = "请停止并保持边界。"
        await instance.on_llm_request(FakeEvent(), request)
        return context, request

    expected_dimension_schema = {
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": "PRESENT"},
                    "intensity_fxp6": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 1_000_000,
                    },
                    "confidence_fxp6": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 1_000_000,
                    },
                },
            },
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": "ABSENT"},
                    "intensity_fxp6": {"const": 0},
                    "confidence_fxp6": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 1_000_000,
                    },
                },
            },
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": "UNAVAILABLE"},
                    "intensity_fxp6": {"type": "null"},
                    "confidence_fxp6": {
                        "type": "integer",
                        "minimum": 0,
                        "maximum": 1_000_000,
                    },
                },
            },
        ]
    }
    expected_prompt_fragments = (
        "State × intensity algebra (mutually exclusive):",
        "PRESENT: intensity_fxp6 must be an integer from 1 through 1000000.",
        "ABSENT: intensity_fxp6 must be the integer 0.",
        "UNAVAILABLE: intensity_fxp6 must be JSON null.",
        "JSON null is allowed only with UNAVAILABLE.",
        "If reliable current-turn presence cannot be determined, select UNAVAILABLE with JSON null.",
        "If current-turn evaluation finds no evidence, select ABSENT with integer 0.",
    )
    contract_failures: list[str] = []
    structured_schema = main_module.SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA
    dimensions_schema = structured_schema["properties"]["dimensions"]
    if dimensions_schema["$ref"] if "$ref" in dimensions_schema else False:
        contract_failures.append("dimensions schema must be inline and closed")
    if dimensions_schema["required"] != list(_V3_TEST_DIMENSIONS):
        contract_failures.append("schema must retain the canonical ordered 15D set")
    if structured_schema["$defs"]["dimension"] != expected_dimension_schema:
        contract_failures.append(
            "schema must expose the three mutually exclusive state branches"
        )

    malformed_estimate = _v3_test_estimate()
    malformed_estimate["dimensions"]["positive"] = {
        "state": "PRESENT",
        "intensity_fxp6": None,
        "confidence_fxp6": 900_000,
    }
    malformed_completion = json.dumps(malformed_estimate)
    with pytest.raises(SemanticEstimateError) as exc_info:
        parse_estimator_output_v3(malformed_completion)
    assert exc_info.value.code == "ESTIMATOR_MALFORMED"
    assert exc_info.value.subcode == "DIMENSION_VALUE"
    assert exc_info.value.diagnostic_json() == {
        "dimension_name": "positive",
        "value_classification": "INTENSITY_NULL_DISALLOWED",
        "json_type": "null",
    }

    valid_recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", valid_recorder)
    valid_native = NativeAbi(closure=_v3_test_native_closure())
    valid_context, _valid_request = asyncio.run(
        run(json.dumps(_v3_test_estimate()), valid_native)
    )
    assert len(valid_context.generate_calls) == 1
    assert valid_native.cursor_calls == 1
    assert valid_native.proposal_calls == 1

    unavailable_estimate = _v3_test_estimate()
    unavailable_estimate["dimensions"]["positive"] = {
        "state": "UNAVAILABLE",
        "intensity_fxp6": None,
        "confidence_fxp6": 900_000,
    }
    unavailable_recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", unavailable_recorder)
    unavailable_native = NativeAbi(closure=None)
    unavailable_context, unavailable_request = asyncio.run(
        run(json.dumps(unavailable_estimate), unavailable_native)
    )
    assert len(unavailable_context.generate_calls) == 1
    assert unavailable_native.cursor_calls == 1
    assert unavailable_native.proposal_calls == 0
    unavailable_record = getattr(
        unavailable_request, "_astrembodiment_semantic_observatory_record_v1", {}
    )
    assert unavailable_record.get("cause_code") == "SEMANTIC_VECTOR_UNAVAILABLE"

    malformed_recorder = _SemanticRecordingLogger()
    monkeypatch.setattr(main_module, "logger", malformed_recorder)
    malformed_native = NativeAbi(closure=None)
    malformed_context, malformed_request = asyncio.run(
        run(malformed_completion, malformed_native)
    )
    assert len(malformed_context.generate_calls) == 1
    assert malformed_native.cursor_calls == 1
    assert malformed_native.proposal_calls == 0
    provider_call = malformed_context.generate_calls[0]
    canonical_schema = json.dumps(
        structured_schema,
        sort_keys=True,
        separators=(",", ":"),
        allow_nan=False,
    )
    if canonical_schema not in provider_call["system_prompt"]:
        contract_failures.append(
            "provider prompt must carry the canonical structured schema"
        )
    for fragment in expected_prompt_fragments:
        if fragment not in provider_call["system_prompt"]:
            contract_failures.append(
                f"provider prompt missing canonical rule: {fragment}"
            )
    malformed_record = getattr(
        malformed_request, "_astrembodiment_semantic_observatory_record_v1", {}
    )
    expected_record = {
        "status": "DEGRADED",
        "code": "EXPRESSION_NOT_ATTEMPTED",
        "reason": "EXPRESSION_NOT_ATTEMPTED",
        "cause_code": "DIMENSION_VALUE",
        "expression_state": "NOT_ATTEMPTED",
        "dimensions_fxp6": None,
    }
    observed_record = {key: malformed_record.get(key) for key in expected_record}
    if observed_record != expected_record:
        contract_failures.append(
            f"observatory must preserve DIMENSION_VALUE, got {observed_record!r}"
        )
    assert len(malformed_recorder.warning_messages) == 2
    provider_warning = json.loads(malformed_recorder.warning_messages[0])
    assert provider_warning.get("subcode") == "DIMENSION_VALUE"
    assert provider_warning.get("dimension_diagnostic") == {
        "dimension_name": "positive",
        "value_classification": "INTENSITY_NULL_DISALLOWED",
        "json_type": "null",
    }
    assert malformed_completion not in "\n".join(malformed_recorder.warning_messages)
    assert not contract_failures, "\n".join(contract_failures)
