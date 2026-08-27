"""Durable, native-sealed asynchronous semantic jobs.

This module owns only queue lifecycle.  It never derives cryptographic
material, changes a semantic base revision, or synthesizes a semantic vector.
The native bridge remains the only authority for seal/open, state CAS, native
deduplication, and the expression projection returned by a committed receipt.
"""

from __future__ import annotations

import asyncio
import base64
import hashlib
import json
import logging
import math
import sqlite3
import time
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .auxiliary_transport import (
    TRANSPORT_SUBCODES,
    AuxiliaryProviderBindingV1,
    AuxiliaryTransportError,
    AuxiliaryTransportMetaV1,
    AuxiliaryTransportResultV1,
    not_applicable_transport_meta,
    normalized_transport_meta,
)
from .bridge import validate_context_summary_payload
from .context_binding import ContextBindingV1, adapt_native_context_summary_v1
from .contracts import FrozenTurn, ScopeTokens
from .semantic_estimator import (
    ESTIMATOR_FORMULA_DIGEST,
    SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA,
    SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
    SemanticEstimateError,
    SemanticProposalError,
    build_perception_proposal_v3,
    make_request_nonce_digest,
    parse_estimator_output_v3,
    proposal_to_json,
    validate_perception_proposal,
)


logger = logging.getLogger("astrbot_plugin_astrembodiment.semantic_outbox")

_DATABASE_NAME = "semantic_jobs_v1.sqlite3"
_PAYLOAD_SCHEMA = "astrembodiment.semantic-outbox-payload.v1"
_AAD_SCHEMA = "astrembodiment.semantic-outbox-aad.v1"
_CRYPTO_STATUS_SCHEMA = "astrembodiment.semantic-outbox-crypto-status.v1"
_SEALED_SCHEMA = "astrembodiment.semantic-outbox-sealed.v1"
_OPENED_SCHEMA = "astrembodiment.semantic-outbox-opened.v1"
_PROTOCOL_SCHEMA = "astrembodiment.semantic-outbox.v1"
_MAX_PAYLOAD_BYTES = 262_144
_TERMINAL_RETENTION_MS = 86_400_000
_TERMINAL_GC_INTERVAL_MS = 60_000
_TERMINAL_GC_BATCH_SIZE = 128

_PENDING_STATES = frozenset({"PENDING", "PENDING_RETRY"})
_ACTIVE_STATES = frozenset({"RUNNING_PROVIDER", "COMMITTING_NATIVE"})
_RECOVERABLE_STAGED_STATES = frozenset({"RESULT_STAGED", "COMMITTING_NATIVE"})
_TERMINAL_STATES = frozenset(
    {
        "COMMITTED",
        "EXPIRED",
        "FAILED_TERMINAL",
        "SUPERSEDED_STALE_REVISION",
        "CANCELLED_REBIRTH",
    }
)
_ALL_STATES = frozenset(
    {
        "NEW",
        *_PENDING_STATES,
        "RUNNING_PROVIDER",
        "RESULT_STAGED",
        "COMMITTING_NATIVE",
        *_TERMINAL_STATES,
    }
)
_TERMINAL_CODES = frozenset(
    {
        "COMMITTED",
        "EXPIRED",
        "CANCELLED_REBIRTH",
        "SUPERSEDED_STALE_REVISION",
        "ASYNC_KEY_UNAVAILABLE",
        "ASYNC_KEY_VERSION_UNSUPPORTED",
        "ASYNC_PAYLOAD_AUTH_FAILED",
        "ASYNC_QUEUE_UNAVAILABLE",
        "ESTIMATOR_UNAVAILABLE",
        "ESTIMATOR_MALFORMED",
        "SEMANTIC_VECTOR_UNAVAILABLE",
        "ESTIMATOR_UNCERTAIN",
        "INVALID_PERCEPTION_PROPOSAL",
        "NATIVE_MALFORMED",
        "NATIVE_ERROR",
        "NATIVE_SYMBOL_UNAVAILABLE",
        "TOTAL_BUDGET_EXHAUSTED",
        "SHUTDOWN",
    }
)
_RETRYABLE_SUBCODES = frozenset({"PROVIDER_CALL_TIMEOUT", "PROVIDER_CALL_FAILED"})
_CLOSED_CRYPTO_CODES = frozenset(
    {
        "ASYNC_KEY_UNAVAILABLE",
        "ASYNC_PAYLOAD_AUTH_FAILED",
        "ASYNC_KEY_VERSION_UNSUPPORTED",
    }
)
_BINDING_SOURCES = frozenset({"CONFIGURED", "LEGACY_COMPAT", "CURRENT_SESSION"})


class SemanticOutboxUnavailable(RuntimeError):
    """Closed async-lane availability failure that never carries native text."""

    def __init__(self, code: str) -> None:
        self.code = code if code in _TERMINAL_CODES else "ASYNC_QUEUE_UNAVAILABLE"
        super().__init__(self.code)


@dataclass(frozen=True, slots=True)
class SemanticOutboxConfig:
    """Validated queue budgets; defaults are the frozen product values."""

    sync_wait_ms: int = 2_000
    job_ttl_ms: int = 600_000
    total_budget_ms: int = 150_000
    provider_attempt_cap_ms: int = 90_000
    worker_concurrency: int = 2
    lease_ms: int = 30_000

    def __post_init__(self) -> None:
        _require_int_range(self.sync_wait_ms, 250, 5_000)
        _require_int_range(self.job_ttl_ms, 60_000, 3_600_000)
        _require_int_range(self.total_budget_ms, 10_000, 300_000)
        _require_int_range(self.provider_attempt_cap_ms, 5_000, 120_000)
        _require_int_range(self.worker_concurrency, 1, 4)
        _require_int_range(self.lease_ms, 10_000, 60_000)


@dataclass(frozen=True, slots=True, repr=False)
class SemanticJobTicket:
    """Opaque local completion handle; the durable identity is never logged."""

    job_id: bytes


@dataclass(slots=True)
class _LatencySamples:
    values: list[int]
    ewma_ms: float | None = None

    def predict_ms(self) -> int:
        if len(self.values) < 5:
            return 60_000
        ordered = sorted(self.values)
        rank = max(1, math.ceil(len(ordered) * 0.95))
        p95 = ordered[rank - 1]
        if self.ewma_ms is None:
            return p95
        return max(p95, math.ceil(self.ewma_ms))

    def observe(self, elapsed_ms: int) -> None:
        if elapsed_ms <= 0:
            return
        self.values.append(elapsed_ms)
        if len(self.values) > 32:
            del self.values[:-32]
        self.ewma_ms = (
            float(elapsed_ms)
            if self.ewma_ms is None
            else (0.20 * float(elapsed_ms)) + (0.80 * self.ewma_ms)
        )


def _require_int_range(value: object, minimum: int, maximum: int) -> None:
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError("semantic outbox configuration")


def _now_ms() -> int:
    return int(time.time() * 1_000)


def _canonical_json_bytes(value: Mapping[str, Any]) -> bytes:
    try:
        return json.dumps(
            value,
            ensure_ascii=False,
            sort_keys=True,
            separators=(",", ":"),
            allow_nan=False,
        ).encode("utf-8")
    except (TypeError, ValueError):
        raise ValueError("closed semantic outbox payload") from None


def _canonical_json_text(value: Mapping[str, Any]) -> str:
    return _canonical_json_bytes(value).decode("utf-8")


def _b64encode(value: bytes) -> str:
    return base64.b64encode(value).decode("ascii")


def _b64decode(value: object) -> bytes:
    if type(value) is not str or not value:
        raise ValueError("canonical base64")
    try:
        decoded = base64.b64decode(value.encode("ascii"), validate=True)
    except (UnicodeEncodeError, ValueError):
        raise ValueError("canonical base64") from None
    if _b64encode(decoded) != value:
        raise ValueError("canonical base64")
    return decoded


def _hex_blob(value: object, *, byte_length: int, allow_empty: bool = False) -> bytes:
    if allow_empty and value is None:
        return b""
    if type(value) is not str or len(value) != byte_length * 2:
        raise ValueError("opaque token")
    try:
        decoded = bytes.fromhex(value)
    except ValueError:
        raise ValueError("opaque token") from None
    if len(decoded) != byte_length or not any(decoded):
        raise ValueError("opaque token")
    return decoded


def _blob_hex(
    value: object, *, byte_length: int, allow_empty: bool = False
) -> str | None:
    if type(value) is not bytes:
        raise ValueError("opaque token")
    if allow_empty and value == b"":
        return None
    if len(value) != byte_length or not any(value):
        raise ValueError("opaque token")
    return value.hex()


def _closed_crypto_code(error: BaseException) -> str:
    code = getattr(error, "code", None)
    if type(code) is str and code in _CLOSED_CRYPTO_CODES:
        return code
    return "ASYNC_KEY_UNAVAILABLE"


def _closed_native_code(value: object) -> str:
    if type(value) is str and value in _TERMINAL_CODES:
        return value
    return "NATIVE_ERROR"


class SemanticOutbox:
    """SQLite queue with native-sealed payloads and finite async workers."""

    def __init__(
        self,
        *,
        runtime_data_dir: str | Path,
        bridge: Any,
        transport: Any,
        config: SemanticOutboxConfig | None = None,
    ) -> None:
        self._runtime_data_dir = Path(runtime_data_dir)
        self._database_path = self._runtime_data_dir / "semantic-async" / _DATABASE_NAME
        self._bridge = bridge
        self._transport = transport
        self._config = config or SemanticOutboxConfig()
        self._connection: sqlite3.Connection | None = None
        self._workers: list[asyncio.Task[None]] = []
        self._wake = asyncio.Event()
        self._stopping = False
        self._ready = False
        self._disabled_code: str | None = None
        self._completions: dict[bytes, asyncio.Future[dict[str, Any]]] = {}
        self._latency: dict[bytes, _LatencySamples] = {}
        self._next_maintenance_at_ms = 0

    @property
    def database_path(self) -> Path:
        return self._database_path

    @property
    def ready(self) -> bool:
        return self._ready

    @property
    def disabled_code(self) -> str | None:
        return self._disabled_code

    async def start(self) -> bool:
        """Verify native key authority, open SQLite, recover, then start workers."""

        if self._ready:
            return True
        if self._stopping:
            self._disable("SHUTDOWN")
            return False
        try:
            key_version = self._verify_crypto_status()
            self._database_path.parent.mkdir(parents=True, exist_ok=True)
            connection = sqlite3.connect(
                self._database_path,
                isolation_level=None,
                check_same_thread=False,
            )
            connection.row_factory = sqlite3.Row
            self._connection = connection
            self._initialize_schema()
            self._recover_startup_rows()
            self._key_version = key_version
            self._next_maintenance_at_ms = _now_ms() + _TERMINAL_GC_INTERVAL_MS
        except SemanticOutboxUnavailable as exc:
            self._close_connection()
            self._disable(exc.code)
            return False
        except (OSError, sqlite3.Error, ValueError):
            self._close_connection()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return False

        self._ready = True
        for worker_index in range(self._config.worker_concurrency):
            self._workers.append(
                asyncio.create_task(
                    self._worker_loop(worker_index),
                    name=f"astr-embodiment-semantic-outbox-{worker_index}",
                )
            )
        self._wake.set()
        return True

    async def close(self) -> None:
        """Stop claiming promptly; unfinished lease owners are restart-recoverable."""

        if self._stopping and self._connection is None:
            return
        self._stopping = True
        self._wake.set()
        workers = tuple(self._workers)
        if workers:
            _done, pending = await asyncio.wait(workers, timeout=0.5)
            for task in pending:
                task.cancel()
            if pending:
                await asyncio.gather(*pending, return_exceptions=True)
        self._workers.clear()
        self._ready = False
        self._close_connection()
        for future in tuple(self._completions.values()):
            if not future.done():
                future.set_result(
                    self._degraded("SHUTDOWN", attempted=False, attempt_count=0)
                )
        self._completions.clear()
        logger.info("AstrEmbodiment semantic async queue stopped")

    def disable(self, code: str) -> None:
        """Fail open to chat while preventing new async claims."""

        self._disable(code)

    async def enqueue(
        self,
        *,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        incarnation_id: str,
        protocol_digest: str,
        request_text: str,
        context_summary: Mapping[str, Any],
        provider_binding: AuxiliaryProviderBindingV1,
    ) -> SemanticJobTicket:
        """Seal and persist first; no Provider call happens before this returns."""

        if not self._ready or self._stopping:
            raise SemanticOutboxUnavailable(
                self._disabled_code or "ASYNC_QUEUE_UNAVAILABLE"
            )
        if type(scope) is not ScopeTokens or type(frozen_turn) is not FrozenTurn:
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE")
        if frozen_turn.scope != scope or frozen_turn.base_revision < 0:
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE")
        if (
            type(incarnation_id) is not str
            or not incarnation_id
            or len(incarnation_id) > 128
        ):
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE")
        if type(request_text) is not str or not request_text:
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE")
        if type(provider_binding) is not AuxiliaryProviderBindingV1:
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE")

        try:
            canonical_summary = validate_context_summary_payload(dict(context_summary))
            protocol_blob = _hex_blob(protocol_digest, byte_length=32)
            tokens = self._row_tokens(scope, frozen_turn)
            job_id = self._job_identity(
                tokens=tokens,
                incarnation_id=incarnation_id,
                protocol_digest=protocol_blob,
            )
            payload = self._payload_for_enqueue(
                scope=scope,
                frozen_turn=frozen_turn,
                incarnation_id=incarnation_id,
                protocol_digest=protocol_blob,
                request_text=request_text,
                context_summary=canonical_summary,
                provider_binding=provider_binding,
            )
            plaintext = _canonical_json_bytes(payload)
            if len(plaintext) > _MAX_PAYLOAD_BYTES:
                raise ValueError("semantic payload too large")
            sealed = self._seal(
                job_id=job_id,
                protocol_digest=protocol_blob,
                incarnation_id=incarnation_id,
                base_revision=frozen_turn.base_revision,
                plaintext=plaintext,
            )
        except SemanticOutboxUnavailable:
            raise
        except BaseException:
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE") from None

        now = _now_ms()
        connection = self._require_connection()
        try:
            connection.execute("BEGIN IMMEDIATE")
            existing = connection.execute(
                "SELECT job_id FROM semantic_jobs_v1 WHERE job_id = ?", (job_id,)
            ).fetchone()
            if existing is None:
                connection.execute(
                    """
                    INSERT INTO semantic_jobs_v1 (
                        job_id, protocol_digest, key_version, state, bot_token, persona_token,
                        session_token, relation_token, event_token, turn_token,
                        incarnation_id, base_revision, attempt_count, lease_epoch,
                        lease_owner_digest, lease_expires_at_ms, created_at_ms,
                        deadline_at_ms, budget_expires_at_ms, updated_at_ms,
                        terminal_code, encrypted_payload, proposal_json,
                        proposal_digest, dimensions_json, confidence_fxp6,
                        native_receipt_json, transport_subcode, provider_elapsed_ms,
                        attempt_budget_ms, completed_at_ms
                    ) VALUES (?, ?, ?, 'PENDING', ?, ?, ?, ?, ?, ?, ?, ?, 0, 0,
                              NULL, NULL, ?, ?, ?, ?, NULL, ?, NULL, NULL,
                              NULL, NULL, NULL, 'NOT_APPLICABLE', NULL, NULL, NULL)
                    """,
                    (
                        job_id,
                        protocol_blob,
                        self._key_version,
                        *tokens,
                        incarnation_id,
                        frozen_turn.base_revision,
                        now,
                        now + self._config.job_ttl_ms,
                        now + self._config.total_budget_ms,
                        now,
                        sealed,
                    ),
                )
            connection.execute("COMMIT")
        except sqlite3.Error:
            self._rollback_quietly()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE") from None

        self._wake.set()
        return SemanticJobTicket(job_id=job_id)

    async def wait_foreground(self, ticket: SemanticJobTicket) -> dict[str, Any]:
        """Wait once behind ``shield``; a timeout never cancels the worker."""

        future, outcome = self._completion_waiter_or_terminal_outcome(ticket.job_id)
        if outcome is not None:
            return outcome
        assert future is not None
        try:
            return await asyncio.wait_for(
                asyncio.shield(future), timeout=self._config.sync_wait_ms / 1_000
            )
        except TimeoutError:
            attempt_count = self._attempt_count(ticket.job_id)
            return {
                "status": "DEFERRED",
                "code": "DEFERRED_ASYNC",
                "expression_state": "DEFERRED",
                "transport_subcode": "PROVIDER_CALL_IN_PROGRESS",
                "attempted": attempt_count > 0,
                "attempt_count": attempt_count,
                "timing": {"sync_wait_ms": self._config.sync_wait_ms},
            }

    async def wait_completion(self, ticket: SemanticJobTicket) -> dict[str, Any]:
        """Await the shared completion future without exposing job identity."""

        future, outcome = self._completion_waiter_or_terminal_outcome(ticket.job_id)
        if outcome is not None:
            return outcome
        assert future is not None
        return await asyncio.shield(future)

    async def cancel_rebirth(
        self,
        *,
        scope: ScopeTokens,
        old_incarnation_id: str | None,
    ) -> int:
        return self.cancel_rebirth_now(
            scope=scope, old_incarnation_id=old_incarnation_id
        )

    def cancel_rebirth_now(
        self,
        *,
        scope: ScopeTokens,
        old_incarnation_id: str | None,
    ) -> int:
        """Atomically stop old-incarnation work and scrub every pending payload."""

        if not self._ready:
            return 0
        try:
            bot = _hex_blob(scope.bot_token, byte_length=16)
            persona = _hex_blob(scope.persona_token, byte_length=16)
        except (AttributeError, ValueError):
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return 0
        if old_incarnation_id is not None and (
            type(old_incarnation_id) is not str or not old_incarnation_id
        ):
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return 0
        connection = self._require_connection()
        now = _now_ms()
        predicate = "bot_token = ? AND persona_token = ?"
        parameters: list[Any] = [bot, persona]
        if old_incarnation_id is not None:
            predicate += " AND incarnation_id = ?"
            parameters.append(old_incarnation_id)
        try:
            connection.execute("BEGIN IMMEDIATE")
            rows = connection.execute(
                f"""
                SELECT job_id FROM semantic_jobs_v1
                WHERE {predicate} AND state NOT IN ({_sql_literals(_TERMINAL_STATES)})
                """,
                tuple(parameters),
            ).fetchall()
            connection.execute(
                f"""
                UPDATE semantic_jobs_v1
                SET state = 'CANCELLED_REBIRTH', terminal_code = 'CANCELLED_REBIRTH',
                    encrypted_payload = NULL, lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?, completed_at_ms = ?
                WHERE {predicate} AND state NOT IN ({_sql_literals(_TERMINAL_STATES)})
                """,
                (now, now, *parameters),
            )
            connection.execute("COMMIT")
        except sqlite3.Error:
            self._rollback_quietly()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return 0
        for row in rows:
            self._resolve_completion(bytes(row["job_id"]))
        if rows:
            logger.info(
                "AstrEmbodiment semantic async terminal: code=CANCELLED_REBIRTH"
            )
        return len(rows)

    def _verify_crypto_status(self) -> int:
        try:
            result = self._bridge.semantic_outbox_crypto_status_v1()
        except BaseException as exc:
            raise SemanticOutboxUnavailable(_closed_crypto_code(exc)) from None
        if type(result) is not dict or set(result) != {
            "schema",
            "status",
            "key_version",
        }:
            raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE")
        if result["schema"] != _CRYPTO_STATUS_SCHEMA:
            raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE")
        key_version = result["key_version"]
        if type(key_version) is not int or key_version <= 0:
            raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE")
        status = result["status"]
        if status == "READY":
            return key_version
        if status == "KEY_VERSION_UNSUPPORTED":
            raise SemanticOutboxUnavailable("ASYNC_KEY_VERSION_UNSUPPORTED")
        raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE")

    def _initialize_schema(self) -> None:
        connection = self._require_connection()
        connection.execute("PRAGMA journal_mode=WAL")
        connection.execute("PRAGMA foreign_keys=ON")
        connection.execute("PRAGMA busy_timeout=5000")
        connection.execute("PRAGMA synchronous=NORMAL")
        connection.execute(
            f"""
            CREATE TABLE IF NOT EXISTS semantic_jobs_v1 (
                job_id BLOB PRIMARY KEY CHECK(length(job_id) = 32),
                protocol_digest BLOB NOT NULL CHECK(length(protocol_digest) = 32),
                key_version INTEGER NOT NULL CHECK(key_version > 0),
                state TEXT NOT NULL CHECK(state IN ({_sql_literals(_ALL_STATES)})),
                bot_token BLOB NOT NULL CHECK(length(bot_token) = 16),
                persona_token BLOB NOT NULL CHECK(length(persona_token) = 16),
                session_token BLOB NOT NULL CHECK(length(session_token) = 16),
                relation_token BLOB NOT NULL CHECK(length(relation_token) IN (0, 16)),
                event_token BLOB NOT NULL CHECK(length(event_token) = 16),
                turn_token BLOB NOT NULL CHECK(length(turn_token) = 16),
                incarnation_id TEXT NOT NULL CHECK(length(incarnation_id) BETWEEN 1 AND 128),
                base_revision INTEGER NOT NULL CHECK(base_revision >= 0),
                attempt_count INTEGER NOT NULL CHECK(attempt_count IN (0, 1, 2)),
                lease_epoch INTEGER NOT NULL CHECK(lease_epoch >= 0),
                lease_owner_digest BLOB CHECK(lease_owner_digest IS NULL OR length(lease_owner_digest) = 32),
                lease_expires_at_ms INTEGER,
                created_at_ms INTEGER NOT NULL CHECK(created_at_ms > 0),
                deadline_at_ms INTEGER NOT NULL CHECK(deadline_at_ms > 0),
                budget_expires_at_ms INTEGER NOT NULL CHECK(budget_expires_at_ms > 0),
                updated_at_ms INTEGER NOT NULL CHECK(updated_at_ms > 0),
                terminal_code TEXT CHECK(terminal_code IS NULL OR terminal_code IN ({_sql_literals(_TERMINAL_CODES)})),
                encrypted_payload BLOB,
                proposal_json BLOB,
                proposal_digest BLOB CHECK(proposal_digest IS NULL OR length(proposal_digest) = 32),
                dimensions_json BLOB,
                confidence_fxp6 INTEGER CHECK(confidence_fxp6 IS NULL OR (confidence_fxp6 BETWEEN 1 AND 1000000)),
                native_receipt_json BLOB,
                native_apply_started INTEGER NOT NULL DEFAULT 0
                    CHECK(native_apply_started IN (0, 1)),
                transport_subcode TEXT NOT NULL CHECK(transport_subcode IN ({_sql_literals(TRANSPORT_SUBCODES)})),
                provider_elapsed_ms INTEGER,
                attempt_budget_ms INTEGER,
                completed_at_ms INTEGER
            )
            """
        )
        expected_columns = {
            "job_id",
            "protocol_digest",
            "key_version",
            "state",
            "bot_token",
            "persona_token",
            "session_token",
            "relation_token",
            "event_token",
            "turn_token",
            "incarnation_id",
            "base_revision",
            "attempt_count",
            "lease_epoch",
            "lease_owner_digest",
            "lease_expires_at_ms",
            "created_at_ms",
            "deadline_at_ms",
            "budget_expires_at_ms",
            "updated_at_ms",
            "terminal_code",
            "encrypted_payload",
            "proposal_json",
            "proposal_digest",
            "dimensions_json",
            "confidence_fxp6",
            "native_receipt_json",
            "native_apply_started",
            "transport_subcode",
            "provider_elapsed_ms",
            "attempt_budget_ms",
            "completed_at_ms",
        }
        actual_columns = {
            str(column["name"])
            for column in connection.execute("PRAGMA table_info(semantic_jobs_v1)")
        }
        if actual_columns != expected_columns:
            # This is intentionally fail-closed rather than a best-effort
            # migration: a partially known row must never be opened under a
            # different AAD or reinterpreted as a live proposal.
            raise sqlite3.DatabaseError("semantic outbox schema")
        connection.execute(
            """
            CREATE UNIQUE INDEX IF NOT EXISTS semantic_jobs_v1_identity
            ON semantic_jobs_v1 (
                bot_token, persona_token, session_token, relation_token,
                event_token, turn_token, incarnation_id, protocol_digest
            )
            """
        )
        connection.execute(
            """
            CREATE INDEX IF NOT EXISTS semantic_jobs_v1_claim
            ON semantic_jobs_v1 (state, created_at_ms, job_id)
            """
        )
        self._close_legacy_identity_rows()

    def _close_legacy_identity_rows(self) -> None:
        """Scrub and close rows sealed under the pre-incarnation identity."""

        connection = self._require_connection()
        try:
            connection.execute("BEGIN IMMEDIATE")
            rows = connection.execute(
                """
                SELECT job_id, protocol_digest, bot_token, persona_token,
                       session_token, relation_token, event_token, turn_token,
                       incarnation_id
                FROM semantic_jobs_v1
                """
            ).fetchall()
            legacy_job_ids: list[bytes] = []
            for row in rows:
                try:
                    expected_job_id = self._job_identity(
                        tokens=(
                            bytes(row["bot_token"]),
                            bytes(row["persona_token"]),
                            bytes(row["session_token"]),
                            bytes(row["relation_token"]),
                            bytes(row["event_token"]),
                            bytes(row["turn_token"]),
                        ),
                        incarnation_id=row["incarnation_id"],
                        protocol_digest=bytes(row["protocol_digest"]),
                    )
                    job_id = bytes(row["job_id"])
                except (TypeError, ValueError):
                    job_id = row["job_id"]
                    legacy_job_ids.append(job_id)
                    continue
                if job_id != expected_job_id:
                    legacy_job_ids.append(job_id)
            if legacy_job_ids:
                connection.executemany(
                    """
                    UPDATE semantic_jobs_v1
                    SET encrypted_payload = NULL, lease_owner_digest = NULL,
                        lease_expires_at_ms = NULL
                    WHERE job_id = ?
                    """,
                    [(job_id,) for job_id in legacy_job_ids],
                )
                connection.executemany(
                    "DELETE FROM semantic_jobs_v1 WHERE job_id = ?",
                    [(job_id,) for job_id in legacy_job_ids],
                )
            connection.execute("COMMIT")
        except sqlite3.Error:
            self._rollback_quietly()
            raise
        if legacy_job_ids:
            logger.info(
                "AstrEmbodiment semantic async legacy identity rows closed: count=%s",
                len(legacy_job_ids),
            )

    def _recover_startup_rows(self) -> None:
        now = _now_ms()
        connection = self._require_connection()
        connection.execute("BEGIN IMMEDIATE")
        try:
            self._expire_due_locked(now)
            connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'PENDING_RETRY', lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?
                WHERE state = 'RUNNING_PROVIDER' AND lease_expires_at_ms <= ?
                  AND deadline_at_ms > ? AND attempt_count < 2
                """,
                (now, now, now),
            )
            connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'FAILED_TERMINAL', terminal_code = 'ESTIMATOR_UNAVAILABLE',
                    encrypted_payload = NULL, lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?, completed_at_ms = ?
                WHERE state = 'RUNNING_PROVIDER' AND lease_expires_at_ms <= ?
                  AND attempt_count >= 2
                """,
                (now, now, now),
            )
            connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'RESULT_STAGED', lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?
                WHERE state = 'COMMITTING_NATIVE' AND lease_expires_at_ms <= ?
                """,
                (now, now),
            )
            connection.execute(
                """
                DELETE FROM semantic_jobs_v1
                WHERE state IN ('COMMITTED', 'EXPIRED', 'FAILED_TERMINAL',
                                'SUPERSEDED_STALE_REVISION', 'CANCELLED_REBIRTH')
                  AND completed_at_ms IS NOT NULL AND completed_at_ms < ?
                """,
                (now - _TERMINAL_RETENTION_MS,),
            )
            connection.execute("COMMIT")
        except sqlite3.Error:
            self._rollback_quietly()
            raise

    async def _worker_loop(self, _worker_index: int) -> None:
        try:
            while not self._stopping:
                self._collect_terminal_rows_if_due()
                row = self._claim_next()
                if row is None:
                    self._wake.clear()
                    try:
                        await asyncio.wait_for(self._wake.wait(), timeout=0.100)
                    except TimeoutError:
                        pass
                    continue
                if str(row["state"]) == "RUNNING_PROVIDER":
                    await self._run_provider(row)
                elif str(row["state"]) == "COMMITTING_NATIVE":
                    await self._commit_staged(row)
        except asyncio.CancelledError:
            raise
        except (sqlite3.Error, OSError, ValueError):
            self._disable("ASYNC_QUEUE_UNAVAILABLE")

    def _collect_terminal_rows_if_due(self) -> None:
        """Bound terminal retention work so an always-on queue does not grow."""

        if not self._ready or self._stopping:
            return
        now = _now_ms()
        if now < self._next_maintenance_at_ms:
            return
        self._next_maintenance_at_ms = now + _TERMINAL_GC_INTERVAL_MS
        connection = self._require_connection()
        try:
            connection.execute("BEGIN IMMEDIATE")
            rows = connection.execute(
                f"""
                SELECT * FROM semantic_jobs_v1
                WHERE state IN ({_sql_literals(_TERMINAL_STATES)})
                  AND completed_at_ms IS NOT NULL AND completed_at_ms < ?
                ORDER BY completed_at_ms, job_id
                LIMIT ?
                """,
                (now - _TERMINAL_RETENTION_MS, _TERMINAL_GC_BATCH_SIZE),
            ).fetchall()
            if rows:
                connection.executemany(
                    "DELETE FROM semantic_jobs_v1 WHERE job_id = ?",
                    [(row["job_id"],) for row in rows],
                )
            connection.execute("COMMIT")
        except sqlite3.Error:
            self._rollback_quietly()
            raise
        for row in rows:
            self._resolve_completion_from_row(row)

    def _claim_next(self) -> sqlite3.Row | None:
        if not self._ready or self._stopping:
            return None
        connection = self._require_connection()
        now = _now_ms()
        expired_job_ids: list[bytes] = []
        lease_exhausted_job_ids: list[bytes] = []
        try:
            connection.execute("BEGIN IMMEDIATE")
            expired_job_ids = self._expire_due_locked(now)
            connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'PENDING_RETRY', lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?
                WHERE state = 'RUNNING_PROVIDER' AND lease_expires_at_ms <= ?
                  AND deadline_at_ms > ? AND attempt_count < 2
                """,
                (now, now, now),
            )
            lease_exhausted_job_ids = [
                bytes(row["job_id"])
                for row in connection.execute(
                    """
                    SELECT job_id FROM semantic_jobs_v1
                    WHERE state = 'RUNNING_PROVIDER' AND lease_expires_at_ms <= ?
                      AND deadline_at_ms > ? AND attempt_count >= 2
                    """,
                    (now, now),
                ).fetchall()
            ]
            connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'FAILED_TERMINAL', terminal_code = 'ESTIMATOR_UNAVAILABLE',
                    encrypted_payload = NULL, lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?, completed_at_ms = ?
                WHERE state = 'RUNNING_PROVIDER' AND lease_expires_at_ms <= ?
                  AND deadline_at_ms > ? AND attempt_count >= 2
                """,
                (now, now, now, now),
            )
            connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'RESULT_STAGED', lease_owner_digest = NULL,
                    lease_expires_at_ms = NULL, updated_at_ms = ?
                WHERE state = 'COMMITTING_NATIVE' AND lease_expires_at_ms <= ?
                """,
                (now, now),
            )
            candidates = connection.execute(
                """
                SELECT * FROM semantic_jobs_v1
                WHERE state IN ('RESULT_STAGED', 'PENDING', 'PENDING_RETRY')
                  AND deadline_at_ms > ?
                ORDER BY CASE state WHEN 'RESULT_STAGED' THEN 0 ELSE 1 END,
                         created_at_ms, job_id
                """,
                (now,),
            ).fetchall()
            selected: sqlite3.Row | None = None
            for candidate in candidates:
                if not self._is_bot_fifo_head_locked(candidate):
                    continue
                state = str(candidate["state"])
                if state in _PENDING_STATES and int(candidate["attempt_count"]) >= 2:
                    continue
                selected = candidate
                break
            if selected is None:
                connection.execute("COMMIT")
                for job_id in (*expired_job_ids, *lease_exhausted_job_ids):
                    self._resolve_completion(job_id)
                return None

            previous_epoch = int(selected["lease_epoch"])
            next_epoch = previous_epoch + 1
            owner = hashlib.sha256(
                b"astr-embodiment/semantic-outbox-lease-v1\x00"
                + selected["job_id"]
                + next_epoch.to_bytes(8, "big")
                + _now_ms().to_bytes(8, "big")
            ).digest()
            next_state = (
                "COMMITTING_NATIVE"
                if str(selected["state"]) == "RESULT_STAGED"
                else "RUNNING_PROVIDER"
            )
            attempt_count = int(selected["attempt_count"])
            if next_state == "RUNNING_PROVIDER":
                attempt_count += 1
            updated = connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = ?, attempt_count = ?, lease_epoch = ?,
                    lease_owner_digest = ?, lease_expires_at_ms = ?, updated_at_ms = ?
                WHERE job_id = ? AND state = ? AND lease_epoch = ?
                """,
                (
                    next_state,
                    attempt_count,
                    next_epoch,
                    owner,
                    now + self._config.lease_ms,
                    now,
                    selected["job_id"],
                    selected["state"],
                    previous_epoch,
                ),
            ).rowcount
            if updated != 1:
                connection.execute("COMMIT")
                return None
            claimed = connection.execute(
                "SELECT * FROM semantic_jobs_v1 WHERE job_id = ?",
                (selected["job_id"],),
            ).fetchone()
            connection.execute("COMMIT")
        except sqlite3.Error:
            self._rollback_quietly()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return None
        for job_id in (*expired_job_ids, *lease_exhausted_job_ids):
            self._resolve_completion(job_id)
        return claimed

    def _is_bot_fifo_head_locked(self, candidate: sqlite3.Row) -> bool:
        connection = self._require_connection()
        head = connection.execute(
            f"""
            SELECT job_id FROM semantic_jobs_v1
            WHERE bot_token = ? AND state NOT IN ({_sql_literals(_TERMINAL_STATES)})
            ORDER BY created_at_ms, job_id LIMIT 1
            """,
            (candidate["bot_token"],),
        ).fetchone()
        return head is not None and bytes(head["job_id"]) == bytes(candidate["job_id"])

    async def _run_provider(self, row: sqlite3.Row) -> None:
        job_id = bytes(row["job_id"])
        epoch = int(row["lease_epoch"])
        try:
            payload = self._open_row_payload(row)
        except SemanticOutboxUnavailable as exc:
            self._terminal_if_leased(row, exc.code)
            return
        except (TypeError, ValueError, json.JSONDecodeError):
            self._terminal_if_leased(row, "ASYNC_PAYLOAD_AUTH_FAILED")
            return

        try:
            scope, turn, binding, request_text, summary = self._payload_runtime_values(
                row, payload
            )
            nonce_digest = make_request_nonce_digest(scope, turn)
            adapted_summary = adapt_native_context_summary_v1(
                summary,
                scope=scope,
                nonce_digest=nonce_digest,
                estimator_formula_digest=ESTIMATOR_FORMULA_DIGEST,
            )
            ContextBindingV1.from_json(adapted_summary["binding"])
            system_prompt = self._provider_system_prompt()
            attempt_budget = self._attempt_budget_ms(row, binding.provider_id)
        except SemanticProposalError as exc:
            self._terminal_if_leased(row, exc.code)
            return
        except (TypeError, ValueError, KeyError):
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return

        if attempt_budget is None:
            self._terminal_if_leased(row, "TOTAL_BUDGET_EXHAUSTED")
            return

        started = _now_ms()
        try:
            result = await self._with_lease_heartbeat(
                row,
                self._transport.generate_bound_once(
                    binding=binding,
                    prompt=request_text,
                    system_prompt=system_prompt,
                    attempt_count=int(row["attempt_count"]),
                    timeout_ms=attempt_budget,
                ),
            )
        except AuxiliaryTransportError as exc:
            self._handle_provider_failure(
                row, exc.meta, attempt_budget, binding.provider_id
            )
            return
        except asyncio.CancelledError:
            raise
        except BaseException:
            meta = AuxiliaryTransportMetaV1(
                "PROVIDER_CALL_FAILED", True, int(row["attempt_count"])
            )
            self._handle_provider_failure(
                row, meta, attempt_budget, binding.provider_id
            )
            return

        if result is None:
            # A lost lease means this worker is stale.  It must never stage or
            # invoke native, even if the Host call eventually returned.
            return
        if type(result) is not AuxiliaryTransportResultV1:
            self._terminal_if_leased(row, "ESTIMATOR_MALFORMED")
            return
        if result.meta.attempt_count != int(row["attempt_count"]):
            self._terminal_if_leased(row, "ESTIMATOR_MALFORMED")
            return
        elapsed = max(1, _now_ms() - started)
        try:
            estimate = parse_estimator_output_v3(result.text)
            estimate = type(estimate)(
                dimensions=dict(estimate.dimensions),
                schema=estimate.schema,
                transport_meta=result.meta,
            )
            proposal = build_perception_proposal_v3(
                scope=scope,
                turn=turn,
                estimate=estimate,
                base_revision=int(row["base_revision"]),
                nonce_digest=nonce_digest,
            )
            proposal_json = proposal_to_json(proposal, scope=scope).encode("utf-8")
            dimensions_json = _canonical_json_bytes(
                {"dimensions": proposal["dimensions"]}
            )
        except SemanticEstimateError as exc:
            self._terminal_if_leased(row, exc.code)
            return
        except SemanticProposalError as exc:
            self._terminal_if_leased(row, exc.code)
            return
        except (TypeError, ValueError):
            self._terminal_if_leased(row, "ESTIMATOR_MALFORMED")
            return

        if not self._stage_result(
            row,
            proposal_json=proposal_json,
            proposal_digest=hashlib.sha256(proposal_json).digest(),
            dimensions_json=dimensions_json,
            confidence=int(proposal["estimator_confidence"]),
            transport_meta=result.meta,
            elapsed_ms=elapsed,
            attempt_budget_ms=attempt_budget,
        ):
            return
        self._latency_for(row, binding.provider_id).observe(elapsed)
        committing = self._begin_commit(job_id, epoch)
        if committing is not None:
            await self._commit_staged(committing)

    def _handle_provider_failure(
        self,
        row: sqlite3.Row,
        meta: AuxiliaryTransportMetaV1,
        attempt_budget_ms: int,
        provider_id: str,
    ) -> None:
        normalized = (
            meta
            if type(meta) is AuxiliaryTransportMetaV1
            else not_applicable_transport_meta()
        )
        now = _now_ms()
        remaining = int(row["budget_expires_at_ms"]) - now
        retry_required = self._retry_required_ms(row, provider_id)
        can_retry = (
            normalized.transport_subcode in _RETRYABLE_SUBCODES
            and int(row["attempt_count"]) < 2
            and remaining >= retry_required + 1_000
            and now < int(row["deadline_at_ms"])
        )
        if can_retry and self._retry_if_leased(row, normalized, attempt_budget_ms):
            self._wake.set()
            return
        self._terminal_if_leased(
            row,
            "ESTIMATOR_UNAVAILABLE",
            transport_meta=normalized,
            attempt_budget_ms=attempt_budget_ms,
        )
        logger.warning(
            "AstrEmbodiment semantic async provider failure: "
            "code=ESTIMATOR_UNAVAILABLE transport_subcode=%s attempt_count=%d",
            normalized.transport_subcode,
            normalized.attempt_count,
        )

    def _stage_result(
        self,
        row: sqlite3.Row,
        *,
        proposal_json: bytes,
        proposal_digest: bytes,
        dimensions_json: bytes,
        confidence: int,
        transport_meta: AuxiliaryTransportMetaV1,
        elapsed_ms: int,
        attempt_budget_ms: int,
    ) -> bool:
        connection = self._require_connection()
        now = _now_ms()
        try:
            connection.execute("BEGIN IMMEDIATE")
            updated = connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'RESULT_STAGED', encrypted_payload = NULL,
                    proposal_json = ?, proposal_digest = ?, dimensions_json = ?,
                    confidence_fxp6 = ?, transport_subcode = ?,
                    provider_elapsed_ms = ?, attempt_budget_ms = ?,
                    lease_expires_at_ms = ?, updated_at_ms = ?
                WHERE job_id = ? AND state = 'RUNNING_PROVIDER' AND lease_epoch = ?
                  AND lease_expires_at_ms > ?
                """,
                (
                    proposal_json,
                    proposal_digest,
                    dimensions_json,
                    confidence,
                    transport_meta.transport_subcode,
                    elapsed_ms,
                    attempt_budget_ms,
                    now + self._config.lease_ms,
                    now,
                    row["job_id"],
                    row["lease_epoch"],
                    now,
                ),
            ).rowcount
            connection.execute("COMMIT")
            return updated == 1
        except sqlite3.Error:
            self._rollback_quietly()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return False

    def _begin_commit(self, job_id: bytes, expected_epoch: int) -> sqlite3.Row | None:
        connection = self._require_connection()
        now = _now_ms()
        next_epoch = expected_epoch + 1
        owner = hashlib.sha256(
            b"astr-embodiment/semantic-outbox-commit-v1\x00"
            + job_id
            + next_epoch.to_bytes(8, "big")
        ).digest()
        try:
            connection.execute("BEGIN IMMEDIATE")
            updated = connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'COMMITTING_NATIVE', lease_epoch = ?,
                    lease_owner_digest = ?, lease_expires_at_ms = ?, updated_at_ms = ?
                WHERE job_id = ? AND state = 'RESULT_STAGED' AND lease_epoch = ?
                  AND lease_expires_at_ms > ?
                """,
                (
                    next_epoch,
                    owner,
                    now + self._config.lease_ms,
                    now,
                    job_id,
                    expected_epoch,
                    now,
                ),
            ).rowcount
            committed = (
                connection.execute(
                    "SELECT * FROM semantic_jobs_v1 WHERE job_id = ?", (job_id,)
                ).fetchone()
                if updated == 1
                else None
            )
            connection.execute("COMMIT")
            return committed
        except sqlite3.Error:
            self._rollback_quietly()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return None

    async def _commit_staged(self, row: sqlite3.Row) -> None:
        try:
            scope = self._scope_from_row(row)
            proposal_raw = row["proposal_json"]
            if type(proposal_raw) is not bytes:
                raise ValueError("proposal")
            proposal = validate_perception_proposal(
                proposal_raw.decode("utf-8"), scope=scope
            )
            if proposal["base_revision"] != int(row["base_revision"]):
                raise ValueError("proposal base")
        except (TypeError, ValueError, UnicodeDecodeError, SemanticProposalError):
            self._terminal_if_leased(row, "INVALID_PERCEPTION_PROPOSAL")
            return

        if not self._lease_is_current(row):
            return
        try:
            cursor = self._bridge.semantic_revision_v1(scope)
            inspected = self._bridge.inspect(scope.scope_json())
        except BaseException:
            self._terminal_if_leased(row, "NATIVE_ERROR")
            return
        if (
            type(cursor) is not dict
            or set(cursor) != {"schema", "revision"}
            or cursor.get("schema") != "astrembodiment.semantic-revision.v1"
            or type(cursor.get("revision")) is not int
            or int(cursor["revision"]) < 0
            or not isinstance(inspected, Mapping)
            or inspected.get("bound") is not True
        ):
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return
        current_incarnation = inspected.get("incarnation_id")
        if type(current_incarnation) is not str or not current_incarnation:
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return
        if current_incarnation != row["incarnation_id"]:
            self._terminal_if_leased(row, "CANCELLED_REBIRTH")
            return
        try:
            replay_after_possible_native = int(row["native_apply_started"]) == 1
        except (IndexError, KeyError, TypeError, ValueError):
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return
        if (
            int(cursor["revision"]) != int(row["base_revision"])
            and not replay_after_possible_native
        ):
            self._terminal_if_leased(row, "SUPERSEDED_STALE_REVISION")
            return
        if not self._lease_is_current(row):
            return

        if not replay_after_possible_native:
            marked = self._mark_native_apply_started(row)
            if marked is None:
                return
            row = marked

        try:
            closure = self._bridge.apply_perception_proposal_v1(scope, proposal)
        except BaseException:
            self._terminal_if_leased(row, "NATIVE_ERROR")
            return
        if type(closure) is not dict:
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return
        if closure.get("status") == "DEGRADED":
            code = closure.get("code")
            if code == "STALE_REVISION":
                self._terminal_if_leased(row, "SUPERSEDED_STALE_REVISION")
            else:
                self._terminal_if_leased(row, _closed_native_code(code))
            return
        if not self._valid_committed_closure(closure):
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return
        try:
            receipt_json = _canonical_json_bytes(closure)
        except ValueError:
            self._terminal_if_leased(row, "NATIVE_MALFORMED")
            return
        self._complete_if_leased(row, receipt_json)

    def _mark_native_apply_started(self, row: sqlite3.Row) -> sqlite3.Row | None:
        """Durably mark the only interval where a native dedup replay is safe.

        If the process dies after this transaction and before completion, the
        next lease owner may call native with the identical frozen proposal.
        Native event/proposal dedup is then the sole authority for deciding
        whether that call observes the earlier effect or a stale CAS.  A
        row that has never crossed this marker still fails stale revision
        locally without invoking native.
        """

        connection = self._require_connection()
        now = _now_ms()
        try:
            connection.execute("BEGIN IMMEDIATE")
            updated = connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET native_apply_started = 1, updated_at_ms = ?
                WHERE job_id = ? AND state = 'COMMITTING_NATIVE' AND lease_epoch = ?
                  AND native_apply_started = 0 AND lease_expires_at_ms > ?
                """,
                (now, row["job_id"], row["lease_epoch"], now),
            ).rowcount
            marked = (
                connection.execute(
                    "SELECT * FROM semantic_jobs_v1 WHERE job_id = ?",
                    (row["job_id"],),
                ).fetchone()
                if updated == 1
                else None
            )
            connection.execute("COMMIT")
            return marked
        except sqlite3.Error:
            self._rollback_quietly()
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return None

    async def _with_lease_heartbeat(
        self, row: sqlite3.Row, awaitable: Any
    ) -> Any | None:
        task = asyncio.ensure_future(awaitable)
        interval = max(1.0, self._config.lease_ms / 3_000)
        try:
            while True:
                done, _pending = await asyncio.wait(
                    {task}, timeout=interval, return_when=asyncio.FIRST_COMPLETED
                )
                if done:
                    return task.result()
                if not self._extend_lease(row):
                    try:
                        await asyncio.shield(task)
                    except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
                        raise
                    except BaseException:
                        pass
                    return None
        finally:
            if not task.done():
                task.cancel()
                await asyncio.gather(task, return_exceptions=True)

    def _extend_lease(self, row: sqlite3.Row) -> bool:
        connection = self._require_connection()
        now = _now_ms()
        try:
            return (
                connection.execute(
                    """
                    UPDATE semantic_jobs_v1
                    SET lease_expires_at_ms = ?, updated_at_ms = ?
                    WHERE job_id = ? AND state = ? AND lease_epoch = ?
                      AND lease_expires_at_ms > ?
                    """,
                    (
                        now + self._config.lease_ms,
                        now,
                        row["job_id"],
                        row["state"],
                        row["lease_epoch"],
                        now,
                    ),
                ).rowcount
                == 1
            )
        except sqlite3.Error:
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return False

    def _lease_is_current(self, row: sqlite3.Row) -> bool:
        current = self._row_for_job(bytes(row["job_id"]))
        return bool(
            current is not None
            and current["state"] == row["state"]
            and current["lease_epoch"] == row["lease_epoch"]
            and current["lease_expires_at_ms"] is not None
            and int(current["lease_expires_at_ms"]) > _now_ms()
        )

    def _retry_if_leased(
        self,
        row: sqlite3.Row,
        meta: AuxiliaryTransportMetaV1,
        attempt_budget_ms: int,
    ) -> bool:
        connection = self._require_connection()
        now = _now_ms()
        try:
            return (
                connection.execute(
                    """
                    UPDATE semantic_jobs_v1
                    SET state = 'PENDING_RETRY', lease_owner_digest = NULL,
                        lease_expires_at_ms = NULL, transport_subcode = ?,
                        attempt_budget_ms = ?, updated_at_ms = ?
                    WHERE job_id = ? AND state = 'RUNNING_PROVIDER' AND lease_epoch = ?
                      AND lease_expires_at_ms > ?
                    """,
                    (
                        meta.transport_subcode,
                        attempt_budget_ms,
                        now,
                        row["job_id"],
                        row["lease_epoch"],
                        now,
                    ),
                ).rowcount
                == 1
            )
        except sqlite3.Error:
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return False

    def _terminal_if_leased(
        self,
        row: sqlite3.Row,
        code: str,
        *,
        transport_meta: AuxiliaryTransportMetaV1 | None = None,
        attempt_budget_ms: int | None = None,
    ) -> bool:
        if code not in _TERMINAL_CODES:
            code = "NATIVE_ERROR"
        state = (
            "EXPIRED"
            if code == "EXPIRED"
            else "CANCELLED_REBIRTH"
            if code == "CANCELLED_REBIRTH"
            else "SUPERSEDED_STALE_REVISION"
            if code == "SUPERSEDED_STALE_REVISION"
            else "FAILED_TERMINAL"
        )
        connection = self._require_connection()
        now = _now_ms()
        meta = transport_meta or normalized_transport_meta(
            row["transport_subcode"],
            int(row["attempt_count"]) > 0,
            int(row["attempt_count"]),
        )
        try:
            updated = connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = ?, terminal_code = ?, encrypted_payload = NULL,
                    lease_owner_digest = NULL, lease_expires_at_ms = NULL,
                    transport_subcode = ?, attempt_budget_ms = COALESCE(?, attempt_budget_ms),
                    updated_at_ms = ?, completed_at_ms = ?
                WHERE job_id = ? AND state = ? AND lease_epoch = ?
                  AND lease_expires_at_ms > ?
                """,
                (
                    state,
                    code,
                    meta.transport_subcode,
                    attempt_budget_ms,
                    now,
                    now,
                    row["job_id"],
                    row["state"],
                    row["lease_epoch"],
                    now,
                ),
            ).rowcount
        except sqlite3.Error:
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return False
        if updated == 1:
            if code in _CLOSED_CRYPTO_CODES or code == "NATIVE_MALFORMED":
                logger.warning(
                    "AstrEmbodiment semantic async terminal failure: code=%s", code
                )
            elif code in {
                "EXPIRED",
                "CANCELLED_REBIRTH",
                "SUPERSEDED_STALE_REVISION",
            }:
                logger.info("AstrEmbodiment semantic async terminal: code=%s", code)
            self._resolve_completion(bytes(row["job_id"]))
            return True
        return False

    def _complete_if_leased(self, row: sqlite3.Row, receipt_json: bytes) -> bool:
        connection = self._require_connection()
        now = _now_ms()
        try:
            updated = connection.execute(
                """
                UPDATE semantic_jobs_v1
                SET state = 'COMMITTED', terminal_code = 'COMMITTED',
                    encrypted_payload = NULL, native_receipt_json = ?,
                    lease_owner_digest = NULL, lease_expires_at_ms = NULL,
                    updated_at_ms = ?, completed_at_ms = ?
                WHERE job_id = ? AND state = 'COMMITTING_NATIVE' AND lease_epoch = ?
                  AND lease_expires_at_ms > ?
                """,
                (
                    receipt_json,
                    now,
                    now,
                    row["job_id"],
                    row["lease_epoch"],
                    now,
                ),
            ).rowcount
        except sqlite3.Error:
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return False
        if updated == 1:
            logger.info("AstrEmbodiment semantic async terminal: code=COMMITTED")
            self._resolve_completion(bytes(row["job_id"]))
            return True
        return False

    def _expire_due_locked(self, now: int) -> list[bytes]:
        connection = self._require_connection()
        rows = connection.execute(
            f"""
            SELECT job_id FROM semantic_jobs_v1
            WHERE state NOT IN ({_sql_literals(_TERMINAL_STATES)})
              AND deadline_at_ms <= ?
            """,
            (now,),
        ).fetchall()
        connection.execute(
            f"""
            UPDATE semantic_jobs_v1
            SET state = 'EXPIRED', terminal_code = 'EXPIRED', encrypted_payload = NULL,
                lease_owner_digest = NULL, lease_expires_at_ms = NULL,
                updated_at_ms = ?, completed_at_ms = ?
            WHERE state NOT IN ({_sql_literals(_TERMINAL_STATES)})
              AND deadline_at_ms <= ?
            """,
            (now, now, now),
        )
        if rows:
            logger.info("AstrEmbodiment semantic async terminal: code=EXPIRED")
        return [bytes(row["job_id"]) for row in rows]

    def _payload_for_enqueue(
        self,
        *,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        incarnation_id: str,
        protocol_digest: bytes,
        request_text: str,
        context_summary: Mapping[str, Any],
        provider_binding: AuxiliaryProviderBindingV1,
    ) -> dict[str, Any]:
        if provider_binding.source not in _BINDING_SOURCES:
            raise ValueError("provider binding")
        return {
            "schema": _PAYLOAD_SCHEMA,
            "request_text": request_text,
            "context_summary": dict(context_summary),
            "provider_id": provider_binding.provider_id,
            "provider_source": provider_binding.source,
            "event_id": frozen_turn.event_id,
            "turn_id": frozen_turn.turn_id,
            "observed_at_ms": frozen_turn.observed_at_ms,
            "incarnation_id": incarnation_id,
            "protocol_digest": protocol_digest.hex(),
            "base_revision": frozen_turn.base_revision,
            "scope": scope.scope_json(),
        }

    def _open_row_payload(self, row: sqlite3.Row) -> dict[str, Any]:
        encrypted = row["encrypted_payload"]
        if type(encrypted) is not bytes or not encrypted:
            raise ValueError("sealed payload")
        if int(row["key_version"]) != self._key_version:
            raise SemanticOutboxUnavailable("ASYNC_KEY_VERSION_UNSUPPORTED")
        aad = self._aad_for_row(row)
        try:
            result = self._bridge.semantic_outbox_open_v1(
                _b64encode(aad), _b64encode(encrypted)
            )
        except BaseException as exc:
            raise SemanticOutboxUnavailable(_closed_crypto_code(exc)) from None
        if type(result) is not dict or set(result) != {"schema", "plaintext_b64"}:
            raise ValueError("opened payload")
        if result["schema"] != _OPENED_SCHEMA:
            raise ValueError("opened payload")
        plain = _b64decode(result["plaintext_b64"])
        if len(plain) > _MAX_PAYLOAD_BYTES:
            raise ValueError("opened payload")
        payload = json.loads(plain.decode("utf-8"), object_pairs_hook=_unique_pairs)
        if type(payload) is not dict:
            raise ValueError("opened payload")
        return payload

    def _payload_runtime_values(
        self,
        row: sqlite3.Row,
        payload: Mapping[str, Any],
    ) -> tuple[
        ScopeTokens, FrozenTurn, AuxiliaryProviderBindingV1, str, dict[str, Any]
    ]:
        expected = {
            "schema",
            "request_text",
            "context_summary",
            "provider_id",
            "provider_source",
            "event_id",
            "turn_id",
            "observed_at_ms",
            "incarnation_id",
            "protocol_digest",
            "base_revision",
            "scope",
        }
        if set(payload) != expected or payload["schema"] != _PAYLOAD_SCHEMA:
            raise ValueError("payload schema")
        scope = self._scope_from_row(row)
        if payload["scope"] != scope.scope_json():
            raise ValueError("payload scope")
        if (
            payload["event_id"] != _blob_hex(row["event_token"], byte_length=16)
            or payload["turn_id"] != _blob_hex(row["turn_token"], byte_length=16)
            or payload["incarnation_id"] != row["incarnation_id"]
            or payload["protocol_digest"] != bytes(row["protocol_digest"]).hex()
            or payload["base_revision"] != row["base_revision"]
        ):
            raise ValueError("payload binding")
        request_text = payload["request_text"]
        if type(request_text) is not str or not request_text:
            raise ValueError("payload request")
        summary = validate_context_summary_payload(payload["context_summary"])
        binding = AuxiliaryProviderBindingV1(
            provider_id=payload["provider_id"],
            source=payload["provider_source"],
            request_key=bytes(row["job_id"]).hex(),
        )
        turn = FrozenTurn(
            scope=scope,
            event_id=payload["event_id"],
            turn_id=payload["turn_id"],
            base_revision=payload["base_revision"],
            observed_at_ms=payload["observed_at_ms"],
        )
        return scope, turn, binding, request_text, summary

    def _seal(
        self,
        *,
        job_id: bytes,
        protocol_digest: bytes,
        incarnation_id: str,
        base_revision: int,
        plaintext: bytes,
    ) -> bytes:
        aad = self._aad(
            job_id=job_id,
            protocol_digest=protocol_digest,
            incarnation_id=incarnation_id,
            base_revision=base_revision,
            key_version=self._key_version,
        )
        try:
            result = self._bridge.semantic_outbox_seal_v1(
                _b64encode(aad),
                _b64encode(plaintext),
                key_version=self._key_version,
            )
        except BaseException as exc:
            raise SemanticOutboxUnavailable(_closed_crypto_code(exc)) from None
        if type(result) is not dict or set(result) != {
            "schema",
            "key_version",
            "envelope_b64",
        }:
            raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE")
        if (
            result["schema"] != _SEALED_SCHEMA
            or result["key_version"] != self._key_version
        ):
            raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE")
        try:
            return _b64decode(result["envelope_b64"])
        except ValueError:
            raise SemanticOutboxUnavailable("ASYNC_KEY_UNAVAILABLE") from None

    def _aad_for_row(self, row: sqlite3.Row) -> bytes:
        return self._aad(
            job_id=bytes(row["job_id"]),
            protocol_digest=bytes(row["protocol_digest"]),
            incarnation_id=str(row["incarnation_id"]),
            base_revision=int(row["base_revision"]),
            key_version=int(row["key_version"]),
        )

    @staticmethod
    def _aad(
        *,
        job_id: bytes,
        protocol_digest: bytes,
        incarnation_id: str,
        base_revision: int,
        key_version: int,
    ) -> bytes:
        return _canonical_json_bytes(
            {
                "schema": _AAD_SCHEMA,
                "schema_version": 1,
                "job_id": job_id.hex(),
                "protocol_digest": protocol_digest.hex(),
                "incarnation_id": incarnation_id,
                "base_revision": base_revision,
                "key_version": key_version,
            }
        )

    @staticmethod
    def _row_tokens(scope: ScopeTokens, frozen_turn: FrozenTurn) -> tuple[bytes, ...]:
        return (
            _hex_blob(scope.bot_token, byte_length=16),
            _hex_blob(scope.persona_token, byte_length=16),
            _hex_blob(scope.session_token, byte_length=16),
            _hex_blob(scope.relation_token, byte_length=16, allow_empty=True),
            _hex_blob(frozen_turn.event_id, byte_length=16),
            _hex_blob(frozen_turn.turn_id, byte_length=16),
        )

    @staticmethod
    def _job_identity(
        *,
        tokens: tuple[bytes, ...],
        incarnation_id: str,
        protocol_digest: bytes,
    ) -> bytes:
        if type(incarnation_id) is not str or not 1 <= len(incarnation_id) <= 128:
            raise ValueError("incarnation identity")
        canonical = {
            "schema": _PROTOCOL_SCHEMA,
            "bot_token": tokens[0].hex(),
            "persona_token": tokens[1].hex(),
            "session_token": tokens[2].hex(),
            "relation_token": tokens[3].hex() if tokens[3] else None,
            "event_token": tokens[4].hex(),
            "turn_token": tokens[5].hex(),
            "incarnation_id": incarnation_id,
        }
        return hashlib.sha256(
            _canonical_json_bytes(canonical) + b"\x00" + protocol_digest
        ).digest()

    @staticmethod
    def _scope_from_row(row: sqlite3.Row) -> ScopeTokens:
        return ScopeTokens(
            bot_token=str(_blob_hex(row["bot_token"], byte_length=16)),
            persona_token=str(_blob_hex(row["persona_token"], byte_length=16)),
            session_token=str(_blob_hex(row["session_token"], byte_length=16)),
            relation_token=_blob_hex(
                row["relation_token"], byte_length=16, allow_empty=True
            ),
        )

    def _attempt_budget_ms(self, row: sqlite3.Row, provider_id: str) -> int | None:
        remaining = min(
            int(row["deadline_at_ms"]) - _now_ms(),
            int(row["budget_expires_at_ms"]) - _now_ms(),
        )
        if remaining < 5_000:
            return None
        predicted = self._latency_for(row, provider_id).predict_ms()
        requested = math.ceil(predicted * 1.50 + 1_000)
        bounded = min(max(requested, 5_000), self._config.provider_attempt_cap_ms)
        return min(bounded, remaining)

    def _retry_required_ms(self, row: sqlite3.Row, provider_id: str) -> int:
        samples = self._latency.get(self._latency_key(row, provider_id))
        predicted = samples.predict_ms() if samples is not None else 60_000
        requested = math.ceil(predicted * 1.25 + 1_000)
        return min(max(requested, 5_000), self._config.provider_attempt_cap_ms)

    def _latency_for(self, row: sqlite3.Row, provider_id: str) -> _LatencySamples:
        key = self._latency_key(row, provider_id)
        return self._latency.setdefault(key, _LatencySamples(values=[]))

    @staticmethod
    def _latency_key(row: sqlite3.Row, provider_id: str | None) -> bytes:
        provider = provider_id.encode("utf-8") if provider_id is not None else b""
        return hashlib.sha256(
            b"astr-embodiment/semantic-outbox-latency-v1\x00"
            + provider
            + b"\x00"
            + bytes(row["bot_token"])
            + bytes(row["persona_token"])
        ).digest()

    @staticmethod
    def _provider_system_prompt() -> str:
        canonical_schema = _canonical_json_text(SEMANTIC_ESTIMATE_V3_STRUCTURED_SCHEMA)
        return (
            f"{SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT}\n\n"
            "Closed output schema (canonical JSON):\n"
            f"{canonical_schema}\n"
            "Return exactly one JSON object matching this closed schema."
        )

    @staticmethod
    def _valid_committed_closure(closure: Mapping[str, Any]) -> bool:
        vector = closure.get("semantic_vector_receipt")
        return bool(
            closure.get("schema")
            in {
                "astrembodiment.semantic-perception-closure.v1",
                "astrembodiment.semantic-perception-closure.v2",
            }
            and type(closure.get("revision")) is int
            and int(closure["revision"]) >= 0
            and closure.get("full_vector_state") == "FULL_VECTOR_CONFIRMED"
            and closure.get("node_observability_state") == "CONFIRMED"
            and isinstance(vector, Mapping)
            and vector.get("dimension_slot_count") == 15
            and vector.get("evaluated_dimension_count") == 15
            and vector.get("injected_dimension_count") == 15
            and vector.get("unavailable_dimension_count") == 0
            and isinstance(closure.get("expression_projection"), Mapping)
        )

    def _future_for(self, job_id: bytes) -> asyncio.Future[dict[str, Any]]:
        future = self._completions.get(job_id)
        if future is None:
            future = asyncio.get_running_loop().create_future()
            self._completions[job_id] = future
        return future

    def _completion_waiter_or_terminal_outcome(
        self, job_id: bytes
    ) -> tuple[asyncio.Future[dict[str, Any]] | None, dict[str, Any] | None]:
        """Join an active job or read its terminal outcome without retaining it."""

        row = self._row_for_job(job_id)
        if row is None:
            return (
                None,
                self._degraded(
                    "SHUTDOWN" if self._stopping else "ASYNC_QUEUE_UNAVAILABLE",
                    attempted=False,
                    attempt_count=0,
                ),
            )
        if str(row["state"]) in _TERMINAL_STATES:
            return None, self._outcome_for_row(row)
        # This method does not await.  A worker therefore cannot terminalize a
        # row between the durable state read and registration of this waiter.
        return self._future_for(job_id), None

    def _resolve_completion(self, job_id: bytes) -> None:
        future = self._completions.pop(job_id, None)
        if future is None or future.done():
            return
        row = self._row_for_job(job_id)
        if row is not None and str(row["state"]) in _TERMINAL_STATES:
            future.set_result(self._outcome_for_row(row))
            return
        future.set_result(
            self._degraded(
                "ASYNC_QUEUE_UNAVAILABLE",
                attempted=False,
                attempt_count=0,
            )
        )

    def _resolve_completion_from_row(self, row: sqlite3.Row) -> None:
        """Wake existing waiters before retention GC drops the durable row."""

        future = self._completions.pop(bytes(row["job_id"]), None)
        if future is not None and not future.done():
            future.set_result(self._outcome_for_row(row))

    def _outcome_for_row(self, row: sqlite3.Row) -> dict[str, Any]:
        attempt_count = int(row["attempt_count"])
        meta = normalized_transport_meta(
            row["transport_subcode"], attempt_count > 0, attempt_count
        )
        if row["state"] != "COMMITTED":
            code = row["terminal_code"]
            return self._degraded(
                code if type(code) is str else "NATIVE_ERROR",
                attempted=meta.attempted,
                attempt_count=meta.attempt_count,
                transport_subcode=meta.transport_subcode,
            )
        try:
            dimensions_payload = json.loads(
                bytes(row["dimensions_json"]).decode("utf-8")
            )
            dimensions = dimensions_payload["dimensions"]
            receipt = json.loads(bytes(row["native_receipt_json"]).decode("utf-8"))
            if type(dimensions) is not dict or not self._valid_committed_closure(
                receipt
            ):
                raise ValueError("receipt")
            confidence = row["confidence_fxp6"]
            if type(confidence) is not int:
                raise ValueError("confidence")
        except (
            KeyError,
            TypeError,
            ValueError,
            UnicodeDecodeError,
            json.JSONDecodeError,
        ):
            return self._degraded(
                "NATIVE_MALFORMED",
                attempted=meta.attempted,
                attempt_count=meta.attempt_count,
                transport_subcode=meta.transport_subcode,
            )
        return {
            "status": "SUCCESS",
            "code": "SEMANTIC_COMMITTED",
            "dimensions_fxp6": dimensions,
            "estimator_confidence_fxp6": confidence,
            "semantic_closure": receipt,
            "migration_subcode": receipt.get("migration_subcode"),
            **meta.as_json(),
        }

    @staticmethod
    def _degraded(
        code: str,
        *,
        attempted: bool,
        attempt_count: int,
        transport_subcode: str = "NOT_APPLICABLE",
    ) -> dict[str, Any]:
        return {
            "status": "DEGRADED",
            "code": code if code in _TERMINAL_CODES else "NATIVE_ERROR",
            "transport_subcode": transport_subcode,
            "attempted": attempted,
            "attempt_count": attempt_count,
        }

    def _attempt_count(self, job_id: bytes) -> int:
        row = self._row_for_job(job_id)
        return int(row["attempt_count"]) if row is not None else 0

    def _row_for_job(self, job_id: bytes) -> sqlite3.Row | None:
        connection = self._connection
        if connection is None:
            return None
        try:
            return connection.execute(
                "SELECT * FROM semantic_jobs_v1 WHERE job_id = ?", (job_id,)
            ).fetchone()
        except sqlite3.Error:
            self._disable("ASYNC_QUEUE_UNAVAILABLE")
            return None

    def _require_connection(self) -> sqlite3.Connection:
        if self._connection is None:
            raise SemanticOutboxUnavailable("ASYNC_QUEUE_UNAVAILABLE")
        return self._connection

    def _rollback_quietly(self) -> None:
        connection = self._connection
        if connection is None:
            return
        try:
            connection.execute("ROLLBACK")
        except sqlite3.Error:
            pass

    def _close_connection(self) -> None:
        connection, self._connection = self._connection, None
        if connection is None:
            return
        try:
            connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        except sqlite3.Error:
            pass
        connection.close()

    def _disable(self, code: str) -> None:
        was_ready = self._ready
        previous_code = self._disabled_code
        self._ready = False
        self._disabled_code = (
            code if code in _TERMINAL_CODES else "ASYNC_QUEUE_UNAVAILABLE"
        )
        if (
            was_ready
            and self._disabled_code != previous_code
            and self._disabled_code != "SHUTDOWN"
        ):
            logger.warning(
                "AstrEmbodiment semantic async lane disabled: code=%s",
                self._disabled_code,
            )
        self._wake.set()


def _unique_pairs(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate semantic outbox field")
        result[key] = value
    return result


def _sql_literals(values: frozenset[str]) -> str:
    return ", ".join("'" + value + "'" for value in sorted(values))
