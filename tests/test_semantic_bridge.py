from __future__ import annotations

import asyncio
import json
from dataclasses import replace

import pytest

from astr_embodiment.bridge import NativeBridge
from astr_embodiment.contracts import FrozenTurn, ScopeTokens
from astr_embodiment.coordinator import GenesisCoordinator
from astr_embodiment.semantic_estimator import (
    DIMENSION_NAMES,
    make_request_nonce_digest,
)


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


def _calculation(
    *,
    active_nodes: int = 0,
    active_edges: int = 0,
    state_changed: bool = True,
) -> dict:
    return {
        "state_changed": state_changed,
        "active_nodes": active_nodes,
        "active_edges": active_edges,
        "residuals_fxp6": {
            "authority": 0,
            "continuity": 0,
            "energy": 0,
            "renormalization": 0,
            "capacity": 0,
        },
    }


def _diagnostic(
    *,
    stage: str,
    commit_state: str,
    values_state: str,
    dimensions: dict[str, int] | None = None,
    estimator_confidence: int | None = None,
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
        native_calculation = _calculation()
    return {
        "stage": stage,
        "commit_state": commit_state,
        "values_state": values_state,
        "dimensions_fxp6": dict(dimensions) if dimensions is not None else None,
        "estimator_confidence_fxp6": estimator_confidence,
        "base_revision": base_revision,
        "revision": revision,
        "deduplicated": deduplicated,
        "receipt_status": receipt_status,
        "calculation_state": calculation_state,
        "native_calculation": native_calculation,
    }


def _outcome(status: str, code: str, diagnostic: dict) -> dict:
    return {"status": status, "code": code, "diagnostic": diagnostic}


def _valid_receipt(
    *,
    base_revision: int = 0,
    next_revision: int = 1,
    state_before: str = "01" * 32,
    state_after: str = "02" * 32,
    status: str = "committed",
) -> dict:
    return {
        "schema_version": 1,
        "formula_digest": "00" * 32,
        "scope_digest": "11" * 32,
        "event_digest": "22" * 32,
        "authority_digest": "33" * 32,
        "base_revision": base_revision,
        "next_revision": next_revision,
        "state_before": state_before,
        "state_after": state_after,
        "graph_after": "44" * 32,
        "active_nodes": 0,
        "active_edges": 0,
        "residuals": {
            "authority": 0,
            "continuity": 0,
            "energy": 0,
            "renormalization": 0,
            "capacity": 0,
        },
        "status": status,
    }


def _valid_result(*, deduplicated: bool = False, **receipt_overrides: object) -> dict:
    receipt = _valid_receipt(**receipt_overrides)
    return {
        "schema": "astrembodiment.semantic-perception-closure.v1",
        "receipt": receipt,
        "revision": receipt["next_revision"],
        "deduplicated": deduplicated,
    }


def _proposal(
    *,
    scope: ScopeTokens | None = None,
    turn: FrozenTurn | None = None,
    nonce: str | None = None,
) -> dict:
    scope = scope or _scope()
    turn = turn or _turn()
    dimensions = {name: 0 for name in DIMENSION_NAMES}
    dimensions["positive"] = 1
    return {
        "schema_version": 1,
        "event_id": turn.event_id,
        "turn_id": turn.turn_id,
        "observed_at_ms": turn.observed_at_ms,
        "base_revision": turn.base_revision,
        "dimensions": dimensions,
        "estimator_confidence": 1,
        "protocol_version": 1,
        "request_nonce_digest": nonce
        or make_request_nonce_digest(scope, turn),
    }


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
            _valid_result()
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
        "request_nonce_digest": make_request_nonce_digest(_scope(), _turn()),
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
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(_proposal())
    )

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
        "request_nonce_digest": make_request_nonce_digest(_scope(), _turn()),
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
                "request_nonce_digest": make_request_nonce_digest(_scope(), _turn()),
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
            return _valid_result(base_revision=3, next_revision=4)

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
    assert first["diagnostic"] == _diagnostic(
        stage="RECEIPT",
        commit_state="CONFIRMED_NEW",
        values_state="COMMITTED",
        dimensions=_estimate()["dimensions"],
        estimator_confidence=800_000,
        base_revision=3,
        revision=4,
        deduplicated=False,
        receipt_status="committed",
    )
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
    expected_dimensions = {name: 0 for name in DIMENSION_NAMES}
    expected_dimensions["affiliation"] = 1
    assert result == _outcome(
        "NOOP",
        "ZERO_LOAD",
        _diagnostic(
            stage="ESTIMATOR",
            commit_state="NOT_ATTEMPTED",
            values_state="ESTIMATED_NOT_COMMITTED",
            dimensions=expected_dimensions,
            estimator_confidence=1,
        ),
    )
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
    assert result == _outcome(
        "DEGRADED",
        "ESTIMATOR_MALFORMED",
        _diagnostic(
            stage="ESTIMATOR",
            commit_state="NOT_ATTEMPTED",
            values_state="UNAVAILABLE",
        ),
    )
    assert "RAW_SENTINEL" not in json.dumps(result)


def test_concurrent_same_request_joins_one_estimator_call() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return _valid_result()

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


@pytest.mark.parametrize(
    "mutated_turn",
    [
        replace(_turn(), event_id="66" * 16),
        replace(_turn(), turn_id="77" * 16),
        replace(_turn(), observed_at_ms=1_700_000_000_002),
        replace(_turn(), base_revision=1),
    ],
)
def test_bridge_rejects_nonce_replayed_across_any_frozen_turn_fact(
    mutated_turn: FrozenTurn,
) -> None:
    class Native:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            return json.dumps(_valid_result())

    bridge = NativeBridge()
    bridge._native = Native()
    original = _turn()
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(),
        json.dumps(_proposal(turn=mutated_turn, nonce=make_request_nonce_digest(_scope(), original))),
    )

    assert result == {"status": "DEGRADED", "code": "INVALID_PERCEPTION_PROPOSAL"}


def test_bridge_rejects_nonce_replayed_under_a_different_scope() -> None:
    class Native:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            return json.dumps(_valid_result())

    bridge = NativeBridge()
    bridge._native = Native()
    original_scope = _scope()
    altered_scope = replace(original_scope, relation_token="44" * 16)
    result = bridge.apply_perception_proposal_v1(
        altered_scope.scope_json(),
        json.dumps(
            _proposal(
                scope=altered_scope,
                nonce=make_request_nonce_digest(original_scope, _turn()),
            )
        ),
    )

    assert result == {"status": "DEGRADED", "code": "INVALID_PERCEPTION_PROPOSAL"}


def test_bridge_rejects_arbitrary_nonce_even_when_native_returns_valid_receipt() -> None:
    class Native:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            return json.dumps(_valid_result())

    bridge = NativeBridge()
    bridge._native = Native()
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(_proposal(nonce="aa" * 32))
    )

    assert result == {"status": "DEGRADED", "code": "INVALID_PERCEPTION_PROPOSAL"}


def test_bridge_zero_load_returns_fixed_noop_without_invoking_native() -> None:
    class TrapNative:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            raise AssertionError("zero-load must not invoke native")

    bridge = NativeBridge()
    bridge._native = TrapNative()
    proposal = _proposal()
    proposal["dimensions"] = {name: 0 for name in DIMENSION_NAMES}
    proposal["dimensions"]["affiliation"] = 1
    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(proposal)
    )

    assert result == {"status": "NOOP", "code": "ZERO_LOAD"}


@pytest.mark.parametrize(
    "result",
    [
        {"schema": "astrembodiment.semantic-perception-closure.v1", "receipt": {}, "revision": 1, "deduplicated": False},
        {"schema": "astrembodiment.semantic-perception-closure.v1", "receipt": _valid_receipt(next_revision=1) | {"status": "stale"}, "revision": 1, "deduplicated": False},
        {"schema": "astrembodiment.semantic-perception-closure.v1", "receipt": _valid_receipt(next_revision=2), "revision": 1, "deduplicated": False},
        {"schema": "astrembodiment.semantic-perception-closure.v1", "receipt": _valid_receipt(state_before="01" * 32, state_after="01" * 32), "revision": 1, "deduplicated": False},
        {"schema": "astrembodiment.semantic-perception-closure.v1", "receipt": _valid_receipt() | {"action_contract": None}, "revision": 1, "deduplicated": False},
    ],
)
def test_bridge_rejects_weak_or_non_closed_native_receipts(result: dict) -> None:
    class Native:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            return json.dumps(result)

    bridge = NativeBridge()
    bridge._native = Native()
    output = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(_proposal())
    )

    assert output == {"status": "DEGRADED", "code": "NATIVE_MALFORMED"}


@pytest.mark.parametrize(
    "native_result",
    [
        {
            "schema": "astrembodiment.semantic-perception-closure.v1",
            "receipt": {},
            "revision": 1,
            "deduplicated": False,
        },
        {
            "schema": "astrembodiment.semantic-perception-closure.v1",
            "receipt": _valid_receipt(status="stale"),
            "revision": 1,
            "deduplicated": False,
        },
        {
            "schema": "astrembodiment.semantic-perception-closure.v1",
            "receipt": _valid_receipt(next_revision=2),
            "revision": 1,
            "deduplicated": False,
        },
        {
            "schema": "astrembodiment.semantic-perception-closure.v1",
            "receipt": _valid_receipt(
                state_before="01" * 32, state_after="01" * 32
            ),
            "revision": 1,
            "deduplicated": False,
        },
    ],
)
def test_coordinator_never_promotes_a_weak_receipt_to_success(
    native_result: dict,
) -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return native_result

    async def run() -> dict:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]

        async def estimator(_request_text: str) -> dict:
            return _estimate()

        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", estimator
        )

    result = asyncio.run(run())
    assert result == _outcome(
        "DEGRADED",
        "NATIVE_MALFORMED",
        _diagnostic(
            stage="RECEIPT",
            commit_state="UNKNOWN",
            values_state="ESTIMATED_NOT_CONFIRMED",
            dimensions=_estimate()["dimensions"],
            estimator_confidence=800_000,
            base_revision=0,
        ),
    )


def test_coordinator_accepts_a_direct_bridge_zero_load_noop() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return {"status": "NOOP", "code": "ZERO_LOAD"}

    async def run() -> dict:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]

        async def estimator(_request_text: str) -> dict:
            return _estimate()

        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", estimator
        )

    assert asyncio.run(run()) == _outcome(
        "NOOP",
        "ZERO_LOAD",
        _diagnostic(
            stage="ESTIMATOR",
            commit_state="NOT_ATTEMPTED",
            values_state="ESTIMATED_NOT_COMMITTED",
            dimensions=_estimate()["dimensions"],
            estimator_confidence=800_000,
        ),
    )


def test_coordinator_cancellation_keeps_one_shared_attempt_for_later_retry() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return _valid_result()

    async def run() -> tuple[dict, list[str]]:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]
        calls: list[str] = []
        gate = asyncio.Event()

        async def estimator(text: str) -> dict:
            calls.append(text)
            await gate.wait()
            return _estimate()

        cancelled = asyncio.create_task(
            coordinator.preflight_stimulus(_scope(), _turn(), "cancelled", estimator)
        )
        await asyncio.sleep(0)
        cancelled.cancel()
        with pytest.raises(asyncio.CancelledError):
            await cancelled

        retry = asyncio.create_task(
            coordinator.preflight_stimulus(_scope(), _turn(), "retry", estimator)
        )
        await asyncio.sleep(0)
        gate.set()
        return await retry, calls

    result, calls = asyncio.run(run())
    assert result["status"] == "SUCCESS"
    assert calls == ["cancelled"]


def test_coordinator_key_includes_frozen_base_and_observed_time() -> None:
    class Bridge:
        def __init__(self) -> None:
            self.proposals: list[dict] = []

        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, proposal_json: str) -> dict:
            self.proposals.append(json.loads(proposal_json))
            return _valid_result()

    async def run() -> tuple[list[str], Bridge]:
        bridge = Bridge()
        coordinator = GenesisCoordinator(bridge)  # type: ignore[arg-type]
        calls: list[str] = []

        async def estimator(text: str) -> dict:
            calls.append(text)
            return _estimate()

        await coordinator.preflight_stimulus(_scope(), _turn(), "one", estimator)
        altered = replace(_turn(), base_revision=9, observed_at_ms=1_700_000_000_009)
        await coordinator.preflight_stimulus(_scope(), altered, "two", estimator)
        return calls, bridge

    calls, bridge = asyncio.run(run())
    assert calls == ["one", "two"]
    assert len(bridge.proposals) == 2


def test_bridge_rejects_scope_dict_and_str_subclasses_before_nonce_binding() -> None:
    class EvilStr(str):
        def lower(self) -> str:
            return "ab" * 16

    class ScopeDict(dict):
        pass

    class Native:
        def __init__(self) -> None:
            self.apply_calls = 0

        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            self.apply_calls += 1
            return json.dumps(_valid_result())

    native = Native()
    bridge = NativeBridge()
    bridge._native = native
    hostile_scope = ScopeDict(
        {
            "bot_token": EvilStr("11" * 16),
            "persona_token": "22" * 16,
            "relation_token": None,
            "session_token": "33" * 16,
        }
    )
    nonce_scope = replace(_scope(), bot_token="ab" * 16)
    nonce_turn = replace(_turn(), scope=nonce_scope)

    result = bridge.apply_perception_proposal_v1(
        hostile_scope,
        json.dumps(
            _proposal(
                nonce=make_request_nonce_digest(nonce_scope, nonce_turn),
            )
        ),
    )

    assert result == {"status": "DEGRADED", "code": "INVALID_PERCEPTION_SCOPE"}
    assert native.apply_calls == 0


def test_coordinator_consumes_baseexception_and_caches_fixed_retry_result() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            raise AssertionError("fatal estimator must stop before native")

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            raise AssertionError("fatal estimator must stop before native")

    class FatalProvider(BaseException):
        pass

    async def run() -> tuple[dict, dict, list[str], GenesisCoordinator]:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]
        calls: list[str] = []

        async def estimator(text: str) -> dict:
            calls.append(text)
            raise FatalProvider("RAW_SENTINEL")

        first = await coordinator.preflight_stimulus(
            _scope(), _turn(), "first", estimator
        )
        second = await coordinator.preflight_stimulus(
            _scope(), _turn(), "second", estimator
        )
        return first, second, calls, coordinator

    first, second, calls, coordinator = asyncio.run(run())

    assert first == _outcome(
        "DEGRADED",
        "ESTIMATOR_UNAVAILABLE",
        _diagnostic(
            stage="ESTIMATOR",
            commit_state="NOT_ATTEMPTED",
            values_state="UNAVAILABLE",
        ),
    )
    assert second == first
    assert calls == ["first"]
    assert coordinator._preflight_inflight == {}
    assert len(coordinator._preflight_results) == 1
    assert "RAW_SENTINEL" not in json.dumps(second)


def test_bridge_rejects_hidden_receipt_mapping_and_forbidden_payload() -> None:
    class Hidden(dict):
        def __iter__(self):
            return (key for key in super().__iter__() if key != "raw_text")

    class Native:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> dict:
            receipt = Hidden(_valid_receipt())
            receipt["raw_text"] = "RAW_SENTINEL"
            return {
                "schema": "astrembodiment.semantic-perception-closure.v1",
                "receipt": receipt,
                "revision": 1,
                "deduplicated": False,
            }

    bridge = NativeBridge()
    bridge._native = Native()

    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(_proposal())
    )

    assert result == {"status": "DEGRADED", "code": "NATIVE_MALFORMED"}
    assert "RAW_SENTINEL" not in json.dumps(result)


def test_bridge_rejects_case_equivalent_state_transition() -> None:
    class Native:
        def apply_perception_proposal_v1(self, _scope: str, _proposal: str) -> str:
            return json.dumps(
                _valid_result(state_before="AA" * 32, state_after="aa" * 32)
            )

    bridge = NativeBridge()
    bridge._native = Native()

    result = bridge.apply_perception_proposal_v1(
        _scope().scope_json(), json.dumps(_proposal())
    )

    assert result == {"status": "DEGRADED", "code": "NATIVE_MALFORMED"}


def test_coordinator_rejects_receipt_base_not_bound_to_proposal() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return _valid_result(base_revision=9, next_revision=10)

    async def run() -> dict:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]

        async def estimator(_request_text: str) -> dict:
            return _estimate()

        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", estimator
        )

    assert asyncio.run(run()) == _outcome(
        "DEGRADED",
        "NATIVE_MALFORMED",
        _diagnostic(
            stage="RECEIPT",
            commit_state="UNKNOWN",
            values_state="ESTIMATED_NOT_CONFIRMED",
            dimensions=_estimate()["dimensions"],
            estimator_confidence=800_000,
            base_revision=0,
        ),
    )


def test_coordinator_exposes_confirmed_existing_receipt_diagnostic() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return _valid_result(deduplicated=True)

    async def run() -> dict:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]
        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", lambda _request: _estimate()
        )

    result = asyncio.run(run())
    assert result["diagnostic"] == _diagnostic(
        stage="RECEIPT",
        commit_state="CONFIRMED_EXISTING",
        values_state="COMMITTED",
        dimensions=_estimate()["dimensions"],
        estimator_confidence=800_000,
        base_revision=0,
        revision=1,
        deduplicated=True,
        receipt_status="committed",
    )


def test_coordinator_keeps_valid_estimate_when_native_apply_fails() -> None:
    class FailingBridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            raise RuntimeError("NATIVE_APPLY_RAW_SENTINEL")

    async def run() -> dict:
        coordinator = GenesisCoordinator(FailingBridge())  # type: ignore[arg-type]
        return await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", lambda _request: _estimate()
        )

    result = asyncio.run(run())
    assert result == _outcome(
        "DEGRADED",
        "NATIVE_ERROR",
        _diagnostic(
            stage="NATIVE_APPLY",
            commit_state="UNKNOWN",
            values_state="ESTIMATED_NOT_CONFIRMED",
            dimensions=_estimate()["dimensions"],
            estimator_confidence=800_000,
            base_revision=0,
        ),
    )
    assert "NATIVE_APPLY_RAW_SENTINEL" not in json.dumps(result)


def test_coordinator_marks_unlocatable_shared_task_failure_internal() -> None:
    class ExplodingCoordinator(GenesisCoordinator):
        async def _run_preflight(self, **_kwargs: object) -> dict:
            raise RuntimeError("OUTER_RAW_SENTINEL")

    async def run() -> tuple[dict, dict]:
        coordinator = ExplodingCoordinator(object())  # type: ignore[arg-type]
        first = await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", lambda _request: _estimate()
        )
        second = await coordinator.preflight_stimulus(
            _scope(), _turn(), "request", lambda _request: _estimate()
        )
        return first, second

    first, second = asyncio.run(run())
    assert first == _outcome(
        "DEGRADED",
        "NATIVE_ERROR",
        _diagnostic(
            stage="INTERNAL",
            commit_state="UNKNOWN",
            values_state="UNAVAILABLE",
        ),
    )
    assert second == first
    assert "OUTER_RAW_SENTINEL" not in json.dumps(first)


def test_coordinator_deduplicates_case_equivalent_hex_identity() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: dict) -> dict:
            return {"schema": "astrembodiment.semantic-revision.v1", "revision": 0}

        def apply_perception_proposal_v1(self, _scope: dict, _proposal: str) -> dict:
            return _valid_result()

    async def run() -> tuple[list[str], dict, dict]:
        coordinator = GenesisCoordinator(Bridge())  # type: ignore[arg-type]
        calls: list[str] = []
        lower_scope = ScopeTokens(
            bot_token="ab" * 16,
            persona_token="cd" * 16,
            session_token="ef" * 16,
        )
        lower_turn = FrozenTurn(
            scope=lower_scope,
            turn_id="a4" * 16,
            event_id="b5" * 16,
            base_revision=0,
            observed_at_ms=1_700_000_000_010,
        )
        upper_scope = replace(
            lower_scope,
            bot_token=lower_scope.bot_token.upper(),
            persona_token=lower_scope.persona_token.upper(),
            session_token=lower_scope.session_token.upper(),
        )
        upper_turn = FrozenTurn(
            scope=upper_scope,
            turn_id=lower_turn.turn_id.upper(),
            event_id=lower_turn.event_id.upper(),
            base_revision=lower_turn.base_revision,
            observed_at_ms=lower_turn.observed_at_ms,
        )

        async def estimator(text: str) -> dict:
            calls.append(text)
            return _estimate()

        first = await coordinator.preflight_stimulus(
            lower_scope, lower_turn, "lower", estimator
        )
        second = await coordinator.preflight_stimulus(
            upper_scope, upper_turn, "upper", estimator
        )
        return calls, first, second

    calls, first, second = asyncio.run(run())

    assert calls == ["lower"]
    assert second == first
