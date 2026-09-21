"""Safety checks do not require a native build or touch user databases."""

import ast
import importlib.util
import json
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

    def test_probe_success_returns_json(self):
        self.assertTrue(hasattr(self.verifier, "run_probe"), "probe runner is missing")
        self.verifier.PROBE = 'print(\'{"build_info": {"version": "test"}}\')'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.assertEqual(
                self.verifier.run_probe(root, root, root),
                {"build_info": {"version": "test"}},
            )

    def test_probe_failure_reports_exit_code_and_captured_streams(self):
        self.assertTrue(hasattr(self.verifier, "run_probe"), "probe runner is missing")
        self.verifier.PROBE = "import sys; print('stage: historical'); print('assertion evidence', file=sys.stderr); sys.exit(7)"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError) as caught:
                self.verifier.run_probe(root, root, root)
        diagnostic = json.loads(str(caught.exception))
        self.assertEqual(diagnostic["status"], "NATIVE_PROBE_FAILED")
        self.assertEqual(diagnostic["exit_code"], 7)
        self.assertIn("stage: historical", diagnostic["stdout"])
        self.assertIn("assertion evidence", diagnostic["stderr"])
        self.assertNotIn(self.verifier.PROBE, str(caught.exception))

    def test_probe_timeout_preserves_partial_output(self):
        self.assertTrue(hasattr(self.verifier, "run_probe"), "probe runner is missing")
        self.verifier.PROBE = "import sys,time; print('before timeout', flush=True); print('waiting', file=sys.stderr, flush=True); time.sleep(30)"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError) as caught:
                self.verifier.run_probe(root, root, root, timeout_seconds=1)
        diagnostic = json.loads(str(caught.exception))
        self.assertEqual(diagnostic["status"], "NATIVE_PROBE_TIMEOUT")
        self.assertIsNone(diagnostic["exit_code"])
        self.assertEqual(diagnostic["timeout_seconds"], 1)
        self.assertIn("before timeout", diagnostic["stdout"])
        self.assertIn("waiting", diagnostic["stderr"])
        self.assertNotIn(self.verifier.PROBE, str(caught.exception))

    def test_probe_invalid_receipt_reports_output(self):
        self.assertTrue(hasattr(self.verifier, "run_probe"), "probe runner is missing")
        self.verifier.PROBE = "print('not a JSON receipt')"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError) as caught:
                self.verifier.run_probe(root, root, root)
        diagnostic = json.loads(str(caught.exception))
        self.assertEqual(diagnostic["status"], "NATIVE_PROBE_INVALID_RECEIPT")
        self.assertEqual(diagnostic["exit_code"], 0)
        self.assertIn("not a JSON receipt", diagnostic["stdout"])

    def test_probe_failure_preserves_non_utf8_error_bytes(self):
        self.verifier.PROBE = (
            "import sys; sys.stderr.buffer.write(bytes([255, 129])); sys.exit(3)"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError) as caught:
                self.verifier.run_probe(root, root, root)
        diagnostic = json.loads(str(caught.exception))
        self.assertEqual(diagnostic["exit_code"], 3)
        self.assertEqual(diagnostic["stderr"], r"\xff\x81")

    def test_probe_large_failure_output_is_bounded_with_head_and_tail(self):
        self.verifier.PROBE = (
            "import sys; "
            "sys.stdout.write('OUT_HEAD' + 'x'*20000 + 'OUT_TAIL'); "
            "sys.stderr.write('ERR_HEAD' + 'y'*20000 + 'ERR_TAIL'); sys.exit(9)"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            with self.assertRaises(RuntimeError) as caught:
                self.verifier.run_probe(root, root, root)
        diagnostic = json.loads(str(caught.exception))
        for stream, prefix in (("stdout", "OUT"), ("stderr", "ERR")):
            self.assertLessEqual(len(diagnostic[stream]), 8192)
            self.assertTrue(diagnostic[stream].startswith(prefix + "_HEAD"))
            self.assertTrue(diagnostic[stream].endswith(prefix + "_TAIL"))
            self.assertIn("truncated", diagnostic[stream])
            self.assertTrue(diagnostic[stream + "_truncated"])
            self.assertEqual(diagnostic[stream + "_bytes"], 20016)
            self.assertEqual(diagnostic[stream + "_characters"], 20016)

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

    def test_retired_sidecar_preservation_detects_mutations(self):
        helper = next(
            node
            for node in ast.parse(self.verifier.PROBE).body
            if isinstance(node, ast.FunctionDef)
            and node.name == "exercise_retired_authority_link"
        )
        for mutation in (
            None,
            "target_bytes",
            "target_members",
            "database",
            "fresh_sidecar",
        ):
            with (
                self.subTest(mutation=mutation),
                tempfile.TemporaryDirectory() as temporary,
            ):
                root = Path(temporary)

                def lifecycle(path):
                    if not path.exists():
                        path.write_bytes(b"initialized fixture database")
                        if mutation == "fresh_sidecar":
                            (path.parent / ".native-authority").mkdir()
                    elif mutation == "target_bytes":
                        (path.parent / ".native-authority/legacy-key.bin").write_bytes(
                            b"changed"
                        )
                    elif mutation == "target_members":
                        (path.parent / ".native-authority/extra").write_bytes(
                            b"unexpected"
                        )
                    elif mutation == "database":
                        path.write_bytes(b"changed")

                namespace = {
                    "Path": Path,
                    "stat": stat,
                    "sys": sys,
                    "lifecycle": lifecycle,
                    "catalog": lambda path: path.read_bytes(),
                }
                exec(
                    compile(
                        ast.Module(body=[helper], type_ignores=[]),
                        "probe-helper",
                        "exec",
                    ),
                    namespace,
                )
                run = namespace["exercise_retired_authority_link"]
                if mutation:
                    with self.assertRaises(AssertionError):
                        run(root)
                else:
                    self.assertIn(run(root), {"symlink", "junction"})


if __name__ == "__main__":
    unittest.main()
