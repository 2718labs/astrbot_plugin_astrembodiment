#!/usr/bin/env python3
"""Verify a release ZIP on the current native platform, without compiling.

Run from the frozen source checkout. All databases and imported files are created
in a unique temporary child of --work-dir, then removed. A PASS receipt means
this platform ran the native probe; CI must require both platform receipts.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import stat
import subprocess
import sys
import tempfile
import zipfile
from pathlib import Path, PurePosixPath

ROOT = Path(__file__).resolve().parents[1]
FIXTURE_RELATIVE = Path("crates/ae-store/tests/fixtures/semantic_upgrades_1774023.sql")


def validate_sha(value: str) -> str:
    if len(value) != 40 or any(c not in "0123456789abcdef" for c in value):
        raise ValueError("--source-sha must be a full lowercase 40-character Git SHA")
    return value


def checked_path(path: Path) -> Path:
    """Reject links/reparse points before resolving, including ancestor links."""
    path = Path(os.path.abspath(path))
    for candidate in (path, *path.parents):
        try:
            metadata = candidate.lstat()
        except FileNotFoundError:
            continue
        if (
            stat.S_ISLNK(metadata.st_mode)
            or getattr(metadata, "st_file_attributes", 0) & 0x400
        ):
            raise ValueError(f"symlink/reparse path refused: {candidate}")
    return path


def validate_members(infos: list[zipfile.ZipInfo]) -> None:
    seen = set()
    reserved = {
        "aux",
        "clock$",
        "con",
        "nul",
        "prn",
        *(f"com{i}" for i in range(1, 10)),
        *(f"lpt{i}" for i in range(1, 10)),
        "com¹",
        "com²",
        "com³",
        "lpt¹",
        "lpt²",
        "lpt³",
    }
    for info in infos:
        name = info.filename
        path = PurePosixPath(name)
        if (
            not name
            or "\\" in name
            or path.is_absolute()
            or path.as_posix() != name
            or name.endswith("/")
        ):
            raise ValueError(f"unsafe ZIP member: {name}")
        for part in path.parts:
            if (
                part in {".", ".."}
                or part.endswith((".", " "))
                or part.split(".", 1)[0].casefold() in reserved
                or any(ord(c) < 32 or c in '<>:"|?*' for c in part)
            ):
                raise ValueError(f"unsafe ZIP member: {name}")
        mode = stat.S_IFMT(info.external_attr >> 16)
        if mode not in (0, stat.S_IFREG) or info.external_attr & 0x400:
            raise ValueError(f"link/reparse/special ZIP member refused: {name}")
        key = name.casefold()
        if key in seen:
            raise ValueError(f"duplicate ZIP member: {name}")
        seen.add(key)
    for name in seen:
        if any(str(parent) in seen for parent in PurePosixPath(name).parents):
            raise ValueError(f"file/directory ZIP collision: {name}")


def sha256_file(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def write_receipt(path: Path, result: dict) -> None:
    path = checked_path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("x", encoding="utf-8") as stream:
        stream.write(json.dumps(result, indent=2, sort_keys=True) + "\n")


PROBE = r'''
import hashlib
import json
import sqlite3
import stat
import sys
import tempfile
from contextlib import closing
from pathlib import Path

archive_root = Path(sys.argv[1]).resolve(strict=True)
fixture_path = Path(sys.argv[2]).resolve(strict=True)
db_parent = Path(sys.argv[3]).resolve(strict=True)
sys.path.insert(0, str(archive_root))

import astrembodiment_core as native
import astr_embodiment as host
from astr_embodiment import bridge as host_bridge

assert Path(native.__file__).resolve().is_relative_to(archive_root)
assert Path(host.__file__).resolve().is_relative_to(archive_root)
assert host_bridge._STORE_FILENAME == "astrembodiment.sqlite3"

identity = json.loads(native.build_info_v1())
assert sorted(set(native.__all__) - {"NativeCoreError"}) == sorted(identity["methods"])
assert len(identity["methods"]) == len(set(identity["methods"])) == 19
from astrembodiment_core import _native as extension
origin = Path(extension.__file__).resolve(strict=True)
assert origin.is_relative_to(archive_root)
methods = sorted(name for name in dir(extension)
                 if not name.startswith("__") and name != "NativeCoreError"
                 and callable(getattr(extension, name)))
assert methods == sorted(identity["methods"])
binary_sha256 = hashlib.sha256(origin.read_bytes()).hexdigest()
fixture_sql = fixture_path.read_text(encoding="utf-8")


def catalog(path):
    with closing(sqlite3.connect(path)) as connection:
        return connection.execute(
            "SELECT type,name,sql FROM sqlite_schema ORDER BY type,name"
        ).fetchall()


def table_columns(path, table):
    with closing(sqlite3.connect(path)) as connection:
        return connection.execute(f"PRAGMA table_info({table!r})").fetchall()


def lifecycle(path):
    native.open(str(path))
    native.flush_and_close()
    native.open(str(path))
    native.flush_and_close()


def prepare_store_mode_baseline(path, sql):
    """Create a fixture in the same persistent SQLite mode Store::open selects."""
    with closing(sqlite3.connect(path)) as connection:
        connection.executescript(sql)
        connection.commit()
        assert connection.execute("PRAGMA journal_mode=WAL").fetchone()[0].lower() == "wal"
        connection.execute("PRAGMA synchronous=NORMAL")
        connection.execute("PRAGMA foreign_keys=ON")
        connection.execute("PRAGMA wal_checkpoint(TRUNCATE)").fetchone()
        connection.commit()
    assert not Path(f"{path}-wal").exists()


def exercise_retired_authority_link(db_root):
    # This retired sidecar is not an authority input to the current core.
    # Prove preservation, not rejection or absence of reads.
    unsafe = db_root / "unsafe-authority"
    unsafe.mkdir()
    unsafe_db = unsafe / "astrembodiment.sqlite3"
    lifecycle(unsafe_db)
    before_unsafe_db = unsafe_db.read_bytes()
    before_unsafe_catalog = catalog(unsafe_db)
    authority = unsafe / ".native-authority"
    target = db_root / "authority-target"
    assert not authority.exists() and not authority.is_symlink()
    target.mkdir()
    (target / "legacy-key.bin").write_bytes(b"synthetic retired key sentinel\x00\xff")
    (target / "legacy-state.json").write_bytes(b'{"fixture":"retired-sidecar"}\n')
    before_target = {p.name: p.read_bytes() for p in target.iterdir() if p.is_file()}
    assert before_target
    import os
    import subprocess
    link_kind = "symlink"
    try:
        authority.symlink_to(target, target_is_directory=True)
    except OSError:
        if sys.platform != "win32":
            raise
        # PowerShell receives paths as environment values, never shell code.
        environment = dict(os.environ, AE_LINK=str(authority), AE_TARGET=str(target))
        subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                        "New-Item -ItemType Junction -Path $env:AE_LINK -Target $env:AE_TARGET | Out-Null"],
                       env=environment, check=True, capture_output=True)
        link_kind = "junction"
    try:
        def link_identity():
            metadata = authority.lstat()
            attributes = getattr(metadata, "st_file_attributes", 0)
            assert stat.S_ISLNK(metadata.st_mode) or attributes & 0x400
            return (metadata.st_ino, metadata.st_mode, attributes, os.readlink(authority))

        before_link = link_identity()
        assert authority.resolve() == target.resolve()
        lifecycle(unsafe_db)
        assert link_identity() == before_link
        assert authority.resolve() == target.resolve()
        assert {p.name: p.read_bytes() for p in target.iterdir() if p.is_file()} == before_target
        assert sorted(p.name for p in target.iterdir()) == sorted(before_target)
        assert unsafe_db.read_bytes() == before_unsafe_db
        assert catalog(unsafe_db) == before_unsafe_catalog
    finally:
        if link_kind == "junction":
            os.rmdir(authority)
        else:
            authority.unlink()

    return link_kind


with tempfile.TemporaryDirectory(prefix="native-db-", dir=db_parent) as db_temp:
    db_root = Path(db_temp)

    fresh = db_root / "fresh" / "astrembodiment.sqlite3"
    fresh.parent.mkdir()
    lifecycle(fresh)
    assert fresh.is_file()
    retired_fresh = fresh.parent / ".native-authority"
    assert not retired_fresh.exists() and not retired_fresh.is_symlink()

    historical = db_root / "historical-empty" / "astrembodiment.sqlite3"
    historical.parent.mkdir()
    with closing(sqlite3.connect(historical)) as connection:
        connection.executescript(fixture_sql)
    native.open(str(historical))
    native.flush_and_close()
    columns = table_columns(historical, "legacy_semantic_formula_upgrades")
    backup = [column for column in columns if column[1] == "backup_digest"]
    assert len(backup) == 1 and backup[0][2].upper() == "BLOB" and backup[0][3] == 1
    migrated_catalog = catalog(historical)
    native.open(str(historical))
    native.flush_and_close()
    assert catalog(historical) == migrated_catalog

    nonempty = db_root / "historical-nonempty" / "astrembodiment.sqlite3"
    nonempty.parent.mkdir()
    insert = """
    INSERT INTO legacy_semantic_formula_upgrades VALUES (
      zeroblob(32),zeroblob(32),zeroblob(32),0,1,zeroblob(32),zeroblob(32),
      zeroblob(32),zeroblob(32),zeroblob(32),zeroblob(32),zeroblob(32),X'010203'
    );
    """
    prepare_store_mode_baseline(nonempty, fixture_sql + insert)
    before_bytes = nonempty.read_bytes()
    before_catalog = catalog(nonempty)
    with closing(sqlite3.connect(nonempty)) as connection:
        before_rows = connection.execute(
            "SELECT hex(upgrade_bytes) FROM legacy_semantic_formula_upgrades "
            "ORDER BY migration_id"
        ).fetchall()
    try:
        native.open(str(nonempty))
    except Exception as error:
        rejection = str(error)
        assert "verified backup migration" in rejection, rejection
    else:
        try:
            native.flush_and_close()
        finally:
            raise AssertionError("non-empty historical schema was not rejected")
    assert nonempty.read_bytes() == before_bytes
    assert catalog(nonempty) == before_catalog
    with closing(sqlite3.connect(nonempty)) as connection:
        after_rows = connection.execute(
            "SELECT hex(upgrade_bytes) FROM legacy_semantic_formula_upgrades "
            "ORDER BY migration_id"
        ).fetchall()
    assert after_rows == before_rows == [("010203",)]

    link_kind = exercise_retired_authority_link(db_root)

print(json.dumps({
    "build_info": identity,
    "methods": methods,
    "loaded_binary_sha256": binary_sha256,
    "extension_origin": str(origin),
    "public_namespace": "LOADED",
    "native_version": identity["version"],
    "native_origin": str(Path(native.__file__).resolve()),
    "host_origin": str(Path(host.__file__).resolve()),
    "database_filename": "astrembodiment.sqlite3",
    "database_lifecycle": {
        "fresh_open_flush_reopen": "PASS",
        "fresh_retired_authority_absent": "PASS",
        "retired_authority_link_target_preserved": "PASS",
        "unsafe_path_kind": link_kind,
        "historical_1774023_empty_open_flush_reopen": "PASS",
        "historical_1774023_nonempty_rejected_unchanged": "PASS",
    },
}))
'''


def run_probe(
    extracted: Path, fixture: Path, root: Path, *, timeout_seconds: float = 180
) -> dict:
    """Run the isolated probe and expose evidence without dumping its source."""

    def diagnostic(status, exit_code, stdout, stderr):
        def decoded(name, value):
            rendered = (
                value.decode("utf-8", errors="backslashreplace")
                if isinstance(value, bytes)
                else value or ""
            )
            characters = len(rendered)
            byte_count = (
                len(value)
                if isinstance(value, bytes)
                else len(rendered.encode("utf-8"))
            )
            truncated = characters > 8192
            if truncated:
                marker = "\n... [truncated; head and tail retained] ...\n"
                head = (8192 - len(marker)) // 2
                tail = 8192 - len(marker) - head
                rendered = rendered[:head] + marker + rendered[-tail:]
            return {
                name: rendered,
                name + "_truncated": truncated,
                name + "_bytes": byte_count,
                name + "_characters": characters,
            }

        return RuntimeError(
            json.dumps(
                {
                    "status": status,
                    "exit_code": exit_code,
                    **decoded("stdout", stdout),
                    **decoded("stderr", stderr),
                    "timeout_seconds": timeout_seconds,
                },
                sort_keys=True,
            )
        )

    try:
        completed = subprocess.run(
            [
                sys.executable,
                "-I",
                "-c",
                PROBE,
                str(extracted),
                str(fixture),
                str(root),
            ],
            cwd=root,
            capture_output=True,
            timeout=timeout_seconds,
            check=False,
        )
    except subprocess.TimeoutExpired as error:
        raise diagnostic(
            "NATIVE_PROBE_TIMEOUT", None, error.stdout, error.stderr
        ) from None
    if completed.returncode:
        raise diagnostic(
            "NATIVE_PROBE_FAILED",
            completed.returncode,
            completed.stdout,
            completed.stderr,
        ) from None
    try:
        imported = json.loads(completed.stdout)
        if not isinstance(imported, dict):
            raise TypeError("probe receipt must be an object")
    except (ValueError, TypeError):
        raise diagnostic(
            "NATIVE_PROBE_INVALID_RECEIPT",
            completed.returncode,
            completed.stdout,
            completed.stderr,
        ) from None
    return imported


def verify(archive_path: Path, source_sha: str, work_dir: Path) -> dict:
    source_sha = validate_sha(source_sha)
    archive_path = checked_path(archive_path)
    work_dir = checked_path(work_dir)
    work_dir.mkdir(parents=True, exist_ok=True)
    if sys.platform not in {"win32", "linux"} or platform.machine().lower() not in {
        "amd64",
        "x86_64",
    }:
        raise ValueError("verification requires a supported native x86_64 host")
    sys.path.insert(0, str(ROOT / "scripts"))
    import package_plugin as package

    package.require_clean_source(source_sha)
    fixture = ROOT / FIXTURE_RELATIVE
    archive_digest = sha256_file(archive_path)
    with zipfile.ZipFile(archive_path) as archive:
        infos = archive.infolist()
        validate_members(infos)
        if sum(info.file_size for info in infos) > 128 * 1024 * 1024:
            raise ValueError("unpacked archive exceeds 128 MiB safety limit")
        names = {info.filename for info in infos}
        for name in names:
            key = name.casefold()
            if (
                key in package.HEADLESS_FORBIDDEN_EXACT_MEMBERS
                or any(
                    key.startswith(prefix)
                    for prefix in package.HEADLESS_FORBIDDEN_PREFIXES
                )
                or key.endswith(
                    (".db", ".sqlite", ".sqlite3", ".pyc", ".pyo", "-wal", "-shm")
                )
            ):
                raise ValueError(f"forbidden ZIP member: {name}")
        manifest = json.loads(archive.read(package.NATIVE_MANIFEST))
        if manifest.get("schema") != package.NATIVE_MANIFEST_SCHEMA:
            raise ValueError("unknown native manifest schema")
        package.validate_identity(manifest["build_info"], source_sha)
        if set(manifest["platforms"]) != {"linux", "win32"}:
            raise ValueError("both native platforms are required")
        expected = {package.NATIVE_INIT, package.NATIVE_MANIFEST}
        for entry in manifest["platforms"].values():
            member = (
                f"{package.NATIVE_BUNDLE_ROOT}/{entry['build_id']}/{entry['filename']}"
            )
            payload = archive.read(member)
            digest = hashlib.sha256(payload).hexdigest()
            if digest != entry["binary_sha256"] or digest != entry["build_id"]:
                raise ValueError(f"native digest mismatch: {member}")
            if entry["build_info"] != manifest["build_info"]:
                raise ValueError("cross-platform native identity mismatch")
            expected.add(member)
        for path in package._source_files():
            relative = path.relative_to(ROOT).as_posix()
            if archive.read(relative) != path.read_bytes():
                raise ValueError(f"source member differs: {relative}")
            expected.add(relative)
        if names != expected:
            raise ValueError(
                f"archive member allowlist mismatch: {sorted(names ^ expected)}"
            )
        if archive.read(package.NATIVE_INIT) != package.NATIVE_SOURCE_INIT.read_bytes():
            raise ValueError("native loader differs from source")
        with tempfile.TemporaryDirectory(
            prefix="ae-archive-verify-", dir=work_dir
        ) as temporary:
            root = Path(temporary)
            extracted = root / "archive"
            extracted.mkdir()
            archive.extractall(extracted)
            imported = run_probe(extracted, fixture, root)
        if imported["build_info"] != manifest["build_info"]:
            raise ValueError("runtime identity differs from manifest")
        if (
            imported["loaded_binary_sha256"]
            != manifest["platforms"][sys.platform]["binary_sha256"]
        ):
            raise ValueError("loaded binary differs from platform manifest")
    if sha256_file(archive_path) != archive_digest:
        raise ValueError("archive changed during verification")
    return {
        "status": "PASS",
        "source_sha": source_sha,
        "archive": archive_path.name,
        "archive_sha256": archive_digest,
        "archive_bytes": archive_path.stat().st_size,
        "members": len(infos),
        "platform": sys.platform,
        "machine": platform.machine(),
        "python": sys.version,
        "fixture": {
            "path": FIXTURE_RELATIVE.as_posix(),
            "sha256": sha256_file(fixture),
        },
        "source_member_parity": "PASS",
        "forbidden_members": "ABSENT",
        "import": imported,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--archive", required=True, type=Path)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--work-dir", required=True, type=Path)
    parser.add_argument("--receipt", required=True, type=Path)
    args = parser.parse_args()
    receipt = checked_path(args.receipt)
    if receipt.exists():
        raise FileExistsError(f"refusing to overwrite receipt: {receipt}")
    result = verify(args.archive, args.source_sha, args.work_dir)
    write_receipt(receipt, result)
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
