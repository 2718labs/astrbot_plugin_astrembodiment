from __future__ import annotations

import asyncio
import json
from pathlib import Path

import main as main_module
from astr_embodiment.contracts import ScopeTokens
from main import AstrEmbodimentPlugin


DIMENSIONS = (
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
RESIDUALS = ("authority", "continuity", "energy", "renormalization", "capacity")
PROFILE = (
    "warmth",
    "sensitivity",
    "guardedness",
    "repair_orientation",
    "engagement",
    "epistemic_caution",
)


def test_user_visible_semantic_config_uses_fifteen_dimension_wording() -> None:
    schema = json.loads(
        (Path(__file__).resolve().parents[1] / "_conf_schema.json").read_text(
            encoding="utf-8"
        )
    )
    settings = schema["model_settings"]["items"]

    assert settings["semantic_estimator_provider_id"]["description"] == (
        "十五维语义估计 Provider"
    )
    assert settings["semantic_estimator_timeout_ms"]["description"] == (
        "十五维语义估计超时（毫秒）"
    )
    assert "闭合的十五维语义估计" in settings["assistant_provider_id"]["hint"]


DETAILED_FIELDS = (
    "schema",
    "status",
    "code",
    "stage",
    "commit_state",
    "values_state",
    "fxp_scale",
    "dimensions_fxp6",
    "estimator_confidence_fxp6",
    "dimension_confidence_fxp6",
    "base_revision",
    "revision",
    "deduplicated",
    "receipt_status",
    "calculation_state",
    "native_calculation",
    "expression_state",
    "expression_profile_fxp6",
)


class LogRecorder:
    def __init__(self) -> None:
        self.entries: list[tuple[str, str]] = []

    def _record(self, level: str, message: str, *args: object) -> None:
        self.entries.append((level, message % args if args else message))

    def info(self, message: str, *args: object) -> None:
        self._record("info", message, *args)

    def warning(self, message: str, *args: object) -> None:
        self._record("warning", message, *args)


def _outcome() -> dict[str, object]:
    dimensions = {name: index * 62_500 for index, name in enumerate(DIMENSIONS)}
    residuals = {name: index * 11 for index, name in enumerate(RESIDUALS)}
    profile = {name: (index + 1) * 100_000 for index, name in enumerate(PROFILE)}
    return {
        "status": "SUCCESS",
        "code": "SEMANTIC_COMMITTED",
        "stage": "RECEIPT",
        "commit_state": "CONFIRMED_NEW",
        "values_state": "COMMITTED",
        "dimensions_fxp6": dimensions,
        "estimator_confidence_fxp6": 800_000,
        "dimension_confidence_fxp6": None,
        "base_revision": 14,
        "revision": 15,
        "deduplicated": False,
        "receipt_status": "committed",
        "calculation_state": "CONFIRMED",
        "native_calculation": {
            "state_changed": True,
            "active_nodes": 4096,
            "active_edges": 262_144,
            "residuals_fxp6": residuals,
        },
        "expression_state": "APPLIED",
        "expression_profile_fxp6": profile,
    }


def test_observatory_uses_closed_compact_detailed_and_unmasked_failure(
    monkeypatch,
) -> None:
    schema = json.loads(
        (Path(__file__).resolve().parents[1] / "_conf_schema.json").read_text(
            encoding="utf-8"
        )
    )
    assert schema["observatory_enabled"]["default"] is True
    assert schema["node_observability_detailed_logging"]["default"] is False
    assert schema["continuity_vault_dir"]["default"] == ""

    logs = LogRecorder()
    monkeypatch.setattr(main_module, "logger", logs)
    plugin = AstrEmbodimentPlugin(None, {"observatory_enabled": True})
    outcome = _outcome()

    compact = plugin._emit_observatory(outcome)
    expected_compact = "运算已完成｜十五维：" + ",".join(
        f"{name}={outcome['dimensions_fxp6'][name]}" for name in DIMENSIONS
    )
    assert logs.entries == [("info", expected_compact)]
    assert compact["native_calculation"]["active_nodes"] == 4096
    assert "active_nodes" not in logs.entries[0][1]

    plugin._config_values = {
        "observatory_enabled": False,
        "node_observability_detailed_logging": True,
    }
    logs.entries.clear()
    detailed = plugin._emit_observatory(outcome)
    assert len(logs.entries) == 1
    level, encoded = logs.entries[0]
    assert level == "info"
    assert encoded.startswith("AstrEmbodiment SPC1 observatory: ")
    assert tuple(detailed) == DETAILED_FIELDS
    assert (
        json.loads(encoded.removeprefix("AstrEmbodiment SPC1 observatory: "))
        == detailed
    )
    assert tuple(detailed["dimensions_fxp6"]) == DIMENSIONS
    assert tuple(detailed["native_calculation"]["residuals_fxp6"]) == RESIDUALS
    assert tuple(detailed["expression_profile_fxp6"]) == PROFILE
    assert "nodes" not in detailed["native_calculation"]
    assert "edges" not in detailed["native_calculation"]

    plugin._config_values = {"observatory_enabled": False}
    logs.entries.clear()
    failed = dict(outcome)
    failed.update(
        status="FAILED",
        code="EXPRESSION_INJECTION_FAILED",
        stage="EXPRESSION_INJECTION",
        expression_state="INJECTION_FAILED",
    )
    failed_record = plugin._emit_observatory(failed)
    assert logs.entries[0][0] == "warning"
    assert logs.entries[0][1].startswith(
        "运算失败｜失败码=EXPRESSION_INJECTION_FAILED｜阶段=EXPRESSION_INJECTION｜十五维："
    )
    assert "active_nodes=4096" in logs.entries[0][1]
    assert failed_record["revision"] == 15

    logs.entries.clear()
    malformed = dict(outcome)
    malformed["raw_nonce"] = "RAW_NONCE_MUST_NOT_LEAK"
    malformed["native_calculation"] = {
        **outcome["native_calculation"],
        "nodes": ["RAW_NODE_ID_MUST_NOT_LEAK"],
    }
    fallback = plugin._emit_observatory(malformed)
    assert fallback["code"] == "OBSERVATORY_FORMATTER_FAILED"
    assert all("RAW_" not in text for _, text in logs.entries)

    class PersistConfig(dict):
        async def save_config_async(self) -> None:
            return None

    class Event:
        def set_extra(self, _name: str, _value: object) -> None:
            return None

    class Request:
        prompt = "RAW_REQUEST_MUST_NOT_LEAK"
        system_prompt = "RAW_SYSTEM_PROMPT_MUST_NOT_LEAK"

    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )
    plugin = AstrEmbodimentPlugin(None, PersistConfig(observatory_enabled=False))
    context_summary = {
        "schema": "astrembodiment.context-summary.v1",
        "summary_revision": 1,
        "source_continuum_revision": 15,
        "dimensions_ema_fxp6": [0] * 15,
        "unresolved_boundary": False,
        "unresolved_repair": False,
        "repetition_count": 1,
        "delivery_outcome": "pending",
        "summary_digest": "cd" * 32,
    }

    async def missing_receipt_request(*_args, **_kwargs):
        return (
            {
                "schema": "astrembodiment.decision.v1",
                "genesis": {
                    "seed_code": "AE-S1-0123456789ABCDEF",
                    "incarnation_id": "ab" * 32,
                },
                "seed_code": "AE-S1-0123456789ABCDEF",
                "incarnation_id": "ab" * 32,
                "contract": {},
                "context_summary": context_summary,
                "revision": 15,
                "deduplicated": False,
            },
            scope,
            scope.session_token,
            15,
            "turn-15",
            14,
        )

    class ProjectionUnavailableCoordinator:
        async def preflight_semantic_v3(self, **_kwargs):
            return {
                "status": "DEGRADED",
                "code": "EXPRESSION_PROJECTION_UNAVAILABLE",
            }

        async def apply_delivery(self, **_kwargs):
            return {
                "schema": "astrembodiment.decision.v1",
                "revision": 16,
                "deduplicated": False,
            }

    plugin._run_genesis = missing_receipt_request
    plugin._coordinator = ProjectionUnavailableCoordinator()
    event = Event()
    logs.entries.clear()
    asyncio.run(plugin.on_llm_request(event, Request()))
    assert len(logs.entries) == 1
    assert logs.entries[0][0] == "warning"
    structured = json.loads(
        logs.entries[0][1].removeprefix("AstrEmbodiment SPC1 observatory: ")
    )
    assert {
        key: structured[key]
        for key in (
            "schema",
            "status",
            "code",
            "reason",
            "cause_code",
            "expression_state",
        )
    } == {
        "schema": "astr-embodiment.semantic-observatory.v2",
        "status": "DEGRADED",
        "code": "EXPRESSION_NOT_ATTEMPTED",
        "reason": "EXPRESSION_NOT_ATTEMPTED",
        "cause_code": "EXPRESSION_PROJECTION_UNAVAILABLE",
        "expression_state": "NOT_ATTEMPTED",
    }
    assert all("calibration" not in key for key in structured)
    assert all(
        sensitive not in logs.entries[0][1]
        for sensitive in (
            Request.prompt,
            Request.system_prompt,
            "AE-S1-0123456789ABCDEF",
            "ab" * 32,
            context_summary["summary_digest"],
            "turn-15",
        )
    )

    logs.entries.clear()
    asyncio.run(plugin.after_message_sent(event))
    assert logs.entries == []
    assert plugin._revisions[scope.persona_token] == 16
