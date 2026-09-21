from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import sqlite3
import time
from pathlib import Path

from astr_embodiment.auxiliary_transport import (
    AuxiliaryProviderBindingV1,
    AuxiliaryTransportMetaV1,
    AuxiliaryTransportResultV1,
)
from astr_embodiment.contracts import FrozenTurn, ScopeTokens
from astr_embodiment.coordinator import GenesisCoordinator
from astr_embodiment.semantic_contract import DIMENSION_NAMES, PRESENT
from astr_embodiment.semantic_estimator import (
    ESTIMATOR_FORMULA_DIGEST,
    build_perception_proposal_v3,
    make_request_nonce_digest,
    proposal_to_json,
)
from astr_embodiment.semantic_outbox import (
    SemanticJobTicket,
    SemanticOutbox,
    SemanticOutboxConfig,
)


class _ClosedCryptoError(RuntimeError):
    def __init__(self, code: str) -> None:
        self.code = code
        super().__init__(code)


class _StrictBridgeFake:
    """A non-reversible test seal/open boundary; plaintext stays out of SQLite."""

    def __init__(self) -> None:
        self._sealed: dict[str, tuple[str, str]] = {}
        self.apply_count = 0

    def semantic_outbox_crypto_status_v1(self) -> dict[str, object]:
        return {
            "schema": "astrembodiment.semantic-outbox-crypto-status.v1",
            "status": "READY",
            "key_version": 1,
        }

    def semantic_outbox_seal_v1(
        self, aad_b64: str, plaintext_b64: str, *, key_version: int = 1
    ) -> dict[str, object]:
        if key_version != 1:
            raise _ClosedCryptoError("ASYNC_KEY_VERSION_UNSUPPORTED")
        envelope = base64.b64encode(
            hashlib.sha256(
                aad_b64.encode("ascii") + b"\x00" + plaintext_b64.encode("ascii")
            ).digest()
        ).decode("ascii")
        self._sealed[envelope] = (aad_b64, plaintext_b64)
        return {
            "schema": "astrembodiment.semantic-outbox-sealed.v1",
            "key_version": 1,
            "envelope_b64": envelope,
        }

    def semantic_outbox_open_v1(
        self, aad_b64: str, envelope_b64: str
    ) -> dict[str, object]:
        sealed = self._sealed.get(envelope_b64)
        if sealed is None or sealed[0] != aad_b64:
            raise _ClosedCryptoError("ASYNC_PAYLOAD_AUTH_FAILED")
        return {
            "schema": "astrembodiment.semantic-outbox-opened.v1",
            "plaintext_b64": sealed[1],
        }

    def semantic_revision_v1(self, _scope: ScopeTokens) -> dict[str, object]:
        return {"schema": "astrembodiment.semantic-revision.v1", "revision": 7}

    def inspect(self, _scope: dict[str, object]) -> dict[str, object]:
        return {
            "bound": True,
            "revision": 7,
            "incarnation_id": "AE-I1-FAKE",
        }

    def apply_perception_proposal_v1(
        self, _scope: ScopeTokens, _proposal: dict[str, object]
    ) -> dict[str, object]:
        self.apply_count += 1
        return {
            "schema": "astrembodiment.semantic-perception-closure.v1",
            "revision": 8,
            "deduplicated": False,
            "full_vector_state": "FULL_VECTOR_CONFIRMED",
            "node_observability_state": "CONFIRMED",
            "semantic_vector_receipt": {
                "dimension_slot_count": 15,
                "evaluated_dimension_count": 15,
                "injected_dimension_count": 15,
                "unavailable_dimension_count": 0,
            },
            "expression_projection": {
                "schema": "astr-embodiment.expression-projection.v1",
                "revision": 8,
                "profile_fxp6": {
                    "warmth": 100_000,
                    "sensitivity": 100_000,
                    "guardedness": 100_000,
                    "repair_orientation": 100_000,
                    "engagement": 100_000,
                    "epistemic_caution": 100_000,
                },
            },
        }


class _GatedTransportFake:
    def __init__(self) -> None:
        self.started = asyncio.Event()
        self.release = asyncio.Event()

    async def generate_bound_once(
        self,
        *,
        binding: AuxiliaryProviderBindingV1,
        prompt: str,
        system_prompt: str,
        attempt_count: int,
        timeout_ms: int,
    ) -> AuxiliaryTransportResultV1:
        assert binding.provider_id == "aux-provider"
        assert prompt == "current turn only"
        assert "Evaluate only current_turn_text" in system_prompt
        assert attempt_count == 1
        assert timeout_ms >= 5_000
        self.started.set()
        await self.release.wait()
        return AuxiliaryTransportResultV1(
            text=_estimator_text(),
            meta=AuxiliaryTransportMetaV1("NONE", True, 1),
        )


def _scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        session_token="30" * 16,
        relation_token=None,
    )


def _summary() -> dict[str, object]:
    return {
        "schema": "astrembodiment.context-summary.v1",
        "summary_revision": 1,
        "source_continuum_revision": 7,
        "dimensions_ema_fxp6": [0] * 15,
        "unresolved_boundary": False,
        "unresolved_repair": False,
        "repetition_count": 1,
        "delivery_outcome": "pending",
        "summary_digest": "cd" * 32,
    }


def _estimator_text() -> str:
    dimensions = {
        name: {
            "state": PRESENT if name == "positive" else "ABSENT",
            "intensity_fxp6": 400_000 if name == "positive" else 0,
            "confidence_fxp6": 900_000,
        }
        for name in DIMENSION_NAMES
    }
    return json.dumps(
        {
            "schema": "astr-embodiment.semantic-estimate.v3",
            "dimensions": dimensions,
        },
        sort_keys=True,
    )


def _proposal_bytes(scope: ScopeTokens, turn: FrozenTurn) -> tuple[bytes, bytes, int]:
    proposal = build_perception_proposal_v3(
        scope=scope,
        turn=turn,
        estimate=_estimator_text(),
        base_revision=turn.base_revision,
        nonce_digest=make_request_nonce_digest(scope, turn),
    )
    proposal_json = proposal_to_json(proposal, scope=scope).encode("utf-8")
    dimensions_json = json.dumps(
        {"dimensions": proposal["dimensions"]},
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return proposal_json, dimensions_json, int(proposal["estimator_confidence"])


def _legacy_job_identity_v1(
    *, tokens: tuple[bytes, ...], protocol_digest: bytes
) -> bytes:
    """Pre-incarnation durable identity, retained only to exercise migration."""

    canonical = {
        "schema": "astrembodiment.semantic-outbox.v1",
        "bot_token": tokens[0].hex(),
        "persona_token": tokens[1].hex(),
        "session_token": tokens[2].hex(),
        "relation_token": tokens[3].hex() if tokens[3] else None,
        "event_token": tokens[4].hex(),
        "turn_token": tokens[5].hex(),
    }
    return hashlib.sha256(
        json.dumps(
            canonical,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
        + b"\x00"
        + protocol_digest
    ).digest()


def _seed_job(
    database_path: Path,
    *,
    scope: ScopeTokens,
    turn: FrozenTurn,
    incarnation_id: str,
    state: str,
    encrypted_payload: bytes | None,
    deadline_at_ms: int,
    lease_epoch: int = 0,
    proposal_json: bytes | None = None,
    dimensions_json: bytes | None = None,
    confidence_fxp6: int | None = None,
    forced_job_id: bytes | None = None,
) -> bytes:
    tokens = SemanticOutbox._row_tokens(scope, turn)
    protocol_digest = bytes.fromhex(ESTIMATOR_FORMULA_DIGEST)
    job_id = forced_job_id or SemanticOutbox._job_identity(
        tokens=tokens,
        incarnation_id=incarnation_id,
        protocol_digest=protocol_digest,
    )
    now = int(time.time() * 1_000)
    with sqlite3.connect(database_path) as connection:
        connection.execute(
            """
            INSERT INTO semantic_jobs_v1 (
                job_id, protocol_digest, key_version, state, bot_token, persona_token,
                session_token, relation_token, event_token, turn_token,
                incarnation_id, base_revision, attempt_count, lease_epoch,
                lease_owner_digest, lease_expires_at_ms, created_at_ms,
                deadline_at_ms, budget_expires_at_ms, updated_at_ms,
                terminal_code, encrypted_payload, proposal_json, proposal_digest,
                dimensions_json, confidence_fxp6, native_receipt_json,
                transport_subcode, provider_elapsed_ms, attempt_budget_ms, completed_at_ms
            ) VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                      NULL, ?, ?, ?, ?, ?, NULL, ?, ?, ?, NULL)
            """,
            (
                job_id,
                protocol_digest,
                state,
                *tokens,
                incarnation_id,
                turn.base_revision,
                1 if state != "PENDING" else 0,
                lease_epoch,
                b"x" * 32 if state == "RUNNING_PROVIDER" else None,
                now + 60_000 if state == "RUNNING_PROVIDER" else None,
                now,
                deadline_at_ms,
                now + 150_000,
                now,
                encrypted_payload,
                proposal_json,
                hashlib.sha256(proposal_json).digest()
                if proposal_json is not None
                else None,
                dimensions_json,
                confidence_fxp6,
                "NONE" if proposal_json is not None else "NOT_APPLICABLE",
                1 if proposal_json is not None else None,
                5_000 if proposal_json is not None else None,
            ),
        )
    return job_id


async def _enqueue_job(
    outbox: SemanticOutbox,
    *,
    scope: ScopeTokens,
    event_id: str,
    turn_id: str,
    request_text: str = "current turn only",
    incarnation_id: str = "AE-I1-FAKE",
) -> SemanticJobTicket:
    return await outbox.enqueue(
        scope=scope,
        frozen_turn=FrozenTurn(
            scope=scope,
            event_id=event_id,
            turn_id=turn_id,
            base_revision=7,
            observed_at_ms=1,
        ),
        incarnation_id=incarnation_id,
        protocol_digest=ESTIMATOR_FORMULA_DIGEST,
        request_text=request_text,
        context_summary=_summary(),
        provider_binding=AuxiliaryProviderBindingV1(
            provider_id="aux-provider",
            source="CONFIGURED",
            request_key="opaque-request-key",
        ),
    )


def test_foreground_defer_keeps_worker_alive_then_commits_once(tmp_path: Path) -> None:
    async def run() -> None:
        bridge = _StrictBridgeFake()
        transport = _GatedTransportFake()
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path,
            bridge=bridge,
            transport=transport,
            config=SemanticOutboxConfig(sync_wait_ms=250),
        )
        assert await outbox.start() is True
        scope = _scope()
        ticket = await outbox.enqueue(
            scope=scope,
            frozen_turn=FrozenTurn(
                scope=scope,
                event_id="40" * 16,
                turn_id="50" * 16,
                base_revision=7,
                observed_at_ms=1,
            ),
            incarnation_id="AE-I1-FAKE",
            protocol_digest="60" * 32,
            request_text="current turn only",
            context_summary=_summary(),
            provider_binding=AuxiliaryProviderBindingV1(
                provider_id="aux-provider",
                source="CONFIGURED",
                request_key="opaque-request-key",
            ),
        )
        await asyncio.wait_for(transport.started.wait(), timeout=1)

        deferred = await outbox.wait_foreground(ticket)

        assert {
            key: deferred[key]
            for key in ("status", "code", "expression_state", "transport_subcode")
        } == {
            "status": "DEFERRED",
            "code": "DEFERRED_ASYNC",
            "expression_state": "DEFERRED",
            "transport_subcode": "PROVIDER_CALL_IN_PROGRESS",
        }
        assert bridge.apply_count == 0

        transport.release.set()
        committed = await asyncio.wait_for(outbox.wait_completion(ticket), timeout=2)

        assert committed["code"] == "SEMANTIC_COMMITTED"
        assert bridge.apply_count == 1
        database_path = tmp_path / "semantic-async" / "semantic_jobs_v1.sqlite3"
        with sqlite3.connect(database_path) as connection:
            encrypted_payload = connection.execute(
                "SELECT encrypted_payload FROM semantic_jobs_v1"
            ).fetchone()[0]
        assert encrypted_payload is None
        await outbox.close()
        assert all(
            b"current turn only" not in artifact.read_bytes()
            for artifact in (
                database_path,
                database_path.with_name(database_path.name + "-wal"),
            )
            if artifact.exists()
        )

    asyncio.run(run())


def test_coordinator_enqueues_before_the_foreground_wait() -> None:
    class Bridge:
        def semantic_revision_v1(self, _scope: ScopeTokens) -> dict[str, object]:
            return {
                "schema": "astrembodiment.semantic-revision.v1",
                "revision": 7,
            }

    class DeferredOutbox:
        def __init__(self) -> None:
            self.enqueue_calls: list[dict[str, object]] = []

        async def enqueue(self, **kwargs: object) -> object:
            self.enqueue_calls.append(kwargs)
            return object()

        async def wait_foreground(self, _ticket: object) -> dict[str, object]:
            return {
                "status": "DEFERRED",
                "code": "DEFERRED_ASYNC",
                "expression_state": "DEFERRED",
                "transport_subcode": "PROVIDER_CALL_IN_PROGRESS",
                "attempted": True,
                "attempt_count": 1,
            }

    async def run() -> tuple[dict[str, object], DeferredOutbox]:
        outbox = DeferredOutbox()
        coordinator = GenesisCoordinator(Bridge(), semantic_outbox=outbox)
        scope = _scope()
        outcome = await coordinator.preflight_semantic_v3(
            scope=scope,
            frozen_turn=FrozenTurn(
                scope=scope,
                event_id="40" * 16,
                turn_id="50" * 16,
                base_revision=0,
                observed_at_ms=1,
            ),
            incarnation_id="AE-I1-FAKE",
            provider_binding=AuxiliaryProviderBindingV1(
                provider_id="aux-provider",
                source="CONFIGURED",
                request_key="opaque-request-key",
            ),
            request_text="current turn only",
            context_summary=_summary(),
            estimator=lambda _request: (_ for _ in ()).throw(
                AssertionError("direct call")
            ),
        )
        return outcome, outbox

    outcome, outbox = asyncio.run(run())

    assert outcome["code"] == "DEFERRED_ASYNC"
    assert len(outbox.enqueue_calls) == 1


def test_restart_staged_replay_and_late_lease_epoch_skip_native(tmp_path: Path) -> None:
    async def run() -> None:
        bridge = _StrictBridgeFake()
        scope = _scope()
        turn = FrozenTurn(
            scope=scope,
            event_id="41" * 16,
            turn_id="51" * 16,
            base_revision=7,
            observed_at_ms=1,
        )
        bootstrap = SemanticOutbox(
            runtime_data_dir=tmp_path / "replay",
            bridge=bridge,
            transport=object(),
        )
        assert await bootstrap.start() is True
        database_path = bootstrap.database_path
        await bootstrap.close()

        proposal_json, dimensions_json, confidence = _proposal_bytes(scope, turn)
        job_id = _seed_job(
            database_path,
            scope=scope,
            turn=turn,
            incarnation_id="AE-I1-FAKE",
            state="RESULT_STAGED",
            encrypted_payload=None,
            deadline_at_ms=int(time.time() * 1_000) + 60_000,
            proposal_json=proposal_json,
            dimensions_json=dimensions_json,
            confidence_fxp6=confidence,
        )
        replay = SemanticOutbox(
            runtime_data_dir=tmp_path / "replay",
            bridge=bridge,
            transport=object(),
        )
        assert await replay.start() is True
        result = await asyncio.wait_for(
            replay.wait_completion(SemanticJobTicket(job_id)), timeout=2
        )
        assert result["code"] == "SEMANTIC_COMMITTED"
        assert bridge.apply_count == 1
        await replay.close()

        late = SemanticOutbox(
            runtime_data_dir=tmp_path / "late",
            bridge=bridge,
            transport=object(),
        )
        assert await late.start() is True
        late_path = late.database_path
        await late.close()
        late_job_id = _seed_job(
            late_path,
            scope=scope,
            turn=FrozenTurn(
                scope=scope,
                event_id="42" * 16,
                turn_id="52" * 16,
                base_revision=7,
                observed_at_ms=1,
            ),
            incarnation_id="AE-I1-FAKE",
            state="RUNNING_PROVIDER",
            encrypted_payload=hashlib.sha256(b"sealed").digest(),
            deadline_at_ms=int(time.time() * 1_000) + 60_000,
        )
        late = SemanticOutbox(
            runtime_data_dir=tmp_path / "late",
            bridge=bridge,
            transport=object(),
        )
        assert await late.start() is True
        old_row = late._row_for_job(late_job_id)
        assert old_row is not None
        late._require_connection().execute(
            "UPDATE semantic_jobs_v1 SET lease_epoch = 2 WHERE job_id = ?",
            (late_job_id,),
        )
        before_apply = bridge.apply_count
        assert (
            late._stage_result(
                old_row,
                proposal_json=proposal_json,
                proposal_digest=hashlib.sha256(proposal_json).digest(),
                dimensions_json=dimensions_json,
                confidence=confidence,
                transport_meta=AuxiliaryTransportMetaV1("NONE", True, 1),
                elapsed_ms=1,
                attempt_budget_ms=5_000,
            )
            is False
        )
        assert bridge.apply_count == before_apply
        await late.close()

    asyncio.run(run())


def test_per_bot_fifo_blocks_second_provider_until_first_finishes(
    tmp_path: Path,
) -> None:
    class FifoTransport:
        def __init__(self) -> None:
            self.first_started = asyncio.Event()
            self.second_started = asyncio.Event()
            self.release_first = asyncio.Event()
            self.release_second = asyncio.Event()

        async def generate_bound_once(
            self,
            *,
            binding: AuxiliaryProviderBindingV1,
            prompt: str,
            system_prompt: str,
            attempt_count: int,
            timeout_ms: int,
        ) -> AuxiliaryTransportResultV1:
            assert binding.provider_id == "aux-provider"
            assert attempt_count == 1
            assert timeout_ms >= 5_000
            assert "Evaluate only current_turn_text" in system_prompt
            if prompt == "first":
                self.first_started.set()
                await self.release_first.wait()
            else:
                assert prompt == "second"
                self.second_started.set()
                await self.release_second.wait()
            return AuxiliaryTransportResultV1(
                text=_estimator_text(),
                meta=AuxiliaryTransportMetaV1("NONE", True, 1),
            )

    async def run() -> None:
        bridge = _StrictBridgeFake()
        transport = FifoTransport()
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path,
            bridge=bridge,
            transport=transport,
            config=SemanticOutboxConfig(worker_concurrency=2),
        )
        assert await outbox.start() is True
        scope = _scope()
        first = await _enqueue_job(
            outbox,
            scope=scope,
            event_id="43" * 16,
            turn_id="53" * 16,
            request_text="first",
        )
        await asyncio.wait_for(transport.first_started.wait(), timeout=1)
        second = await _enqueue_job(
            outbox,
            scope=scope,
            event_id="44" * 16,
            turn_id="54" * 16,
            request_text="second",
        )
        await asyncio.sleep(0.15)
        assert transport.second_started.is_set() is False

        transport.release_first.set()
        await asyncio.wait_for(transport.second_started.wait(), timeout=1)
        transport.release_second.set()
        assert (await asyncio.wait_for(outbox.wait_completion(first), timeout=2))[
            "code"
        ] == ("SEMANTIC_COMMITTED")
        assert (await asyncio.wait_for(outbox.wait_completion(second), timeout=2))[
            "code"
        ] == ("SEMANTIC_COMMITTED")
        assert bridge.apply_count == 2
        await outbox.close()

    asyncio.run(run())


def test_ttl_and_rebirth_scrub_payload_without_native_apply(tmp_path: Path) -> None:
    async def run() -> None:
        bridge = _StrictBridgeFake()
        scope = _scope()
        turn = FrozenTurn(
            scope=scope,
            event_id="45" * 16,
            turn_id="55" * 16,
            base_revision=7,
            observed_at_ms=1,
        )
        bootstrap = SemanticOutbox(
            runtime_data_dir=tmp_path / "ttl",
            bridge=bridge,
            transport=object(),
        )
        assert await bootstrap.start() is True
        database_path = bootstrap.database_path
        await bootstrap.close()
        expired_job_id = _seed_job(
            database_path,
            scope=scope,
            turn=turn,
            incarnation_id="AE-I1-FAKE",
            state="PENDING",
            encrypted_payload=hashlib.sha256(b"sealed").digest(),
            deadline_at_ms=int(time.time() * 1_000) - 1,
        )
        expired = SemanticOutbox(
            runtime_data_dir=tmp_path / "ttl",
            bridge=bridge,
            transport=object(),
        )
        assert await expired.start() is True
        outcome = await asyncio.wait_for(
            expired.wait_completion(SemanticJobTicket(expired_job_id)), timeout=1
        )
        assert outcome["code"] == "EXPIRED"
        with sqlite3.connect(database_path) as connection:
            assert (
                connection.execute(
                    "SELECT encrypted_payload FROM semantic_jobs_v1 WHERE job_id = ?",
                    (expired_job_id,),
                ).fetchone()[0]
                is None
            )
        await expired.close()

        transport = _GatedTransportFake()
        rebirth = SemanticOutbox(
            runtime_data_dir=tmp_path / "rebirth",
            bridge=bridge,
            transport=transport,
        )
        assert await rebirth.start() is True
        ticket = await _enqueue_job(
            rebirth,
            scope=scope,
            event_id="46" * 16,
            turn_id="56" * 16,
        )
        await asyncio.wait_for(transport.started.wait(), timeout=1)
        assert (
            await rebirth.cancel_rebirth(
                scope=scope,
                old_incarnation_id="AE-I1-FAKE",
            )
            == 1
        )
        assert (await rebirth.wait_completion(ticket))["code"] == "CANCELLED_REBIRTH"
        with sqlite3.connect(rebirth.database_path) as connection:
            assert (
                connection.execute(
                    "SELECT encrypted_payload FROM semantic_jobs_v1 WHERE job_id = ?",
                    (ticket.job_id,),
                ).fetchone()[0]
                is None
            )
        transport.release.set()
        await asyncio.sleep(0.05)
        assert bridge.apply_count == 0
        await rebirth.close()

    asyncio.run(run())


def test_key_and_auth_failure_fail_closed_without_native_apply(tmp_path: Path) -> None:
    class UnavailableBridge:
        def semantic_outbox_crypto_status_v1(self) -> dict[str, object]:
            return {
                "schema": "astrembodiment.semantic-outbox-crypto-status.v1",
                "status": "UNAVAILABLE",
                "key_version": 1,
            }

    class AuthFailBridge(_StrictBridgeFake):
        def semantic_outbox_open_v1(
            self, aad_b64: str, envelope_b64: str
        ) -> dict[str, object]:
            raise _ClosedCryptoError("ASYNC_PAYLOAD_AUTH_FAILED")

    async def run() -> None:
        unavailable = SemanticOutbox(
            runtime_data_dir=tmp_path / "key-unavailable",
            bridge=UnavailableBridge(),
            transport=object(),
        )
        assert await unavailable.start() is False
        assert unavailable.disabled_code == "ASYNC_KEY_UNAVAILABLE"

        bridge = AuthFailBridge()
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path / "auth-failure",
            bridge=bridge,
            transport=object(),
        )
        assert await outbox.start() is True
        ticket = await _enqueue_job(
            outbox,
            scope=_scope(),
            event_id="47" * 16,
            turn_id="57" * 16,
        )
        outcome = await asyncio.wait_for(outbox.wait_completion(ticket), timeout=1)
        assert outcome["code"] == "ASYNC_PAYLOAD_AUTH_FAILED"
        assert bridge.apply_count == 0
        with sqlite3.connect(outbox.database_path) as connection:
            assert (
                connection.execute(
                    "SELECT encrypted_payload FROM semantic_jobs_v1 WHERE job_id = ?",
                    (ticket.job_id,),
                ).fetchone()[0]
                is None
            )
        await outbox.close()

    asyncio.run(run())


def test_terminal_waiter_is_released_and_late_join_reads_durable_outcome(
    tmp_path: Path,
) -> None:
    async def run() -> None:
        bridge = _StrictBridgeFake()
        transport = _GatedTransportFake()
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path / "completion-release",
            bridge=bridge,
            transport=transport,
        )
        assert await outbox.start() is True
        try:
            ticket = await _enqueue_job(
                outbox,
                scope=_scope(),
                event_id="48" * 16,
                turn_id="58" * 16,
            )
            waiting = asyncio.create_task(outbox.wait_completion(ticket))
            await asyncio.wait_for(transport.started.wait(), timeout=1)
            transport.release.set()
            assert (await asyncio.wait_for(waiting, timeout=1))["code"] == (
                "SEMANTIC_COMMITTED"
            )

            # Terminal results stay durable in SQLite, not indefinitely in RAM.
            assert ticket.job_id not in outbox._completions
            assert (await asyncio.wait_for(outbox.wait_completion(ticket), timeout=1))[
                "code"
            ] == "SEMANTIC_COMMITTED"
        finally:
            await outbox.close()

    asyncio.run(run())


def test_idle_worker_periodically_reclaims_retention_expired_terminal_rows(
    tmp_path: Path,
) -> None:
    async def run() -> None:
        bridge = _StrictBridgeFake()
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path / "periodic-terminal-gc",
            bridge=bridge,
            transport=object(),
        )
        assert await outbox.start() is True
        try:
            scope = _scope()
            turn = FrozenTurn(
                scope=scope,
                event_id="49" * 16,
                turn_id="59" * 16,
                base_revision=7,
                observed_at_ms=1,
            )
            job_id = _seed_job(
                outbox.database_path,
                scope=scope,
                turn=turn,
                incarnation_id="AE-I1-FAKE",
                state="FAILED_TERMINAL",
                encrypted_payload=None,
                deadline_at_ms=int(time.time() * 1_000) + 60_000,
            )
            with sqlite3.connect(outbox.database_path) as connection:
                connection.execute(
                    """
                    UPDATE semantic_jobs_v1
                    SET terminal_code = 'ESTIMATOR_UNAVAILABLE',
                        completed_at_ms = ?
                    WHERE job_id = ?
                    """,
                    (int(time.time() * 1_000) - 24 * 60 * 60 * 1_000 - 1, job_id),
                )

            # The row arrives after startup; an idle worker must still collect it.
            outbox._next_maintenance_at_ms = 0
            for _ in range(20):
                await asyncio.sleep(0.05)
                with sqlite3.connect(outbox.database_path) as connection:
                    remaining = connection.execute(
                        "SELECT COUNT(*) FROM semantic_jobs_v1 WHERE job_id = ?",
                        (job_id,),
                    ).fetchone()[0]
                if remaining == 0:
                    break
            assert remaining == 0
        finally:
            await outbox.close()

    asyncio.run(run())


def test_legacy_identity_row_is_closed_before_reborn_sequence_restarts(
    tmp_path: Path,
) -> None:
    class RebornBridge(_StrictBridgeFake):
        def inspect(self, _scope: dict[str, object]) -> dict[str, object]:
            return {
                "bound": True,
                "revision": 7,
                "incarnation_id": "AE-I2-NEW",
            }

    async def run() -> None:
        bridge = RebornBridge()
        scope = _scope()
        turn = FrozenTurn(
            scope=scope,
            event_id="4a" * 16,
            turn_id="5a" * 16,
            base_revision=7,
            observed_at_ms=1,
        )
        bootstrap = SemanticOutbox(
            runtime_data_dir=tmp_path / "legacy-identity",
            bridge=bridge,
            transport=object(),
        )
        assert await bootstrap.start() is True
        database_path = bootstrap.database_path
        await bootstrap.close()

        legacy_job_id = _legacy_job_identity_v1(
            tokens=SemanticOutbox._row_tokens(scope, turn),
            protocol_digest=bytes.fromhex(ESTIMATOR_FORMULA_DIGEST),
        )
        _seed_job(
            database_path,
            scope=scope,
            turn=turn,
            incarnation_id="AE-I1-OLD",
            state="RUNNING_PROVIDER",
            encrypted_payload=hashlib.sha256(b"legacy-sealed").digest(),
            deadline_at_ms=int(time.time() * 1_000) + 60_000,
            forced_job_id=legacy_job_id,
        )

        transport = _GatedTransportFake()
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path / "legacy-identity",
            bridge=bridge,
            transport=transport,
        )
        assert await outbox.start() is True
        try:
            with sqlite3.connect(database_path) as connection:
                legacy = connection.execute(
                    "SELECT state, encrypted_payload FROM semantic_jobs_v1 WHERE job_id = ?",
                    (legacy_job_id,),
                ).fetchone()
            assert legacy is None or (
                legacy[0] == "FAILED_TERMINAL" and legacy[1] is None
            )

            ticket = await _enqueue_job(
                outbox,
                scope=scope,
                event_id="4a" * 16,
                turn_id="5a" * 16,
                incarnation_id="AE-I2-NEW",
            )
            assert ticket.job_id != legacy_job_id
            await asyncio.wait_for(transport.started.wait(), timeout=1)
            transport.release.set()
            assert (await asyncio.wait_for(outbox.wait_completion(ticket), timeout=1))[
                "code"
            ] == ("SEMANTIC_COMMITTED")
        finally:
            await outbox.close()

    asyncio.run(run())


def test_live_expired_second_attempt_is_terminalized_and_wakes_waiter(
    tmp_path: Path,
) -> None:
    async def run() -> None:
        outbox = SemanticOutbox(
            runtime_data_dir=tmp_path / "live-lease-exhausted",
            bridge=_StrictBridgeFake(),
            transport=object(),
        )
        assert await outbox.start() is True
        try:
            scope = _scope()
            turn = FrozenTurn(
                scope=scope,
                event_id="4b" * 16,
                turn_id="5b" * 16,
                base_revision=7,
                observed_at_ms=1,
            )
            job_id = _seed_job(
                outbox.database_path,
                scope=scope,
                turn=turn,
                incarnation_id="AE-I1-FAKE",
                state="RUNNING_PROVIDER",
                encrypted_payload=hashlib.sha256(b"sealed").digest(),
                deadline_at_ms=int(time.time() * 1_000) + 60_000,
            )
            ticket = SemanticJobTicket(job_id)
            waiting = asyncio.create_task(outbox.wait_completion(ticket))
            await asyncio.sleep(0)
            assert job_id in outbox._completions
            with sqlite3.connect(outbox.database_path) as connection:
                connection.execute(
                    """
                    UPDATE semantic_jobs_v1
                    SET attempt_count = 2, lease_expires_at_ms = ?
                    WHERE job_id = ?
                    """,
                    (int(time.time() * 1_000) - 1, job_id),
                )
            outbox._wake.set()

            outcome = await asyncio.wait_for(waiting, timeout=1)
            assert outcome["code"] == "ESTIMATOR_UNAVAILABLE"
            with sqlite3.connect(outbox.database_path) as connection:
                row = connection.execute(
                    """
                    SELECT state, terminal_code, encrypted_payload
                    FROM semantic_jobs_v1 WHERE job_id = ?
                    """,
                    (job_id,),
                ).fetchone()
            assert row == ("FAILED_TERMINAL", "ESTIMATOR_UNAVAILABLE", None)
        finally:
            await outbox.close()

    asyncio.run(run())
