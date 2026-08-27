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
    version = _metadata_version()

    assert re.fullmatch(r"\d+\.\d+\.\d+", version)
    assert pyproject["project"]["version"] == version
    assert cargo["workspace"]["package"]["version"] == version
    assert f"## [{version}]" in changelog


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


def test_release_contract_verifier_derives_and_checks_the_production_tag() -> None:
    verifier = ROOT / "scripts" / "verify_release_contract.py"
    assert verifier.is_file()
    version = _metadata_version()
    tag = f"v{version}"

    derived_version = subprocess.run(
        [sys.executable, str(verifier), "--field", "version"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    derived_tag = subprocess.run(
        [sys.executable, str(verifier), "--field", "tag"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    accepted = subprocess.run(
        [
            sys.executable,
            str(verifier),
            "--tag",
            tag,
            "--version",
            version,
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
            f"{tag}-rc1",
            "--version",
            version,
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert derived_version.returncode == 0, derived_version.stderr
    assert derived_version.stdout.strip() == version
    assert derived_tag.returncode == 0, derived_tag.stderr
    assert derived_tag.stdout.strip() == tag
    assert accepted.returncode == 0, accepted.stderr
    assert rejected.returncode != 0
    assert "version mismatch" in rejected.stderr

    wrong_version = subprocess.run(
        [
            sys.executable,
            str(verifier),
            "--tag",
            tag,
            "--version",
            "0.0.0",
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
    assert "astrbot_plugin_astrembodiment-v1.0.0" not in ci
    assert "astrbot-plugin-1.0.0-release-candidate" not in ci
    assert "--field version" in ci

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
        "workflow_run:",
        "workflows:\n      - CI",
        "types:\n      - completed",
        "workflow_dispatch:",
        "github.event.workflow_run.conclusion",
        "github.event.workflow_run.event",
        "github.event.workflow_run.head_branch",
        "github.event.workflow_run.head_repository.full_name",
        "github.event.workflow_run.head_sha",
        "workflow_dispatch must run on master",
        "stale successful CI run",
        "group: production-release",
        "cancel-in-progress: false",
        "--field version",
        "--field tag",
        "refs/heads/master",
        "git fetch --no-tags origin '+refs/heads/master:refs/remotes/origin/master'",
        "actions/workflows/ci.yml/runs?event=push&branch=master&status=completed&head_sha=${control_sha}",
        "--paginate --slurp",
        "workflow_dispatch requires a successful CI push run for current master",
        "git/tags",
        "git/refs",
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
        "release_action=NOOP",
        "release_action=RESUME",
        "release_action=PUBLISH",
        "release_action=RECOVER_TAG_ONLY",
        "::warning",
    ):
        assert required in release
    assert "push:" not in release
    assert "workflow_dispatch:\n    inputs:" not in release
    assert "Full SHA of the current master merge commit to publish" not in release
    assert "inputs.version" not in release
    assert "inputs.tag" not in release
    assert "actions/upload-artifact@" in release
    assert release.count("contents: write") == 1
    rebuilt_archive = 'rebuilt="dist/.${ARCHIVE_NAME%.zip}.rebuild.zip"'
    rebuilt_compare = 'cmp --silent "$archive" "$rebuilt"'
    rebuilt_cleanup = 'rm -f "$rebuilt"'
    assert rebuilt_archive in release
    assert rebuilt_compare in release
    assert rebuilt_cleanup in release
    assert release.index(rebuilt_archive) < release.index(rebuilt_compare)
    assert release.index(rebuilt_compare) < release.index(rebuilt_cleanup)
    assemble_release = release.split("  assemble-release:\n", 1)[1].split(
        "\n  publish-release:", 1
    )[0]
    assert release.count("sha256sum --check") == 5
    assert 'cd "$(dirname "$checksum")"' in assemble_release
    assert 'sha256sum --check "$(basename "$checksum")"' in assemble_release
    assert 'sha256sum --check "$checksum"' not in assemble_release
    select_target_job = release.split("  select-target:\n", 1)[1].split(
        "\n  build-native:", 1
    )[0]
    assert (
        "permissions:\n      contents: read\n      actions: read" in select_target_job
    )
    publish_job = release.split("  publish-release:\n", 1)[1]
    assert "permissions:\n      contents: write" in publish_job
    assert "actions: read" not in publish_job
    assert "actions: write" not in release
    assert "persist-credentials: true" not in publish_job
    assert "persist-credentials: false" in publish_job
    publish_setup_python = (
        "actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065"
    )
    assert publish_job.count(publish_setup_python) == 1
    publish_setup = publish_job.split(publish_setup_python, 1)[1].split(
        "\n      - ", 1
    )[0]
    assert 'python-version: "3.12"' in publish_setup
    publish_setup_index = publish_job.index(publish_setup_python)
    publish_python_calls = [
        match.start() for match in re.finditer(r"(?m)^\s*python(?:\s|$)", publish_job)
    ]
    assert publish_python_calls
    assert all(publish_setup_index < call for call in publish_python_calls)
    assert "git/tags" in publish_job
    assert "git/refs" in publish_job
    assert "--force" not in release
    assert "--clobber" not in release
    assert "per_page=100" not in release
    assert "git ls-remote --exit-code" not in release
    assert "publication is intentionally outside this workflow" in release
    assert release.count("python scripts/verify_release_contract.py") >= 3
    assert "needs.select-target.outputs.release_action != 'NOOP'" in release
    assert "control_sha: ${{ steps.target.outputs.control_sha }}" in release
    assert "tag_object_sha: ${{ steps.target.outputs.tag_object_sha }}" in release

    selector = release.split(
        "      - name: Verify trigger provenance, current master, and release state\n",
        1,
    )[1].split("\n  build-native:", 1)[0]
    for required in (
        'run.get("head_sha") != candidate_sha',
        'run.get("status") != "completed"',
        'run.get("conclusion") != "success"',
        'run.get("event") != "push"',
        'run.get("head_branch") != "master"',
        'head_repository.get("full_name") != repository',
        "published release target does not match its annotated tag",
        "published release asset set is invalid",
        "astr-embodiment-published-assets",
        "gh release download",
        "sha256sum --check",
    ):
        assert required in selector
    assert selector.count("len(asset_records) != 2") >= 1
    assert "release_target_sha" in selector
    assert 'control_sha="$candidate_sha"' in selector
    assert 'target_sha="$control_sha"' in selector
    assert '"$tag_target_sha" == "$control_sha"' in selector
    assert 'git merge-base --is-ancestor "$tag_target_sha" "$control_sha"' in selector
    assert (
        'require_successful_master_ci "$tag_target_sha" "tag-only recovery"' in selector
    )
    assert 'git cat-file -p "$tag_object_sha"' in selector
    assert "github-actions[bot]" in selector
    assert 'if message != f"Release {version}":' in selector
    assert 'git show "${tag_target_sha}:metadata.yaml"' in selector
    assert "printf 'control_sha=%s\\n' \"$control_sha\"" in selector
    assert "printf 'target_sha=%s\\n' \"$target_sha\"" in selector
    assert "printf 'tag_object_sha=%s\\n' \"$tag_object_sha\"" in selector
    assert "RECOVER_TAG_ONLY" in selector
    assert "release tag did not resolve to its advertised annotated object" in selector
    assert (
        'cd "$download_dir"\n              sha256sum --check "$checksum_name"'
        in selector
    )
    assert "head_sha=${control_sha}" in selector
    assert "--paginate --slurp" in selector
    assert 'git ls-remote --tags origin "$tag_ref" "${tag_ref}^{}"' in selector
    assert 'if [[ ! -s "$tag_listing" ]]; then' in selector
    assert "remote release tag lookup failed" in selector
    assert "remote release tag lookup returned an unexpected ref" in selector
    assert "one annotated ref and one peeled ref" in selector

    assert (
        publish_job.count('git ls-remote --tags origin "$tag_ref" "${tag_ref}^{}"') == 1
    )
    assert 'if [[ ! -s "$tag_listing" ]]; then' in publish_job
    assert "remote release tag lookup failed" in publish_job
    assert "remote release tag lookup returned an unexpected ref" in publish_job
    assert "one annotated ref and one peeled ref" in publish_job
    assert (
        'cd release-assets\n            sha256sum --check "$CHECKSUM_NAME"'
        in publish_job
    )
    assert publish_job.count('cd "$download_dir"') >= 2
    assert publish_job.count('sha256sum --check "$CHECKSUM_NAME"') == 3
    assert "GH_REPO: ${{ github.repository }}" in publish_job
    assert "CONTROL_SHA: ${{ needs.select-target.outputs.control_sha }}" in publish_job
    assert (
        "EXPECTED_TAG_OBJECT_SHA: ${{ needs.select-target.outputs.tag_object_sha }}"
        in publish_job
    )
    assert (
        "RELEASE_ACTION: ${{ needs.select-target.outputs.release_action }}"
        in publish_job
    )
    assert 'test "$(git rev-parse origin/master)" = "$CONTROL_SHA"' in publish_job
    assert "assert_publish_preconditions" in publish_job
    assert "release tag drifted after target selection" in publish_job
    release_create = publish_job.split("gh release create", 1)[1].split(
        "state=DRAFT", 1
    )[0]
    assert "--notes-from-tag" in release_create
    assert '--repo "$GITHUB_REPOSITORY"' not in release_create

    draft_assets = publish_job.split(
        "      - name: Verify or upload draft assets without replacing existing "
        "assets\n",
        1,
    )[1].split("\n      - name: Publish the verified draft\n", 1)[0]
    assert "gh release upload" in draft_assets
    assert "gh release download" in draft_assets
    assert "cmp --silent" in draft_assets
    assert "draft release asset set is invalid" in draft_assets
    assert draft_assets.count("len(asset_records) != 2") >= 1
    assert draft_assets.index("gh release upload") < draft_assets.index(
        "gh release download"
    )

    published_assets = publish_job.split(
        "      - name: Verify published assets and report GitHub immutability\n", 1
    )[1]
    assert "published release asset set is invalid" in published_assets
    assert published_assets.count("len(asset_records) != 2") >= 1


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
