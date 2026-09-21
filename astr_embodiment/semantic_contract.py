"""Closed fifteen-dimension contract for the semantic appraisal Provider."""

from __future__ import annotations

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


def validate_state_intensity(state: Any, intensity: Any) -> bool:
    """Return whether one state/intensity pair belongs to the closed algebra."""

    if state == PRESENT:
        return type(intensity) is int and 1 <= intensity <= FXP6_SCALE
    if state == ABSENT:
        return type(intensity) is int and intensity == 0
    if state == UNAVAILABLE:
        return intensity is None
    return False


def build_semantic_estimate_schema_v3() -> dict[str, Any]:
    """Return a fresh strict JSON schema suitable for structured generation."""

    confidence = {"type": "integer", "minimum": 0, "maximum": FXP6_SCALE}
    dimension = {
        "oneOf": [
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": PRESENT},
                    "intensity_fxp6": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": FXP6_SCALE,
                    },
                    "confidence_fxp6": confidence,
                },
            },
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": ABSENT},
                    "intensity_fxp6": {"const": 0},
                    "confidence_fxp6": confidence,
                },
            },
            {
                "type": "object",
                "additionalProperties": False,
                "required": ["state", "intensity_fxp6", "confidence_fxp6"],
                "properties": {
                    "state": {"const": UNAVAILABLE},
                    "intensity_fxp6": {"type": "null"},
                    "confidence_fxp6": confidence,
                },
            },
        ]
    }
    return {
        "type": "object",
        "additionalProperties": False,
        "required": ["schema", "dimensions"],
        "properties": {
            "schema": {"const": "astr-embodiment.semantic-estimate.v3"},
            "dimensions": {
                "type": "object",
                "additionalProperties": False,
                "required": list(DIMENSION_NAMES),
                "properties": {name: dimension for name in DIMENSION_NAMES},
            },
        },
    }

