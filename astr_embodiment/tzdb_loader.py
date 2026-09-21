"""Load only the plugin's pinned TZif bundle; never query system zoneinfo."""
from __future__ import annotations

from functools import lru_cache
from hashlib import sha256
from io import BytesIO
import json
from pathlib import Path
import re
import struct
from datetime import datetime, timezone
from zoneinfo import ZoneInfo

_ASSETS = Path(__file__).resolve().parent / "assets" / "tzdb"
_FIXED = re.compile(r"UTC([+-])((?:0[0-9]|1[0-3]):[0-5][0-9]|14:00)\Z")


@lru_cache(maxsize=1)
def load_manifest() -> dict:
    manifest = json.loads((_ASSETS / "manifest.json").read_text(encoding="utf-8"))
    if manifest["format_version"] != 1 or manifest["tzdb_release"] != "2026c":
        raise ValueError("TZDB_IDENTITY_INVALID")
    return manifest


@lru_cache(maxsize=1)
def _zones() -> dict[str, bytes]:
    data = (_ASSETS / "tzdb-2026c.bin").read_bytes()
    if sha256(data).hexdigest() != load_manifest()["content_sha256"]:
        raise ValueError("TZDB_DIGEST_MISMATCH")
    if data[:5] != b"AETZ1" or len(data) > 4 * 1024 * 1024:
        raise ValueError("TZDB_FORMAT_INVALID")
    count = struct.unpack_from("<I", data, 5)[0]
    if count > 2048:
        raise ValueError("TZDB_FORMAT_INVALID")
    cursor = 9
    zones = {}
    for _ in range(count):
        size = struct.unpack_from("<H", data, cursor)[0]
        cursor += 2
        name = data[cursor:cursor + size].decode("ascii")
        cursor += size
        size = struct.unpack_from("<I", data, cursor)[0]
        cursor += 4
        body = data[cursor:cursor + size]
        cursor += size
        if name in zones or not body.startswith(b"TZif") or len(body) != size:
            raise ValueError("TZDB_FORMAT_INVALID")
        zones[name] = body
    if cursor != len(data) or sorted(zones) != load_manifest()["canonical_zones"]:
        raise ValueError("TZDB_FORMAT_INVALID")
    return zones


def fixed_offset_seconds(tzid: str) -> int | None:
    if tzid == "UTC":
        return 0
    match = _FIXED.fullmatch(tzid)
    if match is None:
        return None
    hours, minutes = map(int, match[2].split(":"))
    if hours == minutes == 0:
        raise ValueError("NONCANONICAL_TIMEZONE")
    return (hours * 3600 + minutes * 60) * (1 if match[1] == "+" else -1)


def canonical_timezone(tzid: str) -> str:
    if fixed_offset_seconds(tzid) is not None:
        return tzid
    name = load_manifest()["aliases"].get(tzid, tzid)
    if name not in _zones():
        raise ValueError("UNKNOWN_PERSONA_TIMEZONE")
    return name


@lru_cache(maxsize=64)
def zone(tzid: str) -> ZoneInfo:
    canonical = canonical_timezone(tzid)
    return ZoneInfo.from_file(BytesIO(_zones()[canonical]), key=canonical)


@lru_cache(maxsize=64)
def _transition_zone(tzid: str):
    # CPython's pure TZif parser retains the explicit transitions and POSIX
    # continuation rule. It reads the identical pinned bytes, never TZPATH.
    from zoneinfo import _zoneinfo
    return _zoneinfo.ZoneInfo.from_file(BytesIO(_zones()[tzid]), key=tzid)


def freeze_persona_time(tzid: str, now_utc_ms: int) -> dict:
    if not isinstance(now_utc_ms, int) or isinstance(now_utc_ms, bool) or now_utc_ms <= 0:
        raise ValueError("INVALID_FROZEN_TIME")
    tzid = canonical_timezone(tzid)
    seconds = now_utc_ms // 1000
    offset = fixed_offset_seconds(tzid)
    next_transition = None
    if offset is None:
        utc = datetime.fromtimestamp(seconds, timezone.utc)
        offset = int(utc.astimezone(zone(tzid)).utcoffset().total_seconds())
        rules = _transition_zone(tzid)
        transitions = [t for t in rules._trans_utc if t > seconds]
        tail = rules._tz_after
        if hasattr(tail, "transitions"):
            for year in (utc.year - 1, utc.year, utc.year + 1, utc.year + 2):
                start, end = tail.transitions(year)
                for candidate in (start - int(tail.std.utcoff.total_seconds()),
                                  end - int(tail.dst.utcoff.total_seconds())):
                    if candidate > seconds and (not rules._trans_utc or candidate > rules._trans_utc[-1]):
                        transitions.append(candidate)
        if transitions:
            next_transition = min(transitions) * 1000
    local_seconds = seconds + offset
    manifest = load_manifest()
    return {"schema_version": 1, "now_utc_ms": now_utc_ms, "persona_tzid": tzid,
            "persona_utc_offset_seconds": offset, "persona_local_minute": (local_seconds % 86400) // 60,
            "persona_day_ordinal": local_seconds // 86400,
            "next_timezone_transition_utc_ms": next_transition,
            "tzdb_release": manifest["tzdb_release"], "tzdb_content_sha256": manifest["content_sha256"]}


def _build_asset(source: Path) -> None:
    """Development-only deterministic conversion of a public tzdata wheel."""
    raw = {}
    for path in source.rglob("*"):
        if path.is_file():
            body = path.read_bytes()
            if body.startswith(b"TZif"):
                raw[path.relative_to(source).as_posix()] = body
    metadata = (source / "tzdata.zi").read_text(encoding="utf-8")
    release = metadata.splitlines()[0].split()[-1]
    if release != "2026c":
        raise ValueError("SOURCE_RELEASE_MISMATCH")
    aliases = {}
    for line in metadata.splitlines():
        if line.startswith("L "):
            _, target, name = line.split()
            aliases[name] = target
    for name in aliases:
        target = aliases[name]
        seen = {name}
        while target in aliases:
            if target in seen:
                raise ValueError("TZDB_ALIAS_CYCLE")
            seen.add(target)
            target = aliases[target]
        aliases[name] = target
    raw = {name: body for name, body in raw.items() if name not in aliases}
    if any(target not in raw for target in aliases.values()):
        raise ValueError("TZDB_ALIAS_MISSING")
    body = bytearray(b"AETZ1" + struct.pack("<I", len(raw)))
    for name, data in sorted(raw.items()):
        encoded = name.encode("ascii")
        body += struct.pack("<H", len(encoded)) + encoded + struct.pack("<I", len(data)) + data
    _ASSETS.mkdir(parents=True, exist_ok=True)
    (_ASSETS / "tzdb-2026c.bin").write_bytes(body)
    manifest = {"format_version": 1, "tzdb_release": release,
                "content_sha256": sha256(body).hexdigest(),
                "canonical_zones": sorted(raw), "aliases": dict(sorted(aliases.items())),
                "license": "IANA timezone data: public domain; tzdata packaging: Apache-2.0"}
    (_ASSETS / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=True, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    license_root = source.parent.parent / "tzdata-2026.3.dist-info" / "licenses"
    (_ASSETS / "LICENSE.tzdata").write_bytes((license_root / "LICENSE").read_bytes())
    (_ASSETS / "LICENSE_APACHE").write_bytes((license_root / "licenses" / "LICENSE_APACHE").read_bytes())


if __name__ == "__main__":
    import argparse
    parser = argparse.ArgumentParser()
    parser.add_argument("--build-source", type=Path, required=True)
    _build_asset(parser.parse_args().build_source)
