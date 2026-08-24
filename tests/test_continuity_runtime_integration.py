from __future__ import annotations

import json
import sqlite3
from pathlib import Path

import pytest

from astr_embodiment.bridge import NativeBridge, NativeCoreError
from astr_embodiment.contracts import ScopeTokens, build_user_stimulus_json
from astr_embodiment.coordinator import GenesisCoordinator
from astr_embodiment.persona_genesis import (
    PERSONA_COMPILER_SCHEMA,
    PersonaGenesisError,
    PersonaSourceSnapshot,
    _ALLOSTATIC_NAMES,
    _EPISTEMIC_NAMES,
    _EXPRESSION_NAMES,
    _SOCIAL_NAMES,
    _TRAIT_NAMES,
    build_closed_request,
    validate_proposal,
)
from main import AstrEmbodimentPlugin


class Request:
    def __init__(self) -> None:
        self.system_prompt = "host system prompt"


def aggregate_context_summary() -> dict[str, object]:
    return {
        "schema": "astrembodiment.context-summary.v1",
        "summary_revision": 1,
        "source_continuum_revision": 1,
        "dimensions_ema_fxp6": [0] * 15,
        "unresolved_boundary": False,
        "unresolved_repair": False,
        "repetition_count": 1,
        "delivery_outcome": "pending",
        "summary_digest": "cd" * 32,
    }


def test_runtime_injects_only_closed_aggregate_context_metadata() -> None:
    plugin = AstrEmbodimentPlugin(None, {})
    request = Request()

    plugin._inject_request(
        request,
        "AE-S1-0123456789ABCDEF",
        {},
        aggregate_context_summary(),
    )

    assert "source_revision=1" in request.system_prompt
    assert "summary_digest=" + ("cd" * 32) in request.system_prompt
    assert (
        "D1_RAW_CONTEXT_SENTINEL__do_not_persist_or_prompt" not in request.system_prompt
    )


def test_runtime_rejects_raw_context_sentinel_before_prompt_injection() -> None:
    plugin = AstrEmbodimentPlugin(None, {})
    request = Request()
    summary = aggregate_context_summary()
    summary["raw_message"] = "D1_RAW_CONTEXT_SENTINEL__do_not_persist_or_prompt"

    with pytest.raises(PersonaGenesisError):
        plugin._inject_request(request, "AE-S1-0123456789ABCDEF", {}, summary)

    assert request.system_prompt == "host system prompt"


class RawContextDecisionBridge:
    def apply_event(
        self, _scope: dict[str, object], _event: dict[str, object]
    ) -> dict[str, object]:
        summary = aggregate_context_summary()
        summary["raw_message"] = "D1_RAW_CONTEXT_SENTINEL__do_not_cache"
        return {
            "schema": "astrembodiment.decision.v1",
            "context_summary": summary,
        }


def test_coordinator_refuses_to_cache_an_unvalidated_context_summary() -> None:
    coordinator = GenesisCoordinator(RawContextDecisionBridge())  # type: ignore[arg-type]
    scope = ScopeTokens(
        bot_token="10" * 16,
        persona_token="20" * 16,
        relation_token=None,
        session_token="30" * 16,
    )

    with pytest.raises(Exception, match="CONTEXT_PROJECTION"):
        import asyncio

        asyncio.run(coordinator._apply_once(scope, "40" * 16, {}))

    assert coordinator.applied_count() == 0


def _real_scope() -> ScopeTokens:
    return ScopeTokens(
        bot_token="11" * 16,
        persona_token="22" * 16,
        relation_token=None,
        session_token="33" * 16,
    )


def _closed_genesis_request(scope: ScopeTokens, raw_source: str) -> dict[str, object]:
    source = PersonaSourceSnapshot.freeze(
        persona_id="d1-privacy-persona",
        persona={
            "prompt": raw_source,
            "begin_dialogs": [],
            "mood_imitation_dialogs": [],
        },
        selection="conversation",
    )
    proposal = validate_proposal(
        {
            "schema": PERSONA_COMPILER_SCHEMA,
            "traits": {
                name: {"value": 0.5, "confidence": 0.5} for name in _TRAIT_NAMES
            },
            "expression": {name: 0.5 for name in _EXPRESSION_NAMES},
            "allostasis": {name: 0.5 for name in _ALLOSTATIC_NAMES},
            "epistemic": {name: 0.5 for name in _EPISTEMIC_NAMES},
            "social": {name: 0.5 for name in _SOCIAL_NAMES},
        }
    )
    return build_closed_request(
        scope=scope,
        source=source,
        proposal=proposal,
        selection="conversation",
        compiler_protocol_digest="00" * 32,
        compiler_model_digest="00" * 32,
        formula_digest="00" * 32,
        incarnation_nonce="a1" * 32,
        observed_at_ms=1_700_000_000_000,
    )


def _sqlite_surface_counts(database_path: Path) -> tuple[dict[str, int], bytes]:
    raw_database = database_path.read_bytes()
    with sqlite3.connect(f"{database_path.as_uri()}?mode=ro", uri=True) as connection:
        table_names = {
            str(row[0])
            for row in connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            )
        }
        required_tables = {
            "journal",
            "applied_events",
            "snapshots",
            "graph_commits",
            "context_commits",
        }
        assert required_tables.issubset(table_names)
        counts = {
            "vault_store": len(raw_database),
            "journal": connection.execute("SELECT COUNT(*) FROM journal").fetchone()[0],
            "snapshot": connection.execute("SELECT COUNT(*) FROM snapshots").fetchone()[
                0
            ],
            "graph": connection.execute(
                "SELECT COUNT(*) FROM graph_commits"
            ).fetchone()[0],
            "context": connection.execute(
                "SELECT COUNT(*) FROM context_commits"
            ).fetchone()[0],
            "migration_shadow": len(
                list(
                    connection.execute(
                        "SELECT type, name, tbl_name, sql FROM sqlite_master"
                    )
                )
            ),
        }
    return counts, raw_database


def test_real_native_vault_context_artifacts_contain_no_dynamic_raw_sentinels(
    tmp_path: Path,
) -> None:
    sentinels = (
        "D1_RAW_TEXT_SENTINEL__do_not_persist",
        "D1_ENTITY_SENTINEL__do_not_persist",
        "D1_PLATFORM_SENTINEL__do_not_persist",
        "D1_PATH_SENTINEL__do_not_persist",
        "D1_PROVIDER_SENTINEL__do_not_persist",
        "D1_RAW_NONCE_SENTINEL__do_not_persist",
    )
    scope = _real_scope()
    bridge = NativeBridge()
    bridge.open(str(tmp_path))
    database_path = tmp_path / "astrembodiment.sqlite3"
    try:
        request = _closed_genesis_request(scope, "\n".join(sentinels))
        genesis = bridge.ensure_genesis(request)
        assert genesis["lease_status"] == "committed"

        event = build_user_stimulus_json(
            scope=scope,
            event_id="44" * 16,
            turn_id="55" * 16,
            base_revision=0,
            observed_at_ms=1_700_000_000_001,
        )
        forged = json.loads(json.dumps(event))
        forged["payload"]["raw_message"] = sentinels[0]
        with pytest.raises(NativeCoreError):
            bridge.apply_event(scope.scope_json(), forged)

        decision = bridge.apply_event(scope.scope_json(), event)
        replay = bridge.verify_replay(scope.scope_json())
        assert decision["schema"] == "astrembodiment.decision.v1"
        assert (
            decision["context_summary"]["schema"] == "astrembodiment.context-summary.v1"
        )
        assert replay["ok"] is True
        assert replay["checked"] == 1
        assert database_path.is_file() and database_path.stat().st_size > 0

        counts, raw_database = _sqlite_surface_counts(database_path)
        output_surfaces = (
            raw_database,
            json.dumps(request, ensure_ascii=False, sort_keys=True).encode(),
            json.dumps(genesis, ensure_ascii=False, sort_keys=True).encode(),
            json.dumps(decision, ensure_ascii=False, sort_keys=True).encode(),
            json.dumps(replay, ensure_ascii=False, sort_keys=True).encode(),
        )
        assert all(count > 0 for count in counts.values()), counts
        assert all(
            sentinel.encode() not in surface
            for sentinel in sentinels
            for surface in output_surfaces
        )
    finally:
        bridge.close()
