#!/usr/bin/env python3
"""Fail closed when public production-release markers disagree."""

from __future__ import annotations

import argparse
import re
import sys
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
METADATA_VERSION = re.compile(r'^version:\s*"([^\"]+)"\s*$', re.MULTILINE)
RELEASE_VERSION = re.compile(r"^\d+\.\d+\.\d+$")


class ReleaseContractError(ValueError):
    """A source tree does not describe one coherent production release."""


def _metadata_version(root: Path) -> str:
    match = METADATA_VERSION.search(
        (root / "metadata.yaml").read_text(encoding="utf-8")
    )
    if match is None:
        raise ReleaseContractError("version mismatch: metadata.yaml has no version")
    version = match.group(1)
    if RELEASE_VERSION.fullmatch(version) is None:
        raise ReleaseContractError(
            f"version mismatch: metadata must be a production SemVer: {version!r}"
        )
    return version


def _locked_workspace_versions(root: Path, cargo: dict[str, object]) -> set[str]:
    workspace = cargo.get("workspace", {})
    members = workspace.get("members", []) if isinstance(workspace, dict) else []
    if not isinstance(members, list):
        raise ReleaseContractError("version mismatch: invalid Cargo workspace members")
    package_names: set[str] = set()
    for member in members:
        if not isinstance(member, str):
            raise ReleaseContractError(
                "version mismatch: invalid Cargo workspace member"
            )
        package = tomllib.loads(
            (root / member / "Cargo.toml").read_text(encoding="utf-8")
        ).get("package", {})
        name = package.get("name") if isinstance(package, dict) else None
        if not isinstance(name, str):
            raise ReleaseContractError("version mismatch: invalid Cargo package name")
        package_names.add(name)
    lock = tomllib.loads((root / "Cargo.lock").read_text(encoding="utf-8"))
    packages = lock.get("package", [])
    if not isinstance(packages, list):
        raise ReleaseContractError("version mismatch: invalid Cargo.lock package list")
    versions = {
        package["version"]
        for package in packages
        if isinstance(package, dict)
        and package.get("name") in package_names
        and isinstance(package.get("version"), str)
    }
    if len(versions) != 1:
        raise ReleaseContractError("version mismatch: Cargo.lock workspace versions")
    return versions


def verify_release_contract(
    root: Path, tag: str | None = None, version: str | None = None
) -> str:
    """Return the expected tag after checking all public production markers."""
    metadata_version = _metadata_version(root)
    cargo = tomllib.loads((root / "Cargo.toml").read_text(encoding="utf-8"))
    workspace = cargo.get("workspace", {})
    package = workspace.get("package", {}) if isinstance(workspace, dict) else {}
    cargo_version = package.get("version") if isinstance(package, dict) else None
    pyproject = tomllib.loads((root / "pyproject.toml").read_text(encoding="utf-8"))
    project = pyproject.get("project", {})
    python_version = project.get("version") if isinstance(project, dict) else None
    changelog = (root / "CHANGELOG.md").read_text(encoding="utf-8")
    expected_tag = f"v{metadata_version}"

    if cargo_version != metadata_version:
        raise ReleaseContractError(
            f"version mismatch: Cargo.toml={cargo_version!r}, metadata={metadata_version!r}"
        )
    if python_version != metadata_version:
        raise ReleaseContractError(
            f"version mismatch: pyproject.toml={python_version!r}, metadata={metadata_version!r}"
        )
    if _locked_workspace_versions(root, cargo) != {metadata_version}:
        raise ReleaseContractError(
            f"version mismatch: Cargo.lock does not use {metadata_version!r}"
        )
    if f"## [{metadata_version}]" not in changelog:
        raise ReleaseContractError(
            "version mismatch: CHANGELOG.md has no release section"
        )
    if tag is not None and tag != expected_tag:
        raise ReleaseContractError(
            f"version mismatch: tag={tag!r}, expected={expected_tag!r}"
        )
    if version is not None and version != metadata_version:
        raise ReleaseContractError(
            f"version mismatch: version={version!r}, expected={metadata_version!r}"
        )
    return expected_tag


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", help="expected Git tag, for example v1.0.0")
    parser.add_argument(
        "--version", help="expected production version, for example 1.0.0"
    )
    parser.add_argument(
        "--field",
        choices=("version", "tag"),
        help="print one metadata-derived release identifier after validation",
    )
    args = parser.parse_args()
    try:
        tag = verify_release_contract(ROOT, args.tag, args.version)
    except (OSError, tomllib.TOMLDecodeError, ReleaseContractError) as exc:
        print(str(exc), file=sys.stderr)
        return 1
    if args.field == "version":
        print(_metadata_version(ROOT))
        return 0
    if args.field == "tag":
        print(tag)
        return 0
    print(f"release contract: OK ({tag})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
