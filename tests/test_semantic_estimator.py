from __future__ import annotations

import json
from dataclasses import replace

import pytest

from astr_embodiment.contracts import FrozenTurn, ScopeTokens
from astr_embodiment.semantic_estimator import (
    DIMENSION_NAMES,
    LOAD_DIMENSIONS,
    SemanticEstimateError,
    SemanticProposalError,
    build_perception_proposal,
    make_request_nonce_digest,
    parse_estimator_output,
)


def _scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="01" * 16,
        persona_token="02" * 16,
        session_token="03" * 16,
        relation_token=None,
    )


def _turn() -> FrozenTurn:
    return FrozenTurn(
        scope=_scope(),
        turn_id="04" * 16,
        event_id="05" * 16,
        base_revision=0,
        observed_at_ms=1_700_000_000_000,
    )


def _flat_estimate(**overrides: int) -> dict[str, int]:
    values = {name: 0 for name in DIMENSION_NAMES}
    values["positive"] = 125_000
    values["harm"] = 250_000
    values["estimator_confidence"] = 900_000
    values.update(overrides)
    return values


def _nested_estimate(**overrides: int) -> dict:
    flat = _flat_estimate(**overrides)
    confidence = flat.pop("estimator_confidence")
    return {"dimensions": flat, "estimator_confidence": confidence}


def test_all_fifteen_raw_fxp6_dimensions_round_trip_as_ints() -> None:
    estimate = parse_estimator_output(_nested_estimate())

    assert tuple(estimate.dimensions) == DIMENSION_NAMES
    assert all(type(value) is int for value in estimate.dimensions.values())
    assert estimate.dimensions["positive"] == 125_000
    assert estimate.estimator_confidence == 900_000


@pytest.mark.parametrize(
    "bad",
    [
        {**_nested_estimate(), "free_text": "RAW_SENTINEL"},
        {**_nested_estimate(), "xml": "<RAW_SENTINEL/>"},
        {**_nested_estimate(), "tool": {"name": "RAW_SENTINEL"}},
        {**_nested_estimate(), "action": {"kind": "RAW_SENTINEL"}},
        {
            "dimensions": {**_nested_estimate()["dimensions"], "unknown": 1},
            "estimator_confidence": 1,
        },
    ],
)
def test_closed_estimate_rejects_control_fields_without_echoing_input(
    bad: dict,
) -> None:
    with pytest.raises(SemanticEstimateError) as error:
        parse_estimator_output(bad)

    rendered = str(error.value)
    assert "RAW_SENTINEL" not in rendered
    assert "<" not in rendered
    assert rendered == "INVALID_ESTIMATE"


@pytest.mark.parametrize(
    "field,value",
    [
        ("positive", 0.5),
        ("positive", True),
        ("positive", -1),
        ("positive", 1_000_001),
        ("estimator_confidence", 0),
        ("estimator_confidence", False),
        ("estimator_confidence", 1.5),
    ],
)
def test_numeric_boundary_is_integer_only_and_fail_closed(
    field: str, value: object
) -> None:
    bad = _nested_estimate()
    if field == "estimator_confidence":
        bad[field] = value
    else:
        bad["dimensions"][field] = value

    with pytest.raises(SemanticEstimateError, match="^INVALID_ESTIMATE$"):
        parse_estimator_output(bad)


def test_zero_vector_is_invalid_but_zero_four_load_is_a_valid_noop_candidate() -> None:
    zero = {
        "dimensions": {name: 0 for name in DIMENSION_NAMES},
        "estimator_confidence": 1,
    }
    with pytest.raises(SemanticEstimateError):
        parse_estimator_output(zero)

    other_only = {
        "dimensions": {name: 0 for name in DIMENSION_NAMES},
        "estimator_confidence": 1,
    }
    other_only["dimensions"]["affiliation"] = 1
    estimate = parse_estimator_output(other_only)
    assert all(estimate.dimensions[name] == 0 for name in LOAD_DIMENSIONS)
    assert estimate.is_load_noop


def test_json_free_text_and_nan_are_rejected_without_provider_payload_echo() -> None:
    for raw in [
        "not json RAW_SENTINEL",
        '{"dimensions": {"positive": NaN}, "estimator_confidence": 1}',
        '{"text": "<RAW_SENTINEL>", "estimator_confidence": 1}',
    ]:
        with pytest.raises(SemanticEstimateError) as error:
            parse_estimator_output(raw)
        assert str(error.value) == "INVALID_ESTIMATE"
        assert "RAW_SENTINEL" not in str(error.value)


def test_adversarial_mapping_failure_is_fixed_and_non_echoing() -> None:
    class HostileMapping(dict):
        def __iter__(self):
            raise RuntimeError("RAW_SENTINEL mapping failure")

    with pytest.raises(SemanticEstimateError) as error:
        parse_estimator_output(HostileMapping())

    assert str(error.value) == "INVALID_ESTIMATE"
    assert "RAW_SENTINEL" not in str(error.value)


def test_nonce_digest_is_nonzero_and_bound_to_opaque_turn_facts() -> None:
    first = make_request_nonce_digest(_scope(), _turn(), entropy=b"a" * 32)
    second = make_request_nonce_digest(
        _scope(), replace(_turn(), turn_id="06" * 16), entropy=b"a" * 32
    )

    assert len(first) == 64
    assert first != "00" * 32
    assert first != second


def test_proposal_contains_only_native_closed_fields_and_raw_integers() -> None:
    proposal = build_perception_proposal(
        scope=_scope(),
        turn=_turn(),
        estimate=parse_estimator_output(_nested_estimate()),
        base_revision=0,
        nonce_digest=make_request_nonce_digest(_scope(), _turn()),
    )

    assert set(proposal) == {
        "schema_version",
        "event_id",
        "turn_id",
        "observed_at_ms",
        "base_revision",
        "dimensions",
        "estimator_confidence",
        "protocol_version",
        "request_nonce_digest",
    }
    assert proposal["base_revision"] == 0
    assert all(type(value) is int for value in proposal["dimensions"].values())
    assert "RAW_SENTINEL" not in json.dumps(proposal)


def test_proposal_rejects_base_revision_that_is_not_the_frozen_turn_base() -> None:
    with pytest.raises(SemanticProposalError, match="^INVALID_PROPOSAL$"):
        build_perception_proposal(
            scope=_scope(),
            turn=_turn(),
            estimate=parse_estimator_output(_nested_estimate()),
            base_revision=1,
            nonce_digest=make_request_nonce_digest(_scope(), _turn()),
        )


def test_nonce_digest_is_deterministic_for_the_complete_frozen_binding() -> None:
    turn = _turn()
    first = make_request_nonce_digest(_scope(), turn)
    second = make_request_nonce_digest(_scope(), turn)

    assert first == second
    assert first != "aa" * 32


def test_nonce_binding_rejects_str_subclass_before_overridable_lower() -> None:
    class EvilStr(str):
        def lower(self) -> str:
            return "ab" * 16

    hostile = replace(_scope(), bot_token=EvilStr("01" * 16))

    with pytest.raises(SemanticProposalError, match="^INVALID_PROPOSAL$"):
        make_request_nonce_digest(hostile, _turn())
