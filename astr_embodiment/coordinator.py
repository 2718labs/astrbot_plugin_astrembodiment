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
import secrets
from collections.abc import Awaitable, Callable, Mapping
from typing import Any

from .bridge import (
    ContextProjectionIntegrity,
    GenesisUnavailable,
    NativeBridge,
    RetryWait,
    SEMANTIC_NATIVE_ERROR_CODES,
    SEMANTIC_NATIVE_FAILURE_STAGES,
    normalize_invalid_neural_state_subcode,
    validate_context_summary_payload,
)
from .contracts import (
    FrozenTurn,
    ScopeTokens,
    build_delivery_outcome_json,
    build_user_stimulus_json,
)
from .context_binding import ContextBindingV1, adapt_native_context_summary_v1
from .persona_genesis import (
    PersonaCompilerMalformed,
    PersonaSourceSnapshot,
    build_closed_request,
    validate_proposal,
)
from .semantic_estimator import (
    ESTIMATOR_FORMULA_DIGEST,
    ESTIMATOR_MALFORMED_SUBCODES,
    SemanticEstimateError,
    SemanticProposalError,
    build_perception_proposal_v3,
    estimate_context_bound,
    make_request_nonce_digest,
)

FORMULA_DIGEST = "00" * 32  # placeholder; G2 fills the real FormulaProfile digest

Compiler = Callable[[PersonaSourceSnapshot], Awaitable[dict[str, Any]]]
ContextBoundEstimatorProvider = Callable[[Mapping[str, Any]], Any]

_RETRY_WAIT_ATTEMPTS = 40
_RETRY_WAIT_DELAY_S = 0.05
_SEMANTIC_FAILURE_CODES = (
    frozenset(
        {
            "EMPTY_REQUEST",
            "INVALID_TURN",
            "ESTIMATOR_UNAVAILABLE",
            "ESTIMATOR_MALFORMED",
            "SEMANTIC_VECTOR_UNAVAILABLE",
            "ESTIMATOR_UNCERTAIN",
            "NATIVE_SYMBOL_UNAVAILABLE",
            "NATIVE_MALFORMED",
            "STALE_REVISION",
            "NATIVE_ERROR",
            "EXPRESSION_PROJECTION_UNAVAILABLE",
        }
    )
    | SEMANTIC_NATIVE_ERROR_CODES
)


class GenesisCoordinator:
    """Owns no brain state: only in-flight futures and turn bookkeeping."""

    def __init__(self, bridge: NativeBridge) -> None:
        self._bridge = bridge
        self._inflight: dict[str, asyncio.Future] = {}
        self._committed: dict[str, dict[str, Any]] = {}
        self._applied: dict[str, dict[str, Any]] = {}
        self._semantic_inflight: dict[str, asyncio.Task[dict[str, Any]]] = {}
        self._semantic_results: dict[str, dict[str, Any]] = {}

    @staticmethod
    def _scope_key(scope: ScopeTokens, source_digest: str) -> str:
        return f"{scope.bot_token}:{scope.persona_token}:{source_digest}"

    def prepare_rebirth(
        self,
        *,
        scope: ScopeTokens,
        expected_incarnation_id: str,
        expected_revision: int,
        action: str,
    ) -> dict[str, Any]:
        """Forward one explicit D1.5 prepare request without retaining consent.

        The Rust lifecycle owner creates and persists the challenge.  Python
        deliberately keeps no nonce, receipt, replay, or incarnation state.
        """
        return self._bridge.prepare_rebirth_v1(
            {
                "scope": scope.scope_json(),
                "expected_incarnation_id": expected_incarnation_id,
                "expected_revision": expected_revision,
                "action": action,
            }
        )

    def confirm_rebirth_payload(
        self,
        payload: dict[str, Any],
    ) -> dict[str, Any]:
        """Forward the user-supplied confirmation unchanged to D1.5."""
        return self._bridge.confirm_rebirth_v1(dict(payload))

    def forget_scope(self, scope: ScopeTokens) -> None:
        """Discard process-local mirrors after a native incarnation changes."""
        prefix = f"{scope.bot_token}:{scope.persona_token}:"
        for key in tuple(self._committed):
            if key.startswith(prefix):
                self._committed.pop(key, None)
        for key in tuple(self._applied):
            if key.startswith(prefix):
                self._applied.pop(key, None)
        for key in tuple(self._semantic_results):
            if key.startswith(prefix):
                self._semantic_results.pop(key, None)
        for key in tuple(self._semantic_inflight):
            if key.startswith(prefix):
                self._semantic_inflight.pop(key, None)

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
    def _semantic_failure(
        code: str,
        *,
        cause_code: str | None = None,
        native_stage: str | None = None,
        state_subcode: object = None,
    ) -> dict[str, str]:
        """Return one non-echoing V3 preview failure."""

        if code not in _SEMANTIC_FAILURE_CODES:
            code = "NATIVE_ERROR"
        result = {"status": "DEGRADED", "code": code}
        if code in SEMANTIC_NATIVE_ERROR_CODES:
            result["cause_code"] = code
            if native_stage in SEMANTIC_NATIVE_FAILURE_STAGES:
                result["native_stage"] = native_stage
                if code == "INVALID_NEURAL_STATE" and native_stage == "NATIVE_APPLY":
                    result["state_subcode"] = normalize_invalid_neural_state_subcode(
                        state_subcode
                    )
        elif (
            code == "ESTIMATOR_MALFORMED" and cause_code in ESTIMATOR_MALFORMED_SUBCODES
        ):
            result["cause_code"] = cause_code
        return result

    @staticmethod
    def _semantic_key(scope: ScopeTokens, frozen_turn: FrozenTurn) -> str:
        return ":".join(
            (
                scope.bot_token,
                scope.persona_token,
                frozen_turn.event_id,
                frozen_turn.turn_id,
            )
        )

    @staticmethod
    def _valid_semantic_request(
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        request_text: str,
    ) -> bool:
        """Validate opaque turn identity before opening the native cursor."""

        if type(scope) is not ScopeTokens or type(frozen_turn) is not FrozenTurn:
            return False
        if (
            frozen_turn.scope != scope
            or type(request_text) is not str
            or not request_text
        ):
            return False
        try:
            # The nonce builder performs the exact closed scope/turn validation.
            make_request_nonce_digest(scope, frozen_turn)
        except SemanticProposalError:
            return False
        return True

    async def preflight_semantic_v3(
        self,
        *,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        request_text: str,
        context_summary: Mapping[str, Any],
        estimator: ContextBoundEstimatorProvider,
    ) -> dict[str, Any]:
        """Run the isolated V3 preview lane at most once for one frozen turn.

        The singleflight key deliberately excludes request text: a concurrent
        retry with different text joins the first frozen turn instead of
        producing a second semantic estimate or native proposal.
        """

        if type(request_text) is not str or not request_text:
            return self._semantic_failure("EMPTY_REQUEST")
        if not self._valid_semantic_request(scope, frozen_turn, request_text):
            return self._semantic_failure("INVALID_TURN")
        key = self._semantic_key(scope, frozen_turn)
        previous = self._semantic_results.get(key)
        if previous is not None:
            return copy.deepcopy(previous)
        task = self._semantic_inflight.get(key)
        if task is None:
            task = asyncio.create_task(
                self._run_semantic_v3(
                    scope=scope,
                    frozen_turn=frozen_turn,
                    request_text=request_text,
                    context_summary=context_summary,
                    estimator=estimator,
                )
            )
            self._semantic_inflight[key] = task
        try:
            result = await asyncio.shield(task)
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            result = self._semantic_failure("NATIVE_ERROR")
        self._semantic_results[key] = copy.deepcopy(result)
        if task.done() and self._semantic_inflight.get(key) is task:
            self._semantic_inflight.pop(key, None)
        return copy.deepcopy(result)

    async def _run_semantic_v3(
        self,
        *,
        scope: ScopeTokens,
        frozen_turn: FrozenTurn,
        request_text: str,
        context_summary: Mapping[str, Any],
        estimator: ContextBoundEstimatorProvider,
    ) -> dict[str, Any]:
        """Execute the V3-only path without consulting the G0 zero builder."""

        try:
            native_summary = validate_context_summary_payload(context_summary)
        except BaseException:
            return self._semantic_failure("NATIVE_MALFORMED")

        cursor = self._bridge.semantic_revision_v1(scope)
        if type(cursor) is not dict:
            return self._semantic_failure("NATIVE_MALFORMED")
        if cursor.get("status") == "DEGRADED":
            return self._semantic_failure(
                str(cursor.get("code", "NATIVE_ERROR")), native_stage="CURSOR"
            )
        if set(cursor) != {"schema", "revision"}:
            return self._semantic_failure("NATIVE_MALFORMED")
        cursor_revision = cursor.get("revision")
        if type(cursor_revision) is not int or cursor_revision < 0:
            return self._semantic_failure("NATIVE_MALFORMED")

        semantic_turn = FrozenTurn(
            scope=scope,
            event_id=frozen_turn.event_id,
            turn_id=frozen_turn.turn_id,
            base_revision=cursor_revision,
            observed_at_ms=frozen_turn.observed_at_ms,
        )
        try:
            nonce_digest = make_request_nonce_digest(scope, semantic_turn)
            adapted_summary = adapt_native_context_summary_v1(
                native_summary,
                scope=scope,
                nonce_digest=nonce_digest,
                estimator_formula_digest=ESTIMATOR_FORMULA_DIGEST,
            )
            binding = ContextBindingV1.from_json(adapted_summary["binding"])
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            return self._semantic_failure("NATIVE_MALFORMED")

        try:
            estimate = await estimate_context_bound(
                estimator,
                request_text,
                binding=binding,
                summary=adapted_summary,
            )
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except SemanticEstimateError as exc:
            return self._semantic_failure(exc.code, cause_code=exc.subcode)
        except BaseException:
            return self._semantic_failure("ESTIMATOR_UNAVAILABLE")

        try:
            proposal = build_perception_proposal_v3(
                scope=scope,
                turn=semantic_turn,
                estimate=estimate,
                base_revision=cursor_revision,
                nonce_digest=nonce_digest,
            )
        except SemanticProposalError as exc:
            return self._semantic_failure(exc.code)
        except BaseException:
            return self._semantic_failure("ESTIMATOR_MALFORMED")

        closure = self._bridge.apply_perception_proposal_v1(scope, proposal)
        if type(closure) is not dict:
            return self._semantic_failure("NATIVE_MALFORMED")
        if closure.get("status") == "DEGRADED":
            return self._semantic_failure(
                str(closure.get("code", "NATIVE_ERROR")),
                native_stage="NATIVE_APPLY",
                state_subcode=closure.get("state_subcode"),
            )
        if closure.get("schema") not in {
            "astrembodiment.semantic-perception-closure.v1",
            "astrembodiment.semantic-perception-closure.v2",
        }:
            return self._semantic_failure("NATIVE_MALFORMED")
        vector = closure.get("semantic_vector_receipt")
        if (
            closure.get("full_vector_state") != "FULL_VECTOR_CONFIRMED"
            or closure.get("node_observability_state") != "CONFIRMED"
            or type(vector) is not dict
            or vector.get("dimension_slot_count") != 15
            or vector.get("evaluated_dimension_count") != 15
            or vector.get("injected_dimension_count") != 15
            or vector.get("unavailable_dimension_count") != 0
        ):
            return self._semantic_failure("SEMANTIC_VECTOR_UNAVAILABLE")
        if closure.get("expression_projection") is None:
            return self._semantic_failure("EXPRESSION_PROJECTION_UNAVAILABLE")
        return {
            "status": "DEGRADED",
            "code": "HUMAN_GOLD_UNVERIFIED",
            "calibration_state": "UNVERIFIED_HUMAN_GOLD",
            "dimensions_fxp6": dict(proposal["dimensions"]),
            "estimator_confidence_fxp6": proposal["estimator_confidence"],
            "semantic_closure": copy.deepcopy(closure),
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
        if not isinstance(decision, dict):
            raise ContextProjectionIntegrity(
                "CONTEXT_PROJECTION", "native decision must be an object"
            )
        if decision.get("schema") == "astrembodiment.decision.v1":
            summary = decision.get("context_summary")
            if summary is None:
                raise ContextProjectionIntegrity(
                    "CONTEXT_PROJECTION",
                    "native decision is missing its committed context summary",
                )
            decision = dict(decision)
            decision["context_summary"] = validate_context_summary_payload(summary)
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
        self._semantic_inflight.clear()
        self._semantic_results.clear()
