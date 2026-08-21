"""Genesis singleflight coordinator and first-message barrier.

In-process singleflight: concurrent first turns for the same
(Bot, Persona, persona_source_digest) join one compiler Future, so the main
provider is called exactly once. The original message is applied exactly once
after the committed Genesis exists; a compiler failure never creates a default
brain and never writes production state.
"""

from __future__ import annotations

import asyncio
import copy
import inspect
import json
import secrets
from collections.abc import Awaitable, Callable
from dataclasses import replace
from typing import Any

from .bridge import (
    GenesisUnavailable,
    NativeBridge,
    RetryWait,
    validate_semantic_result,
)
from .contracts import (
    FrozenTurn,
    ScopeTokens,
    build_delivery_outcome_json,
    build_user_stimulus_json,
)
from .semantic_estimator import (
    SemanticEstimate,
    SemanticEstimateError,
    _canonical_nonzero_hex,
    build_perception_proposal,
    make_request_nonce_digest,
    parse_estimator_output,
    proposal_to_json,
)
from .persona_genesis import (
    PersonaCompilerMalformed,
    PersonaSourceSnapshot,
    build_closed_request,
    validate_proposal,
)

FORMULA_DIGEST = "00" * 32  # placeholder; G2 fills the real FormulaProfile digest

Compiler = Callable[[PersonaSourceSnapshot], Awaitable[dict[str, Any]]]
PreflightEstimator = Callable[[str], Any]

SEMANTIC_SUCCESS = "SUCCESS"
SEMANTIC_NOOP = "NOOP"
SEMANTIC_DEGRADED = "DEGRADED"

_RETRY_WAIT_ATTEMPTS = 40
_RETRY_WAIT_DELAY_S = 0.05


class GenesisCoordinator:
    """Owns no brain state: only in-flight futures and turn bookkeeping."""

    def __init__(self, bridge: NativeBridge) -> None:
        self._bridge = bridge
        self._inflight: dict[str, asyncio.Future] = {}
        self._committed: dict[str, dict[str, Any]] = {}
        self._applied: dict[str, dict[str, Any]] = {}
        # SPC1 is a separate request-local lane.  These maps contain only
        # opaque scope/turn keys and closed outcomes; raw request text is
        # never retained.
        self._preflight_inflight: dict[str, asyncio.Future] = {}
        self._preflight_results: dict[str, dict[str, Any]] = {}

    @staticmethod
    def _scope_key(scope: ScopeTokens, source_digest: str) -> str:
        return f"{scope.bot_token}:{scope.persona_token}:{source_digest}"

    async def ensure_genesis(
        self,
        *,
        scope: ScopeTokens,
        source: PersonaSourceSnapshot,
        selection: str,
        compiler: Compiler,
        compiler_protocol_digest: str,
        compiler_model_digest: str,
        observed_at_ms: int,
    ) -> dict[str, Any]:
        """Join or run the singleflight Genesis compile + commit."""
        key = self._scope_key(scope, source.source_digest)
        committed = self._committed.get(key)
        if committed is not None:
            return copy.deepcopy(committed)
        inflight = self._inflight.get(key)
        if inflight is not None and not inflight.done():
            return copy.deepcopy(await asyncio.shield(inflight))

        future: asyncio.Future = asyncio.get_running_loop().create_future()
        self._inflight[key] = future
        try:
            receipt = await self._run_genesis(
                scope=scope,
                source=source,
                selection=selection,
                compiler=compiler,
                compiler_protocol_digest=compiler_protocol_digest,
                compiler_model_digest=compiler_model_digest,
                observed_at_ms=observed_at_ms,
            )
            self._committed[key] = copy.deepcopy(receipt)
            future.set_result(receipt)
        except BaseException as exc:
            future.set_exception(exc)
            raise
        finally:
            if self._inflight.get(key) is future:
                self._inflight.pop(key, None)
        return copy.deepcopy(await future)

    async def _run_genesis(
        self,
        *,
        scope: ScopeTokens,
        source: PersonaSourceSnapshot,
        selection: str,
        compiler: Compiler,
        compiler_protocol_digest: str,
        compiler_model_digest: str,
        observed_at_ms: int,
    ) -> dict[str, Any]:
        # One compiler call; one closed-schema repair retry. The first user
        # message and chat history are absent from the compiler payload by
        # construction (see PersonaSourceSnapshot.compiler_payload).
        try:
            proposal = await compiler(source)
            proposal = validate_proposal(proposal)
        except PersonaCompilerMalformed:
            # Bounded single repair attempt; a second failure fails closed.
            proposal = await compiler(source)
            proposal = validate_proposal(proposal)

        closed_request = build_closed_request(
            scope=scope,
            source=source,
            proposal=proposal,
            selection=selection,
            compiler_protocol_digest=compiler_protocol_digest,
            compiler_model_digest=compiler_model_digest,
            formula_digest=FORMULA_DIGEST,
            incarnation_nonce=secrets.token_bytes(32).hex(),
            observed_at_ms=observed_at_ms,
        )

        for attempt in range(_RETRY_WAIT_ATTEMPTS):
            try:
                return self._bridge.ensure_genesis(closed_request)
            except RetryWait:
                await asyncio.sleep(_RETRY_WAIT_DELAY_S)
        raise GenesisUnavailable(
            "GENESIS_UNAVAILABLE",
            "genesis lease stayed in flight; no default brain was created",
        )

    async def first_turn(
        self,
        *,
        scope: ScopeTokens,
        event_id: str,
        turn_id: str,
        base_revision: int,
        observed_at_ms: int,
        source: PersonaSourceSnapshot,
        selection: str,
        compiler: Compiler,
        compiler_protocol_digest: str,
        compiler_model_digest: str,
    ) -> dict[str, Any]:
        """First-message barrier: genesis first, then apply exactly once."""
        genesis = await self.ensure_genesis(
            scope=scope,
            source=source,
            selection=selection,
            compiler=compiler,
            compiler_protocol_digest=compiler_protocol_digest,
            compiler_model_digest=compiler_model_digest,
            observed_at_ms=observed_at_ms,
        )
        decision = await self.apply_stimulus(
            scope=scope,
            event_id=event_id,
            turn_id=turn_id,
            base_revision=base_revision,
            observed_at_ms=observed_at_ms,
        )
        # Keep the native identity receipt alongside the turn decision so the
        # host can persist and expose SeedCode without reconstructing identity
        # in Python.
        result = dict(decision)
        result["genesis"] = genesis
        result["seed_code"] = genesis.get("seed_code", "")
        result["seed_code_short"] = genesis.get("seed_code_short", "")
        result["incarnation_id"] = genesis.get("incarnation_id", "")
        return result

    async def apply_stimulus(
        self,
        *,
        scope: ScopeTokens,
        event_id: str,
        turn_id: str,
        base_revision: int,
        observed_at_ms: int,
    ) -> dict[str, Any]:
        event = build_user_stimulus_json(
            scope=scope,
            event_id=event_id,
            turn_id=turn_id,
            base_revision=base_revision,
            observed_at_ms=observed_at_ms,
        )
        return await self._apply_once(scope, event_id, event)

    @staticmethod
    def _preflight_key(scope: ScopeTokens, turn: FrozenTurn) -> str:
        """Build a cache key from opaque facts only (never request text)."""

        relation = (
            _canonical_nonzero_hex(scope.relation_token, 16)
            if scope.relation_token is not None
            else "-"
        )
        return ":".join(
            (
                _canonical_nonzero_hex(scope.bot_token, 16),
                _canonical_nonzero_hex(scope.persona_token, 16),
                _canonical_nonzero_hex(scope.session_token, 16),
                relation,
                _canonical_nonzero_hex(turn.event_id, 16),
                _canonical_nonzero_hex(turn.turn_id, 16),
                str(turn.base_revision),
                str(turn.observed_at_ms),
            )
        )

    @staticmethod
    def _preflight_failure(code: str) -> dict[str, str]:
        return {"status": SEMANTIC_DEGRADED, "code": code}

    @staticmethod
    def _preflight_noop(code: str) -> dict[str, str]:
        return {"status": SEMANTIC_NOOP, "code": code}

    @staticmethod
    def _valid_frozen_turn(scope: ScopeTokens, turn: FrozenTurn) -> bool:
        if type(scope) is not ScopeTokens or type(turn) is not FrozenTurn:
            return False
        if type(turn.scope) is not ScopeTokens:
            return False
        try:
            canonical_scope = ScopeTokens(
                bot_token=_canonical_nonzero_hex(scope.bot_token, 16),
                persona_token=_canonical_nonzero_hex(scope.persona_token, 16),
                session_token=_canonical_nonzero_hex(scope.session_token, 16),
                relation_token=(
                    _canonical_nonzero_hex(scope.relation_token, 16)
                    if scope.relation_token is not None
                    else None
                ),
            )
            turn_scope = ScopeTokens(
                bot_token=_canonical_nonzero_hex(turn.scope.bot_token, 16),
                persona_token=_canonical_nonzero_hex(turn.scope.persona_token, 16),
                session_token=_canonical_nonzero_hex(turn.scope.session_token, 16),
                relation_token=(
                    _canonical_nonzero_hex(turn.scope.relation_token, 16)
                    if turn.scope.relation_token is not None
                    else None
                ),
            )
        except (TypeError, ValueError):
            return False
        if turn_scope != canonical_scope:
            return False
        try:
            _canonical_nonzero_hex(turn.event_id, 16)
            _canonical_nonzero_hex(turn.turn_id, 16)
        except (TypeError, ValueError):
            return False
        return (
            type(turn.base_revision) is int
            and turn.base_revision >= 0
            and type(turn.observed_at_ms) is int
            and turn.observed_at_ms > 0
        )

    @staticmethod
    async def _invoke_preflight_estimator(
        estimator: PreflightEstimator,
        request_text: str,
    ) -> Any:
        candidate: Any = estimator
        if not callable(candidate):
            candidate = getattr(candidate, "estimate", None)
        if not callable(candidate):
            raise SemanticEstimateError("ESTIMATOR_UNAVAILABLE")
        # Exactly one positional argument.  In particular, do not pass tools,
        # history, contexts, provider payloads, or control-plane kwargs.
        result = candidate(request_text)
        if inspect.isawaitable(result):
            result = await result
        return result

    async def preflight_stimulus(
        self,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        request_text: str,
        estimator: PreflightEstimator,
    ) -> dict[str, Any]:
        """Run one request-local SPC1 estimate and, when useful, commit it.

        This method is intentionally additive: callers may invoke it after
        Genesis and before the provider while the existing ``first_turn`` and
        ``apply_stimulus`` G0 paths remain unchanged.  A request key joins
        concurrent calls and prevents a second estimator invocation even when
        the second call supplies different text.
        """

        if not self._valid_frozen_turn(scope, frozen_turn):
            return self._preflight_failure("INVALID_TURN")
        key = self._preflight_key(scope, frozen_turn)
        previous = self._preflight_results.get(key)
        if previous is not None:
            return copy.deepcopy(previous)
        inflight = self._preflight_inflight.get(key)
        if inflight is None:
            task = asyncio.create_task(
                self._run_preflight(
                    scope=scope,
                    frozen_turn=frozen_turn,
                    request_text=request_text,
                    estimator=estimator,
                )
            )
            self._preflight_inflight[key] = task
            task.add_done_callback(
                lambda completed, request_key=key: self._settle_preflight(
                    request_key, completed
                )
            )
            inflight = task

        # The owner task is independent from this caller.  Shielding means a
        # cancelled waiter cannot cancel the shared estimator/native attempt.
        try:
            outcome = await asyncio.shield(inflight)
        except asyncio.CancelledError:
            if inflight.cancelled():
                # The shared owner itself was cancelled (for example, an
                # estimator raised CancelledError); expose the same fixed
                # outcome that the done callback will cache.  A cancellation
                # of this caller alone leaves the owner task untouched.
                return copy.deepcopy(
                    self._preflight_failure("ESTIMATOR_UNAVAILABLE")
                )
            raise
        except BaseException:
            # The done callback consumes the task exception and stores a fixed
            # closed outcome.  Keep this caller fail-closed as well.
            outcome = self._preflight_failure("NATIVE_ERROR")
        try:
            return copy.deepcopy(outcome)
        except BaseException:
            return self._preflight_failure("NATIVE_MALFORMED")

    def _settle_preflight(
        self,
        key: str,
        task: asyncio.Future,
    ) -> None:
        """Consume a shared task exactly once and cache only a closed result."""

        closed: dict[str, Any] = self._preflight_failure("NATIVE_ERROR")
        try:
            try:
                outcome = task.result()
            except asyncio.CancelledError:
                # Never leave a CancelledError as an unobserved Future
                # exception; a later caller receives the same fixed outcome.
                outcome = self._preflight_failure("ESTIMATOR_UNAVAILABLE")
            except BaseException:
                outcome = self._preflight_failure("NATIVE_ERROR")
            try:
                candidate = json.loads(
                    json.dumps(
                        outcome,
                        ensure_ascii=False,
                        sort_keys=True,
                        allow_nan=False,
                    )
                )
                if type(candidate) is not dict:
                    raise ValueError("closed outcome")
                closed = candidate
            except BaseException:
                closed = self._preflight_failure("NATIVE_MALFORMED")
            try:
                self._preflight_results[key] = copy.deepcopy(closed)
            except BaseException:
                self._preflight_results[key] = self._preflight_failure(
                    "NATIVE_MALFORMED"
                )
        except BaseException:
            # A done callback must never leak provider/native exception details
            # into the event loop; retain a fixed closed result if possible.
            try:
                self._preflight_results[key] = self._preflight_failure(
                    "NATIVE_MALFORMED"
                )
            except BaseException:
                pass
        finally:
            try:
                if self._preflight_inflight.get(key) is task:
                    self._preflight_inflight.pop(key, None)
            except BaseException:
                pass

    async def _run_preflight(
        self,
        *,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        request_text: str,
        estimator: PreflightEstimator,
    ) -> dict[str, Any]:
        """Execute one owned estimator/native attempt for a frozen key."""
        try:
            return await self._run_preflight_body(
                scope=scope,
                frozen_turn=frozen_turn,
                request_text=request_text,
                estimator=estimator,
            )
        except asyncio.CancelledError:
            return self._preflight_failure("ESTIMATOR_UNAVAILABLE")
        except BaseException:
            return self._preflight_failure("NATIVE_ERROR")

    async def _run_preflight_body(
        self,
        *,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        request_text: str,
        estimator: PreflightEstimator,
    ) -> dict[str, Any]:
        if not isinstance(request_text, str):
            return self._preflight_failure("ESTIMATOR_MALFORMED")
        if not request_text.strip():
            return self._preflight_noop("EMPTY_REQUEST")
        try:
            raw_estimate = await self._invoke_preflight_estimator(estimator, request_text)
        except asyncio.CancelledError:
            return self._preflight_failure("ESTIMATOR_UNAVAILABLE")
        except SemanticEstimateError as exc:
            return self._preflight_failure(
                exc.code if exc.code in {"ESTIMATOR_MALFORMED", "ESTIMATOR_UNAVAILABLE"} else "ESTIMATOR_UNAVAILABLE"
            )
        except BaseException:
            # Provider exception details are deliberately discarded.
            return self._preflight_failure("ESTIMATOR_UNAVAILABLE")

        try:
            estimate = (
                parse_estimator_output(raw_estimate.as_json())
                if isinstance(raw_estimate, SemanticEstimate)
                else parse_estimator_output(raw_estimate)
            )
        except BaseException:
            return self._preflight_failure("ESTIMATOR_MALFORMED")
        try:
            is_load_noop = estimate.is_load_noop
        except BaseException:
            return self._preflight_failure("ESTIMATOR_MALFORMED")
        if is_load_noop:
            # This is a fixed no-op: no nonce, cursor read, or native commit.
            return self._preflight_noop("ZERO_LOAD")

        # The semantic cursor is content-free.  It is read before the proposal
        # base revision is frozen, so G0's separate revision lane is untouched.
        try:
            cursor = self._bridge.semantic_revision_v1(scope.scope_json())
        except BaseException:
            return self._preflight_failure("NATIVE_ERROR")
        if type(cursor) is not dict:
            return self._preflight_failure("NATIVE_MALFORMED")
        if cursor.get("status") == SEMANTIC_DEGRADED:
            return self._preflight_failure(
                cursor.get("code")
                if isinstance(cursor.get("code"), str)
                else "NATIVE_ERROR"
            )
        if set(cursor) != {"schema", "revision"} or cursor.get("schema") != "astrembodiment.semantic-revision.v1":
            return self._preflight_failure("NATIVE_MALFORMED")
        revision = cursor.get("revision")
        if type(revision) is not int or revision < 0:
            return self._preflight_failure("NATIVE_MALFORMED")

        try:
            bound_turn = replace(frozen_turn, base_revision=revision)
            nonce = make_request_nonce_digest(scope, bound_turn)
            proposal = build_perception_proposal(
                scope=scope,
                turn=bound_turn,
                estimate=estimate,
                base_revision=revision,
                nonce_digest=nonce,
            )
            proposal_json = proposal_to_json(proposal, scope=scope)
        except BaseException:
            return self._preflight_failure("INVALID_PROPOSAL")

        try:
            native_result = self._bridge.apply_perception_proposal_v1(
                scope.scope_json(), proposal_json
            )
        except BaseException:
            return self._preflight_failure("NATIVE_ERROR")
        if type(native_result) is not dict:
            return self._preflight_failure("NATIVE_MALFORMED")
        if native_result.get("status") == SEMANTIC_DEGRADED:
            return self._preflight_failure(
                native_result.get("code")
                if isinstance(native_result.get("code"), str)
                else "NATIVE_ERROR"
            )
        if native_result == {"status": SEMANTIC_NOOP, "code": "ZERO_LOAD"}:
            return self._preflight_noop("ZERO_LOAD")
        try:
            native_result = validate_semantic_result(
                native_result,
                expected_base_revision=proposal["base_revision"],
            )
        except BaseException:
            return self._preflight_failure("NATIVE_MALFORMED")
        return {
            "status": SEMANTIC_SUCCESS,
            "code": "SEMANTIC_COMMITTED",
            "proposal": proposal,
            "result": copy.deepcopy(native_result),
        }

    async def apply_delivery(
        self,
        *,
        scope: ScopeTokens,
        event_id: str,
        turn_id: str,
        base_revision: int,
        delivered: bool,
        visible_action_digest: str,
        delivered_at_ms: int,
    ) -> dict[str, Any]:
        event = build_delivery_outcome_json(
            scope=scope,
            event_id=event_id,
            turn_id=turn_id,
            base_revision=base_revision,
            delivered=delivered,
            visible_action_digest=visible_action_digest,
            delivered_at_ms=delivered_at_ms,
        )
        return await self._apply_once(scope, event_id, event)

    async def _apply_once(
        self,
        scope: ScopeTokens,
        event_id: str,
        event: dict[str, Any],
    ) -> dict[str, Any]:
        """Deduplicate by event id; the native lane also rejects re-application."""
        memo_key = f"{scope.bot_token}:{scope.persona_token}:{event_id}"
        previous = self._applied.get(memo_key)
        if previous is not None:
            previous = dict(previous)
            previous["deduplicated"] = True
            return previous

        decision = self._bridge.apply_event(scope.scope_json(), event)
        if decision.get("deduplicated"):
            # The native lane had already applied this exact event: reuse the
            # original decision instead of double-applying.
            self._applied[memo_key] = dict(decision)
        else:
            self._applied[memo_key] = dict(decision)
        return decision

    def applied_count(self) -> int:
        return len(self._applied)

    def reset(self) -> None:
        self._inflight.clear()
        self._committed.clear()
        self._applied.clear()
        self._preflight_inflight.clear()
        self._preflight_results.clear()
