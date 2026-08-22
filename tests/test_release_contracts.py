from __future__ import annotations

import hashlib
import importlib
import json
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path
from types import ModuleType, SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
FRESH_WHEELS_DIR = ROOT.parents[1] / ".codex-task-temp"
CURRENT_WHEEL_VERSION = tomllib.loads(
    (ROOT / "pyproject.toml").read_text(encoding="utf-8")
)["project"]["version"]


def _fresh_native_wheel(platform: str) -> Path | None:
    candidates = sorted(
        (FRESH_WHEELS_DIR / f"rebuild-native-{platform}-current" / "dist").glob(
            f"*{CURRENT_WHEEL_VERSION}*.whl"
        )
    )
    return candidates[-1] if candidates else None


FRESH_WINDOWS_WHEEL = _fresh_native_wheel("win")
FRESH_LINUX_WHEEL = _fresh_native_wheel("linux")
SEMANTIC_NATIVE_API = {
    "semantic_revision_v1",
    "apply_perception_proposal_v1",
}
NATIVE_API = {
    "version",
    "health",
    "open",
    "ensure_genesis",
    "apply_event",
    "inspect",
    "verify_replay",
    "flush_and_close",
    "NativeCoreError",
} | SEMANTIC_NATIVE_API
NATIVE_API_PAYLOAD = b" ".join(marker.encode() for marker in sorted(NATIVE_API))


def test_release_version_metadata_and_required_files_are_present() -> None:
    metadata = (ROOT / "metadata.yaml").read_text(encoding="utf-8")
    pyproject = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    changelog = (ROOT / "CHANGELOG.md").read_text(encoding="utf-8")
    lock = tomllib.loads((ROOT / "Cargo.lock").read_text(encoding="utf-8"))

    assert 'version: "1.0.0-rc2"' in metadata
    assert 'astrbot_version: ">=4.16,<5"' in metadata
    assert "support_platforms:" in metadata
    assert pyproject["project"]["version"] == "1.0.0rc2"
    assert cargo["workspace"]["package"]["version"] == "1.0.0-rc2"
    assert "## [1.0.0-rc2] - 2026-08-23" in changelog
    assert "## [1.0.1]" not in changelog
    assert "## [1.0.2]" not in changelog

    workspace_packages = {
        tomllib.loads((ROOT / member / "Cargo.toml").read_text(encoding="utf-8"))[
            "package"
        ]["name"]
        for member in cargo["workspace"]["members"]
    }
    locked_workspace_versions = {
        package["name"]: package["version"]
        for package in lock["package"]
        if package["name"] in workspace_packages
    }
    assert locked_workspace_versions.keys() == workspace_packages
    assert set(locked_workspace_versions.values()) == {"1.0.0-rc2"}

    for relative_path in (
        "LICENSE",
        "CHANGELOG.md",
        ".github/workflows/ci.yml",
        ".github/workflows/release.yml",
        ".github/dependabot.yml",
        "scripts/verify_release_contract.py",
    ):
        assert (ROOT / relative_path).is_file()


def test_product_readme_and_metadata_match_rc2_contract() -> None:
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    metadata = (ROOT / "metadata.yaml").read_text(encoding="utf-8")

    assert "# 让你的 Bot 不只记住经历，更能延续「Ta是谁」" in readme
    assert "**用户话语 → 15 维闭合语义证据 → 原生语义提交 → 同轮表达倾向**" in readme
    assert "当前版本：1.0.0-rc2 本地候选" in readme
    assert "尚未创建 GitHub Release，也未上架 AstrBot Marketplace" in readme
    assert "标签发布工作流完成后" in readme
    assert "native semantic commit" not in readme
    assert "## 为什么是人格连续性" in readme
    assert "## 现在已经能做什么" in readme
    assert "## 一次互动如何进入连续性" in readme
    assert "## Observatory：看见每次提交了什么" in readme
    assert (
        "SUCCESS 和 NOOP 以 INFO 记录；DEGRADED、REJECTED 与 INJECTION_FAILED "
        "以 WARNING 记录" in readme
    )
    assert "15 个维度都会进入原生注意力负载" in readme
    assert "同轮表达上下文" in readme
    assert "不等同于意识、主观感受或真实关系" in readme
    assert "calculation_state=CONFIRMED" in readme
    assert "state_changed`、`active_nodes`、`active_edges`" in readme
    assert "expression_state" in readme
    assert "expression_profile_fxp6" in readme
    assert (
        "不记录用户消息、Provider 输出、token、nonce、SeedCode、状态 digest 或原始神经节点"
        in readme
    )

    assert (
        "desc: 让你的 Bot 不只记住经历，更能延续“Ta是谁”。AstrEmbodiment 以 Rust 原生运行时承载人格连续性，将用户话语转化为 15 维闭合语义证据，提交为可追溯的原生状态，并在同一回合提供受限表达倾向。"
        in metadata
    )
    assert (
        "short_desc: Rust 原生人格连续性运行时：15 维语义证据、持久原生提交与受限同轮表达。"
        in metadata
    )
    for unchanged_field in (
        "name: astrbot_plugin_astrembodiment",
        "display_name: AstrEmbodiment",
        'version: "1.0.0-rc2"',
        "repo: https://github.com/2718labs/astrbot_plugin_astrembodiment",
        'astrbot_version: ">=4.16,<5"',
        "support_platforms:",
        "tags:",
    ):
        assert unchanged_field in metadata


def test_release_automation_is_pinned_and_tag_gated() -> None:
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    release = (ROOT / ".github" / "workflows" / "release.yml").read_text(
        encoding="utf-8"
    )
    dependabot = (ROOT / ".github" / "dependabot.yml").read_text(encoding="utf-8")

    for workflow in (ci, release):
        assert "permissions:" in workflow
        assert "actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683" in workflow
        assert (
            "actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065" in workflow
        )
    assert "ruff format --check" in ci
    assert "ruff check --select E,F" in ci
    assert "cargo fmt --all -- --check" in ci
    assert "cargo clippy --workspace --all-targets --locked -- -D warnings" in ci
    assert "AE_RC1_TASK_TEMP: ${{ runner.temp }}" in ci
    assert "artifact: native-wheel-windows" in ci
    assert "artifact: native-wheel-linux" in ci
    assert "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02" in ci
    assert "name: ${{ matrix.artifact }}" in ci
    assert "path: dist/*.whl" in ci
    assert "if-no-files-found: error" in ci
    assert "native-wheel-windows" in release
    assert "native-wheel-linux" in release
    assert "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02" in release
    assert (
        "actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093" in release
    )
    assert "tags:" in release
    assert '- "v*"' in release
    assert "refs/remotes/origin/master" in release
    assert "release tag must point to current origin/master" in release
    assert "contents: write" in release
    assert "verify_release_contract.py --tag" in release
    assert "gh release create" in release
    assert "package-ecosystem: github-actions" in dependabot
    assert "package-ecosystem: cargo" in dependabot


def test_release_contract_checker_accepts_rc2_and_rejects_mismatched_tag() -> None:
    command = [sys.executable, str(ROOT / "scripts" / "verify_release_contract.py")]
    accepted = subprocess.run(
        [*command, "--tag", "v1.0.0-rc2"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    rejected = subprocess.run(
        [*command, "--tag", "v1.0.1-rc2"],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert accepted.returncode == 0, accepted.stderr
    assert rejected.returncode != 0
    assert "version mismatch" in rejected.stderr


def test_plugin_entrypoint_uses_astrbot_auto_discovery() -> None:
    entrypoint = (ROOT / "main.py").read_text(encoding="utf-8")
    assert "from astrbot.api.star import Context, Star, register" not in entrypoint
    assert "@register(" not in entrypoint


def test_plugin_entrypoint_uses_package_relative_host_imports() -> None:
    entrypoint = (ROOT / "main.py").read_text(encoding="utf-8")
    bridge = (ROOT / "astr_embodiment" / "bridge.py").read_text(encoding="utf-8")
    assert (
        "from .astr_embodiment import NativeBridge, NativeCoreUnavailable" in entrypoint
    )
    assert 'import_module("..astrembodiment_core", package_name)' in bridge


def test_runtime_requirements_match_self_contained_archive() -> None:
    requirements = (ROOT / "requirements.txt").read_text(encoding="utf-8")
    install_lines = [
        line.strip()
        for line in requirements.splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]
    assert install_lines == []
    assert "Native Windows and Linux extensions are bundled" in requirements


def test_native_loader_exports_semantic_native_api_as_callables(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    package_root = tmp_path / "astrembodiment_core"
    package_root.mkdir()
    wrapper_path = package_root / "__init__.py"
    wrapper_path.write_bytes(
        (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_bytes()
    )

    native_payload = b"release-contract-native"
    build_id = hashlib.sha256(native_payload).hexdigest()
    bundled_root = package_root / "_bundled"
    native_filename = "_native.pyd" if sys.platform == "win32" else "_native.abi3.so"
    platform = "win32" if sys.platform == "win32" else "linux"
    native_path = bundled_root / build_id / native_filename
    native_path.parent.mkdir(parents=True)
    native_path.write_bytes(native_payload)
    (bundled_root / "manifest.json").write_text(
        json.dumps(
            {
                "schema": "astrembodiment-native-bundle-v1",
                "platforms": {
                    platform: {"build_id": build_id, "filename": native_filename}
                },
            }
        ),
        encoding="utf-8",
    )

    functions = {
        name: (lambda *args, _name=name, **kwargs: (_name, args, kwargs))
        for name in NATIVE_API - {"NativeCoreError"}
    }
    native_error = type("NativeCoreError", (RuntimeError,), {})

    class FakeNativeLoader:
        def exec_module(self, module: ModuleType) -> None:
            for name, function in functions.items():
                setattr(module, name, function)
            module.NativeCoreError = native_error

    fake_spec = SimpleNamespace(loader=FakeNativeLoader())
    monkeypatch.setattr(
        importlib.util,
        "spec_from_file_location",
        lambda name, path: fake_spec,
    )
    monkeypatch.setattr(
        importlib.util,
        "module_from_spec",
        lambda spec: ModuleType("release_contracts._native"),
    )

    module = ModuleType("release_contracts.loader")
    module.__file__ = str(wrapper_path)
    exec(compile(wrapper_path.read_bytes(), str(wrapper_path), "exec"), module.__dict__)

    assert SEMANTIC_NATIVE_API <= set(module.__all__)
    for name in SEMANTIC_NATIVE_API:
        assert getattr(module, name) is functions[name]
        assert callable(getattr(module, name))


def test_release_archive_rejects_wheel_missing_semantic_api_markers(
    tmp_path: Path,
) -> None:
    legacy_payload = b" ".join(
        marker.encode() for marker in sorted(NATIVE_API - SEMANTIC_NATIVE_API)
    )
    wheel = tmp_path / "legacy-core-win.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr("astrembodiment_core/__init__.py", "# legacy wheel\n")
        archive.writestr("astrembodiment_core/_native.pyd", legacy_payload)

    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(tmp_path / "archive.zip"),
            "--native-wheel",
            str(wheel),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode != 0
    for marker in SEMANTIC_NATIVE_API:
        assert marker in result.stderr


def test_release_archive_uses_current_native_initializer_and_not_wheels(
    tmp_path: Path,
) -> None:
    wheel = tmp_path / "core-win.whl"
    with zipfile.ZipFile(wheel, "w") as archive:
        archive.writestr(
            "astrembodiment_core/__init__.py",
            "# stale wheel initializer\n",
        )
        archive.writestr("astrembodiment_core/_native.pyd", NATIVE_API_PAYLOAD)

    output = tmp_path / "archive.zip"
    result = subprocess.run(
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
    assert result.returncode == 0, result.stderr
    source_initializer = (
        ROOT / "python" / "astrembodiment_core" / "__init__.py"
    ).read_bytes()
    build_id = hashlib.sha256(NATIVE_API_PAYLOAD).hexdigest()
    with zipfile.ZipFile(output) as archive:
        assert archive.read("astrembodiment_core/__init__.py") == source_initializer
        assert (
            archive.read(f"astrembodiment_core/_bundled/{build_id}/_native.pyd")
            == NATIVE_API_PAYLOAD
        )
        manifest = json.loads(
            archive.read("astrembodiment_core/_bundled/manifest.json")
        )
        assert manifest == {
            "schema": "astrembodiment-native-bundle-v1",
            "platforms": {"win32": {"build_id": build_id, "filename": "_native.pyd"}},
        }
        assert not any(name.endswith(".whl") for name in archive.namelist())


@pytest.mark.skipif(
    FRESH_WINDOWS_WHEEL is None,
    reason="requires a fresh current-version Windows native wheel",
)
def test_release_archive_bundles_only_runtime_files(tmp_path: Path) -> None:
    assert FRESH_WINDOWS_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-win_amd64.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(FRESH_WINDOWS_WHEEL),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    with zipfile.ZipFile(output) as archive:
        names = set(archive.namelist())
    assert "astrembodiment_core/__init__.py" in names
    assert "astrembodiment_core/_bundled/manifest.json" in names
    assert not any(name.startswith("astrembodiment_core/_native") for name in names)
    assert FRESH_WINDOWS_WHEEL.name not in names
    assert "logo.png" in names
    assert "LICENSE" in names
    assert "CHANGELOG.md" in names
    assert "tests/test_static_contracts.py" not in names
    assert not any(name.startswith("crates/") for name in names)
    assert output.stat().st_size <= 16 * 1024 * 1024


@pytest.mark.skipif(
    FRESH_WINDOWS_WHEEL is None or FRESH_LINUX_WHEEL is None,
    reason="requires fresh current-version Windows and Linux native wheels",
)
def test_fresh_wheel_members_are_copied_byte_for_byte_to_bundled_paths(
    tmp_path: Path,
) -> None:
    assert FRESH_WINDOWS_WHEEL is not None
    assert FRESH_LINUX_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-native-refresh.zip"
    command = [
        sys.executable,
        str(ROOT / "scripts" / "package_plugin.py"),
        "--output",
        str(output),
        "--native-wheel",
        str(FRESH_WINDOWS_WHEEL),
        "--native-wheel",
        str(FRESH_LINUX_WHEEL),
    ]
    result = subprocess.run(
        command,
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr

    with zipfile.ZipFile(output) as archive:
        names = set(archive.namelist())
        assert "logo.png" in names
        assert not any(name.endswith(".whl") for name in names)
        assert "astrembodiment_core/_bundled/manifest.json" in names
        manifest = json.loads(
            archive.read("astrembodiment_core/_bundled/manifest.json")
        )
        for wheel_path in (FRESH_WINDOWS_WHEEL, FRESH_LINUX_WHEEL):
            with zipfile.ZipFile(wheel_path) as wheel:
                native_member = next(
                    name
                    for name in wheel.namelist()
                    if name.startswith("astrembodiment_core/_native")
                    and name.endswith((".pyd", ".so"))
                )
                wheel_bytes = wheel.read(native_member)
            filename = Path(native_member).name
            build_id = hashlib.sha256(wheel_bytes).hexdigest()
            platform = "win32" if filename.endswith(".pyd") else "linux"
            archive_member = f"astrembodiment_core/_bundled/{build_id}/{filename}"
            assert all(symbol.encode("ascii") in wheel_bytes for symbol in NATIVE_API)
            assert archive_member in names
            archive_bytes = archive.read(archive_member)
            assert archive_bytes == wheel_bytes
            assert (
                hashlib.sha256(archive_bytes).hexdigest()
                == hashlib.sha256(wheel_bytes).hexdigest()
            )
            assert all(symbol.encode("ascii") in archive_bytes for symbol in NATIVE_API)
            assert manifest["platforms"][platform] == {
                "build_id": build_id,
                "filename": filename,
            }


@pytest.mark.skipif(
    sys.platform != "win32" or FRESH_WINDOWS_WHEEL is None,
    reason="requires a fresh current-version Windows native wheel",
)
def test_fresh_archive_imports_native_api_in_clean_astrbot_namespace(
    tmp_path: Path,
) -> None:
    assert FRESH_WINDOWS_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-native-smoke.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(FRESH_WINDOWS_WHEEL),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr

    namespace_root = tmp_path / "namespace"
    plugin_root = namespace_root / "data" / "plugins" / "astrbot_plugin_astrembodiment"
    plugin_root.mkdir(parents=True)
    with zipfile.ZipFile(output) as archive:
        archive.extractall(plugin_root)
    for package_dir in (
        namespace_root / "data",
        namespace_root / "data" / "plugins",
        plugin_root,
    ):
        (package_dir / "__init__.py").write_text("", encoding="utf-8")

    module_prefix = "data.plugins.astrbot_plugin_astrembodiment"
    previous_modules = {
        name: module
        for name, module in sys.modules.items()
        if name == "data" or name.startswith("data.")
    }
    sys.path.insert(0, str(namespace_root))
    try:
        sys.modules.pop("astrembodiment_core", None)
        for name in list(sys.modules):
            if name == "data" or name.startswith("data."):
                sys.modules.pop(name, None)
        main_module = importlib.import_module(f"{module_prefix}.main")
        bridge_module = importlib.import_module(
            f"{module_prefix}.astr_embodiment.bridge"
        )
        native = importlib.import_module(f"{module_prefix}.astrembodiment_core")
        assert NATIVE_API <= set(dir(native))
        assert all(
            callable(getattr(native, symbol))
            for symbol in NATIVE_API - {"NativeCoreError"}
        )
        assert native.NativeCoreError.__name__ == "NativeCoreError"
        runtime_dir = tmp_path / "runtime"
        runtime_dir.mkdir()
        health = bridge_module.NativeBridge().open(str(runtime_dir))
        assert health.version == "1.0.0-rc2"
        assert (runtime_dir / "astrembodiment.sqlite3").is_file()
        assert main_module.AstrEmbodimentPlugin is not None
    finally:
        sys.path.remove(str(namespace_root))
        for name in list(sys.modules):
            if name == "data" or name.startswith("data."):
                sys.modules.pop(name, None)
        sys.modules.update(previous_modules)


def test_release_archive_accepts_linux_abi3_extension(tmp_path: Path) -> None:
    linux_wheel = (
        tmp_path / "astrembodiment_core-1.0.0-rc2-cp312-abi3-manylinux_2_17_x86_64.whl"
    )
    with zipfile.ZipFile(linux_wheel, "w") as wheel:
        wheel.writestr(
            "astrembodiment_core/__init__.py", "from ._native import health, version\n"
        )
        wheel.writestr(
            "astrembodiment_core/_native.abi3.so",
            b"linux-native-placeholder " + NATIVE_API_PAYLOAD,
        )

    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-linux_x86_64.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(linux_wheel),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    with zipfile.ZipFile(output) as archive:
        names = set(archive.namelist())
        payload = b"linux-native-placeholder " + NATIVE_API_PAYLOAD
        build_id = hashlib.sha256(payload).hexdigest()
        assert f"astrembodiment_core/_bundled/{build_id}/_native.abi3.so" in names
        assert "astrembodiment_core/_bundled/manifest.json" in names
    assert not any(name.startswith("astrembodiment_core/_native") for name in names)
    assert linux_wheel.name not in names


def test_release_archive_can_bundle_windows_and_linux_extensions(
    tmp_path: Path,
) -> None:
    init = "from ._native import health, version\n"
    windows_wheel = tmp_path / "core-win.whl"
    linux_wheel = tmp_path / "core-linux.whl"
    with zipfile.ZipFile(windows_wheel, "w") as wheel:
        wheel.writestr("astrembodiment_core/__init__.py", init)
        wheel.writestr(
            "astrembodiment_core/_native.pyd",
            b"windows-native-placeholder " + NATIVE_API_PAYLOAD,
        )
    with zipfile.ZipFile(linux_wheel, "w") as wheel:
        wheel.writestr("astrembodiment_core/__init__.py", init)
        wheel.writestr(
            "astrembodiment_core/_native.abi3.so",
            b"linux-native-placeholder " + NATIVE_API_PAYLOAD,
        )

    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-universal.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(windows_wheel),
            "--native-wheel",
            str(linux_wheel),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    with zipfile.ZipFile(output) as archive:
        names = set(archive.namelist())
        windows_build_id = hashlib.sha256(
            b"windows-native-placeholder " + NATIVE_API_PAYLOAD
        ).hexdigest()
        linux_build_id = hashlib.sha256(
            b"linux-native-placeholder " + NATIVE_API_PAYLOAD
        ).hexdigest()
        assert f"astrembodiment_core/_bundled/{windows_build_id}/_native.pyd" in names
        assert f"astrembodiment_core/_bundled/{linux_build_id}/_native.abi3.so" in names
        assert "astrembodiment_core/_bundled/manifest.json" in names
    assert not any(name.startswith("astrembodiment_core/_native") for name in names)
    assert windows_wheel.name not in names
    assert linux_wheel.name not in names
