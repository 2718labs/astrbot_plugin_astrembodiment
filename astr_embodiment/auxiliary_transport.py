"""Closed, request-scoped AstrBot transport for auxiliary model calls.

The module deliberately owns only Provider binding and Host invocation.  It
does not retain a Provider object, prompt, response text, or request history
after a request context is closed.
"""

from __future__ import annotations

import asyncio
import inspect
import math
import time
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any


DEFAULT_SEMANTIC_ESTIMATOR_TIMEOUT_MS = 8_000
MIN_SEMANTIC_ESTIMATOR_TIMEOUT_MS = 1_000
MAX_SEMANTIC_ESTIMATOR_TIMEOUT_MS = 15_000

TRANSPORT_SUBCODES = frozenset(
    {
        "NOT_APPLICABLE",
        "NONE",
        "PROVIDER_RESOLUTION_FAILED",
        "PROVIDER_NOT_FOUND",
        "HOST_API_UNAVAILABLE",
        "PROVIDER_CALL_TIMEOUT",
        "PROVIDER_CALL_FAILED",
        "RESPONSE_TEXT_UNAVAILABLE",
        "UNKNOWN_TRANSPORT_FAILURE",
    }
)
_RETRYABLE_SUBCODES = frozenset({"PROVIDER_CALL_TIMEOUT", "PROVIDER_CALL_FAILED"})
_BINDING_SOURCES = frozenset({"CONFIGURED", "LEGACY_COMPAT", "CURRENT_SESSION"})


def _closed_subcode(value: object) -> str:
    return (
        value
        if type(value) is str and value in TRANSPORT_SUBCODES
        else "UNKNOWN_TRANSPORT_FAILURE"
    )


@dataclass(frozen=True, slots=True, repr=False)
class AuxiliaryProviderBindingV1:
    """One resolved Provider identity, fixed for exactly one request context."""

    provider_id: str = field(repr=False)
    source: str
    request_key: str = field(repr=False)
    validated: bool = True

    def __post_init__(self) -> None:
        if (
            type(self.provider_id) is not str
            or not self.provider_id.strip()
            or self.source not in _BINDING_SOURCES
            or type(self.request_key) is not str
            or not self.request_key
            or self.validated is not True
        ):
            raise ValueError("invalid auxiliary provider binding")


@dataclass(frozen=True, slots=True)
class AuxiliaryTransportMetaV1:
    """Closed non-content transport metadata safe for observability."""

    transport_subcode: str
    attempted: bool
    attempt_count: int

    def __post_init__(self) -> None:
        if (
            self.transport_subcode not in TRANSPORT_SUBCODES
            or type(self.attempted) is not bool
            or type(self.attempt_count) is not int
            or self.attempt_count not in {0, 1, 2}
            or (not self.attempted and self.attempt_count != 0)
            or (self.attempted and self.attempt_count == 0)
        ):
            raise ValueError("invalid auxiliary transport metadata")

    def as_json(self) -> dict[str, str | bool | int]:
        return {
            "transport_subcode": self.transport_subcode,
            "attempted": self.attempted,
            "attempt_count": self.attempt_count,
        }


def not_applicable_transport_meta() -> AuxiliaryTransportMetaV1:
    return AuxiliaryTransportMetaV1("NOT_APPLICABLE", False, 0)


def normalized_transport_meta(
    transport_subcode: object,
    attempted: object,
    attempt_count: object,
) -> AuxiliaryTransportMetaV1:
    """Return the only safe observability projection for untrusted metadata."""

    try:
        return AuxiliaryTransportMetaV1(
            _closed_subcode(transport_subcode),
            attempted,
            attempt_count,
        )
    except (TypeError, ValueError):
        return AuxiliaryTransportMetaV1("UNKNOWN_TRANSPORT_FAILURE", False, 0)


@dataclass(frozen=True, slots=True, repr=False)
class AuxiliaryTransportResultV1:
    """Private successful transport result; text must not enter normal logs."""

    text: str = field(repr=False)
    meta: AuxiliaryTransportMetaV1

    def __post_init__(self) -> None:
        if type(self.text) is not str or not self.text:
            raise ValueError("invalid auxiliary transport result")
        if (
            type(self.meta) is not AuxiliaryTransportMetaV1
            or self.meta.transport_subcode != "NONE"
            or self.meta.attempted is not True
            or self.meta.attempt_count not in {1, 2}
        ):
            raise ValueError("invalid auxiliary transport success metadata")


class AuxiliaryTransportError(RuntimeError):
    """Non-echoing transport error for the semantic consumer boundary."""

    def __init__(self, meta: AuxiliaryTransportMetaV1) -> None:
        self.meta = meta
        super().__init__("ESTIMATOR_UNAVAILABLE")


class RequestTransportContext:
    """Ephemeral request-local binding and unavailable sentinel."""

    __slots__ = (
        "_binding",
        "_closed",
        "_request_key",
        "_resolution_error",
        "_transport",
        "_umo",
    )

    def __init__(self, transport: AuxiliaryProviderTransport, *, umo: Any) -> None:
        self._transport = transport
        self._umo = umo
        self._request_key: str | None = None
        self._binding: AuxiliaryProviderBindingV1 | None = None
        self._resolution_error: AuxiliaryTransportMetaV1 | None = None
        self._closed = False

    def bind_semantic_key(self, request_key: str) -> None:
        """Bind the exact semantic key before any auxiliary invocation."""

        if type(request_key) is not str or not request_key:
            self._resolution_error = AuxiliaryTransportMetaV1(
                "UNKNOWN_TRANSPORT_FAILURE", False, 0
            )
            return
        if self._request_key is None:
            self._request_key = request_key
            return
        if self._request_key != request_key:
            self._binding = None
            self._resolution_error = AuxiliaryTransportMetaV1(
                "UNKNOWN_TRANSPORT_FAILURE", False, 0
            )

    def is_bound_to_semantic_key(self, request_key: str) -> bool:
        """Whether this open context already owns exactly ``request_key``."""

        return (
            not self._closed
            and type(request_key) is str
            and self._request_key == request_key
            and self._resolution_error is None
        )

    def close(self) -> None:
        """Clear every request-local reference without touching other requests."""

        self._binding = None
        self._resolution_error = None
        self._request_key = None
        self._umo = None
        self._closed = True

    async def generate(
        self,
        *,
        prompt: str,
        system_prompt: str,
        semantic_operation: bool,
    ) -> AuxiliaryTransportResultV1:
        if type(prompt) is not str or type(system_prompt) is not str:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("UNKNOWN_TRANSPORT_FAILURE", False, 0)
            )
        binding = await self._binding_for_request()
        if semantic_operation:
            return await self._transport._generate_semantic(
                binding=binding,
                prompt=prompt,
                system_prompt=system_prompt,
            )
        return await self._transport._generate_once(
            binding=binding,
            prompt=prompt,
            system_prompt=system_prompt,
            attempt_count=1,
            deadline=None,
        )

    async def _binding_for_request(self) -> AuxiliaryProviderBindingV1:
        if self._closed or self._request_key is None:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("UNKNOWN_TRANSPORT_FAILURE", False, 0)
            )
        if self._resolution_error is not None:
            raise AuxiliaryTransportError(self._resolution_error)
        if self._binding is not None:
            return self._binding
        try:
            binding = await self._transport._resolve_binding(
                request_key=self._request_key,
                umo=self._umo,
            )
        except AuxiliaryTransportError as exc:
            self._binding = None
            self._resolution_error = exc.meta
            raise
        self._binding = binding
        return binding


class AuxiliaryProviderTransport:
    """The sole AstrBot adapter for auxiliary-model transport in one plugin."""

    def __init__(
        self,
        *,
        context: Any,
        configured_provider: Callable[[], tuple[str, str]],
        timeout_ms: Callable[[], int],
    ) -> None:
        self._context = context
        self._configured_provider = configured_provider
        self._timeout_ms = timeout_ms

    def open_request(self, *, umo: Any) -> RequestTransportContext:
        return RequestTransportContext(self, umo=umo)

    async def _resolve_binding(
        self,
        *,
        request_key: str,
        umo: Any,
    ) -> AuxiliaryProviderBindingV1:
        try:
            configured_id, source = self._configured_provider()
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
            ) from None

        provider_id = configured_id
        if source == "CURRENT_SESSION":
            try:
                get_current = getattr(
                    self._context, "get_current_chat_provider_id", None
                )
            except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
                raise
            except BaseException:
                raise AuxiliaryTransportError(
                    AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
                ) from None
            if not callable(get_current):
                raise AuxiliaryTransportError(
                    AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
                )
            try:
                provider_id = await self._maybe_await(get_current(umo=umo))
            except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
                raise
            except BaseException:
                raise AuxiliaryTransportError(
                    AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
                ) from None

        if (
            source not in _BINDING_SOURCES
            or type(provider_id) is not str
            or not provider_id.strip()
        ):
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
            )
        provider_id = provider_id.strip()
        try:
            get_provider = getattr(self._context, "get_provider_by_id", None)
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
            ) from None
        if not callable(get_provider):
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
            )
        try:
            provider = await self._maybe_await(get_provider(provider_id))
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_RESOLUTION_FAILED", False, 0)
            ) from None
        if provider is None:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_NOT_FOUND", False, 0)
            )
        try:
            return AuxiliaryProviderBindingV1(
                provider_id=provider_id,
                source=source,
                request_key=request_key,
            )
        except ValueError:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("UNKNOWN_TRANSPORT_FAILURE", False, 0)
            ) from None

    async def _generate_semantic(
        self,
        *,
        binding: AuxiliaryProviderBindingV1,
        prompt: str,
        system_prompt: str,
    ) -> AuxiliaryTransportResultV1:
        timeout_ms = self._validated_timeout_ms()
        started = time.monotonic()
        deadline = started + timeout_ms / 1_000
        first_cap = math.ceil(2 * timeout_ms / 3) / 1_000
        first_deadline = min(deadline, started + first_cap)
        try:
            return await self._generate_once(
                binding=binding,
                prompt=prompt,
                system_prompt=system_prompt,
                attempt_count=1,
                deadline=first_deadline,
            )
        except AuxiliaryTransportError as first_error:
            if first_error.meta.transport_subcode not in _RETRYABLE_SUBCODES:
                raise
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise
            return await self._generate_once(
                binding=binding,
                prompt=prompt,
                system_prompt=system_prompt,
                attempt_count=2,
                deadline=deadline,
            )

    async def _generate_once(
        self,
        *,
        binding: AuxiliaryProviderBindingV1,
        prompt: str,
        system_prompt: str,
        attempt_count: int,
        deadline: float | None,
    ) -> AuxiliaryTransportResultV1:
        try:
            generate = getattr(self._context, "llm_generate", None)
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("HOST_API_UNAVAILABLE", False, 0)
            ) from None
        if not callable(generate):
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("HOST_API_UNAVAILABLE", False, 0)
            )
        try:
            call_kwargs = {
                "chat_provider_id": binding.provider_id,
                "prompt": prompt,
                "system_prompt": system_prompt,
                "tools": None,
            }
            if inspect.iscoroutinefunction(generate):
                generated = generate(**call_kwargs)
            else:
                # to_thread propagates the request's contextvars while letting
                # the event loop return at deadline if a sync Host call stalls.
                generated = await self._await_with_deadline(
                    asyncio.to_thread(generate, **call_kwargs), deadline=deadline
                )
            if inspect.isawaitable(generated):
                result = await self._await_with_deadline(generated, deadline=deadline)
            else:
                result = generated
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except TimeoutError:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_CALL_TIMEOUT", True, attempt_count)
            ) from None
        except BaseException:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_CALL_FAILED", True, attempt_count)
            ) from None

        if deadline is not None and time.monotonic() >= deadline:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1("PROVIDER_CALL_TIMEOUT", True, attempt_count)
            )

        text = self._canonical_response_text(result)
        if text is None:
            raise AuxiliaryTransportError(
                AuxiliaryTransportMetaV1(
                    "RESPONSE_TEXT_UNAVAILABLE", True, attempt_count
                )
            )
        return AuxiliaryTransportResultV1(
            text=text,
            meta=AuxiliaryTransportMetaV1("NONE", True, attempt_count),
        )

    def _validated_timeout_ms(self) -> int:
        try:
            value = self._timeout_ms()
        except Exception:
            return DEFAULT_SEMANTIC_ESTIMATOR_TIMEOUT_MS
        if (
            type(value) is not int
            or not MIN_SEMANTIC_ESTIMATOR_TIMEOUT_MS
            <= value
            <= MAX_SEMANTIC_ESTIMATOR_TIMEOUT_MS
        ):
            return DEFAULT_SEMANTIC_ESTIMATOR_TIMEOUT_MS
        return value

    @staticmethod
    async def _await_with_deadline(awaitable: Any, *, deadline: float | None) -> Any:
        if deadline is None:
            return await awaitable
        return await asyncio.wait_for(
            awaitable,
            timeout=max(0.0, deadline - time.monotonic()),
        )

    @staticmethod
    async def _maybe_await(value: Any) -> Any:
        if inspect.isawaitable(value):
            return await value
        return value

    @staticmethod
    def _canonical_response_text(result: Any) -> str | None:
        if type(result) is str:
            return result if result else None
        try:
            completion_text = getattr(result, "completion_text", None)
        except (asyncio.CancelledError, KeyboardInterrupt, SystemExit):
            raise
        except BaseException:
            return None
        return (
            completion_text
            if type(completion_text) is str and completion_text
            else None
        )
