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
    "health",
    "open",
    "ensure_genesis",
    "apply_event",
    "inspect",
    "verify_replay",
    "flush_and_close",
    "NativeCoreError",
)
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024


def _safe_wheel_member(name: str) -> bool:
    path = PurePosixPath(name)
    return not path.is_absolute() and ".." not in path.parts and not name.endswith("/")


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


def _write_native_package(archive: zipfile.ZipFile, wheel_paths: list[Path]) -> None:
    for member, payload in native_package_entries(wheel_paths):
        archive.writestr(member, payload, compress_type=zipfile.ZIP_DEFLATED)


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
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(args.output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for item in INCLUDE:
            path = ROOT / item
            if path.is_dir():
                for child in sorted(path.rglob("*")):
                    if child.is_file() and "__pycache__" not in child.parts:
                        archive.write(child, child.relative_to(ROOT))
            elif path.is_file():
                archive.write(path, path.relative_to(ROOT))
        _write_native_package(archive, args.native_wheel)
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
