from __future__ import annotations

import asyncio
import copy
import json
from types import SimpleNamespace
from typing import Any

import pytest

import main as main_module
from astr_embodiment.contracts import ScopeTokens
from astr_embodiment.semantic_contract import DIMENSION_NAMES
from main import AstrEmbodimentPlugin


class _Config(dict):
    def __init__(self, **values: Any) -> None:
        super().__init__(values)
        self.save_calls = 0

    async def save_config_async(self) -> bool:
        self.save_calls += 1
        return True


class _Event:
    unified_msg_origin = "test:private:ordinary-reply"
    platform_id = "test"
    bot_id = "test-bot"
    message_id = "immutable-message-1"

    def __init__(self, message: str = "你好，今天过得怎么样？") -> None:
        self.message_str = message
        self.stopped = False
        self.sent: list[str] = []
        self.extra: dict[str, str] = {}

    def get_message_str(self) -> str:
        return self.message_str

    def stop_event(self) -> None:
        self.stopped = True

    def plain_result(self, text: str) -> str:
        return text

    async def send(self, result: str) -> None:
        self.sent.append(result)

    def set_extra(self, key: str, value: str) -> None:
        self.extra[key] = value


class _Request:
    def __init__(self) -> None:
        self.prompt = "用户原始问题"
        self.system_prompt = "AstrBot 原始系统提示"
        self.contexts = [{"role": "user", "content": "先前对话"}]


def _valid_estimate() -> str:
    payload = {
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
    return json.dumps(payload, ensure_ascii=False, sort_keys=True)


class _RecordingContext:
    def __init__(
        self,
        *,
        failure: str | None = None,
        current_provider: str = "ordinary-main",
    ) -> None:
        self.failure = failure
        self.current_provider = current_provider
        self.provider_queries: list[str] = []
        self.current_provider_queries: list[str | None] = []
        self.semantic_calls: list[dict[str, Any]] = []
        self.ordinary_calls: list[dict[str, Any]] = []

    def get_provider_by_id(self, provider_id: str) -> object:
        self.provider_queries.append(provider_id)
        return object()

    async def get_current_chat_provider_id(self, *, umo: str | None) -> str:
        self.current_provider_queries.append(umo)
        return self.current_provider

    def llm_generate(self, **kwargs: Any) -> Any:
        if kwargs.get("max_tokens") == 512:
            self.semantic_calls.append(copy.deepcopy(kwargs))

            async def semantic_result() -> Any:
                if self.failure == "transport":
                    raise ConnectionError("semantic transport unavailable")
                if self.failure == "timeout":
                    await asyncio.Event().wait()
                    raise AssertionError("unreachable")
                return SimpleNamespace(
                    completion_text=_valid_estimate(),
                    usage=SimpleNamespace(total=37),
                )

            return semantic_result()

        self.ordinary_calls.append(copy.deepcopy(kwargs))

        async def ordinary_result() -> Any:
            return SimpleNamespace(completion_text="AstrBot ordinary reply")

        return ordinary_result()

    async def run_ordinary_reply(self, request: _Request) -> Any:
        """Model the one normal AstrBot generation after pre-request hooks."""
        return await self.llm_generate(
            chat_provider_id=self.current_provider,
            prompt=request.prompt,
            system_prompt=request.system_prompt,
            contexts=request.contexts,
            tools=None,
        )


class _WaitForProbe:
    def __init__(self, original: Any, *, force_semantic_timeout: bool) -> None:
        self.original = original
        self.force_semantic_timeout = force_semantic_timeout
        self.timeouts: list[float] = []

    async def __call__(self, awaitable: Any, timeout: float) -> Any:
        self.timeouts.append(timeout)
        if self.force_semantic_timeout and timeout == 15.0:
            # Exercise cancellation/TimeoutError without waiting fifteen real
            # seconds. The production timeout value is still observed above.
            return await self.original(awaitable, timeout=0.001)
        return await self.original(awaitable, timeout=timeout)


class _Harness:
    def __init__(
        self,
        *,
        config_values: dict[str, Any],
        failure: str | None = None,
        retry_settlement: bool = False,
    ) -> None:
        self.context = _RecordingContext(failure=failure)
        self.config = _Config(**config_values)
        self.plugin = AstrEmbodimentPlugin(self.context, self.config)
        self.scope = ScopeTokens("61" * 16, "62" * 16, "63" * 16)
        self.begin_requests: list[dict[str, Any]] = []
        self.settlement_requests: list[dict[str, Any]] = []
        self.retry_settlement = retry_settlement

        receipt = {
            "seed_code": "AE-S1-NATIVE-ORDINARY",
            "incarnation_id": "AE-I1-NATIVE-ORDINARY",
        }

        async def established_genesis(_event: Any, _request: Any = None):
            return (
                {"genesis": dict(receipt), **receipt},
                self.scope,
                self.scope.session_token,
                0,
                None,
                0,
            )

        def begin(request: dict[str, Any]) -> dict[str, Any]:
            self.begin_requests.append(copy.deepcopy(request))
            observation = request["observation"]
            return {
                "commit_status": "committed", "provider_authorized_now": True,
                "initial_receipt": {
                "disposition": "claimed",
                "event": {"scope": observation["scope"],
                          "operation_id": observation["operation_id"],
                          "turn_id": observation["turn_id"],
                          "transition": {"next_revision": 1}},
                "challenge": {
                    "request_nonce_digest": "71" * 32,
                    "origin": {"origin_digest": "72" * 32,
                               "scope": {**observation["scope"], "relation_token": None,
                                         "session_token": observation["turn_id"]}},
                },
                "reply_affect": None,
                },
            }

        def settle(request: dict[str, Any]) -> dict[str, Any]:
            self.settlement_requests.append(copy.deepcopy(request))
            if self.retry_settlement and len(self.settlement_requests) == 1:
                raise OSError("transient Native settlement boundary")
            proposal = request["proposal"]
            return {
                "status": "committed" if proposal is not None else "zero_mutation",
                "charged_tokens": 37 if proposal is not None else 1_024,
                "canonical_revision": 2 if proposal is not None else 1,
                "contract": None,
                "reply_affect": None,
            }

        self.plugin._run_genesis = established_genesis
        self.plugin._native_revision = lambda _scope: 0
        self.plugin._bridge.scope_digests = lambda _scope: {
            "persona_scope": "81" * 32,
            "relation_scope": None,
        }
        self.plugin._bridge.bootstrap_autonomy = lambda _request: {}
        import astrembodiment_core
        self.plugin._bridge.compile_core_host_request_v1 = lambda op, request: json.loads(
            astrembodiment_core.compile_core_host_request_v1(op, json.dumps(request)))
        self.plugin._bridge.commit_core_inbound_v1 = begin
        self.plugin._bridge.settle_semantic_appraisal_v1 = settle

    async def run(self) -> tuple[_Event, _Request, Any]:
        event = _Event()
        request = _Request()
        await self.plugin.on_llm_request(event, request)
        ordinary_response = None
        if not event.stopped:
            ordinary_response = await self.context.run_ordinary_reply(request)
            await self.plugin.on_llm_response(event, ordinary_response)
        return event, request, ordinary_response


@pytest.mark.parametrize(
    ("config_values", "expected_provider", "expected_current_queries"),
    [
        pytest.param(
            {
                "semantic_estimator_provider_id": "semantic-dedicated",
                "assistant_provider_id": "legacy-assistant",
            },
            "semantic-dedicated",
            0,
            id="semantic-estimator-over-legacy-and-current",
        ),
        pytest.param(
            {
                "semantic_estimator_provider_id": "",
                "assistant_provider_id": "legacy-assistant",
            },
            "legacy-assistant",
            0,
            id="legacy-assistant-over-current",
        ),
        pytest.param(
            {
                "semantic_estimator_provider_id": "",
                "assistant_provider_id": "",
            },
            "ordinary-main",
            1,
            id="current-provider-final-fallback",
        ),
    ],
)
def test_semantic_provider_selection_has_exact_priority_before_ordinary_reply(
    monkeypatch: pytest.MonkeyPatch,
    config_values: dict[str, Any],
    expected_provider: str,
    expected_current_queries: int,
) -> None:
    original_wait_for = asyncio.wait_for
    wait_for = _WaitForProbe(original_wait_for, force_semantic_timeout=False)
    monkeypatch.setattr(main_module.asyncio, "wait_for", wait_for)
    harness = _Harness(config_values=config_values)

    event, request, ordinary_response = asyncio.run(harness.run())

    assert harness.context.provider_queries == [expected_provider]
    assert len(harness.context.current_provider_queries) == expected_current_queries
    assert len(harness.context.semantic_calls) == 1
    semantic_call = harness.context.semantic_calls[0]
    assert semantic_call["chat_provider_id"] == expected_provider
    assert semantic_call["max_tokens"] == 512
    assert wait_for.timeouts.count(15.0) == 1
    assert len(harness.begin_requests) == len(harness.settlement_requests) == 1
    assert harness.settlement_requests[0]["outcome"] == "success"
    assert harness.settlement_requests[0]["proposal"] is not None

    assert event.stopped is False
    assert event.sent == []
    assert ordinary_response.completion_text == "AstrBot ordinary reply"
    assert len(harness.context.ordinary_calls) == 1
    assert harness.context.ordinary_calls[0]["chat_provider_id"] == "ordinary-main"
    assert request.prompt == "用户原始问题"
    assert request.contexts == [{"role": "user", "content": "先前对话"}]


@pytest.mark.parametrize(
    ("failure", "expected_outcome"),
    [
        pytest.param("transport", "provider_error", id="transport-exception"),
        pytest.param("timeout", "timeout", id="bounded-timeout"),
    ],
)
def test_ordinary_reply_uses_main_llm_path_without_becoming_proactive(
    monkeypatch: pytest.MonkeyPatch,
    failure: str,
    expected_outcome: str,
) -> None:
    original_wait_for = asyncio.wait_for
    wait_for = _WaitForProbe(
        original_wait_for,
        force_semantic_timeout=failure == "timeout",
    )
    monkeypatch.setattr(main_module.asyncio, "wait_for", wait_for)
    harness = _Harness(
        config_values={
            "semantic_estimator_provider_id": "semantic-dedicated",
            "assistant_provider_id": "legacy-must-not-be-fallback",
        },
        failure=failure,
        retry_settlement=True,
    )

    event, request, ordinary_response = asyncio.run(harness.run())

    # Provider transport/timeout is never retried or rerouted. Only the Native
    # settlement may retry, and its immutable failure outcome must be identical.
    assert harness.context.provider_queries == ["semantic-dedicated"]
    assert harness.context.current_provider_queries == []
    assert len(harness.context.semantic_calls) == 1
    semantic_call = harness.context.semantic_calls[0]
    assert semantic_call["chat_provider_id"] == "semantic-dedicated"
    assert semantic_call["max_tokens"] == 512
    assert wait_for.timeouts.count(15.0) == 1
    assert len(harness.begin_requests) == 1
    assert len(harness.settlement_requests) == 2
    assert harness.settlement_requests[0] == harness.settlement_requests[1]
    settlement = harness.settlement_requests[0]
    assert settlement["outcome"] == expected_outcome
    assert settlement["provider_usage"] == {"known": False, "used_tokens": None}
    assert settlement["proposal"] is None

    # The pre-request hook leaves AstrBot's one ordinary main-LLM reply open.
    # event.send() remains untouched, proving no plugin-owned proactive send.
    assert event.stopped is False
    assert event.sent == []
    assert ordinary_response.completion_text == "AstrBot ordinary reply"
    assert len(harness.context.ordinary_calls) == 1
    assert request.prompt == "用户原始问题"
    assert request.contexts == [{"role": "user", "content": "先前对话"}]
    assert request.system_prompt.count("AstrEmbodiment Runtime Context") == 1


@pytest.mark.parametrize("settle_fails", [False, True])
def test_pending_survives_provider_and_settlement_failure_then_delivery(settle_fails):
    harness = _Harness(config_values={}, failure="transport")
    plugin = harness.plugin
    original_settle = plugin._bridge.settle_semantic_appraisal_v1
    def settle(request):
        assert plugin._pending
        assert plugin._persona_locks[plugin._semantic_lock_key(harness.scope)].locked()
        if settle_fails:
            raise OSError("settlement unavailable")
        return original_settle(request)
    plugin._bridge.settle_semantic_appraisal_v1 = settle
    deliveries = []
    def delivery(request):
        assert "base_revision" not in request
        assert request["delivered"] is False
        deliveries.append(request)
        return {"commit_status":"committed", "receipt":{
            "operation_id":request["operation_id"],"scope":request["scope"],
            "turn_id":request["turn_id"],"transition":{"next_revision":99}}}
    plugin._bridge.commit_core_delivery_outcome_v1 = delivery
    async def run():
        event, request, response = await harness.run()
        frozen = plugin._pending[event.turn_token]
        assert "base_revision" not in frozen
        with pytest.raises(TypeError): frozen["turn_id"] = "changed"
        assert response.completion_text == "AstrBot ordinary reply"
        await plugin.after_message_sent(event, delivered=False)
        assert not plugin._pending and len(deliveries)==1
        assert plugin._revisions[harness.scope.persona_token] == 99
        await plugin.after_message_sent(event)
        assert len(deliveries)==1
    asyncio.run(run())


def test_delivery_terminal_error_clears_exact_pending_and_cancellation_compensates():
    harness = _Harness(config_values={}, failure="timeout")
    plugin = harness.plugin
    async def cancelled_provider(**kwargs):
        assert plugin._pending
        assert not plugin._persona_locks[plugin._semantic_lock_key(harness.scope)].locked()
        raise asyncio.CancelledError()
    plugin._semantic_generate = cancelled_provider
    async def run():
        event = _Event()
        with pytest.raises(asyncio.CancelledError):
            await plugin.on_llm_request(event, _Request())
        assert plugin._pending and len(harness.settlement_requests)==1
        def fail(request): raise OSError("terminal delivery error")
        plugin._bridge.commit_core_delivery_outcome_v1 = fail
        await plugin.after_message_sent(event)
        assert plugin._pending == {}
        assert len(plugin.delivery_diagnostics) == 1
    asyncio.run(run())
