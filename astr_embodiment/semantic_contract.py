"""Canonical V3 state-by-intensity contract for every semantic dimension."""

from __future__ import annotations

from collections.abc import Mapping
from types import MappingProxyType
from typing import Any


FXP6_SCALE = 1_000_000
DIMENSION_NAMES = (
    "positive",
    "affiliation",
    "harm",
    "boundary",
    "repair",
    "repetition",
    "new_information",
    "constraint_instability",
    "epistemic_conflict",
    "self_responsibility",
    "other_responsibility",
    "hostility",
    "publicness",
    "engagement",
    "rejection",
)

PRESENT = "PRESENT"
ABSENT = "ABSENT"
UNAVAILABLE = "UNAVAILABLE"
DIMENSION_STATES = (PRESENT, ABSENT, UNAVAILABLE)

STATE_INVALID = "STATE_INVALID"
INTENSITY_BOOLEAN = "INTENSITY_BOOLEAN"
INTENSITY_NULL_DISALLOWED = "INTENSITY_NULL_DISALLOWED"
INTENSITY_STRING = "INTENSITY_STRING"
INTENSITY_NON_INTEGRAL_NUMBER = "INTENSITY_NON_INTEGRAL_NUMBER"
INTENSITY_INTEGER_RANGE = "INTENSITY_INTEGER_RANGE"
INTENSITY_STATE_CONSTRAINT = "INTENSITY_STATE_CONSTRAINT"
VALUE_OTHER_TYPE = "VALUE_OTHER_TYPE"

STATE_INTENSITY_RULES: Mapping[str, Mapping[str, int | str]] = MappingProxyType(
    {
        PRESENT: MappingProxyType(
            {
                "json_type": "integer",
                "minimum": 1,
                "maximum": FXP6_SCALE,
            }
        ),
        ABSENT: MappingProxyType({"json_type": "integer", "const": 0}),
        UNAVAILABLE: MappingProxyType({"json_type": "null"}),
    }
)


def validate_state_intensity(state: Any, intensity: Any) -> str | None:
    """Return a closed failure classification, or ``None`` for one legal pair."""

    rule = STATE_INTENSITY_RULES.get(state) if type(state) is str else None
    if rule is None:
        return STATE_INVALID
    if type(intensity) is bool:
        return INTENSITY_BOOLEAN
    if intensity is None:
        return None if rule["json_type"] == "null" else INTENSITY_NULL_DISALLOWED
    if type(intensity) is str:
        return INTENSITY_STRING
    if type(intensity) is float:
        return INTENSITY_NON_INTEGRAL_NUMBER
    if type(intensity) is not int:
        return VALUE_OTHER_TYPE
    if rule["json_type"] != "integer":
        return INTENSITY_STATE_CONSTRAINT
    expected = rule.get("const")
    if expected is not None:
        return None if intensity == expected else INTENSITY_STATE_CONSTRAINT
    minimum = rule["minimum"]
    maximum = rule["maximum"]
    if not minimum <= intensity <= maximum:
        return INTENSITY_INTEGER_RANGE
    return None


def _intensity_schema(rule: Mapping[str, int | str]) -> dict[str, int | str]:
    if rule["json_type"] == "null":
        return {"type": "null"}
    expected = rule.get("const")
    if expected is not None:
        return {"const": expected}
    return {
        "type": "integer",
        "minimum": rule["minimum"],
        "maximum": rule["maximum"],
    }


def _confidence_schema() -> dict[str, int | str]:
    return {"type": "integer", "minimum": 0, "maximum": FXP6_SCALE}


def build_dimension_slot_schema() -> dict[str, list[dict[str, Any]]]:
    """Generate the provider's closed three-branch slot schema from the rules."""

    return {
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": state},
                    "intensity_fxp6": _intensity_schema(STATE_INTENSITY_RULES[state]),
                    "confidence_fxp6": _confidence_schema(),
                },
            }
            for state in DIMENSION_STATES
        ]
    }


def _prompt_intensity_description(rule: Mapping[str, int | str]) -> str:
    if rule["json_type"] == "null":
        return "JSON null"
    expected = rule.get("const")
    if expected is not None:
        return f"the integer {expected}"
    return f"an integer from {rule['minimum']} through {rule['maximum']}"


def state_intensity_prompt_rules() -> str:
    """Return the machine-stable provider rule segment derived from the table."""

    unavailable_state = next(
        state
        for state in DIMENSION_STATES
        if STATE_INTENSITY_RULES[state]["json_type"] == "null"
    )
    absent_state = next(
        state
        for state in DIMENSION_STATES
        if STATE_INTENSITY_RULES[state].get("const") == 0
    )
    lines = ["State × intensity algebra (mutually exclusive):"]
    lines.extend(
        f"{state}: intensity_fxp6 must be "
        f"{_prompt_intensity_description(STATE_INTENSITY_RULES[state])}."
        for state in DIMENSION_STATES
    )
    lines.extend(
        (
            f"JSON null is allowed only with {unavailable_state}.",
            "If reliable current-turn presence cannot be determined, select "
            f"{unavailable_state} with JSON null.",
            "If current-turn evaluation finds no evidence, select "
            f"{absent_state} with integer 0.",
        )
    )
    return "\n".join(lines)
