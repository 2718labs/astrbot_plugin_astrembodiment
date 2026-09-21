"""Genesis singleflight coordinator and delivery fact adapter.

In-process singleflight: concurrent first turns for the same
(Bot, Persona, persona_source_digest) join one compiler Future, so the main
provider is called exactly once. A compiler failure never creates a default
brain and never writes production state.
"""

from __future__ import annotations

import asyncio
import copy
import secrets
from collections.abc import Awaitable, Callable
from typing import Any

from .bridge import (
    GenesisUnavailable,
    NativeBridge,
    RetryWait,
)
from .contracts import (
    ScopeTokens,
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
        self._persona_locks: dict[tuple[str, str], asyncio.Lock] = {}

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
                key = (scope.bot_token, scope.persona_token)
                async with self._persona_locks.setdefault(key, asyncio.Lock()):
                    return self._bridge.ensure_genesis(closed_request)
            except RetryWait:
                await asyncio.sleep(_RETRY_WAIT_DELAY_S)
        raise GenesisUnavailable(
            "GENESIS_UNAVAILABLE",
            "genesis lease stayed in flight; no default brain was created",
        )

    def reset(self) -> None:
        self._inflight.clear()
        self._committed.clear()
