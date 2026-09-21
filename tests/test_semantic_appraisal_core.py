from __future__ import annotations

import json

import pytest

from astr_embodiment.contracts import ScopeTokens
from astr_embodiment.interaction import build_core_inbound_request
from astr_embodiment.semantic_contract import DIMENSION_NAMES
from astr_embodiment.semantic_estimator import (
    SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT,
    SemanticEstimateError,
    build_perception_proposal_v3,
    parse_estimator_output_v3,
)

def _estimate(*, unavailable: str | None = None) -> dict[str, object]:
    return {
        "schema": "astr-embodiment.semantic-estimate.v3",
        "dimensions": {
            name: {
                "state": "UNAVAILABLE" if name == unavailable else "PRESENT",
                "intensity_fxp6": None if name == unavailable else 250_000,
                "confidence_fxp6": 900_000,
            }
            for name in DIMENSION_NAMES
        },
    }

def test_strict_15d_parser_and_six_key_native_proposal() -> None:
    estimate = parse_estimator_output_v3(json.dumps(_estimate()))
    proposal = build_perception_proposal_v3(
        estimate=estimate,
        origin_digest="11" * 32,
        request_nonce_digest="22" * 32,
    )
    assert set(proposal) == {
        "schema_version",
        "origin_digest",
        "dimensions",
        "estimator_confidence",
        "protocol_version",
        "request_nonce_digest",
    }
    assert tuple(proposal["dimensions"]) == DIMENSION_NAMES

def test_duplicate_nan_and_unavailable_are_closed() -> None:
    duplicate = '{"schema":"astr-embodiment.semantic-estimate.v3","schema":"x","dimensions":{}}'
    with pytest.raises(SemanticEstimateError):
        parse_estimator_output_v3(duplicate)
    with pytest.raises(SemanticEstimateError):
        parse_estimator_output_v3('{"schema":NaN,"dimensions":{}}')
    with pytest.raises(SemanticEstimateError):
        parse_estimator_output_v3(f"```json\n{json.dumps(_estimate())}\n```")
    assert all(name in SEMANTIC_ESTIMATE_V3_SYSTEM_PROMPT for name in DIMENSION_NAMES)
    unavailable = parse_estimator_output_v3(_estimate(unavailable="harm"))
    with pytest.raises(SemanticEstimateError):
        build_perception_proposal_v3(
            estimate=unavailable,
            origin_digest="11" * 32,
            request_nonce_digest="22" * 32,
        )

def test_inbound_source_digest_binds_nfc_message_and_bounds_utf8() -> None:
    scope = ScopeTokens("01" * 16, "02" * 16, "03" * 16, "04" * 16)
    common = dict(
        scope=scope,
        bridge=type("Compiler", (), {"compile_core_host_request_v1": lambda self, operation, request: request})(),
        event=type("Event", (), {"platform_id": "test", "bot_id": "bot", "unified_msg_origin": "room", "message_id": "msg"})(),
        appraisal=None,
        observed_at_ms=1_700_000_000_000,
    )
    composed = build_core_inbound_request(message="café", **common)
    decomposed = build_core_inbound_request(message="cafe\u0301", **common)
    changed = build_core_inbound_request(message="different", **common)
    assert composed["observation"]["message_digest"] == decomposed["observation"]["message_digest"]
    assert composed["observation"]["message_digest"] != changed["observation"]["message_digest"]
    with pytest.raises(ValueError):
        build_core_inbound_request(message="🙂" * 2_049, **common)
