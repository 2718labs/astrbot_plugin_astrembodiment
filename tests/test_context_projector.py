from __future__ import annotations

import pytest

from astr_embodiment import bridge


def aggregate_context_summary() -> dict[str, object]:
    return {
        "schema": "astrembodiment.context-summary.v1",
        "summary_revision": 2,
        "source_continuum_revision": 9,
        "dimensions_ema_fxp6": [100_000] * 15,
        "unresolved_boundary": False,
        "unresolved_repair": True,
        "repetition_count": 3,
        "delivery_outcome": "delivered",
        "summary_digest": "ab" * 32,
    }


def test_bridge_accepts_only_the_closed_aggregate_context_schema() -> None:
    summary = aggregate_context_summary()

    validated = bridge.validate_context_summary_payload(summary)

    assert validated == summary


def test_bridge_rejects_dynamic_raw_content_sentinel_from_context() -> None:
    summary = aggregate_context_summary()
    summary["raw_message"] = "D1_RAW_CONTEXT_SENTINEL__do_not_persist_or_prompt"

    with pytest.raises(bridge.ContextProjectionIntegrity):
        bridge.validate_context_summary_payload(summary)
