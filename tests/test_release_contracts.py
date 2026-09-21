from __future__ import annotations

import ast
import hashlib
import io
import importlib
import importlib.machinery
import importlib.util
import json
import os
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
    "NativeCoreError", "advance_embodiment_time_v1", "build_info_v1",
    "commit_core_delivery_outcome_v1", "commit_core_inbound_v1",
    "compare_and_swap_embodiment_profile_v1", "compile_core_host_request_v1",
    "create_embodiment_persona_if_missing_v1", "embodiment_clock_status_v1",
    "ensure_genesis", "flush_and_close", "get_embodiment_persona_v1",
    "health", "inspect", "list_embodiment_personas_v1", "open",
    "read_embodiment_profile_v1", "settle_semantic_appraisal_v1",
    "verify_replay", "version",
}
LEGACY_MIND_EXPORTS = {"mood_card", "query_inner_events", "observe_events_v1", "observe_snapshot_v1"}
RETIRED_ALPHA3_OPERATIONS = {"alpha3_call", "apply_event", "autonomy_status", "claim_wake", "settle_dispatch"}
# Legacy placeholder-success packaging tests below still need migration to real
# platform receipts. They are NOT release acceptance and are not silently skipped.
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
    assert set(manifest) == {"schema", "platforms"}
    assert manifest["schema"] == NATIVE_MANIFEST_SCHEMA
    assert set(manifest["platforms"]) == set(NATIVE_FILENAMES)
    native_members: set[str] = set()
    for platform, expected_filename in NATIVE_FILENAMES.items():
        entry = manifest["platforms"][platform]
        assert set(entry) == {"build_id", "filename"}
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


def test_native_initializer_and_packager_require_release_api() -> None:
    initializer = (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_text(
        encoding="utf-8"
    )
    packager = (ROOT / "scripts" / "package_plugin.py").read_text(encoding="utf-8")
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
def test_native_wrapper_exports_readiness_and_wake_for_real_bridge_calls(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    wrapper_root = tmp_path / "astrembodiment_core"
    wrapper_root.mkdir()
    wrapper_init = wrapper_root / "__init__.py"
    wrapper_init.write_bytes(
        (ROOT / "python" / "astrembodiment_core" / "__init__.py").read_bytes()
    )

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
            module.host_readiness_witness_digest_v1 = lambda _items_json: "12" * 32
            module.wake_caller_incarnation_v2 = (
                lambda _event_id, _session_token: "34" * 32
            )

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
    assert callable(wrapper.host_readiness_witness_digest_v1)
    assert callable(wrapper.wake_caller_incarnation_v2)

    monkeypatch.syspath_prepend(str(ROOT))
    from astr_embodiment.bridge import NativeBridge

    bridge = NativeBridge()
    bridge._native = wrapper
    items = [{"kind": "provider", "status": "ready", "witness_revision": 0}]
    assert bridge.host_readiness_witness_v1(items) == {
        "schema_version": 1,
        "items": items,
        "witness_digest": "12" * 32,
    }
    assert (
        bridge.wake_caller_incarnation_v2(
            event_id="0a" * 16, session_token="0b" * 16
        )
        == "34" * 32
    )


def test_release_archive_uses_current_native_initializer_and_not_wheels(
    tmp_path: Path,
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
        manifest = json.loads(
            archive.read("astrembodiment_core/_bundled/manifest.json")
        )
        assert manifest == {
            "schema": "astrembodiment-native-bundle-v1",
            "platforms": {
                "linux": {
                    "build_id": build_id,
                    "filename": "_native.abi3.so",
                },
                "win32": {"build_id": build_id, "filename": "_native.pyd"},
            },
        }
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
            monkeypatch.setattr(
                packager, "_write_native_package", lambda *_args: None
            )
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
            packager._write_member(
                archive, "pkg/module.py", b"second", written_members
            )


def test_release_archive_requires_both_native_platforms(tmp_path: Path) -> None:
    wheel = tmp_path / WINDOWS_WHEEL_NAME
    _write_test_wheel(
        wheel,
        "astrembodiment_core/_native.pyd",
        NATIVE_API_PAYLOAD,
    )

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

    assert result.returncode != 0
    assert "both Windows and Linux" in result.stderr
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
            NATIVE_API_MARKERS_PAYLOAD + b" 1.1.0-alpha1",
            NATIVE_WHEEL_VERSION,
            "cp312-abi3-win_amd64",
            "runtime version marker",
        ),
    ),
)
def test_release_archive_rejects_stale_or_wrong_platform_wheels(
    tmp_path: Path,
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
    result = subprocess.run(
        [
            sys.executable,
            str(ROOT / "scripts" / "package_plugin.py"),
            "--output",
            str(output),
            "--native-wheel",
            str(bad_wheel),
            "--native-wheel",
            str(counterpart),
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode != 0
    assert expected_error in result.stderr
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
            assert manifest["platforms"][platform] == {
                "build_id": build_id,
                "filename": filename,
            }


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
        assert native.version() == NATIVE_RUNTIME_VERSION
        assert json.loads(native.integration_availability_v1()) == {
            "schema_version": 1,
            "state": "UNAVAILABLE_HOST_ATTESTATION",
            "required_capabilities": [
                "service_instance_proof",
                "caller_identity_proof",
                "installation_manifest_binding",
                "bounded_call",
                "lifecycle_revocation",
            ],
        }
        for operation in RETIRED_ALPHA3_OPERATIONS:
            with pytest.raises(native.NativeCoreError, match="CLOSED_SCHEMA"):
                native.alpha3_call(
                    json.dumps({"operation": operation, "request": {}}, sort_keys=True)
                )
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


def test_release_archive_accepts_linux_abi3_extension(tmp_path: Path) -> None:
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
        payload = b"linux-native-placeholder " + NATIVE_API_PAYLOAD
        build_id = hashlib.sha256(payload).hexdigest()
        assert f"astrembodiment_core/_bundled/{build_id}/_native.abi3.so" in names
        assert "astrembodiment_core/_bundled/manifest.json" in names
    assert not any(name.startswith("astrembodiment_core/_native") for name in names)
    assert linux_wheel.name not in names


def test_release_archive_can_bundle_windows_and_linux_extensions(
    tmp_path: Path,
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

def _load_packager():
    spec = importlib.util.spec_from_file_location("release_package_plugin", ROOT / "scripts/package_plugin.py")
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def test_alpha4_dirty_and_mismatched_source_fail_before_build(tmp_path, monkeypatch):
    packager = _load_packager()
    subprocess.run(["git", "init", str(tmp_path)], check=True, capture_output=True)
    subprocess.run(["git", "-C", str(tmp_path), "-c", "user.name=Fixture", "-c",
                    "user.email=fixture@example.invalid", "commit", "--allow-empty", "-m", "fixture"],
                   check=True, capture_output=True)
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


@pytest.mark.parametrize("source", [
    "from .proactive import execute",
    "bridge.claim_wake()",
    "registry = {'ae_wake': handler}",
    "settings = {'proactive_enabled': False}",
    "def submit_proactive_message(): pass",
])
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


@pytest.mark.parametrize("member", [
    "astr_embodiment/autonomy.py", "astr_embodiment/proactive.py",
    "astr_embodiment/proactive_settings.py", "astr_embodiment/secret_store.py",
])
def test_alpha4_packager_rejects_retired_archive_members(member):
    packager = _load_packager()
    with zipfile.ZipFile(io.BytesIO(), "w") as archive:
        with pytest.raises(ValueError, match="forbidden"):
            packager._write_member(archive, member, b"# historical", {})
