from __future__ import annotations

import asyncio
from types import SimpleNamespace

from astr_embodiment.auxiliary_transport import (
    DEFAULT_SEMANTIC_ESTIMATOR_TIMEOUT_MS,
    AuxiliaryProviderTransport,
)
from astr_embodiment.contracts import FrozenTurn, ScopeTokens
from astr_embodiment.coordinator import GenesisCoordinator


def test_configured_provider_is_validated_and_called_with_four_parameters() -> None:
    class Context:
        def __init__(self) -> None:
            self.provider_ids: list[str] = []
            self.calls: list[dict[str, object]] = []

        def get_provider_by_id(self, provider_id: str) -> object | None:
            self.provider_ids.append(provider_id)
            return object() if provider_id == "configured-private-id" else None

        async def llm_generate(self, **kwargs: object) -> object:
            self.calls.append(kwargs)
            return SimpleNamespace(completion_text="closed response")

    async def run() -> tuple[Context, str]:
        context = Context()
        transport = AuxiliaryProviderTransport(
            context=context,
            configured_provider=lambda: ("configured-private-id", "CONFIGURED"),
            timeout_ms=lambda: DEFAULT_SEMANTIC_ESTIMATOR_TIMEOUT_MS,
        )
        request = transport.open_request(umo="private-umo")
        request.bind_semantic_key("semantic-key")
        result = await request.generate(
            prompt="private prompt",
            system_prompt="closed system prompt",
            semantic_operation=False,
        )
        return context, result.text

    context, text = asyncio.run(run())

    assert text == "closed response"
    assert context.provider_ids == ["configured-private-id"]
    assert context.calls == [
        {
            "chat_provider_id": "configured-private-id",
            "prompt": "private prompt",
            "system_prompt": "closed system prompt",
            "tools": None,
        }
    ]


def test_transient_semantic_failure_is_not_saved_in_result_cache() -> None:
    scope = ScopeTokens(
        bot_token="11" * 16,
        persona_token="22" * 16,
        session_token="33" * 16,
    )
    turn = FrozenTurn(
        scope=scope,
        event_id="44" * 16,
        turn_id="55" * 16,
        base_revision=0,
        observed_at_ms=1,
    )

    async def run() -> int:
        coordinator = GenesisCoordinator(SimpleNamespace())
        calls = 0

        async def transient_failure(**_kwargs: object) -> dict[str, object]:
            nonlocal calls
            calls += 1
            return {
                "status": "DEGRADED",
                "code": "ESTIMATOR_UNAVAILABLE",
                "transport_subcode": "PROVIDER_CALL_FAILED",
                "attempted": True,
                "attempt_count": 1,
            }

        coordinator._run_semantic_v3 = transient_failure  # type: ignore[method-assign]
        for _ in range(2):
            outcome = await coordinator.preflight_semantic_v3(
                scope=scope,
                frozen_turn=turn,
                request_text="closed request",
                context_summary={},
                estimator=lambda _request: None,
            )
            assert outcome["code"] == "ESTIMATOR_UNAVAILABLE"
        return calls

    assert asyncio.run(run()) == 2


def test_semantic_retry_reuses_one_bound_provider_and_stops_after_two_calls() -> None:
    class Context:
        def __init__(self) -> None:
            self.provider_ids: list[str] = []
            self.calls: list[str] = []

        def get_provider_by_id(self, provider_id: str) -> object | None:
            self.provider_ids.append(provider_id)
            return object() if provider_id == "fixed-private-id" else None

        async def llm_generate(self, **kwargs: object) -> str:
            self.calls.append(str(kwargs["chat_provider_id"]))
            if len(self.calls) == 1:
                raise RuntimeError("transient host error")
            return "closed response"

    async def run() -> tuple[Context, object]:
        context = Context()
        transport = AuxiliaryProviderTransport(
            context=context,
            configured_provider=lambda: ("fixed-private-id", "CONFIGURED"),
            timeout_ms=lambda: 1_000,
        )
        request = transport.open_request(umo="private-umo")
        request.bind_semantic_key("semantic-key")
        result = await request.generate(
            prompt="private prompt",
            system_prompt="closed system prompt",
            semantic_operation=True,
        )
        return context, result

    context, result = asyncio.run(run())

    assert result.text == "closed response"
    assert result.meta.transport_subcode == "NONE"
    assert result.meta.attempted is True
    assert result.meta.attempt_count == 2
    assert context.provider_ids == ["fixed-private-id"]
    assert context.calls == ["fixed-private-id", "fixed-private-id"]
