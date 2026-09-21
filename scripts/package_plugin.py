#!/usr/bin/env python3
"""Build a self-contained AstrBot plugin archive from native platform wheels."""

from __future__ import annotations

import argparse
import base64
import csv
import hashlib
import io
import json
import os
import platform as host_platform
import re
import subprocess
import sys
import tempfile
import zipfile
from email.parser import BytesParser
from email.policy import compat32
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "scripts"))
from astr_embodiment.build_info import same_identity
from scan_core_boundary import FORBIDDEN_MEMBERS, manifests, scan_python, scan_source
INCLUDE = [
    "main.py",
    "metadata.yaml",
    "requirements.txt",
    "_conf_schema.json",
    "astr_embodiment",
    "README.md",
    "LICENSE",
    "CHANGELOG.md",
    "logo.png",
]
HEADLESS_FORBIDDEN_ROOTS = ("pages", ".astrbot-plugin/i18n")
HEADLESS_FORBIDDEN_EXACT_MEMBERS = frozenset(
    {
        "astr_embodiment/observatory.py",
        "astr_embodiment/observatory_controls.py",
    }
) | FORBIDDEN_MEMBERS
HEADLESS_FORBIDDEN_PREFIXES = ("pages/", ".astrbot-plugin/i18n/")
NATIVE_PACKAGE = "astrembodiment_core"
NATIVE_INIT = f"{NATIVE_PACKAGE}/__init__.py"
NATIVE_BUNDLE_ROOT = f"{NATIVE_PACKAGE}/_bundled"
NATIVE_MANIFEST = f"{NATIVE_BUNDLE_ROOT}/manifest.json"
NATIVE_MANIFEST_SCHEMA = "astrembodiment-native-bundle-v1"
NATIVE_SOURCE_INIT = ROOT / "python" / NATIVE_INIT
NATIVE_SUFFIXES = (".pyd", ".so")
NATIVE_WHEEL_VERSION = "1.1.0"
NATIVE_RUNTIME_VERSION = "1.1.0"
WHEEL_IDENTITY = f"{NATIVE_PACKAGE}/build_identity.json"
NATIVE_WHEEL_TAG_PREFIX = "cp312-abi3-"
LINUX_PLATFORM_TAG = re.compile(
    r"manylinux(?:(?:_\d+){2}|\d{4})_x86_64",
    re.ASCII,
)
NATIVE_API_MARKERS = tuple(manifests()[0]) + ("NativeCoreError",)
MAX_ARCHIVE_BYTES = 16 * 1024 * 1024
REQUIRED_NATIVE_PLATFORMS = {"linux", "win32"}
WINDOWS_INVALID_MEMBER_CHARS = frozenset('<>:"|?*')
WINDOWS_RESERVED_MEMBER_NAMES = frozenset(
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


def _archive_member_key(name: str) -> str | None:
    if not name or "\\" in name or name.endswith("/"):
        return None
    path = PurePosixPath(name)
    if path.is_absolute() or path.as_posix() != name:
        return None
    for part in path.parts:
        if part in {"", ".", ".."} or part.endswith((".", " ")):
            return None
        if any(
            ord(character) < 32 or character in WINDOWS_INVALID_MEMBER_CHARS
            for character in part
        ):
            return None
        if part.split(".", 1)[0].casefold() in WINDOWS_RESERVED_MEMBER_NAMES:
            return None
    return path.as_posix().casefold()


def _safe_archive_member(name: str) -> bool:
    return _archive_member_key(name) is not None


def _write_member(
    archive: zipfile.ZipFile,
    member: str,
    payload: bytes,
    written_members: dict[str, str],
) -> None:
    member_key = _archive_member_key(member)
    if member_key is None:
        raise ValueError(f"unsafe release archive member: {member}")
    if member_key in HEADLESS_FORBIDDEN_EXACT_MEMBERS or any(
        member_key.startswith(prefix)
        for prefix in HEADLESS_FORBIDDEN_PREFIXES
    ):
        raise ValueError(f"forbidden headless archive member: {member}")
    if member_key.endswith((".db", ".sqlite", ".sqlite3", ".db-wal", ".db-shm", ".pyc", ".pyo")):
        raise ValueError(f"forbidden runtime archive member: {member}")
    if member_key in written_members:
        raise ValueError(
            f"duplicate release archive member: {member} conflicts with "
            f"{written_members[member_key]}"
        )
    if member.endswith(".py") and (member == "main.py" or member.startswith("astr_embodiment/")):
        scan_python(payload.decode("utf-8"), member, manifests()[1])
    if "astrcyberhuman" in member_key or (member.endswith((".toml", ".txt")) and b"astrcyberhuman" in payload.lower()):
        raise ValueError("forbidden AstrCyberHuman dependency")
    archive.writestr(member, payload, compress_type=zipfile.ZIP_DEFLATED)
    written_members[member_key] = member


def _headless_surface_entries() -> list[Path]:
    entries: list[Path] = []
    for relative_root in HEADLESS_FORBIDDEN_ROOTS:
        root = ROOT / relative_root
        if root.is_symlink() or root.is_file():
            entries.append(root)
            continue
        if root.is_dir():
            entries.extend(
                child
                for child in root.rglob("*")
                if child.is_file() or child.is_symlink()
            )
    return entries


def _source_files() -> list[Path]:
    forbidden = _headless_surface_entries()
    if forbidden:
        names = ", ".join(
            path.relative_to(ROOT).as_posix() for path in sorted(forbidden)
        )
        raise ValueError(
            "headless release must not contain Pages or i18n files: " + names
        )
    source_files: list[Path] = []
    for item in INCLUDE:
        path = ROOT / item
        if not path.exists():
            raise ValueError(f"required release input does not exist: {path}")
        if path.is_symlink():
            raise ValueError(f"release input must not be a symlink: {path}")
        if path.is_dir():
            for child in sorted(
                path.rglob("*"),
                key=lambda candidate: candidate.relative_to(ROOT).as_posix(),
            ):
                if child.is_symlink():
                    raise ValueError(f"release input must not be a symlink: {child}")
                if child.is_file() and "__pycache__" not in child.parts:
                    source_files.append(child)
        elif path.is_file():
            source_files.append(path)
        else:
            raise ValueError(
                f"required release input is not a file or directory: {path}"
            )
    tracked = set(subprocess.check_output(["git", "ls-files", "-z"], cwd=ROOT).decode().split("\0"))
    if any(path.relative_to(ROOT).as_posix() not in tracked for path in source_files):
        raise ValueError("release input must be tracked source")
    return source_files


def _native_extension(names: list[str]) -> str:
    candidates = sorted(
        name
        for name in names
        if name.startswith(f"{NATIVE_PACKAGE}/_native")
        and name.endswith(NATIVE_SUFFIXES)
    )
    if len(candidates) != 1:
        raise ValueError(
            "native wheel must contain exactly one astrembodiment_core/_native*.pyd "
            "or .so member"
        )
    return candidates[0]


def _wheel_platform(wheel_name: str) -> tuple[str, str]:
    prefix = f"{NATIVE_PACKAGE}-{NATIVE_WHEEL_VERSION}-{NATIVE_WHEEL_TAG_PREFIX}"
    if not wheel_name.startswith(prefix) or not wheel_name.endswith(".whl"):
        raise ValueError(
            f"native wheel filename must match {prefix}<supported-platform>.whl"
        )
    platform_tag = wheel_name[len(prefix) : -len(".whl")]
    if platform_tag == "win_amd64":
        return "win32", platform_tag
    linux_tags = platform_tag.split(".")
    if linux_tags and all(LINUX_PLATFORM_TAG.fullmatch(tag) for tag in linux_tags):
        return "linux", platform_tag
    raise ValueError(f"native wheel has unsupported platform tag: {platform_tag}")


def _validate_wheel_metadata(
    wheel: zipfile.ZipFile,
    names: list[str],
    platform_tag: str,
) -> None:
    dist_info = f"{NATIVE_PACKAGE}-{NATIVE_WHEEL_VERSION}.dist-info"
    metadata_member = f"{dist_info}/METADATA"
    wheel_member = f"{dist_info}/WHEEL"
    dist_info_metadata = sorted(
        name for name in names if ".dist-info/" in name and name.endswith("/METADATA")
    )
    dist_info_wheel = sorted(
        name for name in names if ".dist-info/" in name and name.endswith("/WHEEL")
    )
    if dist_info_metadata != [metadata_member] or dist_info_wheel != [wheel_member]:
        raise ValueError(
            "native wheel metadata must use the exact current release dist-info directory"
        )

    metadata = BytesParser(policy=compat32).parsebytes(wheel.read(metadata_member))
    metadata_names = metadata.get_all("Name", [])
    metadata_versions = metadata.get_all("Version", [])
    if (
        len(metadata_names) != 1
        or metadata_names[0].lower().replace("-", "_") != NATIVE_PACKAGE
        or metadata_versions != [NATIVE_WHEEL_VERSION]
    ):
        raise ValueError(
            "native wheel metadata must declare astrembodiment-core "
            f"version {NATIVE_WHEEL_VERSION}"
        )

    wheel_metadata = BytesParser(policy=compat32).parsebytes(wheel.read(wheel_member))
    actual_tags = set(wheel_metadata.get_all("Tag", []))
    expected_tags = {
        f"{NATIVE_WHEEL_TAG_PREFIX}{tag}" for tag in platform_tag.split(".")
    }
    if actual_tags != expected_tags:
        raise ValueError(
            "native wheel metadata tags must exactly match the wheel filename"
        )


def _validate_native_payload(member: str, payload: bytes) -> None:
    missing = [
        marker for marker in NATIVE_API_MARKERS if marker.encode() not in payload
    ]
    if missing:
        raise ValueError(
            f"native extension {member} is missing expected API markers: "
            + ", ".join(missing)
        )
    if NATIVE_RUNTIME_VERSION.encode() not in payload:
        raise ValueError(
            f"native extension {member} is missing expected runtime version marker: "
            f"{NATIVE_RUNTIME_VERSION}"
        )


def _write_native_package(
    archive: zipfile.ZipFile,
    wheel_paths: list[Path],
    written_members: dict[str, str],
    receipts: list[dict] | None = None,
) -> None:
    source_sha = require_clean_source()
    receipts = receipts or []
    if len(receipts) != 2 or {r.get("platform") for r in receipts} != REQUIRED_NATIVE_PLATFORMS:
        raise ValueError("two actual platform import receipts required")
    if not NATIVE_SOURCE_INIT.is_file():
        raise ValueError(
            f"native source initializer does not exist: {NATIVE_SOURCE_INIT}"
        )
    platforms: dict[str, dict[str, str]] = {}
    native_payloads: list[tuple[str, bytes]] = []
    wheel_names: set[str] = set()
    for wheel_path in wheel_paths:
        if not wheel_path.is_file():
            raise ValueError(f"native wheel does not exist: {wheel_path}")
        wheel_name = wheel_path.name
        if not _safe_archive_member(wheel_name) or not wheel_name.endswith(".whl"):
            raise ValueError(f"native wheel has an unsafe filename: {wheel_name}")
        if wheel_name in wheel_names:
            raise ValueError(f"duplicate native wheel filename: {wheel_name}")
        wheel_names.add(wheel_name)
        declared_platform, platform_tag = _wheel_platform(wheel_name)
        with zipfile.ZipFile(wheel_path) as wheel:
            names = wheel.namelist()
            if len(names) != len(set(names)):
                raise ValueError("native wheel contains duplicate archive members")
            if not all(_safe_archive_member(name) for name in names):
                raise ValueError("native wheel contains an unsafe archive member")
            if NATIVE_INIT not in names:
                raise ValueError(
                    f"native wheel is missing required runtime member: {NATIVE_INIT}"
                )
            _validate_wheel_metadata(wheel, names, platform_tag)
            native_extension = _native_extension(names)
            platform = "win32" if native_extension.endswith(".pyd") else "linux"
            if platform != declared_platform:
                raise ValueError(
                    "native extension suffix conflicts with wheel platform tag: "
                    f"{native_extension} vs {platform_tag}"
                )
            if platform in platforms:
                raise ValueError(f"duplicate native wheel for platform: {platform}")
            native_payload = wheel.read(native_extension)
            _validate_native_payload(native_extension, native_payload)
            identity = json.loads(wheel.read(WHEEL_IDENTITY))
            validate_identity(identity, source_sha)
            receipt = next(r for r in receipts if r["platform"] == platform)
            wheel_hash = hashlib.sha256(wheel_path.read_bytes()).hexdigest()
            if (receipt.get("status") != "IMPORTED" or receipt.get("machine", "").lower() not in {"amd64", "x86_64"}
                or receipt.get("wheel_sha256") != wheel_hash or receipt.get("wheel_filename") != wheel_name
                or receipt.get("binary_sha256") != hashlib.sha256(native_payload).hexdigest()
                or not same_identity(receipt.get("build_info", {}), identity)
                or sorted(receipt.get("methods", [])) != sorted(identity["methods"])):
                raise ValueError("import receipt does not match exact wheel identity")
            if platforms and not same_identity(next(iter(platforms.values()))["build_info"], identity):
                raise ValueError("cross-platform build identity mismatch")
            build_id = hashlib.sha256(native_payload).hexdigest()
            archive_member = f"{NATIVE_BUNDLE_ROOT}/{build_id}/{PurePosixPath(native_extension).name}"
            native_payloads.append((archive_member, native_payload))
            platforms[platform] = {
                "build_id": build_id,
                "filename": PurePosixPath(native_extension).name,
                "wheel_filename": wheel_name,
                "wheel_sha256": wheel_hash,
                "binary_sha256": build_id,
                "build_info": identity,
                "import_receipt": receipt,
            }

    if set(platforms) != REQUIRED_NATIVE_PLATFORMS:
        missing = ", ".join(sorted(REQUIRED_NATIVE_PLATFORMS - set(platforms)))
        raise ValueError(
            "native wheels must include both Windows and Linux extensions; "
            f"missing: {missing}"
        )

    _write_member(
        archive,
        NATIVE_INIT,
        NATIVE_SOURCE_INIT.read_bytes(),
        written_members,
    )
    for archive_member, native_payload in native_payloads:
        _write_member(archive, archive_member, native_payload, written_members)

    manifest = {
        "schema": NATIVE_MANIFEST_SCHEMA,
        "platforms": platforms,
        "build_info": next(iter(platforms.values()))["build_info"],
    }
    _write_member(
        archive,
        NATIVE_MANIFEST,
        json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode("utf-8"),
        written_members,
    )


def require_clean_source(expected: str | None = None) -> str:
    def git(*args):
        return subprocess.check_output(["git", *args], cwd=ROOT, text=True).strip()
    sha = git("rev-parse", "HEAD")
    if not re.fullmatch(r"[0-9a-f]{40}", sha) or (expected is not None and expected != sha):
        raise ValueError("SOURCE_SHA_MISMATCH")
    if git("status", "--porcelain", "--untracked-files=all"):
        raise ValueError("SOURCE_DIRTY")
    return sha


def validate_identity(identity: dict, source_sha: str) -> None:
    if (identity.get("source_sha") != source_sha or identity.get("version") != NATIVE_RUNTIME_VERSION
        or identity.get("contract_version") != 1 or identity.get("methods") != manifests()[0]
        or identity.get("tzdb_release") != "2026c"):
        raise ValueError("WHEEL_BUILD_IDENTITY")
    for field in ("core_api_digest", "core_public_method_manifest_sha256", "retired_surface_manifest_sha256",
                  "autonomy_schema_v9_sql_sha256", "tzdb_content_sha256"):
        if not re.fullmatch(r"[0-9a-f]{64}", identity.get(field, "")):
            raise ValueError(f"BUILD_IDENTITY_HASH: {field}")
    tz = json.loads((ROOT / "astr_embodiment/assets/tzdb/manifest.json").read_bytes())
    schema = (ROOT / "crates/ae-store/src/core_boundary_v9.rs").read_text(encoding="utf-8")
    expected = {
        "core_public_method_manifest_sha256": "800e4dccb2a29b6edbaa2ac7cc46c34f466ece682e0ccf1a272577aacb3790c4",
        "retired_surface_manifest_sha256": "6a28d0e925be34a108b636c47b4ea8ff1a69b763d7de14ba91d4a44127dc291a",
        "autonomy_schema_v9_sql_sha256": re.search(r'const SQL_SHA256: &str = "([0-9a-f]+)"', schema).group(1),
        "tzdb_content_sha256": tz["content_sha256"],
    }
    if any(identity.get(key) != value for key, value in expected.items()):
        raise ValueError("BUILD_IDENTITY_SOURCE_DIGEST")


def build_native(output: Path, source_sha: str, maturin: str, extra: list[str]) -> None:
    """Fresh build, then add the compiled identity to the wheel and update RECORD."""
    require_clean_source(source_sha)
    scan_source()
    if output.exists():
        raise ValueError("build output must be a new directory")
    output.mkdir(parents=True)
    with tempfile.TemporaryDirectory(prefix="ae-build-") as temporary:
        manifest_path = Path(temporary) / "identity.json"
        environment = dict(os.environ, AE_SOURCE_SHA=source_sha, AE_BUILD_MANIFEST_OUT=str(manifest_path),
                           CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1")
        subprocess.run([maturin, "build", "--release", "--locked", "--out", str(output), *extra], cwd=ROOT, env=environment, check=True)
        require_clean_source(source_sha)
        identity = json.loads(manifest_path.read_bytes())
        validate_identity(identity, source_sha)
        wheels = list(output.glob("*.whl"))
        if len(wheels) != 1:
            raise ValueError("one fresh wheel required")
        wheel_path = wheels[0]
        with zipfile.ZipFile(wheel_path) as wheel:
            contents = {name: wheel.read(name) for name in wheel.namelist()}
        contents[WHEEL_IDENTITY] = manifest_path.read_bytes()
        record = f"{NATIVE_PACKAGE}-{NATIVE_WHEEL_VERSION}.dist-info/RECORD"
        rows = io.StringIO(newline="")
        writer = csv.writer(rows, lineterminator="\n")
        for name, payload in sorted(contents.items()):
            if name != record:
                digest = base64.urlsafe_b64encode(hashlib.sha256(payload).digest()).rstrip(b"=").decode()
                writer.writerow([name, f"sha256={digest}", str(len(payload))])
        writer.writerow([record, "", ""])
        contents[record] = rows.getvalue().encode()
        staged = Path(temporary) / wheel_path.name
        with zipfile.ZipFile(staged, "w", zipfile.ZIP_DEFLATED) as wheel:
            for name, payload in contents.items():
                wheel.writestr(name, payload)
        import shutil
        shutil.copyfile(staged, wheel_path)
    print(wheel_path)


def verify_wheel(wheel_path: Path, receipt_path: Path) -> None:
    """Import only on the matching real host; never open a Store."""
    declared, _ = _wheel_platform(wheel_path.name)
    if sys.platform != declared or host_platform.machine().lower() not in {"amd64", "x86_64"}:
        raise ValueError("actual matching x86_64 platform required")
    if receipt_path.exists():
        raise ValueError("receipt already exists")
    with zipfile.ZipFile(wheel_path) as wheel, tempfile.TemporaryDirectory(prefix="ae-import-") as temporary:
        names = wheel.namelist()
        if len(names) != len(set(names)) or not all(_safe_archive_member(n) for n in names):
            raise ValueError("unsafe wheel members")
        identity = json.loads(wheel.read(WHEEL_IDENTITY))
        validate_identity(identity, require_clean_source())
        binary_hash = hashlib.sha256(wheel.read(_native_extension(names))).hexdigest()
        wheel.extractall(temporary)
        probe = """import hashlib, json, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from astrembodiment_core import _native as n
methods=sorted(k for k in dir(n) if not k.startswith('__') and callable(getattr(n,k)) and k != 'NativeCoreError')
origin=Path(n.__file__).resolve()
assert origin.is_relative_to(Path(sys.argv[1]).resolve())
print(json.dumps({'build_info':json.loads(n.build_info_v1()),'methods':methods,'loaded_binary_sha256':hashlib.sha256(origin.read_bytes()).hexdigest()}))
"""
        imported = json.loads(subprocess.check_output([sys.executable, "-I", "-c", probe, temporary], text=True))
        if (not same_identity(imported["build_info"], identity) or imported["methods"] != sorted(identity["methods"])
            or imported["loaded_binary_sha256"] != binary_hash):
            raise ValueError("imported callable/build identity mismatch")
        receipt = dict(imported, status="IMPORTED", platform=sys.platform, machine=host_platform.machine(),
                       python=sys.version, wheel_filename=wheel_path.name,
                       wheel_sha256=hashlib.sha256(wheel_path.read_bytes()).hexdigest(), binary_sha256=binary_hash)
        receipt_path.parent.mkdir(parents=True, exist_ok=True)
        receipt_path.write_text(json.dumps(receipt, sort_keys=True, indent=2), encoding="utf-8")
    print(receipt_path)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--native-wheel", type=Path, action="append", default=[])
    parser.add_argument("--import-receipt", type=Path, action="append", default=[])
    parser.add_argument("--build-native", action="store_true")
    parser.add_argument("--verify-wheel", type=Path)
    parser.add_argument("--source-sha")
    parser.add_argument("--maturin", default="maturin")
    args, extra = parser.parse_known_args()
    if args.build_native:
        if not args.source_sha:
            parser.error("--build-native requires --source-sha")
        build_native(args.output, args.source_sha, args.maturin, extra)
        return
    if extra:
        parser.error(f"unknown arguments: {extra}")
    if args.verify_wheel:
        verify_wheel(args.verify_wheel, args.output)
        return
    if args.output.suffix.lower() != ".zip":
        raise SystemExit("--output must name a .zip archive")
    if args.output.exists():
        raise ValueError("release archive already exists")
    require_clean_source(args.source_sha)
    scan_source()
    receipts = [json.loads(path.read_bytes()) for path in args.import_receipt]
    source_files = _source_files()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        with zipfile.ZipFile(
            args.output, "w", compression=zipfile.ZIP_DEFLATED
        ) as archive:
            written_members: dict[str, str] = {}
            for source_file in source_files:
                _write_member(
                    archive,
                    source_file.relative_to(ROOT).as_posix(),
                    source_file.read_bytes(),
                    written_members,
                )
            _write_native_package(archive, args.native_wheel, written_members, receipts)
    except Exception:
        args.output.unlink(missing_ok=True)
        raise
    if args.output.stat().st_size >= MAX_ARCHIVE_BYTES:
        args.output.unlink()
        raise SystemExit(
            "release archive must be smaller than the 16 MiB AstrBot marketplace limit"
        )
    print(args.output)


if __name__ == "__main__":
    main()
