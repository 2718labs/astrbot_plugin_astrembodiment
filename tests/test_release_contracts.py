from __future__ import annotations

import ast
import hashlib
import io
import importlib
import importlib.machinery
import importlib.util
import json
import os
import re
import subprocess
import sys
import zipfile
from pathlib import Path, PurePosixPath
from types import ModuleType

import pytest

ROOT = Path(__file__).resolve().parents[1]
FRESH_WHEELS_DIR = ROOT.parents[1] / ".codex-task-temp"
NATIVE_WHEEL_VERSION = "1.1.0"
NATIVE_RUNTIME_VERSION = "1.1.0"
WINDOWS_WHEEL_NAME = (
    f"astrembodiment_core-{NATIVE_WHEEL_VERSION}-cp312-abi3-win_amd64.whl"
)
LINUX_WHEEL_NAME = (
    f"astrembodiment_core-{NATIVE_WHEEL_VERSION}-cp312-abi3-manylinux_2_17_x86_64.whl"
)


def _single_fresh_wheel(
    override: str | None,
    directory: Path,
    pattern: str,
) -> Path | None:
    if override:
        return Path(override)
    candidates = sorted(directory.glob(pattern))
    return candidates[0] if len(candidates) == 1 else None


_windows_wheel_override = os.environ.get("AE_FRESH_WINDOWS_WHEEL")
FRESH_WINDOWS_WHEEL = _single_fresh_wheel(
    _windows_wheel_override,
    FRESH_WHEELS_DIR / "rebuild-native-win-current" / "dist",
    WINDOWS_WHEEL_NAME,
)
_linux_wheel_override = os.environ.get("AE_FRESH_LINUX_WHEEL")
FRESH_LINUX_WHEEL = _single_fresh_wheel(
    _linux_wheel_override,
    FRESH_WHEELS_DIR / "rebuild-native-linux-current" / "dist",
    f"astrembodiment_core-{NATIVE_WHEEL_VERSION}-cp312-abi3-manylinux*_x86_64.whl",
)
# Exact frozen Task 12 surface, independent of the packager's runtime lookup.
NATIVE_API = {
    "NativeCoreError",
    "advance_embodiment_time_v1",
    "build_info_v1",
    "commit_core_delivery_outcome_v1",
    "commit_core_inbound_v1",
    "compare_and_swap_embodiment_profile_v1",
    "compile_core_host_request_v1",
    "create_embodiment_persona_if_missing_v1",
    "embodiment_clock_status_v1",
    "ensure_genesis",
    "flush_and_close",
    "get_embodiment_persona_v1",
    "health",
    "inspect",
    "list_embodiment_personas_v1",
    "open",
    "read_embodiment_profile_v1",
    "settle_semantic_appraisal_v1",
    "verify_replay",
    "version",
}
LEGACY_MIND_EXPORTS = {
    "mood_card",
    "query_inner_events",
    "observe_events_v1",
    "observe_snapshot_v1",
}
RETIRED_ALPHA3_OPERATIONS = {
    "alpha3_call",
    "apply_event",
    "autonomy_status",
    "claim_wake",
    "settle_dispatch",
}
# Synthetic wheels below exercise validation only; real release acceptance uses
# AE_RELEASE_ARCHIVE and the matching platform wheels.
NATIVE_API_MARKERS_PAYLOAD = b" ".join(marker.encode() for marker in sorted(NATIVE_API))
NATIVE_API_PAYLOAD = NATIVE_API_MARKERS_PAYLOAD + b" " + NATIVE_RUNTIME_VERSION.encode()
HEADLESS_SURFACE_PATHS = (
    ".astrbot-plugin/i18n/en-US.json",
    ".astrbot-plugin/i18n/zh-CN.json",
    "astr_embodiment/observatory.py",
    "astr_embodiment/observatory_controls.py",
    "pages/observatory/app.js",
    "pages/observatory/index.html",
    "pages/observatory/style.css",
)
HEADLESS_FORBIDDEN_MEMBER_CASES = (
    *HEADLESS_SURFACE_PATHS,
    "ASTR_EMBODIMENT/OBSERVATORY.PY",
    "ASTR_EMBODIMENT/OBSERVATORY_CONTROLS.PY",
    "PAGES/Observatory/APP.JS",
    ".ASTRBOT-PLUGIN/I18N/EN-US.JSON",
)
HEADLESS_CONFIG_KEYS = {
    "observatory_enabled",
    "observatory_pages_enabled",
    "developer_observatory_enabled",
    "observatory_control_admin",
}
RETIRED_LIVE_MIND_MEMBERS = {
    "astr_embodiment/display.py",
    "astr_embodiment/ecosystem.py",
}
RELEASE_SOURCE_INPUTS = (
    "main.py",
    "metadata.yaml",
    "requirements.txt",
    "_conf_schema.json",
    "astr_embodiment",
    "README.md",
    "LICENSE",
    "CHANGELOG.md",
    "logo.png",
)
NATIVE_INIT = "astrembodiment_core/__init__.py"
NATIVE_BUNDLE_ROOT = "astrembodiment_core/_bundled"
NATIVE_MANIFEST = f"{NATIVE_BUNDLE_ROOT}/manifest.json"
NATIVE_MANIFEST_SCHEMA = "astrembodiment-native-bundle-v1"
NATIVE_FILENAMES = {"linux": "_native.abi3.so", "win32": "_native.pyd"}
RELEASE_FORBIDDEN_PREFIXES = ("pages/", ".astrbot-plugin/i18n/")
RELEASE_FORBIDDEN_COMPONENTS = {
    "__pycache__",
    ".pytest_cache",
    "broker",
    "brokers",
    "cache",
    "caches",
    "database",
    "databases",
    "draft",
    "drafts",
    "target",
    "targets",
}
RELEASE_FORBIDDEN_SUFFIXES = (
    ".db",
    ".sqlite",
    ".sqlite3",
    ".db-wal",
    ".db-shm",
    ".pyc",
    ".pyo",
)
WINDOWS_UNSAFE_ARCHIVE_MEMBERS = (
    "module.py.",
    "module.py ",
    "module?.py",
    "module*.py",
    "module<draft>.py",
    'module"draft.py',
    "module|draft.py",
    "control\x1f.py",
    "CON",
    "con.txt",
    "folder/AUX.py",
    "folder/NUL.json",
    "COM1.bin",
    "folder/lpt9.log",
)
WINDOWS_INVALID_ARCHIVE_CHARS = frozenset('<>:"|?*')
WINDOWS_RESERVED_ARCHIVE_NAMES = frozenset(
    {
        "aux",
        "clock$",
        "con",
        "nul",
        "prn",
        *(f"com{number}" for number in range(1, 10)),
        *(f"lpt{number}" for number in range(1, 10)),
        "com¹",
        "com²",
        "com³",
        "lpt¹",
        "lpt²",
        "lpt³",
    }
)


def _literal_assignment(source: str, name: str) -> object:
    tree = ast.parse(source)
    for node in tree.body:
        if not isinstance(node, ast.Assign):
            continue
        if any(
            isinstance(target, ast.Name) and target.id == name
            for target in node.targets
        ):
            return ast.literal_eval(node.value)
    raise AssertionError(f"missing literal assignment: {name}")


def _release_archive_or_skip() -> Path:
    override = os.environ.get("AE_RELEASE_ARCHIVE")
    if not override:
        pytest.skip("set AE_RELEASE_ARCHIVE to the exact built alpha4 archive")
    archive = Path(override)
    assert archive.is_absolute(), "AE_RELEASE_ARCHIVE must be an absolute path"
    assert archive.is_file(), f"release archive does not exist: {archive}"
    assert archive.suffix.casefold() == ".zip"
    return archive


def _portable_release_member_key(name: str) -> str | None:
    if not name or "\\" in name or name.endswith("/"):
        return None
    path = PurePosixPath(name)
    if path.is_absolute() or path.as_posix() != name:
        return None
    for part in path.parts:
        if part in {"", ".", ".."} or part.endswith((".", " ")):
            return None
        if any(
            ord(character) < 32 or character in WINDOWS_INVALID_ARCHIVE_CHARS
            for character in part
        ):
            return None
        if part.split(".", 1)[0].casefold() in WINDOWS_RESERVED_ARCHIVE_NAMES:
            return None
    return path.as_posix().casefold()


def _assert_release_archive_member_contract(names: list[str]) -> None:
    seen: set[str] = set()
    forbidden_exact = {
        "astr_embodiment/observatory.py",
        "astr_embodiment/observatory_controls.py",
    } | RETIRED_LIVE_MIND_MEMBERS

    for name in names:
        normalized = _portable_release_member_key(name)
        assert normalized is not None, f"unsafe release archive member: {name!r}"
        assert normalized not in seen, (
            f"Windows-normalized duplicate release archive member: {name!r}"
        )
        seen.add(normalized)

        path = PurePosixPath(name)
        assert not normalized.startswith(RELEASE_FORBIDDEN_PREFIXES), (
            f"forbidden release archive surface: {name!r}"
        )
        assert normalized not in forbidden_exact, (
            f"forbidden release archive surface: {name!r}"
        )
        assert not RELEASE_FORBIDDEN_COMPONENTS.intersection(
            part.casefold() for part in path.parts
        ), f"runtime evidence leaked into release archive: {name!r}"
        assert not normalized.endswith(RELEASE_FORBIDDEN_SUFFIXES), (
            f"runtime evidence leaked into release archive: {name!r}"
        )


def _expected_release_source_members() -> dict[str, bytes]:
    members: dict[str, bytes] = {}
    for relative in RELEASE_SOURCE_INPUTS:
        path = ROOT / relative
        assert path.exists() and not path.is_symlink(), path
        candidates = (
            sorted(
                (
                    child
                    for child in path.rglob("*")
                    if child.is_file() and "__pycache__" not in child.parts
                ),
                key=lambda child: child.relative_to(ROOT).as_posix(),
            )
            if path.is_dir()
            else [path]
        )
        for source_file in candidates:
            assert not source_file.is_symlink(), source_file
            member = source_file.relative_to(ROOT).as_posix()
            assert _portable_release_member_key(member) is not None
            members[member] = source_file.read_bytes()
    members[NATIVE_INIT] = (
        ROOT / "python" / "astrembodiment_core" / "__init__.py"
    ).read_bytes()
    return members


def _assert_release_native_bundle(archive: zipfile.ZipFile) -> set[str]:
    manifest_payload = archive.read(NATIVE_MANIFEST)
    manifest = json.loads(manifest_payload)
    assert set(manifest) == {"schema", "platforms", "build_info"}
    assert manifest["schema"] == NATIVE_MANIFEST_SCHEMA
    assert set(manifest["platforms"]) == set(NATIVE_FILENAMES)
    native_members: set[str] = set()
    for platform, expected_filename in NATIVE_FILENAMES.items():
        entry = manifest["platforms"][platform]
        assert set(entry) == {
            "build_id",
            "filename",
            "wheel_filename",
            "wheel_sha256",
            "binary_sha256",
            "build_info",
            "import_receipt",
        }
        assert entry["build_info"] == manifest["build_info"]
        receipt = entry["import_receipt"]
        assert receipt["status"] == "IMPORTED"
        assert receipt["platform"] == platform
        assert receipt["wheel_sha256"] == entry["wheel_sha256"]
        assert receipt["wheel_filename"] == entry["wheel_filename"]
        assert receipt["build_info"] == entry["build_info"]
        assert set(receipt["methods"]) == NATIVE_API - {"NativeCoreError"}
        build_id = entry["build_id"]
        assert (
            isinstance(build_id, str)
            and len(build_id) == 64
            and all(character in "0123456789abcdef" for character in build_id)
        )
        assert entry["filename"] == expected_filename
        member = f"{NATIVE_BUNDLE_ROOT}/{build_id}/{expected_filename}"
        payload = archive.read(member)
        assert hashlib.sha256(payload).hexdigest() == build_id
        assert receipt["binary_sha256"] == entry["binary_sha256"] == build_id
        native_members.add(member)
    assert manifest_payload == json.dumps(
        manifest, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    return native_members


def _write_test_wheel(
    path: Path,
    native_member: str,
    payload: bytes,
    *,
    metadata_version: str = NATIVE_WHEEL_VERSION,
    wheel_tag: str | None = None,
) -> None:
    if wheel_tag is None:
        wheel_tag = (
            "cp312-abi3-win_amd64"
            if native_member.endswith(".pyd")
            else "cp312-abi3-manylinux_2_17_x86_64"
        )
    dist_info = f"astrembodiment_core-{metadata_version}.dist-info"
    with zipfile.ZipFile(path, "w") as archive:
        archive.writestr(
            "astrembodiment_core/__init__.py",
            "# wheel initializer must not be packaged\n",
        )
        archive.writestr(native_member, payload)
        archive.writestr(
            f"{dist_info}/METADATA",
            "Metadata-Version: 2.4\n"
            "Name: astrembodiment-core\n"
            f"Version: {metadata_version}\n",
        )
        archive.writestr(
            f"{dist_info}/WHEEL",
            "Wheel-Version: 1.0\n"
            "Generator: release-contract-test\n"
            "Root-Is-Purelib: false\n"
            f"Tag: {wheel_tag}\n",
        )


def test_release_versions_and_required_files_are_present() -> None:
    cargo = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    project = (ROOT / "pyproject.toml").read_text(encoding="utf-8")
    metadata = (ROOT / "metadata.yaml").read_text(encoding="utf-8")
    assert 'version = "1.1.0"' in cargo
    assert 'version = "1.1.0"' in project
    assert 'version: "1.1.0"' in metadata
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
    imports = [
        node
        for node in ast.walk(ast.parse(entrypoint))
        if isinstance(node, ast.ImportFrom)
        and node.level == 1
        and node.module == "astr_embodiment"
    ]
    assert any(
        {"NativeBridge", "NativeCoreUnavailable"} <= {item.name for item in node.names}
        for node in imports
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


def test_native_initializer_and_packager_require_release_api() -> None:
    initializer = (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_text(
        encoding="utf-8"
    )
    initializer_exports = set(_literal_assignment(initializer, "__all__"))
    packager_markers = set(_load_packager().NATIVE_API_MARKERS)
    assert initializer_exports == NATIVE_API
    assert packager_markers == NATIVE_API
    assert "NativeCore" not in initializer_exports
    assert "NativeCore" not in packager_markers
    for symbol in NATIVE_API:
        assert f"{symbol} = _native_module.{symbol}" in initializer
    assert not LEGACY_MIND_EXPORTS & initializer_exports
    assert not LEGACY_MIND_EXPORTS & packager_markers


@pytest.mark.skipif(
    sys.platform not in {"win32", "linux"},
    reason="requires a supported alpha4 native platform",
)
def test_native_wrapper_exports_exact_current_api_with_matching_identity(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    wrapper_root = tmp_path / "astrembodiment_core"
    wrapper_root.mkdir()
    wrapper_init = wrapper_root / "__init__.py"
    wrapper_init.write_bytes(
        (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_bytes()
    )

    identity = _test_identity()
    platform_key = "win32" if sys.platform == "win32" else "linux"
    filename = "_native.pyd" if platform_key == "win32" else "_native.abi3.so"
    native_payload = b"release-wrapper-smoke"
    build_id = hashlib.sha256(native_payload).hexdigest()
    native_path = wrapper_root / "_bundled" / build_id / filename
    native_path.parent.mkdir(parents=True)
    native_path.write_bytes(native_payload)
    (wrapper_root / "_bundled" / "manifest.json").write_text(
        json.dumps(
            {
                "schema": "astrembodiment-native-bundle-v1",
                "build_info": identity,
                "platforms": {
                    platform_key: {"build_id": build_id, "filename": filename}
                },
            }
        ),
        encoding="utf-8",
    )

    class FakeNativeLoader:
        def create_module(self, _spec: object) -> None:
            return None

        def exec_module(self, module: ModuleType) -> None:
            for symbol in NATIVE_API - {"NativeCoreError"}:
                setattr(module, symbol, lambda *_args, **_kwargs: None)
            module.NativeCoreError = type("NativeCoreError", (RuntimeError,), {})
            module.build_info_v1 = lambda: json.dumps(identity)

    real_spec_from_file_location = importlib.util.spec_from_file_location

    def fake_spec_from_file_location(
        name: str, location: str | Path, *args: object, **kwargs: object
    ) -> object:
        if Path(location) == native_path:
            return importlib.machinery.ModuleSpec(
                name, FakeNativeLoader(), origin=str(native_path)
            )
        return real_spec_from_file_location(name, location, *args, **kwargs)

    monkeypatch.setattr(
        importlib.util, "spec_from_file_location", fake_spec_from_file_location
    )
    wrapper_name = "release_wrapper_smoke"
    wrapper_loader = importlib.machinery.SourceFileLoader(
        wrapper_name, str(wrapper_init)
    )
    wrapper_spec = importlib.util.spec_from_loader(
        wrapper_name, wrapper_loader, is_package=True
    )
    assert wrapper_spec is not None
    wrapper = importlib.util.module_from_spec(wrapper_spec)
    sys.modules[wrapper_name] = wrapper
    try:
        wrapper_loader.exec_module(wrapper)
    finally:
        sys.modules.pop(wrapper_name, None)
        sys.modules.pop(f"{wrapper_name}._native", None)

    assert set(wrapper.__all__) == NATIVE_API
    assert json.loads(wrapper.build_info_v1()) == identity
    assert not RETIRED_ALPHA3_OPERATIONS & set(dir(wrapper))
    assert not hasattr(wrapper, "host_readiness_witness_digest_v1")
    assert not hasattr(wrapper, "wake_caller_incarnation_v2")


def test_release_archive_uses_current_native_initializer_and_not_wheels(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    windows_wheel = tmp_path / WINDOWS_WHEEL_NAME
    linux_wheel = tmp_path / LINUX_WHEEL_NAME
    _write_test_wheel(
        windows_wheel,
        "astrembodiment_core/_native.pyd",
        NATIVE_API_PAYLOAD,
    )
    _write_test_wheel(
        linux_wheel,
        "astrembodiment_core/_native.abi3.so",
        NATIVE_API_PAYLOAD,
    )

    output = tmp_path / "archive.zip"
    _write_unit_bundle(output, [windows_wheel, linux_wheel], monkeypatch)
    source_initializer = (
        ROOT / "python" / "astrembodiment_core" / "__init__.py"
    ).read_bytes()
    build_id = hashlib.sha256(NATIVE_API_PAYLOAD).hexdigest()
    with zipfile.ZipFile(output) as archive:
        names = archive.namelist()
        _assert_release_archive_member_contract(names)
        assert archive.read("astrembodiment_core/__init__.py") == source_initializer
        assert (
            archive.read(f"astrembodiment_core/_bundled/{build_id}/_native.pyd")
            == NATIVE_API_PAYLOAD
        )
        _assert_release_native_bundle(archive)
        assert not any(name.endswith(".whl") for name in names)
    assert output.stat().st_size < 16 * 1024 * 1024


def test_headless_source_and_archive_have_no_pages_surface() -> None:
    release_archive = _release_archive_or_skip()
    source_assets = [
        relative_path
        for relative_path in HEADLESS_SURFACE_PATHS
        if (ROOT / relative_path).exists()
    ]
    assert not source_assets, f"baseline Pages assets remain: {source_assets}"

    schema = json.loads((ROOT / "_conf_schema.json").read_text(encoding="utf-8"))
    assert not HEADLESS_CONFIG_KEYS & schema.keys()
    main_source = (ROOT / "main.py").read_text(encoding="utf-8")
    assert "/observatory/" not in main_source
    assert "register_plugin_page" not in main_source
    assert "register_plugin_api" not in main_source

    with zipfile.ZipFile(release_archive) as archive:
        _assert_release_archive_member_contract(archive.namelist())


def test_packager_rejects_forbidden_headless_members(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    packager_spec = importlib.util.spec_from_file_location(
        "headless_package_plugin", ROOT / "scripts" / "package_plugin.py"
    )
    assert packager_spec is not None and packager_spec.loader is not None
    packager = importlib.util.module_from_spec(packager_spec)
    packager_spec.loader.exec_module(packager)
    monkeypatch.setattr(packager, "require_clean_source", lambda *args: "0" * 40)
    monkeypatch.setattr(packager, "scan_source", lambda: None)
    for index, forbidden_member in enumerate(HEADLESS_FORBIDDEN_MEMBER_CASES):
        for file_kind in ("ordinary", "hardlink"):
            case_root = tmp_path / f"forbidden-{index}-{file_kind}"
            source_file = case_root / Path(forbidden_member)
            source_file.parent.mkdir(parents=True)
            if file_kind == "ordinary":
                source_file.write_bytes(b"forbidden headless member")
            else:
                backing = case_root / "backing.bin"
                backing.write_bytes(b"forbidden headless member")
                source_file.hardlink_to(backing)
            output = case_root / "output.zip"
            monkeypatch.setattr(packager, "ROOT", case_root)
            monkeypatch.setattr(
                packager,
                "_source_files",
                lambda source_file=source_file: [source_file],
            )
            monkeypatch.setattr(packager, "_write_native_package", lambda *_args: None)
            monkeypatch.setattr(
                sys,
                "argv",
                [
                    "package_plugin.py",
                    "--output",
                    str(output),
                    "--native-wheel",
                    "placeholder.whl",
                ],
            )
            with pytest.raises(ValueError):
                packager.main()
            assert not output.exists()


@pytest.mark.parametrize("unsafe_member", WINDOWS_UNSAFE_ARCHIVE_MEMBERS)
def test_packager_rejects_windows_unsafe_archive_members(
    unsafe_member: str,
) -> None:
    packager_spec = importlib.util.spec_from_file_location(
        "windows_safe_package_plugin", ROOT / "scripts" / "package_plugin.py"
    )
    assert packager_spec is not None and packager_spec.loader is not None
    packager = importlib.util.module_from_spec(packager_spec)
    packager_spec.loader.exec_module(packager)

    assert not packager._safe_archive_member(unsafe_member)
    assert _portable_release_member_key(unsafe_member) is None


def test_packager_rejects_case_insensitive_windows_member_collision() -> None:
    packager_spec = importlib.util.spec_from_file_location(
        "windows_collision_package_plugin", ROOT / "scripts" / "package_plugin.py"
    )
    assert packager_spec is not None and packager_spec.loader is not None
    packager = importlib.util.module_from_spec(packager_spec)
    packager_spec.loader.exec_module(packager)

    assert _portable_release_member_key(
        "pkg/Module.py"
    ) == _portable_release_member_key("pkg/module.py")
    written_members: dict[str, str] = {}
    with zipfile.ZipFile(io.BytesIO(), "w") as archive:
        packager._write_member(archive, "pkg/Module.py", b"first", written_members)
        with pytest.raises(ValueError, match="duplicate release archive member"):
            packager._write_member(archive, "pkg/module.py", b"second", written_members)


def test_release_archive_requires_both_native_platforms(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    wheel = tmp_path / WINDOWS_WHEEL_NAME
    _write_test_wheel(
        wheel,
        "astrembodiment_core/_native.pyd",
        NATIVE_API_PAYLOAD,
    )

    output = tmp_path / "archive.zip"
    packager = _load_packager()
    monkeypatch.setattr(packager, "require_clean_source", lambda *args: "0" * 40)
    with zipfile.ZipFile(io.BytesIO(), "w") as archive:
        with pytest.raises(ValueError, match="two actual platform import receipts"):
            packager._write_native_package(archive, [wheel], {}, [])
    assert not output.exists()


@pytest.mark.parametrize(
    (
        "bad_wheel_name",
        "native_member",
        "payload",
        "metadata_version",
        "wheel_tag",
        "expected_error",
    ),
    (
        (
            "astrembodiment_core-1.1.0a1-cp312-abi3-win_amd64.whl",
            "astrembodiment_core/_native.pyd",
            NATIVE_API_PAYLOAD,
            "1.1.0a1",
            "cp312-abi3-win_amd64",
            "wheel filename",
        ),
        (
            WINDOWS_WHEEL_NAME,
            "astrembodiment_core/_native.pyd",
            NATIVE_API_PAYLOAD,
            "1.1.0a1",
            "cp312-abi3-win_amd64",
            "wheel metadata",
        ),
        (
            "astrembodiment_core-1.1.0-cp312-abi3-manylinux_2_17_aarch64.whl",
            "astrembodiment_core/_native.abi3.so",
            NATIVE_API_PAYLOAD,
            NATIVE_WHEEL_VERSION,
            "cp312-abi3-manylinux_2_17_aarch64",
            "platform tag",
        ),
        (
            WINDOWS_WHEEL_NAME,
            "astrembodiment_core/_native.pyd",
            NATIVE_API_MARKERS_PAYLOAD + b" 1.0.0",
            NATIVE_WHEEL_VERSION,
            "cp312-abi3-win_amd64",
            "runtime version marker",
        ),
    ),
)
def test_release_archive_rejects_stale_or_wrong_platform_wheels(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    bad_wheel_name: str,
    native_member: str,
    payload: bytes,
    metadata_version: str,
    wheel_tag: str,
    expected_error: str,
) -> None:
    bad_wheel = tmp_path / "bad" / bad_wheel_name
    bad_wheel.parent.mkdir()
    _write_test_wheel(
        bad_wheel,
        native_member,
        payload,
        metadata_version=metadata_version,
        wheel_tag=wheel_tag,
    )
    counterpart = (
        tmp_path / LINUX_WHEEL_NAME
        if native_member.endswith(".pyd")
        else tmp_path / WINDOWS_WHEEL_NAME
    )
    counterpart_member = (
        "astrembodiment_core/_native.abi3.so"
        if native_member.endswith(".pyd")
        else "astrembodiment_core/_native.pyd"
    )
    _write_test_wheel(counterpart, counterpart_member, NATIVE_API_PAYLOAD)

    output = tmp_path / "archive.zip"
    packager = _load_packager()
    with pytest.raises(ValueError, match=expected_error):
        _, platform_tag = packager._wheel_platform(bad_wheel.name)
        with zipfile.ZipFile(bad_wheel) as wheel:
            packager._validate_wheel_metadata(wheel, wheel.namelist(), platform_tag)
            packager._validate_native_payload(native_member, payload)
    assert not output.exists()


def test_release_archive_bundles_only_runtime_files() -> None:
    release_archive = _release_archive_or_skip()
    source_members = _expected_release_source_members()
    with zipfile.ZipFile(release_archive) as archive:
        archive_names = archive.namelist()
        _assert_release_archive_member_contract(archive_names)
        names = set(archive_names)
        native_members = _assert_release_native_bundle(archive)
        assert names == set(source_members) | {NATIVE_MANIFEST} | native_members
        for member, source_payload in source_members.items():
            assert archive.read(member) == source_payload
        metadata = archive.read("metadata.yaml").decode("utf-8")
    assert "astrembodiment_core/__init__.py" in names
    assert "astrembodiment_core/_bundled/manifest.json" in names
    assert not any(name.startswith("astrembodiment_core/_native") for name in names)
    assert not any(name.casefold().endswith(".whl") for name in names)
    assert "logo.png" in names
    assert "LICENSE" in names
    assert "CHANGELOG.md" in names
    assert "tests/test_static_contracts.py" not in names
    assert not any(name.startswith("crates/") for name in names)
    assert 'version: "1.1.0"' in metadata
    assert release_archive.stat().st_size < 16 * 1024 * 1024


def test_fresh_wheel_members_are_copied_byte_for_byte_to_bundled_paths() -> None:
    release_archive = _release_archive_or_skip()
    if FRESH_WINDOWS_WHEEL is None or FRESH_LINUX_WHEEL is None:
        pytest.skip("set both AE_FRESH_*_WHEEL paths for byte-for-byte audit")
    with zipfile.ZipFile(release_archive) as archive:
        archive_names = archive.namelist()
        _assert_release_archive_member_contract(archive_names)
        names = set(archive_names)
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
            entry = manifest["platforms"][platform]
            assert entry["build_id"] == build_id
            assert entry["filename"] == filename
            assert (
                entry["wheel_sha256"]
                == hashlib.sha256(wheel_path.read_bytes()).hexdigest()
            )


@pytest.mark.skipif(
    sys.platform not in {"win32", "linux"},
    reason="requires a supported alpha4 native platform",
)
def test_fresh_archive_imports_native_api_in_clean_astrbot_namespace(
    tmp_path: Path,
) -> None:
    release_archive = _release_archive_or_skip()
    namespace_root = tmp_path / "namespace"
    plugin_root = namespace_root / "data" / "plugins" / "astrbot_plugin_astrembodiment"
    plugin_root.mkdir(parents=True)
    with zipfile.ZipFile(release_archive) as archive:
        _assert_release_archive_member_contract(archive.namelist())
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
        assert set(native.__all__) == NATIVE_API
        assert not hasattr(native, "NativeCore")
        assert all(
            callable(getattr(native, symbol))
            for symbol in NATIVE_API - {"NativeCoreError"}
        )
        assert native.NativeCoreError.__name__ == "NativeCoreError"
        assert not LEGACY_MIND_EXPORTS & set(dir(native))
        assert json.loads(native.health()) == {
            "status": "g0-ready",
            "formula": "aster-ccn-v1",
            "neuron_slots": 16384,
            "version": NATIVE_RUNTIME_VERSION,
        }
        assert not RETIRED_ALPHA3_OPERATIONS & set(dir(native))
        build_info = json.loads(native.build_info_v1())
        assert build_info["version"] == NATIVE_RUNTIME_VERSION
        assert set(build_info["methods"]) == NATIVE_API - {"NativeCoreError"}
        runtime_dir = tmp_path / "runtime"
        runtime_dir.mkdir()
        health = bridge_module.NativeBridge().open(str(runtime_dir))
        assert health == bridge_module.NativeHealth(
            status="g0-ready",
            formula="aster-ccn-v1",
            neuron_slots=16384,
            version=NATIVE_RUNTIME_VERSION,
        )
        assert (runtime_dir / "astrembodiment.sqlite3").is_file()
        assert main_module.AstrEmbodimentPlugin is not None
    finally:
        sys.path.remove(str(namespace_root))
        for name in list(sys.modules):
            if name == "data" or name.startswith("data."):
                sys.modules.pop(name, None)
        sys.modules.update(previous_modules)


def test_release_archive_accepts_linux_abi3_extension(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    linux_wheel = tmp_path / LINUX_WHEEL_NAME
    windows_wheel = tmp_path / WINDOWS_WHEEL_NAME
    _write_test_wheel(
        linux_wheel,
        "astrembodiment_core/_native.abi3.so",
        b"linux-native-placeholder " + NATIVE_API_PAYLOAD,
    )
    _write_test_wheel(
        windows_wheel,
        "astrembodiment_core/_native.pyd",
        b"windows-native-placeholder " + NATIVE_API_PAYLOAD,
    )

    output = tmp_path / "astrbot_plugin_astrembodiment-1.1.0-universal.zip"
    _write_unit_bundle(output, [windows_wheel, linux_wheel], monkeypatch)
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
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    windows_wheel = tmp_path / WINDOWS_WHEEL_NAME
    linux_wheel = tmp_path / LINUX_WHEEL_NAME
    _write_test_wheel(
        windows_wheel,
        "astrembodiment_core/_native.pyd",
        b"windows-native-placeholder " + NATIVE_API_PAYLOAD,
    )
    _write_test_wheel(
        linux_wheel,
        "astrembodiment_core/_native.abi3.so",
        b"linux-native-placeholder " + NATIVE_API_PAYLOAD,
    )

    output = tmp_path / "astrbot_plugin_astrembodiment-1.1.0-universal.zip"
    _write_unit_bundle(output, [windows_wheel, linux_wheel], monkeypatch)
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


def _load_packager():
    spec = importlib.util.spec_from_file_location(
        "release_package_plugin", ROOT / "scripts/package_plugin.py"
    )
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_alpha4_dirty_and_mismatched_source_fail_before_build(tmp_path, monkeypatch):
    packager = _load_packager()
    subprocess.run(["git", "init", str(tmp_path)], check=True, capture_output=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(tmp_path),
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
        check=True,
        capture_output=True,
    )
    monkeypatch.setattr(packager, "ROOT", tmp_path)
    sha = packager.require_clean_source()
    with pytest.raises(ValueError, match="SOURCE_SHA_MISMATCH"):
        packager.require_clean_source("0" * 40)
    (tmp_path / "changed.txt").write_text("dirty", encoding="utf-8")
    with pytest.raises(ValueError, match="SOURCE_DIRTY"):
        packager.build_native(tmp_path / "must-not-exist", sha, "must-not-run", [])
    assert not (tmp_path / "must-not-exist").exists()


def test_alpha4_package_requires_real_platform_receipts(monkeypatch):
    packager = _load_packager()
    monkeypatch.setattr(packager, "require_clean_source", lambda *args: "0" * 40)
    with zipfile.ZipFile(io.BytesIO(), "w") as archive:
        with pytest.raises(ValueError, match="two actual platform import receipts"):
            packager._write_native_package(archive, [], {}, [])


@pytest.mark.parametrize(
    "source",
    [
        "from .proactive import execute",
        "bridge.claim_wake()",
        "registry = {'ae_wake': handler}",
        "settings = {'proactive_enabled': False}",
        "def submit_proactive_message(): pass",
    ],
)
def test_alpha4_scanner_rejects_retired_active_python(source):
    packager = _load_packager()
    with pytest.raises(ValueError, match="RETIRED_"):
        packager.scan_python(source, "fixture.py", packager.manifests()[1])


def test_alpha4_scanner_accepts_current_source_and_manifest():
    packager = _load_packager()
    result = packager.scan_source()
    assert result["status"] == "SOURCE_SCAN_PASS"
    assert set(result["methods"]) == NATIVE_API - {"NativeCoreError"}
    assert len(result["methods"]) == 19


@pytest.mark.parametrize("layout", ["single", "multiline", "mixed"])
@pytest.mark.parametrize("extra", [None, "unexpected_api", "duplicate"])
def test_scanner_native_registration_whitespace_preserves_exact_set(
    monkeypatch, layout, extra
):
    packager = _load_packager()
    public = packager.manifests()[0]
    names = public + ([public[0] if extra == "duplicate" else extra] if extra else [])
    source = "\n".join(
        f"wrap_pyfunction!(\n    {name},\n    module\n)"
        if layout == "multiline" or (layout == "mixed" and index % 2)
        else f"wrap_pyfunction!({name}, module)"
        for index, name in enumerate(names)
    )
    original_read = Path.read_text
    native_path = ROOT / "crates/ae-pyo3/src/lib.rs"

    def read_source(path, *args, **kwargs):
        return source if path == native_path else original_read(path, *args, **kwargs)

    monkeypatch.setattr(Path, "read_text", read_source)
    if extra:
        with pytest.raises(ValueError, match="^NATIVE_REGISTRATION_SET$"):
            packager.scan_source()
    else:
        result = packager.scan_source()
        assert result["methods"] == public
        assert len(result["methods"]) == 19


@pytest.mark.parametrize(
    "member",
    [
        "astr_embodiment/autonomy.py",
        "astr_embodiment/proactive.py",
        "astr_embodiment/proactive_settings.py",
        "astr_embodiment/secret_store.py",
    ],
)
def test_alpha4_packager_rejects_retired_archive_members(member):
    packager = _load_packager()
    with zipfile.ZipFile(io.BytesIO(), "w") as archive:
        with pytest.raises(ValueError, match="forbidden"):
            packager._write_member(archive, member, b"# historical", {})


def _test_identity():
    """Synthetic identity for unit validation, never an actual import receipt."""
    packager = _load_packager()
    schema = (ROOT / "crates/ae-store/src/core_boundary_v9.rs").read_text(
        encoding="utf-8"
    )
    tzdb = json.loads((ROOT / "astr_embodiment/assets/tzdb/manifest.json").read_bytes())
    return {
        "source_sha": "0" * 40,
        "version": NATIVE_RUNTIME_VERSION,
        "contract_version": 1,
        "methods": packager.manifests()[0],
        "tzdb_release": "2026c",
        "core_api_digest": "1" * 64,
        "core_public_method_manifest_sha256": "800e4dccb2a29b6edbaa2ac7cc46c34f466ece682e0ccf1a272577aacb3790c4",
        "retired_surface_manifest_sha256": "6a28d0e925be34a108b636c47b4ea8ff1a69b763d7de14ba91d4a44127dc291a",
        "autonomy_schema_v9_sql_sha256": re.search(
            r'const SQL_SHA256: &str = "([0-9a-f]+)"', schema
        ).group(1),
        "tzdb_content_sha256": tzdb["content_sha256"],
    }


def _write_unit_bundle(output, wheels, monkeypatch, receipt_edit=None):
    """Exercise bundle encoding with synthetic evidence; not release acceptance."""
    packager = _load_packager()
    identity = _test_identity()
    monkeypatch.setattr(
        packager, "require_clean_source", lambda *args: identity["source_sha"]
    )
    receipts = []
    for path in wheels:
        with zipfile.ZipFile(path, "a") as wheel:
            wheel.writestr(packager.WHEEL_IDENTITY, json.dumps(identity))
            payload = wheel.read(packager._native_extension(wheel.namelist()))
        platform, _ = packager._wheel_platform(path.name)
        receipts.append(
            dict(
                status="IMPORTED",
                platform=platform,
                machine="x86_64",
                wheel_filename=path.name,
                wheel_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                binary_sha256=hashlib.sha256(payload).hexdigest(),
                build_info=identity,
                methods=identity["methods"],
            )
        )
    if receipt_edit is not None:
        receipts[0].update(receipt_edit)
    with zipfile.ZipFile(output, "w") as archive:
        packager._write_native_package(archive, wheels, {}, receipts)


@pytest.mark.parametrize(
    "receipt_edit",
    [
        {"status": "STATIC_CHECK"},
        {"wheel_sha256": "f" * 64},
        {"binary_sha256": "f" * 64},
        {"methods": ["alpha3_call"]},
        {"build_info": {}},
        {"machine": "aarch64"},
    ],
)
def test_bundle_rejects_mismatched_import_evidence(tmp_path, monkeypatch, receipt_edit):
    windows = tmp_path / WINDOWS_WHEEL_NAME
    linux = tmp_path / LINUX_WHEEL_NAME
    _write_test_wheel(windows, "astrembodiment_core/_native.pyd", NATIVE_API_PAYLOAD)
    _write_test_wheel(linux, "astrembodiment_core/_native.abi3.so", NATIVE_API_PAYLOAD)
    with pytest.raises(
        ValueError, match="import receipt does not match exact wheel identity"
    ):
        _write_unit_bundle(
            tmp_path / "unit.zip", [windows, linux], monkeypatch, receipt_edit
        )


def test_checksum_output_matches_completed_archive(tmp_path, monkeypatch):
    packager = _load_packager()
    output = tmp_path / "unit.zip"
    checksum = tmp_path / "unit.sha256"
    monkeypatch.setattr(packager, "require_clean_source", lambda *args: "0" * 40)
    monkeypatch.setattr(packager, "scan_source", lambda: None)
    monkeypatch.setattr(packager, "_source_files", lambda: [])
    monkeypatch.setattr(packager, "_write_native_package", lambda *args: None)
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "package_plugin.py",
            "--output",
            str(output),
            "--sha256-output",
            str(checksum),
        ],
    )
    packager.main()
    assert (
        checksum.read_text()
        == f"{hashlib.sha256(output.read_bytes()).hexdigest()}  {output.name}\n"
    )


def test_archive_member_metadata_is_platform_independent():
    packager = _load_packager()
    outputs = []
    for _ in range(2):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w") as archive:
            packager._write_member(archive, "metadata.yaml", b"unit fixture", {})
            info = archive.getinfo("metadata.yaml")
            assert info.date_time == (1980, 1, 1, 0, 0, 0)
            assert info.create_system == 3
            assert info.external_attr == 0o100644 << 16
            assert info.compress_type == zipfile.ZIP_STORED
        outputs.append(buffer.getvalue())
    assert outputs[0] == outputs[1]


@pytest.mark.parametrize(
    "field,value",
    [
        ("source_sha", "2" * 40),
        ("version", "1.1.0-alpha1"),
        ("methods", ["alpha3_call"]),
        ("tzdb_content_sha256", "3" * 64),
    ],
)
def test_identity_rejects_stale_source_version_api_and_assets(field, value):
    packager = _load_packager()
    identity = dict(_test_identity(), **{field: value})
    with pytest.raises(ValueError, match="IDENTITY"):
        packager.validate_identity(identity, "0" * 40)


@pytest.mark.parametrize("same_path", [False, True])
def test_checksum_output_never_overwrites_existing_file(
    tmp_path, monkeypatch, same_path
):
    packager = _load_packager()
    output = tmp_path / "unit.zip"
    checksum = output if same_path else tmp_path / "unit.sha256"
    checksum.write_text("preserve me", encoding="utf-8")
    monkeypatch.setattr(
        sys,
        "argv",
        [
            "package_plugin.py",
            "--output",
            str(output),
            "--sha256-output",
            str(checksum),
        ],
    )
    with pytest.raises((ValueError, SystemExit)):
        packager.main()
    assert checksum.read_text() == "preserve me"
