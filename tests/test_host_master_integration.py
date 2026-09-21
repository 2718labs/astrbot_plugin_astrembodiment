"""Mainline provider regressions on the typed appraisal host."""

import asyncio
import importlib
import threading
from types import SimpleNamespace

import pytest

from main import AstrEmbodimentPlugin
import main as main_module


def test_unified_provider_precedes_legacy_for_genesis_and_appraisal():
    calls = []

    async def generate(**kwargs):
        calls.append(kwargs)
        return SimpleNamespace(completion_text="ok", usage=SimpleNamespace(total=7))

    context = SimpleNamespace(
        get_provider_by_id=lambda provider: object(), llm_generate=generate
    )
    plugin = AstrEmbodimentPlugin(
        context,
        {
            "assistant_provider_id": "unified",
            "semantic_estimator_provider_id": "legacy",
        },
    )
    event = SimpleNamespace(unified_msg_origin="room")

    async def run():
        assert await plugin._semantic_provider_id(event) == "unified"
        await plugin._genesis_generate(event, prompt="current", system_prompt="rules")

    asyncio.run(run())
    assert calls[0]["chat_provider_id"] == "unified"
    assert calls[0]["contexts"] is None
    assert calls[0]["tools"] is None


def test_async_missing_provider_fails_closed_before_genesis_generation():
    async def missing(_provider):
        return None

    calls = []

    async def generate(**kwargs):
        calls.append(kwargs)

    plugin = AstrEmbodimentPlugin(
        SimpleNamespace(get_provider_by_id=missing, llm_generate=generate),
        {"assistant_provider_id": "missing"},
    )
    with pytest.raises(RuntimeError):
        asyncio.run(
            plugin._genesis_generate(
                SimpleNamespace(unified_msg_origin="room"),
                prompt="current",
                system_prompt="rules",
            )
        )
    assert calls == []


def test_sync_generation_runs_off_loop_and_preserves_native_usage():
    loop_thread = threading.get_ident()
    response = SimpleNamespace(completion_text="ok", usage=SimpleNamespace(total=7))

    def generate(**kwargs):
        assert threading.get_ident() != loop_thread
        assert kwargs["contexts"] is None
        assert kwargs["tools"] is None
        return response

    plugin = AstrEmbodimentPlugin(SimpleNamespace(llm_generate=generate), {})
    result = asyncio.run(
        plugin._semantic_generate(
            provider_id="unified", prompt="current", system_prompt="rules"
        )
    )
    assert result is response
    assert plugin._semantic_response_parts(result) == (
        "ok",
        {"known": True, "used_tokens": 7},
    )


def test_sync_provider_cancellation_keeps_cancellation_type():
    def generate(**kwargs):
        raise asyncio.CancelledError()

    plugin = AstrEmbodimentPlugin(SimpleNamespace(llm_generate=generate), {})
    with pytest.raises(asyncio.CancelledError):
        asyncio.run(
            plugin._semantic_generate(
                provider_id="p", prompt="current", system_prompt="rules"
            )
        )


@pytest.mark.parametrize(
    "mode", ["missing_validator", "invalid_id", "private_exception"]
)
def test_provider_resolution_fails_closed_without_disclosing_host_errors(mode):
    def lookup(_provider):
        if mode == "private_exception":
            raise ValueError("private-provider-secret")
        return object()

    context = SimpleNamespace(
        get_current_chat_provider_id=lambda **kwargs: (
            42 if mode == "invalid_id" else "current"
        )
    )
    if mode != "missing_validator":
        context.get_provider_by_id = lookup
    plugin = AstrEmbodimentPlugin(context, {})
    with pytest.raises(RuntimeError) as error:
        asyncio.run(
            plugin._semantic_provider_id(SimpleNamespace(unified_msg_origin="room"))
        )
    assert "private-provider-secret" not in str(error.value)


@pytest.mark.parametrize("failure", ["timeout", "cancel"])
def test_abandoned_sync_workers_stay_bounded_and_dispose_late_coroutines(
    monkeypatch, failure
):
    release = threading.Event()
    started = []
    late_coroutines = []
    executed = []
    original_wait_for = asyncio.wait_for

    async def short_deadline(awaitable, timeout):
        return await original_wait_for(awaitable, 0.03 if timeout == 15.0 else timeout)

    monkeypatch.setattr(main_module.asyncio, "wait_for", short_deadline)

    async def late_result():
        executed.append(True)

    def generate(**kwargs):
        started.append(True)
        release.wait(3)
        result = late_result()
        late_coroutines.append(result)
        return result

    plugin = AstrEmbodimentPlugin(SimpleNamespace(llm_generate=generate), {})

    async def call():
        return await plugin._semantic_generate(
            provider_id="p", prompt="current", system_prompt="rules"
        )

    async def run():
        try:
            for count in range(4):
                task = asyncio.create_task(call())
                while len(started) <= count:
                    await asyncio.sleep(0.001)
                if failure == "cancel":
                    task.cancel()
                with pytest.raises(
                    TimeoutError if failure == "timeout" else asyncio.CancelledError
                ):
                    await task
            with pytest.raises(RuntimeError):
                await call()
            assert len(started) == 4
            importlib.reload(main_module)
            other = main_module.AstrEmbodimentPlugin(
                SimpleNamespace(llm_generate=generate), {}
            )
            with pytest.raises(RuntimeError):
                await other._semantic_generate(
                    provider_id="p", prompt="current", system_prompt="rules"
                )
            assert len(started) == 4
            await original_wait_for(plugin.terminate(), 0.2)
            with pytest.raises(RuntimeError):
                await call()
        finally:
            release.set()
            for _ in range(200):
                if len(late_coroutines) == len(started) and all(
                    c.cr_frame is None for c in late_coroutines
                ):
                    break
                await asyncio.sleep(0.005)
        assert len(late_coroutines) == 4
        assert all(c.cr_frame is None for c in late_coroutines)
        assert executed == []
        fresh = AstrEmbodimentPlugin(
            SimpleNamespace(llm_generate=lambda **kwargs: "released"), {}
        )
        assert (
            await fresh._semantic_generate(
                provider_id="p", prompt="current", system_prompt="rules"
            )
            == "released"
        )

    try:
        asyncio.run(run())
    finally:
        release.set()
        for coroutine in late_coroutines:
            coroutine.close()
