#!/usr/bin/env python3
"""Build a self-contained AstrBot plugin archive from native platform wheels."""

from __future__ import annotations

import argparse
import hashlib
import json
import zipfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
INCLUDE = [
    "main.py",
    "metadata.yaml",
    "requirements.txt",
    "_conf_schema.json",
    "astr_embodiment",
    "README.md",
    "LICENSE",
    "CHANGELOG.md",
    "logo.png",
]
NATIVE_PACKAGE = "astrembodiment_core"
NATIVE_INIT = f"{NATIVE_PACKAGE}/__init__.py"
NATIVE_BUNDLE_ROOT = f"{NATIVE_PACKAGE}/_bundled"
NATIVE_MANIFEST = f"{NATIVE_BUNDLE_ROOT}/manifest.json"
NATIVE_MANIFEST_SCHEMA = "astrembodiment-native-bundle-v1"
NATIVE_SOURCE_INIT = ROOT / "python" / NATIVE_INIT
NATIVE_SUFFIXES = (".pyd", ".so")
NATIVE_API_MARKERS = (
    "version",
    "contract_info",
    "health",
    "open",
    "ensure_genesis",
    "prepare_rebirth_v1",
    "confirm_rebirth_v1",
    "reconcile_seed_config_v1",
    "ack_seed_config_writeback_v1",
    "semantic_revision_v1",
    "apply_perception_proposal_v1",
    "apply_event",
    "apply_perception_proposal_v1",
    "inspect",
    "semantic_revision_v1",
    "verify_replay",
    "flush_and_close",
    "NativeCoreError",
)
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024
# A stored ZIP keeps byte output stable across the Windows and Linux builders.
# The current dual-platform payload is comfortably below the marketplace limit;
# the size gate below remains the final authority if that ever changes.
ZIP_COMPRESSION = zipfile.ZIP_STORED
ZIP_TIMESTAMP = (1980, 1, 1, 0, 0, 0)
ZIP_FILE_MODE = 0o100644


def _safe_wheel_member(name: str) -> bool:
    path = PurePosixPath(name)
    return (
        bool(name)
        and not path.is_absolute()
        and ".." not in path.parts
        and not name.endswith("/")
    )


def _archive_info(member: str) -> zipfile.ZipInfo:
    """Return fixed ZIP metadata for a reproducible release archive."""
    if not _safe_wheel_member(member):
        raise ValueError(f"unsafe release archive member: {member}")
    info = zipfile.ZipInfo(member, date_time=ZIP_TIMESTAMP)
    info.compress_type = ZIP_COMPRESSION
    info.create_system = 3
    info.external_attr = ZIP_FILE_MODE << 16
    return info


def _source_package_entries() -> list[tuple[str, bytes]]:
    """Return allowlisted plugin source entries in a stable lexical order."""
    entries: list[tuple[str, bytes]] = []
    for item in INCLUDE:
        path = ROOT / item
        if path.is_symlink():
            raise ValueError(f"release input must not be a symlink: {path}")
        if path.is_file():
            entries.append((path.relative_to(ROOT).as_posix(), path.read_bytes()))
            continue
        if not path.is_dir():
            raise ValueError(f"required release input does not exist: {path}")
        for child in sorted(
            path.rglob("*"),
            key=lambda candidate: candidate.relative_to(ROOT).as_posix(),
        ):
            if child.is_symlink():
                raise ValueError(f"release input must not be a symlink: {child}")
            if child.is_file() and "__pycache__" not in child.parts:
                entries.append((child.relative_to(ROOT).as_posix(), child.read_bytes()))
    return entries


def _write_entries(archive: zipfile.ZipFile, entries: list[tuple[str, bytes]]) -> None:
    """Write one sorted, duplicate-free and metadata-normalized entry set."""
    seen: set[str] = set()
    for member, payload in sorted(entries, key=lambda entry: entry[0]):
        if member in seen:
            raise ValueError(f"duplicate release archive member: {member}")
        seen.add(member)
        archive.writestr(_archive_info(member), payload)


def _native_extension(names: list[str]) -> str:
    candidates = sorted(
        name
        for name in names
        if name.startswith(f"{NATIVE_PACKAGE}/_native")
        and name.endswith(NATIVE_SUFFIXES)
    )
    if len(candidates) != 1:
        raise ValueError(
            "native wheel must contain exactly one astrembodiment_core/_native*.pyd "
            "or .so member"
        )
    return candidates[0]


def _validate_native_payload(member: str, payload: bytes) -> None:
    missing = [
        marker for marker in NATIVE_API_MARKERS if marker.encode() not in payload
    ]
    if missing:
        raise ValueError(
            f"native extension {member} is missing expected API markers: "
            + ", ".join(missing)
        )


def native_package_entries(wheel_paths: list[Path]) -> list[tuple[str, bytes]]:
    """Return validated content-addressed native runtime archive entries."""
    if not NATIVE_SOURCE_INIT.is_file():
        raise ValueError(
            f"native source initializer does not exist: {NATIVE_SOURCE_INIT}"
        )
    entries = [(NATIVE_INIT, NATIVE_SOURCE_INIT.read_bytes())]
    platforms: dict[str, dict[str, str]] = {}
    wheel_names: set[str] = set()
    for wheel_path in wheel_paths:
        if not wheel_path.is_file():
            raise ValueError(f"native wheel does not exist: {wheel_path}")
        wheel_name = wheel_path.name
        if not _safe_wheel_member(wheel_name) or not wheel_name.endswith(".whl"):
            raise ValueError(f"native wheel has an unsafe filename: {wheel_name}")
        if wheel_name in wheel_names:
            raise ValueError(f"duplicate native wheel filename: {wheel_name}")
        wheel_names.add(wheel_name)
        with zipfile.ZipFile(wheel_path) as wheel:
            names = wheel.namelist()
            if len(names) != len(set(names)):
                raise ValueError("native wheel contains duplicate archive members")
            if not all(_safe_wheel_member(name) for name in names):
                raise ValueError("native wheel contains an unsafe archive member")
            if NATIVE_INIT not in names:
                raise ValueError(
                    f"native wheel is missing required runtime member: {NATIVE_INIT}"
                )
            native_extension = _native_extension(names)
            platform = "win32" if native_extension.endswith(".pyd") else "linux"
            if platform in platforms:
                raise ValueError(f"duplicate native wheel for platform: {platform}")
            native_payload = wheel.read(native_extension)
            _validate_native_payload(native_extension, native_payload)
            build_id = hashlib.sha256(native_payload).hexdigest()
            archive_member = f"{NATIVE_BUNDLE_ROOT}/{build_id}/{PurePosixPath(native_extension).name}"
            entries.append((archive_member, native_payload))
            platforms[platform] = {
                "build_id": build_id,
                "filename": PurePosixPath(native_extension).name,
            }

    manifest = {
        "schema": NATIVE_MANIFEST_SCHEMA,
        "platforms": platforms,
    }
    entries.append(
        (
            NATIVE_MANIFEST,
            json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode("utf-8"),
        )
    )
    return entries


def _release_entries(wheel_paths: list[Path]) -> list[tuple[str, bytes]]:
    """Return every allowlisted source and native entry for the release ZIP."""
    return _source_package_entries() + native_package_entries(wheel_paths)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--sha256-output",
        type=Path,
        help="write '<sha256>  <archive-name>' for the allowlisted ZIP",
    )
    parser.add_argument("--native-wheel", type=Path, action="append", required=True)
    args = parser.parse_args()
    if args.output.suffix.lower() != ".zip":
        raise SystemExit("--output must name a .zip archive")
    if args.output.exists() or args.output.is_symlink():
        raise SystemExit(f"refusing to overwrite release archive: {args.output}")
    if args.sha256_output is not None and (
        args.sha256_output.exists() or args.sha256_output.is_symlink()
    ):
        raise SystemExit(
            f"refusing to overwrite release checksum: {args.sha256_output}"
        )
    if args.sha256_output is not None and args.sha256_output == args.output:
        raise SystemExit("--sha256-output must differ from --output")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    created_archive = False
    try:
        with zipfile.ZipFile(
            args.output,
            "x",
            compression=ZIP_COMPRESSION,
            strict_timestamps=True,
        ) as archive:
            created_archive = True
            _write_entries(archive, _release_entries(args.native_wheel))
    except Exception:
        if created_archive and args.output.exists() and not args.output.is_symlink():
            args.output.unlink()
        raise
    if args.output.stat().st_size > MAX_ARCHIVE_BYTES:
        args.output.unlink()
        raise SystemExit("release archive exceeds the 16 MB AstrBot marketplace limit")
    if args.sha256_output is not None:
        digest = hashlib.sha256(args.output.read_bytes()).hexdigest()
        args.sha256_output.parent.mkdir(parents=True, exist_ok=True)
        args.sha256_output.write_text(
            f"{digest}  {args.output.name}\n", encoding="utf-8"
        )
    print(args.output)


if __name__ == "__main__":
    main()
