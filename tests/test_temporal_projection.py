from __future__ import annotations

from datetime import UTC, datetime

import pytest

from astr_embodiment.temporal import (
    TemporalConfig,
    TemporalProjectionError,
    freeze_time_input,
)


def test_la_persona_and_shanghai_relation_project_independently() -> None:
    now = int(datetime(2026, 8, 28, 12, tzinfo=UTC).timestamp() * 1000)
    value = freeze_time_input(
        observed_now_utc_ms=now,
        last_committed_utc_ms=0,
        config=TemporalConfig("America/Los_Angeles", "Asia/Shanghai"),
    )
    assert value["persona_utc_offset_seconds"] == -7 * 3600
    assert value["relation_utc_offset_seconds"] == 8 * 3600
    assert value["persona_local_minute"] != value["relation_local_minute"]
    assert len(value["frozen_input_digest"]) == 64


def test_dst_transition_preserves_ordered_utc_day_bounds() -> None:
    now = int(datetime(2026, 11, 1, 8, 30, tzinfo=UTC).timestamp() * 1000)
    value = freeze_time_input(
        observed_now_utc_ms=now,
        last_committed_utc_ms=now + 1,
        config=TemporalConfig("America/Los_Angeles", "America/Los_Angeles"),
    )
    assert value["effective_now_utc_ms"] == now + 1
    assert value["budget_day_start_utc_ms"] < now < value["budget_next_day_start_utc_ms"]
    assert value["budget_next_day_start_utc_ms"] - value["budget_day_start_utc_ms"] == 25 * 3600 * 1000


def test_invalid_tzid_fails_closed() -> None:
    with pytest.raises(TemporalProjectionError):
        freeze_time_input(
            observed_now_utc_ms=1,
            last_committed_utc_ms=0,
            config=TemporalConfig("Mars/Olympus", "Asia/Shanghai"),
        )
