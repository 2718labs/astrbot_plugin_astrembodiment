"""Safety checks do not require a native build or touch user databases."""

import ast
import importlib.util
import os
import re
import stat
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "scripts/verify_release_archive.py"


class ArchiveVerifierTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), "production archive verifier is missing")
        spec = importlib.util.spec_from_file_location("archive_verifier", SCRIPT)
        self.verifier = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.verifier)

    def test_safe_members(self):
        self.verifier.validate_members([zipfile.ZipInfo("pkg/main.py")])

    def test_unsafe_and_case_colliding_members(self):
        for name in (
            "../outside",
            "/absolute",
            "C:/outside",
            "a\\b",
            "a/../b",
            "a//b",
            "NUL.txt",
            "file.",
            "a:b",
            "folder/",
        ):
            info = zipfile.ZipInfo("placeholder")
            # ZIP readers preserve raw names, whereas ZipInfo's constructor on
            # Windows normalizes backslashes when creating an archive.
            info.filename = name
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.verifier.validate_members([info])
        with self.assertRaises(ValueError):
            self.verifier.validate_members(
                [zipfile.ZipInfo("A.py"), zipfile.ZipInfo("a.py")]
            )

    def test_symlink_reparse_and_special_zip_members_rejected(self):
        for mode, attrs in (
            (stat.S_IFLNK | 0o777, 0),
            (stat.S_IFIFO | 0o600, 0),
            (stat.S_IFREG | 0o600, 0x400),
        ):
            info = zipfile.ZipInfo("entry")
            info.external_attr = (mode << 16) | attrs
            with self.assertRaises(ValueError):
                self.verifier.validate_members([info])

    def test_file_directory_collision_rejected(self):
        with self.assertRaises(ValueError):
            self.verifier.validate_members(
                [zipfile.ZipInfo("a"), zipfile.ZipInfo("a/b")]
            )

    def test_source_sha_must_be_full(self):
        self.assertEqual(self.verifier.validate_sha("a" * 40), "a" * 40)
        for value in ("HEAD", "a" * 7, "z" * 40):
            with self.assertRaises(ValueError):
                self.verifier.validate_sha(value)

    def test_receipt_never_overwrites_existing_data(self):
        with tempfile.TemporaryDirectory() as temporary:
            receipt = Path(temporary) / "receipt.json"
            receipt.write_text("sentinel")
            with self.assertRaises(FileExistsError):
                self.verifier.write_receipt(receipt, {"status": "PASS"})
            self.assertEqual(receipt.read_text(), "sentinel")

    def test_work_directory_link_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            link = root / "link"
            junction = False
            try:
                link.symlink_to(root, target_is_directory=True)
            except OSError:
                if sys.platform != "win32":
                    raise
                subprocess.run(
                    [
                        "powershell.exe",
                        "-NoProfile",
                        "-NonInteractive",
                        "-Command",
                        "New-Item -ItemType Junction -Path $env:AE_LINK -Target $env:AE_TARGET | Out-Null",
                    ],
                    env=dict(os.environ, AE_LINK=str(link), AE_TARGET=str(root)),
                    check=True,
                    capture_output=True,
                )
                junction = True
            try:
                with self.assertRaises(ValueError):
                    self.verifier.checked_path(link / "child")
            finally:
                if junction:
                    os.rmdir(link)
                else:
                    link.unlink()

    def test_isolated_probe_compiles(self):
        compile(self.verifier.PROBE, "native-probe", "exec")

    def test_probe_calls_only_current_public_methods(self):
        manifest = SCRIPT.parents[1] / "crates/ae-contracts/src/core_surface.rs"
        public_section = (
            manifest.read_text(encoding="utf-8")
            .split("pub const CORE_PUBLIC_METHOD_MANIFEST_V1:", 1)[1]
            .split("];", 1)[0]
        )
        exports = re.findall(r'\("(\w+)",\s*1\)', public_section)
        self.assertEqual(len(exports), 19)
        calls = [
            node
            for node in ast.walk(ast.parse(self.verifier.PROBE))
            if isinstance(node, ast.Call)
            and isinstance(node.func, ast.Attribute)
            and isinstance(node.func.value, ast.Name)
            and node.func.value.id == "native"
        ]
        self.assertTrue(calls)
        for call in calls:
            self.assertIn(call.func.attr, {*exports, "NativeCoreError"})


if __name__ == "__main__":
    unittest.main()
