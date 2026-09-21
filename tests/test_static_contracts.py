from __future__ import annotations

import json
import tomllib
from itertools import pairwise
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def test_formula_and_runtime_are_separate() -> None:
    formula = tomllib.loads(
        (ROOT / "model/formula-v1.toml").read_text(encoding="utf-8")
    )
    runtime = tomllib.loads(
        (ROOT / "model/runtime-1c1g.toml").read_text(encoding="utf-8")
    )
    assert formula["neuron_slots"] == 16_384
    assert formula["edge_capacity"] == 524_288
    assert "worker_threads" not in formula
    assert runtime["worker_threads"] == 1
    assert "neuron_slots" not in runtime


def test_self_action_has_zero_residual_authority() -> None:
    matrix = tomllib.loads(
        (ROOT / "model/authority-matrix-v1.toml").read_text(encoding="utf-8")
    )
    assert matrix["self_action"]["allow"] == []
    assert matrix["self_action"]["eligibility_only"] is True
    assert matrix["time_advance"]["allow"] == []
    assert matrix["time_advance"]["derived_allow"] == [
        "temporal_state",
        "sleep_state",
        "workspace_projection",
        "operational_intention",
        "wake_schedule",
    ]
    assert matrix["self_critique"]["allow"] == []
    assert matrix["platform_observed"]["allow"] == []


def test_region_layout_covers_exact_brain() -> None:
    regions = tomllib.loads(
        (ROOT / "model/regions-v1.toml").read_text(encoding="utf-8")
    )
    entries = regions["region"]
    assert sum(item["count"] for item in entries) == 16_384
    assert entries[0]["start"] == 0
    for left, right in pairwise(entries):
        assert left["start"] + left["count"] == right["start"]


def test_config_schema_parses() -> None:
    schema = json.loads((ROOT / "_conf_schema.json").read_text(encoding="utf-8"))
    assert schema["runtime_envelope"]["default"] == "auto"

    assert {"runtime_envelope", "native_data_dir", "model_settings", "seed_code"} <= schema.keys()
    retired = {
        "proactive_enabled", "proactive_frequency", "user_quiet_hours",
        "proactive_settings_revision", "proactive_daily_max",
        "min_proactive_cooldown_minutes", "quiet_hours_start", "quiet_hours_end",
        "quiet_hours_emergency_bypass", "intention_ttl_minutes",
        "unanswered_backoff_base_minutes", "unanswered_hard_stop",
        "emergency_threshold", "inner_activity_token_daily_max",
    }
    assert not retired & schema.keys()
