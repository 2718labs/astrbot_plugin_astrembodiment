from __future__ import annotations

import asyncio
import copy
import hashlib
import importlib.util
import inspect
import json
import os
import shutil
import sys
import time
import zipfile
from pathlib import Path
from types import SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

import astr_embodiment.bridge as bridge_module  # noqa: E402
from astr_embodiment.contracts import ScopeTokens  # noqa: E402
from astr_embodiment.coordinator import GenesisCoordinator  # noqa: E402
from astr_embodiment.interaction import build_interaction_batch_v1  # noqa: E402
from astr_embodiment.persona_genesis import (  # noqa: E402
    PersonaGenesisError,
    PersonaSourceSnapshot,
    build_closed_request,
)
from astr_embodiment.semantic_contract import DIMENSION_NAMES  # noqa: E402
import main as main_module  # noqa: E402
from main import AstrEmbodimentPlugin  # noqa: E402


AUTONOMY_NATIVE_API = {
    "alpha3_call",
    "autonomy_status",
    "begin_semantic_appraisal_v1",
    "bind_outbound_target",
    "bootstrap_autonomy",
    "claim_wake",
    "gate_and_claim_dispatch",
    "gate_and_claim_externalization",
    "gate_externalization",
    "host_readiness_witness_digest_v1",
    "integration_availability_v1",
    "observe_body_snapshot_v1",
    "observe_budget_summary_v1",
    "observe_execution_receipt_v1",
    "observe_gate_reasons_v1",
    "pending_autonomy_work",
    "record_relation_inbound",
    "recover_autonomy",
    "scope_digests",
    "settle_dispatch",
    "settle_externalization",
    "settle_semantic_appraisal_v1",
    "settle_wake",
    "verify_autonomy_projection",
    "wake_caller_incarnation_v2",
}


def _install_fake_autonomy_api(native) -> None:
    for symbol in AUTONOMY_NATIVE_API:
        setattr(native, symbol, lambda *_args, **_kwargs: "{}")


def _fresh_windows_wheel() -> Path | None:
    override = os.environ.get("AE_FRESH_WINDOWS_WHEEL")
    return Path(override) if override else None


def _fresh_native_genesis_request(scope: ScopeTokens) -> dict[str, object]:
    source = PersonaSourceSnapshot.freeze(
        persona_id="fresh-native-boundary",
        persona={"prompt": "稳定、克制、诚实", "begin_dialogs": ["你好"]},
        selection="conversation",
    )
    trait_names = (
        "baseline_warmth",
        "baseline_patience",
        "sensitivity",
        "irritability",
        "composure",
        "epistemic_pride",
        "epistemic_openness",
        "boundary_strength",
        "forgiveness",
        "attachment_propensity",
        "expression_drive",
        "curiosity",
    )
    proposal = {
        "traits": {
            name: {"value": 0.5, "confidence": 0.5} for name in trait_names
        },
        "expression": {
            name: 0.5
            for name in (
                "warmth",
                "directness",
                "verbosity",
                "self_disclosure",
                "humor",
                "formality",
            )
        },
        "allostasis": {
            name: 0.5
            for name in (
                "energy",
                "arousal",
                "contact_need",
                "quiet_need",
                "expression_pressure",
                "exploration_drive",
            )
        },
        "epistemic": {
            name: 0.5
            for name in (
                "verification_drive",
                "confidence_style",
                "correction_defensiveness",
                "repair_after_error",
            )
        },
        "social": {
            name: 0.5
            for name in (
                "stranger_distance",
                "approach_threshold",
                "rejection_sensitivity",
                "reciprocity_expectation",
            )
        },
    }
    return build_closed_request(
        scope=scope,
        source=source,
        proposal=proposal,
        selection="conversation",
        compiler_protocol_digest="51" * 32,
        compiler_model_digest="52" * 32,
        formula_digest="53" * 32,
        incarnation_nonce="54" * 32,
        observed_at_ms=int(time.time() * 1_000),
    )


class FakeConfig(dict):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.save_calls = 0

    def save_config(self):
        self.save_calls += 1

    async def save_config_async(self):
        self.save_calls += 1
        return True


class FailingConfig(FakeConfig):
    async def save_config_async(self):
        self.save_calls += 1
        raise OSError("configuration storage is unavailable")

    def save_config(self):
        self.save_calls += 1
        raise OSError("configuration storage is unavailable")


def test_proactive_settings_migration_save_and_rollback():
    legacy = {
        "proactive_enabled": True,
        "proactive_daily_max": 2,
        "min_proactive_cooldown_minutes": 360,
        "quiet_hours_start": "00:30",
        "quiet_hours_end": "08:30",
        "quiet_hours_emergency_bypass": True,
        "intention_ttl_minutes": 1_440,
        "unanswered_backoff_base_minutes": 360,
        "unanswered_hard_stop": 3,
        "emergency_threshold": 0.9,
        "inner_activity_token_daily_max": 2_048,
        "native_data_dir": "unchanged",
    }
    legacy_keys = tuple(legacy)

    async def run_success():
        config = FakeConfig(legacy)
        before = dict(config)
        instance = AstrEmbodimentPlugin(FakeContext(), config)
        await instance._prepare_proactive_settings()
        return config, before, instance

    success, success_before, success_instance = asyncio.run(run_success())
    assert success.save_calls == 1
    assert set(success).difference(success_before) == {
        "proactive_frequency",
        "user_quiet_hours",
        "proactive_settings_revision",
    }
    assert {key: success[key] for key in legacy_keys} == success_before
    assert success["proactive_settings_revision"] == 1
    assert success["proactive_frequency"] == {
        "mode": "restrained",
        "custom_daily_max": 2,
        "custom_cooldown_minutes": 360,
    }
    assert success["user_quiet_hours"] == {
        "start": "00:30",
        "end": "08:30",
        "allow_authorized_emergency_bypass": True,
    }
    assert success_instance._proactive_policy.source_kind == "new-restrained"
    assert success_instance._proactive_policy.migration_patch is None

    class RefusingAsyncConfig(FakeConfig):
        async def save_config_async(self):
            self.save_calls += 1
            return False

    async def run_refused():
        config = RefusingAsyncConfig(legacy)
        before = dict(config)
        instance = AstrEmbodimentPlugin(FakeContext(), config)
        await instance._prepare_proactive_settings()
        return config, before, instance

    refused, refused_before, refused_instance = asyncio.run(run_refused())
    assert refused.save_calls == 1
    assert dict(refused) == refused_before
    assert refused_instance._proactive_policy.source_kind == "legacy-restrained"
    assert refused_instance._proactive_policy.configured_mode.value == "restrained"
    assert refused_instance._proactive_policy.fixed_daily_max == 2
    assert refused_instance._proactive_policy.fixed_cooldown_ms == 21_600_000
    assert refused_instance._proactive_policy.failure_reason == "migration-save-failed"

    replacement = {
        "mode": "custom",
        "custom_daily_max": 9,
        "custom_cooldown_minutes": 90,
    }

    class ConcurrentReplacementConfig(FakeConfig):
        async def save_config_async(self):
            self.save_calls += 1
            self["proactive_frequency"] = dict(replacement)
            raise OSError("save lost a race")

    async def run_concurrent_failure():
        config = ConcurrentReplacementConfig(legacy)
        instance = AstrEmbodimentPlugin(FakeContext(), config)
        await instance._prepare_proactive_settings()
        return config, instance

    concurrent, concurrent_instance = asyncio.run(run_concurrent_failure())
    assert concurrent.save_calls == 1
    assert concurrent["proactive_frequency"] == replacement
    assert "user_quiet_hours" not in concurrent
    assert "proactive_settings_revision" not in concurrent
    assert {key: concurrent[key] for key in legacy_keys} == legacy
    assert concurrent_instance._config_values == dict(concurrent)
    assert concurrent_instance._proactive_policy.source_kind == "legacy-restrained"
    assert concurrent_instance._proactive_policy.failure_reason == "migration-save-failed"

    class SecondWriteFailsConfig(FakeConfig):
        def __setitem__(self, key, value):
            if key == "user_quiet_hours":
                raise OSError("second patch write failed")
            super().__setitem__(key, value)

    async def run_partial_write_failure():
        config = SecondWriteFailsConfig(legacy)
        before = dict(config)
        instance = AstrEmbodimentPlugin(FakeContext(), config)
        await instance._prepare_proactive_settings()
        return config, before, instance

    partial, partial_before, partial_instance = asyncio.run(
        run_partial_write_failure()
    )
    assert dict(partial) == partial_before
    assert partial.save_calls == 0
    assert partial_instance._config_values == partial_before
    assert partial_instance._proactive_policy.failure_reason == "migration-save-failed"

    class CancelledSaveConfig(FakeConfig):
        async def save_config_async(self):
            self.save_calls += 1
            raise asyncio.CancelledError

    cancelled = CancelledSaveConfig(legacy)
    cancelled_before = dict(cancelled)
    cancelled_instance = AstrEmbodimentPlugin(FakeContext(), cancelled)
    with pytest.raises(asyncio.CancelledError):
        asyncio.run(cancelled_instance._prepare_proactive_settings())
    assert dict(cancelled) == cancelled_before
    assert cancelled.save_calls == 1
    assert cancelled_instance._config_values == cancelled_before
    assert (
        cancelled_instance._proactive_policy.failure_reason
        == "migration-save-failed"
    )


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

    def get_platform_id(self) -> str:
        return "test"

    def get_self_id(self) -> str:
        return "bot-1"

    def get_group_id(self) -> str:
        return ""

    def get_sender_id(self) -> str:
        return "user-1"


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
    instance = AstrEmbodimentPlugin(context or FakeContext(), config or FakeConfig())
    instance._bridge.scope_digests = lambda _scope: {
        "persona_scope": "a1" * 32,
        "relation_scope": "b2" * 32,
    }
    instance._bridge.bootstrap_autonomy = lambda _request: {}

    def apply_interaction(request):
        base_revision = request["causal"]["base_revision"]
        return {
            "receipt": {
                "canonical_revision": base_revision + 1,
                "event_id": request["event_id"],
            },
            "applied_fact_count": len(request["facts"]),
        }

    instance._bridge.apply_interaction_v1 = apply_interaction
    return instance


def test_explicit_assistant_provider_is_used_without_fallback():
    async def run():
        context = FakeContext(configured_provider="helper", current_provider="chat")
        instance = plugin(FakeConfig(assistant_provider_id="helper"), context)

        response = await instance._genesis_generate(
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

        await instance._genesis_generate(
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

        await instance._genesis_generate(
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

        await instance._genesis_generate(
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
            await instance._genesis_generate(
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

    instance._inject_request(request, "AE-S1-0123456789ABCDEF", contract, None)
    first_prompt = request.system_prompt
    instance._inject_request(request, "AE-S1-0123456789ABCDEF", contract, None)

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

        async def run_inbound(_event, _request):
            return (
                {
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
                    "reply_affect": None,
                },
                ScopeTokens("bot", "persona", "session"),
                "session",
                0,
                "turn",
                1,
            )

        instance._run_inbound = run_inbound
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


def _semantic_budget_receipt(*, exhausted: bool = False):
    daily_token_limit = 16_384
    reserved_tokens = 0 if exhausted else 1_024
    charged_tokens = daily_token_limit if exhausted else 0
    return {
        "utc_day": 20_336,
        "daily_token_limit": daily_token_limit,
        "reserved_tokens": reserved_tokens,
        "charged_tokens": charged_tokens,
        "remaining_tokens": daily_token_limit - reserved_tokens - charged_tokens,
        "blocked": False,
    }


def test_semantic_inbound_calls_provider_once_and_submits_exact_six_key_proposal():
    async def run():
        context = FakeContext(configured_provider="semantic")
        estimate = {
            "schema": "astr-embodiment.semantic-estimate.v3",
            "dimensions": {
                name: {
                    "state": "PRESENT" if name == "positive" else "ABSENT",
                    "intensity_fxp6": 250_000 if name == "positive" else 0,
                    "confidence_fxp6": 800_000,
                }
                for name in DIMENSION_NAMES
            },
        }

        async def llm_generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(
                completion_text=json.dumps(estimate),
                usage=SimpleNamespace(total=41),
            )

        context.llm_generate = llm_generate
        instance = plugin(
            FakeConfig(semantic_estimator_provider_id="semantic"), context
        )
        scope = ScopeTokens("bot", "persona", "session")

        async def genesis(_event, _request=None):
            receipt = {
                "seed_code": "AE-S1-SEMANTIC",
                "incarnation_id": "AE-I1-SEMANTIC",
            }
            return (
                {"genesis": receipt, **receipt},
                scope,
                "session",
                0,
                None,
                0,
            )

        begin_requests = []
        settle_requests = []

        def begin(request):
            begin_requests.append(request)
            return {
                "status": "claimed",
                "settlement_nonce_digest": "11" * 32,
                "interaction": {
                    "receipt": {
                        "canonical_revision": 1,
                        "event_id": request["interaction"]["event_id"],
                    }
                },
                "challenge": {
                    "request_nonce_digest": "11" * 32,
                    "origin": {"origin_digest": "22" * 32},
                },
                "capacity_reason": None,
                "budget": _semantic_budget_receipt(),
                "reply_affect": None,
            }

        def settle(request):
            settle_requests.append(request)
            return {
                "status": "committed",
                "charged_tokens": 41,
                "canonical_revision": 2,
                "contract": {"continuous": {"directness": 450_000}},
                "reply_affect": None,
            }

        instance._run_genesis = genesis
        instance._native_revision = lambda _scope: 0
        instance._bridge.begin_semantic_appraisal_v1 = begin
        instance._bridge.settle_semantic_appraisal_v1 = settle
        event = FakeEvent()
        event.message_str = "谢谢你"
        result = await instance._run_inbound(event, FakeRequest())
        return context, begin_requests, settle_requests, result

    context, begin_requests, settle_requests, result = asyncio.run(run())

    assert len(context.generate_calls) == 1
    assert context.generate_calls[0]["chat_provider_id"] == "semantic"
    assert context.generate_calls[0]["max_tokens"] == 512
    assert len(begin_requests) == len(settle_requests) == 1
    assert begin_requests[0]["daily_token_limit"] == 16_384
    settled = settle_requests[0]
    assert settled["provider_usage"] == {"known": True, "used_tokens": 41}
    assert set(settled["proposal"]) == {
        "schema_version",
        "origin_digest",
        "dimensions",
        "estimator_confidence",
        "protocol_version",
        "request_nonce_digest",
    }
    assert tuple(settled["proposal"]["dimensions"]) == DIMENSION_NAMES
    assert result[0]["revision"] == 2


def test_semantic_budget_exhaustion_skips_provider_and_keeps_normal_reply_open():
    async def run():
        context = FakeContext(configured_provider="semantic")
        instance = plugin(
            FakeConfig(semantic_estimator_provider_id="semantic"), context
        )
        scope = ScopeTokens("bot", "persona", "session")
        receipt = {
            "seed_code": "AE-S1-BUDGET",
            "incarnation_id": "AE-I1-BUDGET",
        }

        async def genesis(_event, _request=None):
            return (
                {"genesis": receipt, **receipt},
                scope,
                "session",
                0,
                None,
                0,
            )

        def begin(request):
            return {
                "status": "budget_exhausted",
                "settlement_nonce_digest": None,
                "interaction": {
                    "receipt": {
                        "canonical_revision": 1,
                        "event_id": request["interaction"]["event_id"],
                    }
                },
                "challenge": None,
                "capacity_reason": None,
                "budget": _semantic_budget_receipt(exhausted=True),
                "reply_affect": None,
            }

        instance._run_genesis = genesis
        instance._native_revision = lambda _scope: 0
        instance._bridge.begin_semantic_appraisal_v1 = begin
        instance._bridge.settle_semantic_appraisal_v1 = lambda _request: (
            pytest.fail("budget-exhausted claim must not settle")
        )
        event = FakeEvent()
        event.message_str = "今天怎么样？"
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return context, instance, event, request

    context, instance, event, request = asyncio.run(run())

    assert context.generate_calls == []
    assert event.stopped is False
    assert "seed_code=AE-S1-BUDGET" in request.system_prompt
    assert instance._pending


def test_autonomous_externalization_is_zero_provider_zero_send():
    async def run():
        context = FakeContext()
        instance = plugin(FakeConfig(), context)
        await instance._on_autonomous_externalization(
            {"message": "must never be sent", "target": "opaque"}
        )
        return context

    context = asyncio.run(run())

    assert context.generate_calls == []


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
            "delivery_event_id": "22" * 16,
        }

        async def apply_delivery(**kwargs):
            assert kwargs["base_revision"] == 4
            return {"revision": 5}

        instance._coordinator.apply_delivery = apply_delivery
        await instance.after_message_sent(event)
        return instance, scope

    instance, scope = asyncio.run(run())

    assert instance._revisions[scope.persona_token] == 5


def test_delivery_stale_causal_base_rebuilds_once_with_frozen_evidence(monkeypatch):
    class FakeBridge:
        def __init__(self):
            self.apply_calls: list[tuple[dict, dict]] = []
            self.inspect_calls: list[dict] = []

        def apply_event(self, scope, event):
            self.apply_calls.append((scope, event))
            if len(self.apply_calls) == 1:
                raise bridge_module.StaleCausalBase(
                    "STALE_CAUSAL_BASE", "autonomous wake advanced revision"
                )
            return {"revision": 3}

        def inspect(self, scope):
            self.inspect_calls.append(scope)
            return {"bound": True, "revision": 2}

    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        bridge = FakeBridge()
        instance._coordinator._bridge = bridge
        event = FakeEvent()
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        event.turn_token = "turn-1"
        instance._turn_seq[scope.session_token] = 1
        instance._pending[event.turn_token] = {
            "scope": scope,
            "turn_id": event.turn_token,
            "base_revision": 1,
            "contract": {},
            "delivery_event_id": "11" * 16,
        }
        await instance.after_message_sent(event)
        return instance, scope, bridge

    clock_calls: list[float] = []

    def frozen_time():
        clock_calls.append(123.456)
        return clock_calls[-1]

    monkeypatch.setattr(main_module.time, "time", frozen_time)
    instance, scope, bridge = asyncio.run(run())

    assert len(bridge.apply_calls) == 2
    first_scope, first_event = bridge.apply_calls[0]
    second_scope, second_event = bridge.apply_calls[1]
    assert first_scope == second_scope == scope.scope_json()
    first_payload = first_event["payload"]
    second_payload = second_event["payload"]
    assert second_payload["event_id"] == first_payload["event_id"]
    assert second_payload["causal"]["turn_id"] == first_payload["causal"]["turn_id"]
    assert second_payload["delivered"] == first_payload["delivered"] is True
    assert (
        second_payload["visible_action_digest"]
        == first_payload["visible_action_digest"]
        == "00" * 32
    )
    assert second_payload["delivered_at_ms"] == first_payload["delivered_at_ms"] == 123456
    assert first_payload["causal"]["base_revision"] == 1
    assert second_payload["causal"]["base_revision"] == 2
    assert bridge.inspect_calls == [scope.scope_json()]
    assert clock_calls == [123.456]
    assert instance._pending == {}
    assert instance._revisions[scope.persona_token] == 3


def test_delivery_stale_causal_base_second_stale_fails_closed_without_third_attempt():
    class AlwaysStaleBridge:
        def __init__(self):
            self.apply_calls: list[tuple[dict, dict]] = []
            self.inspect_calls: list[dict] = []

        def apply_event(self, scope, event):
            self.apply_calls.append((scope, event))
            raise bridge_module.StaleCausalBase(
                "STALE_CAUSAL_BASE", "revision changed again"
            )

        def inspect(self, scope):
            self.inspect_calls.append(scope)
            return {"bound": True, "revision": 2}

    scope = ScopeTokens(
        bot_token="bot",
        persona_token="persona",
        session_token="session",
    )
    bridge = AlwaysStaleBridge()
    coordinator = GenesisCoordinator(bridge)

    with pytest.raises(bridge_module.StaleCausalBase):
        asyncio.run(
            coordinator.apply_delivery(
                scope=scope,
                event_id="event-1",
                turn_id="turn-1",
                base_revision=1,
                delivered=True,
                visible_action_digest="11" * 32,
                delivered_at_ms=123456,
            )
        )

    assert len(bridge.apply_calls) == 2
    assert bridge.inspect_calls == [scope.scope_json()]


def test_delivery_invalid_event_id_fails_closed_with_redacted_diagnostic():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        event = FakeEvent()
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        event.turn_token = "turn-1"
        instance._pending[event.turn_token] = {
            "scope": scope,
            "turn_id": event.turn_token,
            "base_revision": 4,
            "contract": {},
            "delivery_event_id": "corrupted",
        }

        async def must_not_apply(**_kwargs):
            raise AssertionError("invalid delivery id reached native coordinator")

        instance._coordinator.apply_delivery = must_not_apply
        await instance.after_message_sent(event)
        return instance

    instance = asyncio.run(run())

    assert instance._pending == {}
    diagnostics = instance.delivery_diagnostics
    assert len(diagnostics) == 1
    diagnostic = diagnostics[0]
    assert diagnostic["error_code"] == "INVALID_DELIVERY_EVENT_ID"
    assert diagnostic["event_id"] != "corrupted"
    assert diagnostic["turn_id"] != "turn-1"
    assert diagnostic["base_revision"] == 4
    assert isinstance(diagnostic["recorded_at_ms"], int)
    assert "error" not in diagnostic


def test_delivery_terminal_failure_clears_pending_and_records_diagnostic():
    class AlwaysStaleBridge:
        def __init__(self):
            self.apply_calls = 0

        def apply_event(self, _scope, _event):
            self.apply_calls += 1
            raise bridge_module.StaleCausalBase(
                "STALE_CAUSAL_BASE", "canonical revision changed again"
            )

        def inspect(self, _scope):
            return {"bound": True, "revision": 2}

    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        bridge = AlwaysStaleBridge()
        instance._coordinator._bridge = bridge
        event = FakeEvent()
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        event.turn_token = "turn-1"
        instance._pending[event.turn_token] = {
            "scope": scope,
            "turn_id": event.turn_token,
            "base_revision": 1,
            "contract": {},
            "delivery_event_id": "33" * 16,
        }
        await instance.after_message_sent(event)
        return instance, bridge

    instance, bridge = asyncio.run(run())

    assert bridge.apply_calls == 2
    assert instance._pending == {}
    diagnostics = instance.delivery_diagnostics
    assert len(diagnostics) == 1
    diagnostic = diagnostics[0]
    assert diagnostic["error_code"] == "STALE_CAUSAL_BASE"
    assert diagnostic["event_id"] != "33" * 16
    assert diagnostic["turn_id"] != "turn-1"
    assert diagnostic["base_revision"] == 1
    assert isinstance(diagnostic["recorded_at_ms"], int)
    assert "error" not in diagnostic


def test_reload_hydrates_revision_and_turn_id_without_reuse():
    async def run():
        instance = plugin(FakeConfig(), FakeContext())
        event = FakeEvent()
        request = FakeRequest()
        calls: list[str] = []
        interaction: dict = {}

        async def resolve(_event, _request=None):
            return "persona-1", {"prompt": "测试人格"}, "conversation"

        async def ensure_genesis(**_kwargs):
            calls.append("genesis")
            return {
                "seed_code": "AE-S1-RELOAD",
                "incarnation_id": "AE-I1-RELOAD",
            }

        def inspect(_scope):
            calls.append("inspect")
            return {"bound": True, "revision": 7}

        def apply_interaction(batch):
            calls.append("interaction")
            interaction.update(batch)
            return {
                "receipt": {
                    "canonical_revision": batch["causal"]["base_revision"] + 1,
                    "event_id": batch["event_id"],
                },
                "applied_fact_count": len(batch["facts"]),
            }

        instance.resolve_effective_persona = resolve
        instance._coordinator.ensure_genesis = ensure_genesis
        instance._bridge._native = object()
        instance._bridge.inspect = inspect
        instance._bridge.apply_interaction_v1 = apply_interaction
        await instance.on_llm_request(event, request)
        scope = instance._scope_for(event, "persona-1")
        assert scope is not None
        return instance, event, scope, calls, interaction

    instance, event, scope, calls, interaction = asyncio.run(run())

    assert calls == ["genesis", "inspect", "inspect", "interaction"]
    assert interaction["causal"]["base_revision"] == 7
    assert interaction["causal"]["turn_id"] == event.turn_token
    assert instance._turn_seq[scope.session_token] == 8


def test_first_genesis_decision_persists_seed_to_astrbot_config():
    async def run():
        context = FakeContext()
        instance = plugin(FakeConfig(), context)

        async def run_inbound(_event, _request):
            return (
                {
                    "genesis": {
                        "seed_code": "AE-S1-FIRST-GENESIS",
                        "incarnation_id": "AE-I1-FIRST-GENESIS",
                    },
                    "seed_code": "AE-S1-FIRST-GENESIS",
                    "incarnation_id": "AE-I1-FIRST-GENESIS",
                    "revision": 1,
                    "contract": {"continuous": {"directness": 500000}},
                    "reply_affect": None,
                },
                ScopeTokens("bot", "persona", "session"),
                "session",
                0,
                "turn",
                1,
            )

        instance._run_inbound = run_inbound
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
        instance._native_revision = lambda _scope: 0
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

        async def run_inbound(_event, _request):
            return (
                {
                    "genesis": {"seed_code": "AE-S1-INCOMPLETE"},
                    "seed_code": "AE-S1-INCOMPLETE",
                    "revision": 1,
                    "contract": {},
                    "reply_affect": None,
                },
                ScopeTokens("bot", "persona", "session"),
                "session",
                0,
                "turn",
                1,
            )

        instance._run_inbound = run_inbound
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

        async def run_inbound(_event, _request):
            nonlocal calls
            calls += 1
            return (
                {
                    "genesis": {
                        "seed_code": "AE-S1-MARKER-SAFE",
                        "incarnation_id": "AE-I1-MARKER-SAFE",
                    },
                    "seed_code": "AE-S1-MARKER-SAFE",
                    "incarnation_id": "AE-I1-MARKER-SAFE",
                    "revision": 1,
                    "contract": {"continuous": {"directness": 500_000}},
                    "reply_affect": None,
                },
                ScopeTokens("bot", "persona", "session"),
                "session",
                0,
                "turn",
                1,
            )

        instance._run_inbound = run_inbound
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

        instance._run_inbound = genesis
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

        instance._run_inbound = genesis
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

        instance._run_inbound = genesis
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

        instance._run_inbound = genesis
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
            _install_fake_autonomy_api(native)
            native.ensure_genesis = lambda *_args: "{}"
            native.flush_and_close = lambda: None
            native.health = lambda: "{}"
            native.inspect = lambda *_args: "{}"
            native.open = lambda *_args: None
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
    assert callable(module.autonomy_status)
    assert issubclass(module.NativeCoreError, RuntimeError)


@pytest.mark.skipif(
    sys.platform != "win32" or _fresh_windows_wheel() is None,
    reason="requires AE_FRESH_WINDOWS_WHEEL pointing to a current Windows wheel",
)
def test_fresh_windows_native_initializer_and_semantic_boundary(
    tmp_path: Path,
):
    """Load the fresh pyd, then exercise the durable begin/settle ABI."""
    wheel_path = _fresh_windows_wheel()
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
        assert module.version() == "1.1.0-alpha3"
        assert callable(module.apply_event)
        assert Path(sys.modules[f"{module_name}._native"].__file__).parts[-3:] == (
            "_bundled",
            build_id,
            "_native.pyd",
        )
        assert not hasattr(module, "apply_perception_proposal_v1")

        module.open(str(tmp_path / "semantic-boundary.db"))
        scope = ScopeTokens(
            bot_token="61" * 16,
            persona_token="62" * 16,
            relation_token="63" * 16,
            session_token="64" * 16,
        )
        module.ensure_genesis(
            json.dumps(_fresh_native_genesis_request(scope), sort_keys=True)
        )
        scope_payload = scope.scope_json()
        digests = json.loads(module.scope_digests(json.dumps(scope_payload)))
        module.bootstrap_autonomy(
            json.dumps(
                {
                    "scope": scope_payload,
                    "profile": {
                        "schema_version": 1,
                        "persona_scope": digests["persona_scope"],
                        "home_timezone": "UTC",
                        "current_timezone": "UTC",
                        "chronotype": "intermediate",
                        "preferred_sleep_local_minute": 1_380,
                        "preferred_wake_local_minute": 420,
                        "sleep_flex_minutes": 90,
                        "entrainment_rate_minutes_per_day": 60,
                        "revision": 1,
                    },
                    "relation": {
                        "schema_version": 1,
                        "relation_scope": digests["relation_scope"],
                        "user_timezone": "UTC",
                        "timezone_source": "explicit",
                        "quiet_hours_start_minute": 0,
                        "quiet_hours_end_minute": 0,
                        "quiet_hours_emergency_bypass": False,
                        "proactive_enabled": False,
                        "proactive_daily_max": 0,
                        "min_proactive_cooldown_ms": 0,
                        "intention_ttl_ms": 86_400_000,
                        "unanswered_backoff_base_ms": 0,
                        "unanswered_hard_stop": 3,
                        "emergency_threshold": 1_000_000,
                        "daily_submitted": 0,
                        "consecutive_unanswered": 0,
                        "last_inbound_utc_ms": None,
                        "last_proactive_submitted_utc_ms": None,
                        "revision": 1,
                        "auto_policy_version": 0,
                        "next_claim_reservation_tokens": 256,
                    },
                },
                sort_keys=True,
            )
        )
        interaction = build_interaction_batch_v1(
            scope=scope,
            message="真实 PyO3 边界",
            turn_id="65" * 16,
            event_id="66" * 16,
            base_revision=0,
            observed_at_utc_ms=int(time.time() * 1_000),
        )
        begin = json.loads(
            module.begin_semantic_appraisal_v1(
                json.dumps(
                    {
                        "schema_version": 1,
                        "interaction": interaction,
                        "daily_token_limit": 16_384,
                        "reserved_tokens": 1_024,
                        "provider_digest": "67" * 32,
                    },
                    sort_keys=True,
                )
            )
        )
        assert begin["status"] == "claimed"
        assert (
            begin["settlement_nonce_digest"]
            == begin["challenge"]["request_nonce_digest"]
        )
        settle_request = {
            "schema_version": 1,
            "scope": scope_payload,
            "request_nonce_digest": begin["challenge"]["request_nonce_digest"],
            "outcome": "timeout",
            "provider_usage": {"known": False, "used_tokens": None},
            "proposal": None,
        }
        first = json.loads(
            module.settle_semantic_appraisal_v1(
                json.dumps(settle_request, sort_keys=True)
            )
        )
        replay = json.loads(
            module.settle_semantic_appraisal_v1(
                json.dumps(settle_request, sort_keys=True)
            )
        )
        assert first == replay
        assert first["status"] == "zero_mutation"
        assert first["charged_tokens"] == 1_024

        usage_drift = copy.deepcopy(settle_request)
        usage_drift["provider_usage"] = {"known": True, "used_tokens": 0}
        with pytest.raises(module.NativeCoreError, match="INVALID_PERCEPTION_PROPOSAL"):
            module.settle_semantic_appraisal_v1(
                json.dumps(usage_drift, sort_keys=True)
            )
    finally:
        if hasattr(module, "flush_and_close"):
            module.flush_and_close()
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
            _install_fake_autonomy_api(module)
            module.health = lambda: "{}"
            module.open = lambda _data_dir: None
            module.ensure_genesis = lambda *_args: "{}"
            module.apply_event = lambda *_args: "{}"
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


def _closed_semantic_estimate_json() -> str:
    return json.dumps(
        {
            "schema": "astr-embodiment.semantic-estimate.v3",
            "dimensions": {
                name: {
                    "state": "PRESENT" if name == "positive" else "ABSENT",
                    "intensity_fxp6": 250_000 if name == "positive" else 0,
                    "confidence_fxp6": 800_000,
                }
                for name in DIMENSION_NAMES
            },
        }
    )


def _semantic_genesis(scope: ScopeTokens):
    async def genesis(_event, _request=None):
        receipt = {
            "seed_code": "AE-S1-SEMANTIC-BOUNDARY",
            "incarnation_id": "AE-I1-SEMANTIC-BOUNDARY",
        }
        return ({"genesis": receipt, **receipt}, scope, scope.session_token, 0, None, 0)

    return genesis


def _claimed_begin(request, *, nonce: str = "11" * 32):
    return {
        "status": "claimed",
        "settlement_nonce_digest": nonce,
        "interaction": {
            "receipt": {
                "canonical_revision": 1,
                "event_id": request["interaction"]["event_id"],
            }
        },
        "challenge": {
            "request_nonce_digest": nonce,
            "origin": {"origin_digest": "22" * 32},
        },
        "capacity_reason": None,
        "budget": _semantic_budget_receipt(),
        "reply_affect": None,
    }


@pytest.mark.parametrize("status", ["claimed", "budget_exhausted"])
def test_semantic_begin_without_required_budget_is_rejected_before_provider(status):
    async def run():
        context = FakeContext(configured_provider="semantic")
        instance = plugin(
            FakeConfig(semantic_estimator_provider_id="semantic"), context
        )
        scope = ScopeTokens("bot", "persona", f"missing-budget-{status}")
        instance._run_genesis = _semantic_genesis(scope)
        instance._native_revision = lambda _scope: 0
        instance._bridge.apply_interaction_v1 = lambda _request: pytest.fail(
            "malformed begin receipts must not perform a raw interaction apply"
        )
        settlements = []

        def begin(request):
            if status == "claimed":
                result = _claimed_begin(request)
                result.pop("budget", None)
                return result
            return {
                "status": "budget_exhausted",
                "settlement_nonce_digest": None,
                "interaction": {
                    "receipt": {
                        "canonical_revision": 1,
                        "event_id": request["interaction"]["event_id"],
                    }
                },
                "challenge": None,
                "capacity_reason": None,
                "reply_affect": None,
            }

        def settle(payload):
            settlements.append(copy.deepcopy(payload))
            return {
                "status": "zero_mutation",
                "charged_tokens": 1_024,
                "canonical_revision": 1,
                "contract": None,
                "reply_affect": None,
            }

        instance._bridge.begin_semantic_appraisal_v1 = begin
        instance._bridge.settle_semantic_appraisal_v1 = settle
        event = FakeEvent()
        event.message_str = f"missing budget: {status}"
        with pytest.raises(main_module.SemanticAppraisalRuntimeError) as raised:
            await instance._run_inbound(event, FakeRequest())
        return context, settlements, raised.value

    context, settlements, error = asyncio.run(run())
    assert str(error) == "SEMANTIC_BEGIN_STATUS_INVALID"
    assert context.generate_calls == []
    if status == "claimed":
        assert len(settlements) == 1
        assert settlements[0] == {
            "schema_version": 1,
            "scope": ScopeTokens(
                "bot", "persona", "missing-budget-claimed"
            ).scope_json(),
            "request_nonce_digest": "11" * 32,
            "outcome": "malformed",
            "provider_usage": {"known": False, "used_tokens": None},
            "proposal": None,
        }
    else:
        assert settlements == []


def test_semantic_capacity_deferred_and_retry_expired_paths_are_closed():
    error_code = "SEMANTIC_APPRAISAL_RETRY_EXPIRED_OR_UNKNOWN"
    classified = bridge_module._classify(RuntimeError(f"{error_code}::terminal folded"))
    assert isinstance(
        classified, bridge_module.SemanticAppraisalRetryExpiredOrUnknown
    )
    assert classified.code == error_code

    async def run_case(case: str):
        context = FakeContext(configured_provider="semantic")
        instance = plugin(
            FakeConfig(semantic_estimator_provider_id="semantic"), context
        )
        scope = ScopeTokens("bot", "persona", f"{case}-session")
        genesis_calls = 0
        base_genesis = _semantic_genesis(scope)

        async def genesis(event, request=None):
            nonlocal genesis_calls
            genesis_calls += 1
            return await base_genesis(event, request)

        current_revision = 7 if case == "begin_retry" else 2
        revision_reads = iter((0, current_revision))
        instance._run_genesis = genesis
        instance._native_revision = lambda _scope: next(revision_reads)
        instance._bridge.apply_interaction_v1 = lambda _request: pytest.fail(
            "semantic terminal paths must not perform a raw interaction apply"
        )
        begin_requests = []
        settlements = []

        def begin(payload):
            begin_requests.append(copy.deepcopy(payload))
            if case == "begin_retry":
                raise bridge_module.SemanticAppraisalRetryExpiredOrUnknown(
                    error_code, "terminal folded"
                )
            if case.startswith("capacity"):
                result = {
                    "status": "capacity_deferred",
                    "capacity_reason": "retention_capacity_unavailable",
                    "interaction": {
                        "receipt": {
                            "canonical_revision": 1,
                            "event_id": payload["interaction"]["event_id"],
                        }
                    },
                    "challenge": None,
                    "reply_affect": None,
                }
                if case == "capacity_with_budget":
                    result["budget"] = _semantic_budget_receipt()
                return result
            return _claimed_begin(payload)

        def settle(payload):
            settlements.append(copy.deepcopy(payload))
            if case == "settle_retry":
                raise bridge_module.SemanticAppraisalRetryExpiredOrUnknown(
                    error_code, "terminal folded"
                )
            pytest.fail("non-claimed semantic terminal paths must not settle")

        instance._bridge.begin_semantic_appraisal_v1 = begin
        instance._bridge.settle_semantic_appraisal_v1 = settle
        event = FakeEvent()
        event.message_str = f"closed semantic path: {case}"
        request = FakeRequest()
        await instance.on_llm_request(event, request)
        return {
            "case": case,
            "context": context,
            "instance": instance,
            "event": event,
            "request": request,
            "scope": scope,
            "genesis_calls": genesis_calls,
            "begin_requests": begin_requests,
            "settlements": settlements,
            "expected_revision": (
                1 if case.startswith("capacity") else current_revision
            ),
        }

    async def run():
        return [
            await run_case(case)
            for case in (
                "capacity_without_budget",
                "capacity_with_budget",
                "begin_retry",
                "settle_retry",
            )
        ]

    for result in asyncio.run(run()):
        case = result["case"]
        context = result["context"]
        instance = result["instance"]
        event = result["event"]
        request = result["request"]
        scope = result["scope"]
        expected_revision = result["expected_revision"]
        assert result["genesis_calls"] == 1
        assert len(result["begin_requests"]) == 1
        assert event.stopped is False
        assert "seed_code=AE-S1-SEMANTIC-BOUNDARY" in request.system_prompt
        assert instance._revisions[scope.persona_token] == expected_revision
        assert next(iter(instance._pending.values()))["base_revision"] == expected_revision
        if case.startswith("capacity"):
            assert context.generate_calls == []
            assert result["settlements"] == []
            assert "capacity_deferred" in instance._semantic_attempts.values()
            assert any(
                item["code"] == "SEMANTIC_CAPACITY_DEFERRED"
                and item["canonical_revision"] == 1
                for item in instance._semantic_diagnostics
            )
        elif case == "begin_retry":
            assert context.generate_calls == []
            assert result["settlements"] == []
            assert "retry_expired_or_unknown" in instance._semantic_attempts.values()
        else:
            assert len(context.generate_calls) == 1
            assert len(result["settlements"]) == 1
            assert not any(
                item["code"] == "SEMANTIC_SETTLE_RETRY"
                for item in instance._semantic_diagnostics
            )
        if case.endswith("retry"):
            assert any(
                item["code"] == error_code
                for item in instance._semantic_diagnostics
            )


def test_semantic_cancellation_is_shield_settled_at_full_reservation_then_propagated():
    async def run():
        context = FakeContext(configured_provider="semantic")
        provider_entered = asyncio.Event()

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            provider_entered.set()
            await asyncio.Event().wait()

        context.llm_generate = generate
        instance = plugin(FakeConfig(semantic_estimator_provider_id="semantic"), context)
        scope = ScopeTokens("bot", "persona", "cancel-session")
        instance._run_genesis = _semantic_genesis(scope)
        instance._native_revision = lambda _scope: 0
        instance._bridge.begin_semantic_appraisal_v1 = _claimed_begin
        settlements = []
        pending = True

        def settle(payload):
            nonlocal pending
            settlements.append(payload)
            if len(settlements) == 1:
                raise OSError("transient ffi boundary")
            pending = False
            return {
                "status": "zero_mutation",
                "charged_tokens": 1_024,
                "canonical_revision": 1,
                "contract": None,
                "reply_affect": None,
            }

        instance._bridge.settle_semantic_appraisal_v1 = settle
        event = FakeEvent()
        event.message_str = "取消中的消息"
        task = asyncio.create_task(instance._run_inbound(event, FakeRequest()))
        await asyncio.wait_for(provider_entered.wait(), timeout=1.0)
        task.cancel()
        with pytest.raises(asyncio.CancelledError):
            await task
        return context, settlements, pending, instance

    context, settlements, pending, instance = asyncio.run(run())
    assert len(context.generate_calls) == 1
    assert len(settlements) == 2
    assert settlements[0] == settlements[1]
    assert settlements[0]["outcome"] == "provider_error"
    assert settlements[0]["provider_usage"] == {"known": False, "used_tokens": None}
    assert pending is False
    assert any(
        item["code"] == "SEMANTIC_CANCELLED_COMPENSATED"
        for item in instance._semantic_diagnostics
    )


@pytest.mark.parametrize("fault", ["receipt", "status", "challenge"])
def test_invalid_claimed_begin_is_compensated_without_provider_call(fault):
    async def run():
        context = FakeContext(configured_provider="semantic")
        instance = plugin(FakeConfig(semantic_estimator_provider_id="semantic"), context)
        scope = ScopeTokens("bot", "persona", "bad-challenge-session")
        instance._run_genesis = _semantic_genesis(scope)
        instance._native_revision = lambda _scope: 0

        def bad_begin(request):
            result = _claimed_begin(request)
            if fault == "receipt":
                result["interaction"]["receipt"]["canonical_revision"] = 0
            elif fault == "status":
                result["status"] = "invalid"
            else:
                result["challenge"] = {"origin": None}
            return result

        settlements = []
        pending = True
        instance._bridge.begin_semantic_appraisal_v1 = bad_begin

        def settle(payload):
            nonlocal pending
            settlements.append(copy.deepcopy(payload))
            if len(settlements) == 1:
                raise OSError("transient ffi boundary")
            pending = False
            return {
                "status": "zero_mutation",
                "charged_tokens": 1_024,
                "canonical_revision": 1,
                "contract": None,
                "reply_affect": None,
            }

        instance._bridge.settle_semantic_appraisal_v1 = settle
        event = FakeEvent()
        event.message_str = f"坏 begin {fault}"
        with pytest.raises(main_module.SemanticAppraisalRuntimeError) as raised:
            await instance._run_inbound(event, FakeRequest())
        return context, settlements, pending, instance, raised.value

    context, settlements, pending, instance, error = asyncio.run(run())
    assert context.generate_calls == []
    assert len(settlements) == 2
    assert settlements[0] == settlements[1]
    assert settlements[0]["outcome"] == "malformed"
    assert settlements[0]["provider_usage"]["known"] is False
    assert pending is False
    assert str(error) in {
        "SEMANTIC_BEGIN_RECEIPT_INVALID",
        "SEMANTIC_BEGIN_STATUS_INVALID",
        "SEMANTIC_CHALLENGE_INVALID",
    }
    assert any(
        item["code"] == str(error)
        for item in instance._semantic_diagnostics
    )


def test_malformed_estimate_full_charges_and_transient_settle_failure_retries_exact_payload():
    async def run():
        context = FakeContext(configured_provider="semantic")

        async def generate(**kwargs):
            context.generate_calls.append(kwargs)
            return SimpleNamespace(completion_text='{"not":"closed"}', usage=SimpleNamespace(total=9))

        context.llm_generate = generate
        instance = plugin(FakeConfig(semantic_estimator_provider_id="semantic"), context)
        scope = ScopeTokens("bot", "persona", "malformed-session")
        instance._run_genesis = _semantic_genesis(scope)
        instance._native_revision = lambda _scope: 0
        instance._bridge.begin_semantic_appraisal_v1 = _claimed_begin
        settlements = []

        def settle(payload):
            settlements.append(copy.deepcopy(payload))
            if len(settlements) == 1:
                raise OSError("transient ffi boundary")
            return {
                "status": "zero_mutation",
                "charged_tokens": 1_024,
                "canonical_revision": 1,
                "contract": None,
                "reply_affect": None,
            }

        instance._bridge.settle_semantic_appraisal_v1 = settle
        event = FakeEvent()
        event.message_str = "malformed estimator"
        result = await instance._run_inbound(event, FakeRequest())
        return context, settlements, result, instance

    context, settlements, result, instance = asyncio.run(run())
    assert len(context.generate_calls) == 1
    assert len(settlements) == 2
    assert settlements[0] == settlements[1]
    assert settlements[0]["outcome"] == "malformed"
    assert settlements[0]["provider_usage"] == {"known": False, "used_tokens": None}
    assert result[0]["contract"] is None
    assert any(item["code"] == "SEMANTIC_SETTLE_RETRY" for item in instance._semantic_diagnostics)


def test_slow_semantic_providers_overlap_for_same_persona_across_sessions():
    async def run():
        context = FakeContext(configured_provider="semantic")
        both_entered = asyncio.Event()
        release = asyncio.Event()
        entered = 0

        async def generate(**kwargs):
            nonlocal entered
            context.generate_calls.append(kwargs)
            entered += 1
            if entered == 2:
                both_entered.set()
            await release.wait()
            return SimpleNamespace(
                completion_text=_closed_semantic_estimate_json(),
                usage=SimpleNamespace(total=23),
            )

        context.llm_generate = generate
        instance = plugin(FakeConfig(semantic_estimator_provider_id="semantic"), context)

        async def genesis(event, _request=None):
            scope = ScopeTokens("bot", "shared-persona", event.session_name)
            return await _semantic_genesis(scope)(event, _request)

        instance._run_genesis = genesis
        instance._native_revision = lambda _scope: 0
        begin_count = 0

        def begin(request):
            nonlocal begin_count
            begin_count += 1
            return _claimed_begin(request, nonce=f"{begin_count:064x}")

        instance._bridge.begin_semantic_appraisal_v1 = begin
        instance._bridge.settle_semantic_appraisal_v1 = lambda payload: {
            "status": "committed",
            "charged_tokens": payload["provider_usage"]["used_tokens"],
            "canonical_revision": 2,
            "contract": {"continuous": {"directness": 450_000}},
            "reply_affect": None,
        }
        left = FakeEvent()
        left.session_name = "session-left"
        left.message_str = "左侧"
        right = FakeEvent()
        right.session_name = "session-right"
        right.message_str = "右侧"
        left_task = asyncio.create_task(instance._run_inbound(left, FakeRequest()))
        right_task = asyncio.create_task(instance._run_inbound(right, FakeRequest()))
        await asyncio.wait_for(both_entered.wait(), timeout=1.0)
        release.set()
        return await asyncio.gather(left_task, right_task), context, instance

    results, context, instance = asyncio.run(run())
    assert len(results) == 2
    assert len(context.generate_calls) == 2
    assert set(instance._persona_locks) == {("bot", "shared-persona")}
