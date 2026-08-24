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
from collections.abc import Awaitable, Callable
from typing import Any

from .bridge import (
    ContextProjectionIntegrity,
    GenesisUnavailable,
    NativeBridge,
    RetryWait,
    validate_context_summary_payload,
)
from .contracts import (
    ScopeTokens,
    build_delivery_outcome_json,
    build_user_stimulus_json,
)
from .persona_genesis import (
    PersonaCompilerMalformed,
    PersonaSourceSnapshot,
    build_closed_request,
    validate_proposal,
)

FORMULA_DIGEST = "00" * 32  # placeholder; G2 fills the real FormulaProfile digest

Compiler = Callable[[PersonaSourceSnapshot], Awaitable[dict[str, Any]]]

_RETRY_WAIT_ATTEMPTS = 40
_RETRY_WAIT_DELAY_S = 0.05


class GenesisCoordinator:
    """Owns no brain state: only in-flight futures and turn bookkeeping."""

    def __init__(self, bridge: NativeBridge) -> None:
        self._bridge = bridge
        self._inflight: dict[str, asyncio.Future] = {}
        self._committed: dict[str, dict[str, Any]] = {}
        self._applied: dict[str, dict[str, Any]] = {}

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
