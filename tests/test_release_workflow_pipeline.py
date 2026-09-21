"""Release wiring checks supplement real wheel/ZIP probes on both CI hosts."""

import re
import subprocess
import sys
import tomllib
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]


def test_wheel_only_extension_link_mode():
    project = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    assert "pyo3/extension-module" in project["tool"]["maturin"]["features"]
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    assert "extension-module" not in cargo["workspace"]["dependencies"]["pyo3"].get(
        "features", []
    )


@pytest.mark.parametrize("workflow", ["ci.yml", "release.yml"])
def test_exact_sha_receipt_and_archive_gates(workflow):
    text = (ROOT / ".github/workflows" / workflow).read_text(encoding="utf-8")
    assert "--build-native --source-sha" in text
    assert "--verify-wheel" in text
    assert text.count("--import-receipt") >= 2
    assert "scripts/verify_release_archive.py" in text
    for name in (
        "AE_FRESH_WINDOWS_WHEEL",
        "AE_FRESH_LINUX_WHEEL",
        "AE_RELEASE_ARCHIVE",
    ):
        assert f"export {name}=" in text
    assert "windows-2022" in text and "ubuntu-22.04" in text
    assert "core.version()" not in text
    assert "python -m maturin build" not in text
    assert "--sha256-output" not in text
    assert "contents: write" not in text or workflow == "release.yml"
    if workflow == "release.yml":
        publish = text.split("  publish-release:\n", 1)[1]
        assert "      - verify-archive" in publish.split("    steps:", 1)[0]


def test_checkout_preserves_frozen_blob_bytes():
    attributes = (ROOT / ".gitattributes").read_text(encoding="utf-8")
    assert "* -text" in attributes.splitlines()


def test_ci_cargo_integration_targets_exist_in_selected_package():
    packages = {}
    for manifest in (ROOT / "crates").glob("*/Cargo.toml"):
        package = tomllib.loads(manifest.read_text(encoding="utf-8"))["package"]
        packages[package["name"]] = manifest.parent
    ci = (ROOT / ".github/workflows/ci.yml").read_text(encoding="utf-8")
    commands = re.findall(r"cargo test -p ([\w-]+)([^\n]+)", ci)
    assert commands
    for package, arguments in commands:
        targets = re.findall(r"--test ([\w-]+)", arguments)
        assert targets
        for target in targets:
            assert (packages[package] / "tests" / f"{target}.rs").is_file(), (
                package,
                target,
            )
    assert "$env:CARGO_TARGET_DIR = Join-Path" in ci
    assert 'export CARGO_TARGET_DIR="$task_temp/target"' in ci


def test_workflow_packager_flags_are_accepted_by_real_cli():
    result = subprocess.run(
        [sys.executable, str(ROOT / "scripts/package_plugin.py"), "--help"],
        text=True,
        capture_output=True,
        check=True,
    )
    for name in (
        "--build-native",
        "--source-sha",
        "--verify-wheel",
        "--import-receipt",
    ):
        assert name in result.stdout


def test_embedded_release_python_is_valid():
    text = (ROOT / ".github/workflows/release.yml").read_text(encoding="utf-8")
    blocks = re.findall(r"<<'PY'\n(.*?)\n          PY", text, re.S)
    assert len(blocks) >= 8
    for block in blocks:
        compile(
            "\n".join(line[10:] for line in block.splitlines()), "release.yml", "exec"
        )
