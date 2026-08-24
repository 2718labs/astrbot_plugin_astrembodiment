from __future__ import annotations

import hashlib
import importlib
import json
import os
import subprocess
import sys
import zipfile
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
FRESH_WHEELS_DIR_ENV = "ASTREMBODIMENT_FRESH_WHEELS_DIR"
FRESH_WINDOWS_ARTIFACT = "native-wheel-windows"
FRESH_LINUX_ARTIFACT = "native-wheel-linux"
NATIVE_API = {
    "version",
    "health",
    "open",
    "ensure_genesis",
    "prepare_rebirth_v1",
    "confirm_rebirth_v1",
    "semantic_revision_v1",
    "apply_perception_proposal_v1",
    "apply_event",
    "inspect",
    "verify_replay",
    "flush_and_close",
    "NativeCoreError",
}
NATIVE_API_PAYLOAD = b" ".join(marker.encode() for marker in sorted(NATIVE_API))


def _fresh_wheels_root() -> Path:
    configured = os.environ.get(FRESH_WHEELS_DIR_ENV)
    root = Path(configured) if configured else ROOT / "wheels"
    if not root.is_absolute():
        root = ROOT / root
    root = root.resolve()
    if not root.is_dir():
        pytest.fail(
            f"fresh wheel directory is required at {root}; "
            f"set {FRESH_WHEELS_DIR_ENV} or run from the assemble job"
        )
    return root


def _single_fresh_wheel(wheels_root: Path, artifact: str) -> Path:
    candidates = sorted(
        path for path in (wheels_root / artifact).glob("*.whl") if path.is_file()
    )
    if len(candidates) != 1:
        pytest.fail(
            f"expected exactly one fresh wheel in {wheels_root / artifact}, "
            f"found {len(candidates)}; no historical wheel fallback is allowed"
        )
    return candidates[0]


@pytest.fixture(scope="module")
def fresh_native_wheels() -> tuple[Path, Path]:
    wheels_root = _fresh_wheels_root()
    return (
        _single_fresh_wheel(wheels_root, FRESH_WINDOWS_ARTIFACT),
        _single_fresh_wheel(wheels_root, FRESH_LINUX_ARTIFACT),
    )


def test_release_metadata_and_required_files_are_present() -> None:
    metadata = (ROOT / "metadata.yaml").read_text(encoding="utf-8")
    assert 'version: "1.0.0"' in metadata
    assert 'astrbot_version: ">=4.16,<5"' in metadata
    assert "support_platforms:" in metadata
    for relative_path in ("LICENSE", "CHANGELOG.md", ".github/workflows/ci.yml"):
        assert (ROOT / relative_path).is_file()


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


def test_release_archive_bundles_only_runtime_files(
    tmp_path: Path, fresh_native_wheels: tuple[Path, Path]
) -> None:
    fresh_windows_wheel, _ = fresh_native_wheels
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-win_amd64.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(fresh_windows_wheel),
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
    assert fresh_windows_wheel.name not in names
    assert "logo.png" in names
    assert "LICENSE" in names
    assert "CHANGELOG.md" in names
    assert "tests/test_static_contracts.py" not in names
    assert not any(name.startswith("crates/") for name in names)
    assert output.stat().st_size <= 16 * 1024 * 1024


def test_fresh_wheel_members_are_copied_byte_for_byte_to_bundled_paths(
    tmp_path: Path, fresh_native_wheels: tuple[Path, Path]
) -> None:
    fresh_windows_wheel, fresh_linux_wheel = fresh_native_wheels
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-native-refresh.zip"
    command = [
        sys.executable,
        str(ROOT / "scripts" / "package_plugin.py"),
        "--output",
        str(output),
        "--native-wheel",
        str(fresh_windows_wheel),
        "--native-wheel",
        str(fresh_linux_wheel),
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
        for wheel_path in (fresh_windows_wheel, fresh_linux_wheel):
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
    tmp_path: Path, fresh_native_wheels: tuple[Path, Path]
) -> None:
    fresh_windows_wheel, _ = fresh_native_wheels
    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-native-smoke.zip"
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(fresh_windows_wheel),
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
        assert health.version == "1.0.0"
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
        tmp_path / "astrembodiment_core-1.0.0-cp312-abi3-manylinux_2_17_x86_64.whl"
    )
    with zipfile.ZipFile(linux_wheel, "w") as wheel:
        wheel.writestr(
            "astrembodiment_core/__init__.py", "from ._native import health, version\n"
        )
        wheel.writestr(
            "astrembodiment_core/_native.abi3.so",
            b"linux-native-placeholder " + NATIVE_API_PAYLOAD,
        )

    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-linux_x86_64.zip"
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

    output = tmp_path / "astrbot_plugin_astrembodiment-1.0.0-universal.zip"
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
