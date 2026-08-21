from __future__ import annotations

import asyncio
import json

from astr_embodiment.bridge import NativeBridge
from astr_embodiment.contracts import FrozenTurn, ScopeTokens
from astr_embodiment.coordinator import GenesisCoordinator
from astr_embodiment.semantic_estimator import DIMENSION_NAMES


def _scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="11" * 16,
        persona_token="22" * 16,
        session_token="33" * 16,
    )


def _turn() -> FrozenTurn:
    return FrozenTurn(
        scope=_scope(),
        turn_id="44" * 16,
        event_id="55" * 16,
        base_revision=0,
        observed_at_ms=1_700_000_000_001,
    )


def _estimate() -> dict:
    dimensions = {name: 0 for name in DIMENSION_NAMES}
    dimensions["positive"] = 500_000
    dimensions["boundary"] = 100_000
    return {"dimensions": dimensions, "estimator_confidence": 800_000}


class FakeNative:
    def __init__(self) -> None:
        self.revision_calls: list[str] = []
        self.apply_calls: list[tuple[str, str]] = []

    def semantic_revision_v1(self, scope_json: str) -> str:
        self.revision_calls.append(scope_json)
        return json.dumps(
            {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
        )

    def apply_perception_proposal_v1(self, scope_json: str, proposal_json: str) -> str:
        self.apply_calls.append((scope_json, proposal_json))
        return json.dumps(
            {
                "schema": "astrembodiment.semantic-perception-closure.v1",
                "receipt": {"next_revision": 1},
                "revision": 1,
                "deduplicated": False,
            }
        )


def test_bridge_serializes_closed_scope_and_proposal_and_exposes_allowlist() -> None:
    native = FakeNative()
    bridge = NativeBridge()
    bridge._native = native
    proposal = {
        "schema_version": 1,
        "event_id": "55" * 16,
        "turn_id": "44" * 16,
        "observed_at_ms": 1_700_000_000_001,
        "base_revision": 0,
        "dimensions": {name: 0 for name in DIMENSION_NAMES},
        "estimator_confidence": 800_000,
        "protocol_version": 1,
        "request_nonce_digest": "aa" * 32,
    }
    proposal["dimensions"]["positive"] = 500_000

    cursor = bridge.semantic_revision_v1(_scope().scope_json())
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(proposal)
    )

    assert cursor == {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}
    assert set(result) == {"schema", "receipt", "revision", "deduplicated"}
    assert json.loads(native.revision_calls[0]) == _scope().scope_json()
    sent_scope, sent_proposal = native.apply_calls[0]
    assert json.loads(sent_scope) == _scope().scope_json()
    assert json.loads(sent_proposal) == proposal
    assert "RAW_SENTINEL" not in sent_proposal


def test_missing_native_semantic_symbols_are_fixed_degraded() -> None:
    bridge = NativeBridge()
    bridge._native = object()

    cursor = bridge.semantic_revision_v1(_scope().scope_json())
    result = bridge.apply_perception_proposal_v1(_scope().scope_json(), "{}")

    assert cursor == {"status": "DEGRADED", "code": "NATIVE_SYMBOL_UNAVAILABLE"}
    assert result == {"status": "DEGRADED", "code": "NATIVE_SYMBOL_UNAVAILABLE"}


def test_native_exception_is_classified_without_raw_error_or_sentinel() -> None:
    class FailingNative:
        def semantic_revision_v1(self, _scope: str) -> str:
            raise RuntimeError("STALE_REVISION::RAW_SENTINEL user text")

        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            raise RuntimeError("STORAGE::RAW_SENTINEL")

    bridge = NativeBridge()
    bridge._native = FailingNative()

    cursor = bridge.semantic_revision_v1(_scope().scope_json())
    proposal = {
        "schema_version": 1,
        "event_id": "55" * 16,
        "turn_id": "44" * 16,
        "observed_at_ms": 1_700_000_000_001,
        "base_revision": 0,
        "dimensions": {name: 0 for name in DIMENSION_NAMES},
        "estimator_confidence": 800_000,
        "protocol_version": 1,
        "request_nonce_digest": "aa" * 32,
    }
    proposal["dimensions"]["positive"] = 1
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(proposal)
    )

    assert cursor == {"status": "DEGRADED", "code": "STALE_REVISION"}
    assert result == {"status": "DEGRADED", "code": "STORAGE"}
    assert "RAW_SENTINEL" not in json.dumps(cursor)
    assert "RAW_SENTINEL" not in json.dumps(result)


def test_unknown_receipt_fields_are_degraded_before_coordinator_success() -> None:
    class LeakyNative:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            return json.dumps(
                {
                    "schema": "astrembodiment.semantic-perception-closure.v1",
                    "receipt": {"secret": "RAW_SENTINEL"},
                    "revision": 1,
                    "deduplicated": False,
                }
            )

    bridge = NativeBridge()
    bridge._native = LeakyNative()
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(),
        json.dumps(
            {
                "schema_version": 1,
                "event_id": "55" * 16,
                "turn_id": "44" * 16,
                "observed_at_ms": 1_700_000_000_001,
                "base_revision": 0,
                "dimensions": {
                    **{name: 0 for name in DIMENSION_NAMES},
                    "positive": 1,
                },
                "estimator_confidence": 1,
                "protocol_version": 1,
                "request_nonce_digest": "aa" * 32,
            }
        ),
    )

    assert result == {"status": "DEGRADED", "code": "NATIVE_MALFORMED"}
    assert "RAW_SENTINEL" not in json.dumps(result)


def test_coordinator_preflight_calls_estimator_once_and_keeps_native_order() -> None:
    class OrderedBridge:
        def __init__(self) -> None:
            self.calls: list[tuple] = []

        def semantic_revision_v1(self, scope_json: dict) -> dict:
            self.calls.append(("revision", scope_json))
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 3}

        def apply_perception_proposal_v1(self, scope_json: dict, proposal_json: str) -> dict:
            self.calls.append(("apply", scope_json, json.loads(proposal_json)))
            return {
                "schema": "astrembodiment.semantic-perception-closure.v1",
                "receipt": {"next_revision": 4},
                "revision": 4,
                "deduplicated": False,
            }

    async def run() -> tuple[dict, dict, OrderedBridge, list[str]]:
        bridge = OrderedBridge()
        coordinator = GenesisCoordinator(bridge)  # type: ignore[arg-type]
        calls: list[str] = []

        async def estimator(request_text: str) -> dict:
            calls.append(request_text)
            return _estimate()

        first = await coordinator.preflight_stimulus(
            _scope(), _turn(), "CURRENT_REQUEST_RAW_SENTINEL", estimator
        )
        second = await coordinator.preflight_stimulus(
            _scope(), _turn(), "DIFFERENT_RAW_SENTINEL", estimator
        )
        return first, second, bridge, calls

    first, second, bridge, calls = asyncio.run(run())

    assert first["status"] == "SUCCESS"
    assert second == first
    assert calls == ["CURRENT_REQUEST_RAW_SENTINEL"]
    assert [call[0] for call in bridge.calls] == ["revision", "apply"]
    assert bridge.calls[1][2]["base_revision"] == 3
    assert "CURRENT_REQUEST_RAW_SENTINEL" not in json.dumps(first)
    assert "DIFFERENT_RAW_SENTINEL" not in json.dumps(first)


def test_zero_load_is_fixed_noop_and_never_calls_native_or_provider() -> None:
    class TrapBridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            raise AssertionError("zero-load must not read semantic cursor")

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            raise AssertionError("zero-load must not commit")

    async def run() -> tuple[dict, list[str]]:
        bridge = TrapBridge()
        coordinator = GenesisCoordinator(bridge)  # type: ignore[arg-type]
        calls: list[str] = []

        async def estimator(request_text: str) -> dict:
            calls.append(request_text)
            dimensions = {name: 0 for name in DIMENSION_NAMES}
            dimensions["affiliation"] = 1
            return {"dimensions": dimensions, "estimator_confidence": 1}

        result = await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", estimator
        )
        return result, calls

    result, calls = asyncio.run(run())
    assert result == {"status": "NOOP", "code": "ZERO_LOAD"}
    assert calls == ["request"]


def test_malformed_provider_result_is_degraded_and_does_not_call_native() -> None:
    class TrapBridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            raise AssertionError("malformed estimator must stop before native")

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            raise AssertionError("malformed estimator must stop before native")

    async def run() -> dict:
        coordinator = GenesisCoordinator(TrapBridge())  # type: ignore[arg-type]

        async def estimator(_request_text: str) -> dict:
            return {"text": "RAW_SENTINEL"}

        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", estimator
        )

    result = asyncio.run(run())
    assert result == {"status": "DEGRADED", "code": "ESTIMATOR_MALFORMED"}
    assert "RAW_SENTINEL" not in json.dumps(result)


def test_concurrent_same_request_joins_one_estimator_call() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return {
                "schema": "astrembodiment.semantic-perception-closure.v1",
                "receipt": {"next_revision": 1},
                "revision": 1,
                "deduplicated": False,
            }

    async def run() -> tuple[list[dict], list[str]]:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]
        calls: list[str] = []
        gate = asyncio.Event()

        async def estimator(text: str) -> dict:
            calls.append(text)
            await gate.wait()
            return _estimate()

        first = asyncio.create_task(
            coordinator.preflight_stimulus(_scope(), _turn(), "one", estimator)
        )
        await asyncio.sleep(0)
        second = asyncio.create_task(
            coordinator.preflight_stimulus(_scope(), _turn(), "two", estimator)
        )
        await asyncio.sleep(0)
        gate.set()
        return await asyncio.gather(first, second), calls

    results, calls = asyncio.run(run())
    assert calls == ["one"]
    assert results[0] == results[1]
