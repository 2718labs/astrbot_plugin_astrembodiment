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
FRESH_WINDOWS_WHEEL = next(
    (FRESH_WHEELS_DIR / "rebuild-native-win-current" / "dist").glob("*.whl"),
    None,
)
FRESH_LINUX_WHEEL = next(
    (FRESH_WHEELS_DIR / "rebuild-native-linux-current" / "dist").glob("*.whl"),
    None,
)
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

    assert 'version: "1.0.0-rc1"' in metadata
    assert 'astrbot_version: ">=4.16,<5"' in metadata
    assert "support_platforms:" in metadata
    assert pyproject["project"]["version"] == "1.0.0rc1"
    assert cargo["workspace"]["package"]["version"] == "1.0.0-rc1"
    assert "## [1.0.0-rc1] - 2026-08-22" in changelog

    workspace_packages = {
        tomllib.loads((ROOT / member / "Cargo.toml").read_text(encoding="utf-8"))["package"][
            "name"
        ]
        for member in cargo["workspace"]["members"]
    }
    locked_workspace_versions = {
        package["name"]: package["version"]
        for package in lock["package"]
        if package["name"] in workspace_packages
    }
    assert locked_workspace_versions.keys() == workspace_packages
    assert set(locked_workspace_versions.values()) == {"1.0.0-rc1"}

    for relative_path in ("LICENSE", "CHANGELOG.md", ".github/workflows/ci.yml"):
        assert (ROOT / relative_path).is_file()


def test_product_readme_and_metadata_match_rc1_contract() -> None:
    readme = (ROOT / "README.md").read_text(encoding="utf-8")
    metadata = (ROOT / "metadata.yaml").read_text(encoding="utf-8")

    assert "# 让你的 Bot 不只记住经历，更能延续「Ta是谁」" in readme
    assert "**用户话语 → 15 维闭合语义证据 → native semantic commit**" in readme
    assert "当前版本：1.0.0-rc1 本地候选" in readme
    assert "尚未创建 GitHub Release，也未上架 AstrBot Marketplace" in readme
    assert "## 为什么是人格连续性" in readme
    assert "## 现在已经能做什么" in readme
    assert "## 一次互动如何进入连续性" in readme
    assert "## Observatory：看见每次提交了什么" in readme
    assert "SUCCESS 和 NOOP 以 INFO 记录；DEGRADED 以 WARNING 记录" in readme
    assert "positive`、`harm`、`boundary` 和 `epistemic_conflict`" in readme
    assert "受控回应策略和外显人格漂移仍是后续能力" in readme
    assert "不记录用户消息、Provider 输出、token、nonce、SeedCode 或状态摘要" in readme

    assert (
        "desc: 让你的 Bot 不只记住经历，更能延续“Ta是谁”。AstrEmbodiment 以 Rust 原生运行时承载人格连续性，将用户话语转化为 15 维闭合语义证据并提交原生状态，为受控回应与人格演化提供试验性基础。"
        in metadata
    )
    assert (
        "short_desc: Rust 原生人格连续性运行时：让用户话语成为可验证、可提交的连续语义证据。"
        in metadata
    )
    for unchanged_field in (
        "name: astrbot_plugin_astrembodiment",
        "display_name: AstrEmbodiment",
        'version: "1.0.0-rc1"',
        "repo: https://github.com/2718labs/astrbot_plugin_astrembodiment",
        'astrbot_version: ">=4.16,<5"',
        "support_platforms:",
        "tags:",
    ):
        assert unchanged_field in metadata


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


def test_release_archive_bundles_only_runtime_files(tmp_path: Path) -> None:
    assert FRESH_WINDOWS_WHEEL is not None, (
        "build the native wheel before package verification"
    )
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc1-win_amd64.zip"
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


def test_fresh_wheel_members_are_copied_byte_for_byte_to_bundled_paths(
    tmp_path: Path,
) -> None:
    assert FRESH_WINDOWS_WHEEL is not None
    assert FRESH_LINUX_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc1-native-refresh.zip"
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


@pytest.mark.skipif(sys.platform != "win32", reason="requires the Windows fresh wheel")
def test_fresh_archive_imports_native_api_in_clean_astrbot_namespace(
    tmp_path: Path,
) -> None:
    assert FRESH_WINDOWS_WHEEL is not None
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc1-native-smoke.zip"
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
        assert health.version == "1.0.0-rc1"
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
        tmp_path / "astrembodiment_core-1.0.0-rc1-cp312-abi3-manylinux_2_17_x86_64.whl"
    )
    with zipfile.ZipFile(linux_wheel, "w") as wheel:
        wheel.writestr(
            "astrembodiment_core/__init__.py", "from ._native import health, version\n"
        )
        wheel.writestr(
            "astrembodiment_core/_native.abi3.so",
            b"linux-native-placeholder " + NATIVE_API_PAYLOAD,
        )

    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc1-linux_x86_64.zip"
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

    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-rc1-universal.zip"
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
