from __future__ import annotations

import re
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def _metadata_version() -> str:
    metadata = (ROOT / "metadata.yaml").read_text(encoding="utf-8")
    match = re.search(r'^version:\s*"([^"]+)"\s*$', metadata, re.MULTILINE)
    assert match is not None
    return match.group(1)


def test_production_version_markers_and_changelog_are_coupled() -> None:
    pyproject = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")

    assert _metadata_version() == "1.0.0"
    assert pyproject["project"]["version"] == "1.0.0"
    assert cargo["workspace"]["package"]["version"] == "1.0.0"
    assert "## [1.0.0] - 2026-08-24" in changelog
    assert "## [1.0.1]" not in changelog
    assert "## [1.0.2]" not in changelog


def test_production_readme_states_the_bounded_native_capability_loop() -> None:
    readme = (ROOT / "README.md").read_text(encoding="utf-8")

    for required in (
        "用户话语 → 15 维闭合语义证据 → 原生状态原子提交 → 受限表达投影",
        "插件升级后继续从持久化原生状态恢复",
        "Windows x64 与 Linux x86_64",
        "简洁模式",
        "调试模式",
        "共享 API 头",
        "不等同于意识、主观感受或真实关系",
    ):
        assert required in readme


def test_release_contract_verifier_accepts_only_the_production_tag() -> None:
    verifier = ROOT / "scripts" / "verify_release_contract.py"
    assert verifier.is_file()

    accepted = subprocess.run(
        [sys.executable, str(verifier), "--tag", "v1.0.0"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    rejected = subprocess.run(
        [sys.executable, str(verifier), "--tag", "v1.0.0-rc2"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert accepted.returncode == 0, accepted.stderr
    assert rejected.returncode != 0
    assert "version mismatch" in rejected.stderr


def test_ci_and_release_workflows_are_cross_platform_and_dispatch_only() -> None:
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    release_path = ROOT / ".github" / "workflows" / "release.yml"
    assert release_path.is_file()
    release = release_path.read_text(encoding="utf-8")

    for required in (
        "windows-2022",
        "ubuntu-22.04",
        "ruff format --check",
        "ruff check --select E,F",
        "cargo fmt --all -- --check",
        "cargo clippy --workspace --all-targets --locked -- -D warnings",
        "python scripts/verify_release_contract.py",
    ):
        assert required in ci

    package_matrix = ci.split("  native-package:\n", 1)[1].split(
        "\n  assemble-allowlisted-zip:", 1
    )[0]
    release_contract = ci.split("  release-contract:\n", 1)[1].split(
        "\n  native-package:", 1
    )[0]
    assemble = ci.split("  assemble-allowlisted-zip:\n", 1)[1]

    release_test = "python -m pytest -q tests/test_production_release_contract.py"
    assert "pytest>=8,<10" in release_contract
    assert release_contract.index("pytest>=8,<10") < release_contract.index(
        release_test
    )

    native_build = "python -m maturin build --release --out dist"
    native_stage = (
        "python scripts/stage_native_runtime.py --wheel-dir dist --destination ."
    )
    native_regressions = "python -m pytest -q --ignore=tests/test_release_contracts.py"
    assert native_build in package_matrix
    assert native_stage in package_matrix
    assert native_regressions in package_matrix
    assert package_matrix.index(native_build) < package_matrix.index(native_stage)
    assert package_matrix.index(native_stage) < package_matrix.index(native_regressions)

    package_contract = "python -m pytest -q tests/test_release_contracts.py"
    assert "actions/download-artifact@" in assemble
    assert "wheels/native-wheel-windows/*.whl" in assemble
    assert "wheels/native-wheel-linux/*.whl" in assemble
    assert "pytest>=8,<10" in assemble
    assert package_contract in assemble
    assert assemble.index("actions/download-artifact@") < assemble.index(
        package_contract
    )
    assert assemble.index("python scripts/package_plugin.py") < assemble.index(
        package_contract
    )
    assert assemble.index("pytest>=8,<10") < assemble.index(package_contract)

    for required in (
        "ruff format --check",
        "ruff check --select E,F",
        "cargo fmt --all -- --check",
        "cargo clippy --workspace --all-targets --locked -- -D warnings",
        native_regressions,
        "cargo test --workspace --locked",
    ):
        assert required in package_matrix

    assert "workflow_dispatch:" in release
    assert "channel:" in release
    assert "production" in release
    assert "github.event.repository.default_branch" in release
    assert "push:" not in release
    assert "sha256sum" in release
    assert "allowlisted ZIP" in release
    assert "actions/upload-artifact@" in release
    assert "gh release create" not in release


def test_packager_writes_an_allowlisted_zip_sha256_sidecar(tmp_path: Path) -> None:
    wheel = tmp_path / "astrembodiment_core-1.0.0-cp312-abi3-win_amd64.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr("astrembodiment_core/__init__.py", "# wheel initializer\n")
        archive.writestr(
            "astrembodiment_core/_native.pyd",
            b"version health open ensure_genesis apply_event inspect verify_replay "
            b"flush_and_close NativeCoreError",
        )

    output = tmp_path / "astrbot_plugin_astrembodiment-v1.0.0.zip"
    checksum = tmp_path / "astrbot_plugin_astrembodiment-v1.0.0.zip.sha256"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--sha256-output",
            str(checksum),
            "--native-wheel",
            str(wheel),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0, result.stderr
    digest, archived_name = checksum.read_text(encoding="utf-8").strip().split("  ")
    assert digest == __import__("hashlib").sha256(output.read_bytes()).hexdigest()
    assert archived_name == output.name
    with zipfile.ZipFile(output) as archive:
        assert "main.py" in archive.namelist()
        assert not any(name.startswith("tests/") for name in archive.namelist())
