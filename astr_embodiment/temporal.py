"""Freeze host timezone facts for deterministic Native replay."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from datetime import UTC, datetime, time, timedelta
from zoneinfo import TZPATH, ZoneInfo, ZoneInfoNotFoundError


class TemporalProjectionError(ValueError):
    """A timezone fact could not be frozen without guessing."""


@dataclass(frozen=True, slots=True)
class TemporalConfig:
    persona_tzid: str
    relation_tzid: str


def _zone(tzid: str) -> ZoneInfo:
    if not tzid or tzid.startswith(('/', '\\')):
        raise TemporalProjectionError("invalid timezone id")
    try:
        return ZoneInfo(tzid)
    except (ZoneInfoNotFoundError, ValueError) as exc:
        raise TemporalProjectionError(f"unknown timezone: {tzid}") from exc


def _offset_seconds(value: datetime) -> int:
    offset = value.utcoffset()
    if offset is None:
        raise TemporalProjectionError("timezone returned no UTC offset")
    return int(offset.total_seconds())


def _first_valid_local(date_value, zone: ZoneInfo) -> datetime:
    candidate = datetime.combine(date_value, time.min, tzinfo=zone)
    for _ in range(181):
        if candidate.astimezone(UTC).astimezone(zone).replace(fold=candidate.fold) == candidate:
            return candidate
        candidate += timedelta(minutes=1)
    raise TemporalProjectionError("local day has no valid boundary")


def _next_transition(now: datetime, zones: tuple[ZoneInfo, ...]) -> int | None:
    initial = tuple(_offset_seconds(now.astimezone(zone)) for zone in zones)
    previous = now
    for day in range(1, 371):
        probe = now + timedelta(days=day)
        if tuple(_offset_seconds(probe.astimezone(zone)) for zone in zones) != initial:
            low, high = previous, probe
            while (high - low) > timedelta(seconds=1):
                mid = low + (high - low) / 2
                if tuple(_offset_seconds(mid.astimezone(zone)) for zone in zones) == initial:
                    low = mid
                else:
                    high = mid
            return int(high.timestamp() * 1000)
        previous = probe
    return None


def freeze_time_input(
    *, observed_now_utc_ms: int, last_committed_utc_ms: int,
    config: TemporalConfig,
) -> dict[str, object]:
    """Return astrembodiment.frozen-time-input.v1 with a canonical digest."""
    if observed_now_utc_ms < 0 or last_committed_utc_ms < 0:
        raise TemporalProjectionError("UTC milliseconds must be non-negative")
    persona_zone, relation_zone = _zone(config.persona_tzid), _zone(config.relation_tzid)
    effective = max(observed_now_utc_ms, last_committed_utc_ms)
    now = datetime.fromtimestamp(effective / 1000, tz=UTC)
    persona_local, relation_local = now.astimezone(persona_zone), now.astimezone(relation_zone)
    day_start = _first_valid_local(relation_local.date(), relation_zone).astimezone(UTC)
    next_day_start = _first_valid_local(relation_local.date() + timedelta(days=1), relation_zone).astimezone(UTC)
    transition = _next_transition(now, (persona_zone, relation_zone))
    transition_slice = {
        "effective": effective,
        "persona_offset": _offset_seconds(persona_local),
        "relation_offset": _offset_seconds(relation_local),
        "next_transition": transition,
    }
    provider = "python.zoneinfo"
    tzdb_material = json.dumps(
        [provider, [str(path) for path in TZPATH], transition_slice],
        ensure_ascii=True, separators=(",", ":"), sort_keys=True,
    ).encode()
    frozen: dict[str, object] = {
        "schema_version": 1,
        "observed_now_utc_ms": observed_now_utc_ms,
        "effective_now_utc_ms": effective,
        "persona_tzid": config.persona_tzid,
        "persona_utc_offset_seconds": _offset_seconds(persona_local),
        "persona_local_minute": persona_local.hour * 60 + persona_local.minute,
        "persona_day_ordinal": persona_local.date().toordinal(),
        "relation_tzid": config.relation_tzid,
        "relation_utc_offset_seconds": _offset_seconds(relation_local),
        "relation_local_minute": relation_local.hour * 60 + relation_local.minute,
        "relation_day_ordinal": relation_local.date().toordinal(),
        "budget_day_start_utc_ms": int(day_start.timestamp() * 1000),
        "budget_next_day_start_utc_ms": int(next_day_start.timestamp() * 1000),
        "next_timezone_transition_utc_ms": transition,
        "tzdb_fingerprint": hashlib.sha256(tzdb_material).hexdigest(),
    }
    canonical = json.dumps(frozen, ensure_ascii=True, separators=(",", ":"), sort_keys=True).encode()
    frozen["frozen_input_digest"] = hashlib.sha256(b"ae.frozen-time-input.v1\0" + canonical).hexdigest()
    return frozen
