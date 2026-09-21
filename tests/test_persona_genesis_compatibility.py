from __future__ import annotations

import asyncio
import copy
import json
from types import SimpleNamespace
from typing import Any

import pytest

from astr_embodiment.contracts import ScopeTokens
from astr_embodiment.coordinator import GenesisCoordinator
from astr_embodiment.persona_genesis import (
    PersonaCompilerMalformed,
    PersonaSourceSnapshot,
    compile_with_provider,
)


def _proposal_from_prompt(prompt: str) -> dict[str, Any]:
    template = prompt.split("Target template:\n", 1)[1].split(
        "\nPersona source data", 1
    )[0]
    return json.loads(template)


class _ScriptedCompilerProvider:
    def __init__(self, response_kind: str) -> None:
        self.response_kind = response_kind
        self.calls: list[dict[str, str]] = []

    async def generate(self, *, prompt: str, system_prompt: str) -> Any:
        self.calls.append({"prompt": prompt, "system_prompt": system_prompt})
        # Give a concurrent coordinator waiter a chance to join the real
        # singleflight Future. This also makes the failure Future observable.
        await asyncio.sleep(0)
        if self.response_kind == "non_json":
            completion = "this is not JSON"
        else:
            proposal = _proposal_from_prompt(prompt)
            if self.response_kind == "identity_smuggling":
                proposal["seed_code"] = "PROVIDER-MUST-NOT-CHOOSE-IDENTITY"
            elif self.response_kind != "valid":
                raise AssertionError(f"unknown response kind: {self.response_kind}")
            completion = json.dumps(proposal, ensure_ascii=False, sort_keys=True)
        return SimpleNamespace(completion_text=completion)


class _RecordingNativeBridge:
    def __init__(self, receipts: dict[str, dict[str, Any]] | None = None) -> None:
        self.receipts = receipts or {}
        self.ensure_requests: list[dict[str, Any]] = []

    def ensure_genesis(self, closed_request: dict[str, Any]) -> dict[str, Any]:
        self.ensure_requests.append(copy.deepcopy(closed_request))
        persona_token = closed_request["source"]["scope"]["persona_token"]
        return copy.deepcopy(self.receipts[persona_token])


def _source(
    persona_id: str,
    *,
    selection: str,
    tool: str,
) -> PersonaSourceSnapshot:
    return PersonaSourceSnapshot.freeze(
        persona_id=persona_id,
        persona={
            "prompt": "稳定、诚实，并尊重边界。",
            "begin_dialogs": ["你好。"],
            "mood_imitation_dialogs": ["我会先核实事实。"],
            "tools": [tool],
            "skills": [f"{tool}-skill"],
            "custom_error_message": "现在无法回答。",
        },
        selection=selection,
    )


def _compiler(provider: _ScriptedCompilerProvider):
    async def compile(source: PersonaSourceSnapshot) -> dict[str, Any]:
        return await compile_with_provider(generate=provider.generate, source=source)

    return compile


@pytest.mark.parametrize("response_kind", ["non_json", "identity_smuggling"])
def test_genesis_provider_output_still_requires_closed_validation_and_native_authority(
    response_kind: str,
) -> None:
    provider = _ScriptedCompilerProvider(response_kind)
    native = _RecordingNativeBridge()
    coordinator = GenesisCoordinator(native)  # type: ignore[arg-type]
    scope = ScopeTokens("01" * 16, "02" * 16, "03" * 16)
    source = _source(
        "existing-persona-malformed",
        selection="conversation",
        tool="private-tool",
    )

    async def run() -> tuple[BaseException, BaseException]:
        # Both callers use the real coordinator/compiler path. One malformed
        # compile plus its single closed-schema repair is shared by both.
        first, joined = await asyncio.gather(
            coordinator.ensure_genesis(
                scope=scope,
                source=source,
                selection="conversation",
                compiler=_compiler(provider),
                compiler_protocol_digest="11" * 32,
                compiler_model_digest="12" * 32,
                observed_at_ms=1_700_000_000_000,
            ),
            coordinator.ensure_genesis(
                scope=scope,
                source=source,
                selection="conversation",
                compiler=_compiler(provider),
                compiler_protocol_digest="11" * 32,
                compiler_model_digest="12" * 32,
                observed_at_ms=1_700_000_000_000,
            ),
            return_exceptions=True,
        )
        assert isinstance(first, BaseException)
        assert isinstance(joined, BaseException)
        return first, joined

    first_error, joined_error = asyncio.run(run())

    assert isinstance(first_error, PersonaCompilerMalformed)
    assert isinstance(joined_error, PersonaCompilerMalformed)
    assert len(provider.calls) == 2  # initial compile + one bounded repair
    assert native.ensure_requests == []


def test_valid_genesis_is_persona_global_but_identity_isolated_and_native_authored() -> None:
    persona_a = "existing-persona-a"
    persona_b = "existing-persona-b"
    source_a = _source(persona_a, selection="conversation", tool="calendar-a")
    source_b = _source(persona_b, selection="session_forced", tool="calendar-b")
    scope_a_first = ScopeTokens("21" * 16, "31" * 16, "41" * 16)
    scope_a_other_session = ScopeTokens("21" * 16, "31" * 16, "42" * 16)
    scope_b = ScopeTokens("21" * 16, "32" * 16, "43" * 16)
    receipt_a = {
        "seed_code": "AE-S1-NATIVE-PERSONA-A",
        "incarnation_id": "AE-I1-NATIVE-PERSONA-A",
        "manifest_digest": "a1" * 32,
    }
    receipt_b = {
        "seed_code": "AE-S1-NATIVE-PERSONA-B",
        "incarnation_id": "AE-I1-NATIVE-PERSONA-B",
        "manifest_digest": "b2" * 32,
    }
    native = _RecordingNativeBridge(
        {
            scope_a_first.persona_token: receipt_a,
            scope_b.persona_token: receipt_b,
        }
    )
    provider = _ScriptedCompilerProvider("valid")
    coordinator = GenesisCoordinator(native)  # type: ignore[arg-type]

    async def run() -> tuple[dict[str, Any], dict[str, Any], dict[str, Any]]:
        first = await coordinator.ensure_genesis(
            scope=scope_a_first,
            source=source_a,
            selection="conversation",
            compiler=_compiler(provider),
            compiler_protocol_digest="51" * 32,
            compiler_model_digest="52" * 32,
            observed_at_ms=1_700_000_000_000,
        )
        same_persona_new_session = await coordinator.ensure_genesis(
            scope=scope_a_other_session,
            source=source_a,
            selection="conversation",
            compiler=_compiler(provider),
            compiler_protocol_digest="51" * 32,
            compiler_model_digest="52" * 32,
            observed_at_ms=1_700_000_000_001,
        )
        other_persona = await coordinator.ensure_genesis(
            scope=scope_b,
            source=source_b,
            selection="session_forced",
            compiler=_compiler(provider),
            compiler_protocol_digest="51" * 32,
            compiler_model_digest="52" * 32,
            observed_at_ms=1_700_000_000_002,
        )
        return first, same_persona_new_session, other_persona

    first, same_persona_new_session, other_persona = asyncio.run(run())

    assert first == same_persona_new_session == receipt_a
    assert other_persona == receipt_b
    assert first["seed_code"] == "AE-S1-NATIVE-PERSONA-A"
    assert other_persona["seed_code"] == "AE-S1-NATIVE-PERSONA-B"
    assert len(provider.calls) == len(native.ensure_requests) == 2

    # Existing persona IDs, selections and capabilities remain provenance and
    # scope facts; they never become compiler-inferred phenotype or identity.
    assert source_a.compiler_payload() == source_b.compiler_payload()
    assert source_a.source_digest != source_b.source_digest
    assert source_a.capability_digest != source_b.capability_digest
    for call in provider.calls:
        assert persona_a not in call["prompt"]
        assert persona_b not in call["prompt"]
        assert "calendar-a" not in call["prompt"]
        assert "calendar-b" not in call["prompt"]

    request_a, request_b = native.ensure_requests
    assert request_a["source"]["scope"]["persona_token"] == scope_a_first.persona_token
    assert request_b["source"]["scope"]["persona_token"] == scope_b.persona_token
    assert request_a["source"]["source_digest"] == source_a.source_digest
    assert request_b["source"]["source_digest"] == source_b.source_digest
    for closed_request in native.ensure_requests:
        proposal = closed_request["proposal"]
        assert "seed_code" not in proposal
        assert "incarnation_id" not in proposal
