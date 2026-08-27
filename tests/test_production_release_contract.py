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
        [
            sys.executable,
            str(verifier),
            "--tag",
            "v1.0.0",
            "--version",
            "1.0.0",
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    rejected = subprocess.run(
        [
            sys.executable,
            str(verifier),
            "--tag",
            "v1.0.0-rc2",
            "--version",
            "1.0.0",
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert accepted.returncode == 0, accepted.stderr
    assert rejected.returncode != 0
    assert "version mismatch" in rejected.stderr

    wrong_version = subprocess.run(
        [
            sys.executable,
            str(verifier),
            "--tag",
            "v1.0.0",
            "--version",
            "1.0.1",
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert wrong_version.returncode != 0
    assert "version mismatch" in wrong_version.stderr


def test_ci_and_release_workflows_guard_merge_and_publication() -> None:
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

    assert "push:\n    branches:\n      - master" in ci
    assert "pull_request:\n    branches:\n      - master" in ci
    assert "github.event.pull_request.number || github.ref" in ci

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

    for required in (
        "workflow_dispatch:",
        "target_sha:",
        "version:",
        "refs/heads/master",
        "git fetch origin master",
        "git tag -a",
        "gh release create",
        "--draft",
        "--verify-tag",
        "gh release upload",
        "gh release edit",
        "isImmutable",
        "sha256sum",
        "cmp --silent",
        "native-wheel-windows",
        "native-wheel-linux",
        "python scripts/package_plugin.py",
    ):
        assert required in release
    assert "push:" not in release
    assert "actions/upload-artifact@" in release
    assert release.count("contents: write") == 1
    publish_job = release.split("  publish-release:\n", 1)[1]
    assert "permissions:\n      contents: write" in publish_job
    assert "--force" not in release
    assert "--clobber" not in release
    assert "publication is intentionally outside this workflow" in release


def test_packager_writes_an_allowlisted_zip_sha256_sidecar(tmp_path: Path) -> None:
    wheel = tmp_path / "astrembodiment_core-1.0.0-cp312-abi3-win_amd64.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr("astrembodiment_core/__init__.py", "# wheel initializer\n")
        archive.writestr(
            "astrembodiment_core/_native.pyd",
            b"version contract_info health open ensure_genesis "
            b"prepare_rebirth_v1 confirm_rebirth_v1 "
            b"reconcile_seed_config_v1 ack_seed_config_writeback_v1 "
            b"semantic_revision_v1 apply_perception_proposal_v1 apply_event inspect "
            b"verify_replay flush_and_close NativeCoreError",
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
        assert all(
            info.date_time == (1980, 1, 1, 0, 0, 0) for info in archive.infolist()
        )
        assert all(
            info.compress_type == zipfile.ZIP_STORED for info in archive.infolist()
        )

    repeated_output = tmp_path / "astrbot_plugin_astrembodiment-repeat.zip"
    repeated = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(repeated_output),
            "--native-wheel",
            str(wheel),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert repeated.returncode == 0, repeated.stderr
    assert repeated_output.read_bytes() == output.read_bytes()

    overwrite = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(wheel),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert overwrite.returncode != 0
    assert "refusing to overwrite" in overwrite.stderr
