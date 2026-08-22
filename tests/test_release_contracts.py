from __future__ import annotations

import hashlib
import importlib
import json
import os
import subprocess
import sys
import tomllib
import zipfile
from pathlib import Path
from types import ModuleType, SimpleNamespace

import pytest

ROOT = Path(__file__).resolve().parents[1]
CURRENT_WHEEL_VERSION = tomllib.loads(
    (ROOT / "pyproject.toml").read_text(encoding="utf-8")
)["project"]["version"]
FRESH_NATIVE_WHEEL_DIR_ENV = "AE_FRESH_NATIVE_WHEEL_DIR"


def _fresh_native_wheel() -> Path | None:
    """Accept exactly the current job's native wheel, never a host scratch wheel."""
    configured_directory = os.environ.get(FRESH_NATIVE_WHEEL_DIR_ENV)
    if configured_directory is None:
        return None

    wheel_directory = Path(configured_directory)
    if not wheel_directory.is_dir():
        raise RuntimeError(
            f"{FRESH_NATIVE_WHEEL_DIR_ENV} must name an existing wheel directory: "
            f"{wheel_directory}"
        )

    version_marker = f"-{CURRENT_WHEEL_VERSION}-"
    if sys.platform == "win32":
        candidates = [
            path
            for path in sorted(wheel_directory.glob("*.whl"))
            if version_marker in path.name and path.name.endswith("-win_amd64.whl")
        ]
    elif sys.platform.startswith("linux"):
        candidates = [
            path
            for path in sorted(wheel_directory.glob("*.whl"))
            if version_marker in path.name
            and path.name.endswith("_x86_64.whl")
            and ("manylinux" in path.name or "musllinux" in path.name)
        ]
    else:
        raise RuntimeError(f"unsupported native wheel platform: {sys.platform}")

    if len(candidates) != 1:
        raise RuntimeError(
            f"{FRESH_NATIVE_WHEEL_DIR_ENV} must contain exactly one current "
            f"{sys.platform} {CURRENT_WHEEL_VERSION} native wheel; found: {candidates}"
        )
    return candidates[0]


FRESH_NATIVE_WHEEL = _fresh_native_wheel()
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


def test_fresh_native_wheel_requires_one_explicit_current_platform_candidate(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.delenv(FRESH_NATIVE_WHEEL_DIR_ENV, raising=False)
    assert _fresh_native_wheel() is None

    monkeypatch.setenv(FRESH_NATIVE_WHEEL_DIR_ENV, str(tmp_path))
    with pytest.raises(RuntimeError, match="exactly one current"):
        _fresh_native_wheel()

    if sys.platform == "win32":
        filename = (
            f"astrembodiment_core-{CURRENT_WHEEL_VERSION}-cp312-abi3-win_amd64.whl"
        )
        stale_filename = "astrembodiment_core-1.0.0rc1-cp312-abi3-win_amd64.whl"
    elif sys.platform.startswith("linux"):
        filename = (
            f"astrembodiment_core-{CURRENT_WHEEL_VERSION}-cp312-abi3-"
            "manylinux_2_34_x86_64.whl"
        )
        stale_filename = (
            "astrembodiment_core-1.0.0rc1-cp312-abi3-manylinux_2_34_x86_64.whl"
        )
    else:
        pytest.skip(f"unsupported native wheel platform: {sys.platform}")

    wheel = tmp_path / filename
    wheel.touch()
    (tmp_path / stale_filename).touch()
    assert _fresh_native_wheel() == wheel

    duplicate = tmp_path / f"duplicate-{filename}"
    duplicate.touch()
    with pytest.raises(RuntimeError, match="exactly one current"):
        _fresh_native_wheel()


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
    assert "CODEX_TASK_TEMP: ${{ runner.temp }}" in ci
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
    for workflow in (ci, release):
        assert "- name: 保存原生 wheel" in workflow
        assert "- name: 冒烟导入原生 wheel" in workflow
        assert "- name: 冒烟导入完整插件 bundle" in workflow
        assert (
            workflow.index("- name: 保存原生 wheel")
            < workflow.index("- name: 冒烟导入原生 wheel")
            < workflow.index("- name: 冒烟导入完整插件 bundle")
        )
        assert "continue-on-error" not in workflow
        assert FRESH_NATIVE_WHEEL_DIR_ENV in workflow


def test_rust_quality_pins_cpython_before_pyo3_builds() -> None:
    ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
    rust_quality = ci.split("  rust-quality:\n", 1)[1].split(
        "\n  release-contract:", 1
    )[0]
    setup_python = "actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065"

    assert setup_python in rust_quality
    assert 'python-version: "3.12"' in rust_quality
    assert rust_quality.index(setup_python) < rust_quality.index("cargo fmt --all")


def test_pyo3_extension_linking_is_owned_by_maturin() -> None:
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    pyproject = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))

    assert cargo["workspace"]["dependencies"]["pyo3"]["features"] == ["abi3-py312"]
    assert pyproject["build-system"]["build-backend"] == "maturin"
    assert "maturin>=1.14.1,<2.0" in pyproject["build-system"]["requires"]
    assert pyproject["tool"]["maturin"]["bindings"] == "pyo3"


def test_wheel_initializer_is_separate_from_plugin_bundle_loader() -> None:
    pyproject = tomllib.loads((ROOT / "pyproject.toml").read_text(encoding="utf-8"))
    wheel_initializer = ROOT / "python-wheel" / "astrembodiment_core" / "__init__.py"
    plugin_initializer = ROOT / "python" / "astrembodiment_core" / "__init__.py"
    packager = (ROOT / "scripts" / "package_plugin.py").read_text(encoding="utf-8")

    assert pyproject["tool"]["maturin"]["python-source"] == "python-wheel"
    assert wheel_initializer.is_file()
    wheel_source = wheel_initializer.read_text(encoding="utf-8")
    plugin_source = plugin_initializer.read_text(encoding="utf-8")
    assert "from ._native import" in wheel_source
    assert "_bundled" not in wheel_source
    assert "__all__" in wheel_source
    for name in NATIVE_API:
        assert f'"{name}"' in wheel_source
    assert "_bundled" in plugin_source
    assert "manifest.json" in plugin_source
    assert wheel_source != plugin_source
    assert 'NATIVE_SOURCE_INIT = ROOT / "python" / NATIVE_INIT' in packager
    assert "python-wheel" not in packager


def test_wheel_initializer_exports_current_native_public_api(tmp_path: Path) -> None:
    wheel_initializer = ROOT / "python-wheel" / "astrembodiment_core" / "__init__.py"
    assert wheel_initializer.is_file()

    package_dir = tmp_path / "astrembodiment_core"
    package_dir.mkdir()
    (package_dir / "__init__.py").write_bytes(wheel_initializer.read_bytes())
    native_source = ["class NativeCoreError(RuntimeError):\n    pass\n"]
    for name in sorted(NATIVE_API - {"NativeCoreError"}):
        if name == "version":
            native_source.append("def version():\n    return '1.0.0-rc2'\n")
        else:
            native_source.append(
                f"def {name}(*_args, **_kwargs):\n    return '{name}'\n"
            )
    native_path = package_dir / "_native.py"
    native_path.write_text("\n".join(native_source), encoding="utf-8")

    module_name = "_astrembodiment_core_wheel_initializer"
    spec = importlib.util.spec_from_file_location(
        module_name,
        package_dir / "__init__.py",
        submodule_search_locations=[str(package_dir)],
    )
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    try:
        spec.loader.exec_module(module)
        assert set(module.__all__) == NATIVE_API
        assert module.version() == "1.0.0-rc2"
        assert all(
            callable(getattr(module, name)) for name in NATIVE_API - {"NativeCoreError"}
        )
        assert Path(sys.modules[f"{module_name}._native"].__file__) == native_path
    finally:
        sys.modules.pop(module_name, None)
        sys.modules.pop(f"{module_name}._native", None)


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
    FRESH_NATIVE_WHEEL is None,
    reason="requires an explicit fresh current-platform native wheel",
)
def test_release_archive_bundles_only_runtime_files(tmp_path: Path) -> None:
    assert FRESH_NATIVE_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-native.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(FRESH_NATIVE_WHEEL),
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
    assert FRESH_NATIVE_WHEEL.name not in names
    assert "logo.png" in names
    assert "LICENSE" in names
    assert "CHANGELOG.md" in names
    assert "tests/test_static_contracts.py" not in names
    assert not any(name.startswith("crates/") for name in names)
    assert output.stat().st_size <= 16 * 1024 * 1024


@pytest.mark.skipif(
    FRESH_NATIVE_WHEEL is None,
    reason="requires an explicit fresh current-platform native wheel",
)
def test_fresh_wheel_member_is_copied_byte_for_byte_to_a_bundled_path(
    tmp_path: Path,
) -> None:
    assert FRESH_NATIVE_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-native-refresh.zip"
    command = [
        sys.executable,
        str(ROOT / "scripts" / "package_plugin.py"),
        "--output",
        str(output),
        "--native-wheel",
        str(FRESH_NATIVE_WHEEL),
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
        with zipfile.ZipFile(FRESH_NATIVE_WHEEL) as wheel:
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
    FRESH_NATIVE_WHEEL is None,
    reason="requires an explicit fresh current-platform native wheel",
)
def test_fresh_archive_imports_native_api_in_clean_astrbot_namespace(
    tmp_path: Path,
) -> None:
    assert FRESH_NATIVE_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc2-native-smoke.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(FRESH_NATIVE_WHEEL),
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
    (plugin_root / "astrembodiment_core" / "_native.py").write_text(
        "raise AssertionError('stale root native must never be imported')\n",
        encoding="utf-8",
    )
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
        assert native.version() == "1.0.0-rc2"
        assert all(
            callable(getattr(native, symbol))
            for symbol in NATIVE_API - {"NativeCoreError"}
        )
        assert native.NativeCoreError.__name__ == "NativeCoreError"
        native_module = sys.modules[f"{module_prefix}.astrembodiment_core._native"]
        native_path = Path(native_module.__file__).resolve()
        assert "_bundled" in native_path.parts
        bundled_index = native_path.parts.index("_bundled")
        assert len(native_path.parts) > bundled_index + 2
        assert native_path.parent.parent.name == "_bundled"
        assert native_path.name != "_native.py"
        assert len(native_path.parent.name) == 64
        assert all(
            character in "0123456789abcdef" for character in native_path.parent.name
        )
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
