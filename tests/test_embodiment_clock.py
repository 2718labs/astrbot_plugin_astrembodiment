"""Scheduler checks plus real Native clock/inbound/delivery integration."""

import asyncio
import sqlite3
from types import SimpleNamespace

import pytest

from astr_embodiment.embodiment_clock import EmbodimentClock
from astr_embodiment.interaction import build_core_inbound_request
from astr_embodiment.bridge import NativeBridge
from astr_embodiment.contracts import ScopeTokens
from astr_embodiment.tzdb_loader import freeze_persona_time


SCOPE = {"bot_token": "21" * 16, "persona_token": "22" * 16}


@pytest.mark.parametrize(
    "cadence,ceiling", [("active", 300000), ("asleep", 3600000), ("calm", 21600000)]
)
def test_not_due_is_only_a_poll_and_due_bytes_are_forwarded(cadence, ceiling):
    class Bridge:
        def __init__(self):
            self.due = False
            self.advances = []

        def embodiment_clock_status_v1(self, request):
            assert locks[clock.key(SCOPE)].locked()
            assert request["frozen"] == freeze_persona_time("UTC", 1000000)
            if self.due:
                return {"status": "due", "exact_advance_request_bytes": exact}
            return {
                "status": "not_due",
                "next_due_at_utc_ms": 999999999,
                "cadence_class": cadence,
            }

        def advance_embodiment_time_v1(self, data):
            assert data is exact
            self.advances.append(data)
            return {
                "receipt": {
                    "result": {
                        "next_due_at_utc_ms": 999999999,
                        "cadence_class": cadence,
                    }
                }
            }

    exact = [5, 0, 11, 1, 0, 99]
    locks, bridge = {}, Bridge()
    clock = EmbodimentClock(
        bridge=bridge,
        persona_locks=locks,
        config=lambda k, d: d,
        now_ms=lambda: 1000000,
    )
    clock.prepare_locked = lambda scope: {
        "profile": {"persona_tzid": "UTC"},
        "profile_revision": 1,
        "schedule_revision": 1,
    }

    async def run():
        await clock.tick(SCOPE)
        assert bridge.advances == []
        assert clock._heap == [(1000000 + ceiling, clock.key(SCOPE))]
        bridge.due = True
        await clock.tick(SCOPE)
        assert len(bridge.advances) == 1
        assert len(clock._heap) == 1

    asyncio.run(run())


def test_inventory_epoch_restarts_and_stop_cancels_error_path():
    class Bridge:
        def __init__(self):
            self.calls = 0

        def list_embodiment_personas_v1(self, request):
            self.calls += 1
            if self.calls == 2:
                raise RuntimeError("INVENTORY_CHANGED")
            return {
                "snapshot_token": "11" * 32,
                "entries": [{"scope": SCOPE}],
                "next_after": SCOPE if self.calls == 1 else None,
            }

        def get_embodiment_persona_v1(self, scope):
            raise RuntimeError("bounded failure")

    bridge = Bridge()
    clock = EmbodimentClock(bridge=bridge, persona_locks={}, config=lambda k, d: d)

    async def run():
        await clock.rescan()
        assert bridge.calls == 3 and len(clock._heap) == 1
        await clock.start()
        task = clock._task
        await asyncio.sleep(0.01)
        assert len(clock._heap) == 1
        await clock.stop()
        await clock.stop()
        assert task.done() and clock._heap == []

    asyncio.run(run())


def test_real_native_restart_not_due_and_delivery_after_clock(tmp_path):
    from test_runtime_integration import _fresh_native_genesis_request

    bridge = NativeBridge()
    bridge.open(str(tmp_path))
    tokens = ScopeTokens(SCOPE["bot_token"], SCOPE["persona_token"], "23" * 16)
    bridge.ensure_genesis(_fresh_native_genesis_request(tokens))
    now = [1773000000000]
    clock = EmbodimentClock(
        bridge=bridge,
        persona_locks={},
        config=lambda k, d: "UTC" if k == "persona_timezone" else d,
        now_ms=lambda: now[0],
    )
    event = SimpleNamespace(
        platform_id="test",
        bot_id="bot",
        unified_msg_origin="test:private:one",
        message_id="message-1",
    )

    def snapshot():
        with sqlite3.connect(
            f"file:{tmp_path / 'astrembodiment.sqlite3'}?mode=ro", uri=True
        ) as db:
            return tuple(db.iterdump())

    advances = []
    native_advance = bridge.advance_embodiment_time_v1

    def advance(data):
        result = native_advance(data)
        advances.append(result)
        return result

    bridge.advance_embodiment_time_v1 = advance
    try:
        asyncio.run(clock.tick(SCOPE))
        first = bridge.get_embodiment_persona_v1(SCOPE)
        before = snapshot()
        asyncio.run(clock.tick(SCOPE))
        assert bridge.get_embodiment_persona_v1(SCOPE) == first
        assert snapshot() == before
        inbound = build_core_inbound_request(
            bridge,
            scope=tokens,
            event=event,
            message="hello",
            appraisal=None,
            observed_at_ms=now[0],
        )
        committed = bridge.commit_core_inbound_v1(inbound)
        assert committed["provider_authorized_now"] is False
        now[0] += 30 * 86400000
        asyncio.run(clock.tick(SCOPE))
        advanced = bridge.get_embodiment_persona_v1(SCOPE)
        assert len(advances) == 1
        assert advances[0]["receipt"]["result"]["capped_gap"] is True
        assert advances[0]["receipt"]["result"]["applied_elapsed_ms"] == 604800000
        assert advanced["clock_head"]["sequence"] == first["clock_head"]["sequence"] + 1
        replay = bridge.commit_core_inbound_v1(inbound)
        assert replay["commit_status"] == "existing"
        assert replay["initial_receipt"] == committed["initial_receipt"]
        observation = inbound["observation"]
        delivery = bridge.compile_core_host_request_v1(
            "delivery",
            {
                "schema_version": 1,
                "operation_id": "00" * 16,
                "scope": SCOPE,
                "turn_id": observation["turn_id"],
                "inbound_operation_id": observation["operation_id"],
                "delivered": True,
                "observed_at_utc_ms": now[0],
                "visible_action_digest": "11" * 32,
            },
        )
        result = bridge.commit_core_delivery_outcome_v1(delivery)
        assert result["commit_status"] == "committed"
        assert (
            bridge.commit_core_delivery_outcome_v1(delivery)["commit_status"]
            == "existing"
        )
        altered = dict(delivery, delivered=False)
        with pytest.raises(Exception, match="IDEMPOTENCY_CONFLICT"):
            bridge.commit_core_delivery_outcome_v1(altered)
    finally:
        bridge.close()


def test_real_native_host_semantic_matrix_and_ordinary_delivery(tmp_path):
    from test_runtime_integration import _fresh_native_genesis_request
    from test_ordinary_reply_flow import _Harness

    harness = _Harness(config_values={})
    bridge = NativeBridge()
    bridge.open(str(tmp_path))
    bridge.ensure_genesis(_fresh_native_genesis_request(harness.scope))
    harness.plugin._bridge = bridge
    harness.plugin._clock = EmbodimentClock(
        bridge=bridge,
        persona_locks=harness.plugin._persona_locks,
        config=harness.plugin._config_value,
    )

    async def run():
        event, request, response = await harness.run()
        assert event.stopped is False
        assert len(harness.context.semantic_calls) == 1
        assert response.completion_text == "AstrBot ordinary reply"
        assert any(
            item["code"] == "SEMANTIC_COMMITTED"
            for item in harness.plugin._semantic_diagnostics
        )
        assert "AstrEmbodiment Runtime Context" in request.system_prompt
        assert harness.plugin._pending
        await harness.plugin.after_message_sent(event)
        assert not harness.plugin._pending
        assert not harness.plugin.delivery_diagnostics

    try:
        asyncio.run(run())
    finally:
        bridge.close()
