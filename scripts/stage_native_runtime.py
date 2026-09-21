#!/usr/bin/env python3
"""Retired staging entrypoint: require real-host wheel verification and installation."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path, PurePosixPath

NATIVE_PACKAGE = "astrembodiment_core"
NATIVE_MANIFEST = f"{NATIVE_PACKAGE}/_bundled/manifest.json"
NATIVE_MANIFEST_SCHEMA = "astrembodiment-native-bundle-v1"
STAGING_RETIRED = (
    "STAGING_RETIRED: single-wheel staging cannot establish universal release identity. "
    "On the matching platform, use python scripts/package_plugin.py --verify-wheel "
    "<exact-wheel.whl> --output <new-import-receipt.json>, then python -m pip install "
    "--no-deps --force-reinstall <exact-wheel.whl> in an isolated environment. "
    "Universal packaging requires both platforms' verified wheels and import receipts."
)


def _single_wheel(wheel_dir: Path) -> Path:
    if not wheel_dir.is_dir():
        raise ValueError(f"wheel directory does not exist: {wheel_dir}")
    wheels = sorted(
        path for path in wheel_dir.iterdir() if path.is_file() and path.suffix == ".whl"
    )
    if len(wheels) != 1:
        raise ValueError(
            f"wheel directory must contain exactly one .whl file, found {len(wheels)}"
        )
    return wheels[0]


def _stage_entries(destination: Path, entries: list[tuple[str, bytes]]) -> None:
    if not destination.is_dir():
        raise ValueError(f"stage destination does not exist: {destination}")

    package_root = destination / NATIVE_PACKAGE
    if package_root.exists() or package_root.is_symlink():
        raise ValueError(
            "stage destination already contains an astrembodiment_core package"
        )

    targets: list[tuple[Path, bytes]] = []
    seen: set[Path] = set()
    for member, payload in entries:
        member_path = PurePosixPath(member)
        if (
            member_path.is_absolute()
            or ".." in member_path.parts
            or len(member_path.parts) < 2
            or member_path.parts[0] != NATIVE_PACKAGE
        ):
            raise ValueError(f"unsafe staged native member: {member}")
        target = destination.joinpath(*member_path.parts)
        if target in seen:
            raise ValueError(f"duplicate staged native member: {member}")
        seen.add(target)
        targets.append((target, payload))

    for target, payload in targets:
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(payload)


def _current_platform_key() -> str:
    if sys.platform == "win32":
        return "win32"
    if sys.platform.startswith("linux"):
        return "linux"
    raise ValueError(f"unsupported native platform: {sys.platform}")


def _verify_staged_runtime(destination: Path) -> Path:
    package_root = destination / NATIVE_PACKAGE
    manifest_path = destination / PurePosixPath(NATIVE_MANIFEST)
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"invalid staged native manifest: {manifest_path}") from exc
    if (
        not isinstance(manifest, dict)
        or manifest.get("schema") != NATIVE_MANIFEST_SCHEMA
    ):
        raise ValueError(f"unsupported staged native manifest: {manifest_path}")

    platforms = manifest.get("platforms")
    platform_key = _current_platform_key()
    entry = platforms.get(platform_key) if isinstance(platforms, dict) else None
    if not isinstance(entry, dict):
        raise ValueError(f"staged native manifest lacks platform: {platform_key}")
    build_id = entry.get("build_id")
    filename = entry.get("filename")
    if (
        not isinstance(build_id, str)
        or len(build_id) != 64
        or any(character not in "0123456789abcdef" for character in build_id)
        or not isinstance(filename, str)
        or PurePosixPath(filename).name != filename
        or not filename.startswith("_native")
    ):
        raise ValueError(f"invalid staged native manifest entry for {platform_key}")

    native_path = package_root / "_bundled" / build_id / filename
    if not native_path.is_file():
        raise ValueError(f"staged native binary does not exist: {native_path}")
    if hashlib.sha256(native_path.read_bytes()).hexdigest() != build_id:
        raise ValueError(f"staged native binary hash mismatch: {native_path}")
    if any(package_root.glob("_native*.pyd")) or any(package_root.glob("_native*.so")):
        raise ValueError("staged runtime must not contain a root native extension")
    return native_path


def stage_native_runtime(wheel_dir: Path, destination: Path) -> Path:
    """Fail before reading or writing paths; the old staging contract is retired."""
    raise ValueError(STAGING_RETIRED)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--wheel-dir", type=Path, required=True)
    parser.add_argument("--destination", type=Path, required=True)
    args = parser.parse_args()
    try:
        print(stage_native_runtime(args.wheel_dir, args.destination))
    except ValueError as exc:
        parser.error(str(exc))


if __name__ == "__main__":
    main()
