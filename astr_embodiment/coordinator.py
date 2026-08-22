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
    DIMENSION_NAMES,
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
_CALCULATION_RESIDUAL_NAMES = (
    "authority",
    "continuity",
    "energy",
    "renormalization",
    "capacity",
)


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
    def _preflight_estimate_values(
        estimate: SemanticEstimate | None,
    ) -> tuple[dict[str, int] | None, int | None]:
        """Project only a parsed closed estimate into observatory-safe values."""

        if type(estimate) is not SemanticEstimate:
            return None, None
        try:
            dimensions = {name: estimate.dimensions[name] for name in DIMENSION_NAMES}
            confidence = estimate.estimator_confidence
            if (
                set(estimate.dimensions) != set(DIMENSION_NAMES)
                or type(confidence) is not int
                or not 1 <= confidence <= 1_000_000
                or any(
                    type(value) is not int or not 0 <= value <= 1_000_000
                    for value in dimensions.values()
                )
            ):
                raise ValueError("closed estimate")
        except BaseException:
            return None, None
        return dimensions, confidence

    @staticmethod
    def _preflight_calculation(receipt: Any) -> dict[str, Any] | None:
        """Project a validated native receipt into content-free result values."""

        if type(receipt) is not dict:
            return None
        try:
            state_before = receipt["state_before"]
            state_after = receipt["state_after"]
            if type(state_before) is not str or type(state_after) is not str:
                raise ValueError("state digest")
            if len(state_before) != 64 or len(state_after) != 64:
                raise ValueError("state digest")
            bytes.fromhex(state_before)
            bytes.fromhex(state_after)
            if state_before == state_after:
                raise ValueError("unchanged state")
            active_nodes = receipt["active_nodes"]
            active_edges = receipt["active_edges"]
            if type(active_nodes) is not int or active_nodes < 0:
                raise ValueError("active nodes")
            if type(active_edges) is not int or active_edges < 0:
                raise ValueError("active edges")
            residuals = receipt["residuals"]
            if type(residuals) is not dict or set(residuals) != set(
                _CALCULATION_RESIDUAL_NAMES
            ):
                raise ValueError("residuals")
            closed_residuals: dict[str, int] = {}
            for name in _CALCULATION_RESIDUAL_NAMES:
                value = residuals[name]
                if type(value) is not int or not -(1 << 63) <= value <= (1 << 63) - 1:
                    raise ValueError("residual")
                closed_residuals[name] = value
        except BaseException:
            return None
        return {
            "state_changed": True,
            "active_nodes": active_nodes,
            "active_edges": active_edges,
            "residuals_fxp6": closed_residuals,
        }

    @classmethod
    def _preflight_diagnostic(
        cls,
        *,
        stage: str,
        commit_state: str,
        values_state: str,
        estimate: SemanticEstimate | None = None,
        base_revision: int | None = None,
        revision: int | None = None,
        deduplicated: bool | None = None,
        receipt_status: str | None = None,
        receipt: Any = None,
    ) -> dict[str, Any]:
        dimensions, confidence = cls._preflight_estimate_values(estimate)
        if dimensions is None:
            values_state = "UNAVAILABLE"
        native_calculation = cls._preflight_calculation(receipt)
        if native_calculation is not None:
            calculation_state = "CONFIRMED"
        elif commit_state in {"UNKNOWN", "CONFIRMED_NEW", "CONFIRMED_EXISTING"}:
            calculation_state = "UNCONFIRMED"
        else:
            calculation_state = "NOT_ATTEMPTED"
        return {
            "stage": stage,
            "commit_state": commit_state,
            "values_state": values_state,
            "dimensions_fxp6": dimensions,
            "estimator_confidence_fxp6": confidence,
            "base_revision": base_revision,
            "revision": revision,
            "deduplicated": deduplicated,
            "receipt_status": receipt_status,
            "calculation_state": calculation_state,
            "native_calculation": native_calculation,
        }

    @classmethod
    def _preflight_failure(
        cls,
        code: str,
        *,
        stage: str = "INTERNAL",
        commit_state: str = "UNKNOWN",
        estimate: SemanticEstimate | None = None,
        base_revision: int | None = None,
    ) -> dict[str, Any]:
        return {
            "status": SEMANTIC_DEGRADED,
            "code": code,
            "diagnostic": cls._preflight_diagnostic(
                stage=stage,
                commit_state=commit_state,
                values_state="ESTIMATED_NOT_CONFIRMED",
                estimate=estimate,
                base_revision=base_revision,
            ),
        }

    @classmethod
    def _preflight_noop(
        cls,
        code: str,
        *,
        stage: str,
        estimate: SemanticEstimate | None = None,
    ) -> dict[str, Any]:
        return {
            "status": SEMANTIC_NOOP,
            "code": code,
            "diagnostic": cls._preflight_diagnostic(
                stage=stage,
                commit_state="NOT_ATTEMPTED",
                values_state="ESTIMATED_NOT_COMMITTED",
                estimate=estimate,
            ),
        }

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
            return self._preflight_failure(
                "INVALID_TURN", stage="INPUT", commit_state="NOT_ATTEMPTED"
            )
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
                return copy.deepcopy(self._preflight_failure("ESTIMATOR_UNAVAILABLE"))
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
            return self._preflight_failure(
                "ESTIMATOR_MALFORMED", stage="INPUT", commit_state="NOT_ATTEMPTED"
            )
        if not request_text.strip():
            return self._preflight_noop("EMPTY_REQUEST", stage="INPUT")
        try:
            raw_estimate = await self._invoke_preflight_estimator(
                estimator, request_text
            )
        except asyncio.CancelledError:
            return self._preflight_failure(
                "ESTIMATOR_UNAVAILABLE",
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
            )
        except SemanticEstimateError as exc:
            return self._preflight_failure(
                exc.code
                if exc.code in {"ESTIMATOR_MALFORMED", "ESTIMATOR_UNAVAILABLE"}
                else "ESTIMATOR_UNAVAILABLE",
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
            )
        except BaseException:
            # Provider exception details are deliberately discarded.
            return self._preflight_failure(
                "ESTIMATOR_UNAVAILABLE",
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
            )

        try:
            estimate = (
                parse_estimator_output(raw_estimate.as_json())
                if isinstance(raw_estimate, SemanticEstimate)
                else parse_estimator_output(raw_estimate)
            )
        except BaseException:
            return self._preflight_failure(
                "ESTIMATOR_MALFORMED",
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
            )
        try:
            is_load_noop = estimate.is_load_noop
        except BaseException:
            return self._preflight_failure(
                "ESTIMATOR_MALFORMED",
                stage="ESTIMATOR",
                commit_state="NOT_ATTEMPTED",
            )
        if is_load_noop:
            # This is a fixed no-op: no nonce, cursor read, or native commit.
            return self._preflight_noop(
                "ZERO_LOAD", stage="ESTIMATOR", estimate=estimate
            )

        # The semantic cursor is content-free.  It is read before the proposal
        # base revision is frozen, so G0's separate revision lane is untouched.
        try:
            cursor = self._bridge.semantic_revision_v1(scope.scope_json())
        except BaseException:
            return self._preflight_failure(
                "NATIVE_ERROR",
                stage="CURSOR",
                commit_state="NOT_ATTEMPTED",
                estimate=estimate,
            )
        if type(cursor) is not dict:
            return self._preflight_failure(
                "NATIVE_MALFORMED",
                stage="CURSOR",
                commit_state="NOT_ATTEMPTED",
                estimate=estimate,
            )
        if cursor.get("status") == SEMANTIC_DEGRADED:
            return self._preflight_failure(
                cursor.get("code")
                if type(cursor.get("code")) is str
                else "NATIVE_ERROR",
                stage="CURSOR",
                commit_state="NOT_ATTEMPTED",
                estimate=estimate,
            )
        if (
            set(cursor) != {"schema", "revision"}
            or cursor.get("schema") != "astrembodiment.semantic-revision.v1"
        ):
            return self._preflight_failure(
                "NATIVE_MALFORMED",
                stage="CURSOR",
                commit_state="NOT_ATTEMPTED",
                estimate=estimate,
            )
        revision = cursor.get("revision")
        if type(revision) is not int or revision < 0:
            return self._preflight_failure(
                "NATIVE_MALFORMED",
                stage="CURSOR",
                commit_state="NOT_ATTEMPTED",
                estimate=estimate,
            )

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
            return self._preflight_failure(
                "INVALID_PROPOSAL",
                stage="PROPOSAL",
                commit_state="NOT_ATTEMPTED",
                estimate=estimate,
            )

        try:
            native_result = self._bridge.apply_perception_proposal_v1(
                scope.scope_json(), proposal_json
            )
        except BaseException:
            return self._preflight_failure(
                "NATIVE_ERROR",
                stage="NATIVE_APPLY",
                estimate=estimate,
                base_revision=revision,
            )
        if type(native_result) is not dict:
            return self._preflight_failure(
                "NATIVE_MALFORMED",
                stage="RECEIPT",
                estimate=estimate,
                base_revision=revision,
            )
        if native_result.get("status") == SEMANTIC_DEGRADED:
            return self._preflight_failure(
                native_result.get("code")
                if type(native_result.get("code")) is str
                else "NATIVE_ERROR",
                stage="NATIVE_APPLY",
                estimate=estimate,
                base_revision=revision,
            )
        if native_result == {"status": SEMANTIC_NOOP, "code": "ZERO_LOAD"}:
            return self._preflight_noop(
                "ZERO_LOAD", stage="ESTIMATOR", estimate=estimate
            )
        try:
            native_result = validate_semantic_result(
                native_result,
                expected_base_revision=proposal["base_revision"],
            )
        except BaseException:
            return self._preflight_failure(
                "NATIVE_MALFORMED",
                stage="RECEIPT",
                estimate=estimate,
                base_revision=revision,
            )
        return {
            "status": SEMANTIC_SUCCESS,
            "code": "SEMANTIC_COMMITTED",
            "proposal": proposal,
            "result": copy.deepcopy(native_result),
            "diagnostic": self._preflight_diagnostic(
                stage="RECEIPT",
                commit_state=(
                    "CONFIRMED_EXISTING"
                    if native_result["deduplicated"]
                    else "CONFIRMED_NEW"
                ),
                values_state="COMMITTED",
                estimate=estimate,
                base_revision=proposal["base_revision"],
                revision=native_result["revision"],
                deduplicated=native_result["deduplicated"],
                receipt_status=native_result["receipt"]["status"],
                receipt=native_result["receipt"],
            ),
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
