from __future__ import annotations

import importlib.util
import subprocess
import sys
from pathlib import Path

import pytest


SCRIPT = Path(__file__).resolve().parents[1] / "scripts/stage_native_runtime.py"


def test_legacy_stage_cli_fails_closed_without_mutation(tmp_path):
    wheel_dir = tmp_path / "wheels"
    wheel_dir.mkdir()
    (wheel_dir / "untrusted.whl").write_bytes(b"not a wheel")
    destination = tmp_path / "destination"
    destination.mkdir()
    sentinel = destination / "preserve.txt"
    sentinel.write_text("existing data", encoding="utf-8")
    result = subprocess.run(
        [
            sys.executable,
            str(SCRIPT),
            "--wheel-dir",
            str(wheel_dir),
            "--destination",
            str(destination),
        ],
        text=True,
        capture_output=True,
    )
    assert result.returncode != 0
    assert "STAGING_RETIRED" in result.stderr
    assert "--verify-wheel" in result.stderr
    assert "pip install" in result.stderr
    assert "ImportError" not in result.stderr
    assert list(destination.iterdir()) == [sentinel]
    assert sentinel.read_text() == "existing data"


def test_legacy_stage_python_api_fails_before_creating_paths(tmp_path):
    spec = importlib.util.spec_from_file_location("legacy_stage_runtime", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    with pytest.raises(ValueError, match="STAGING_RETIRED"):
        module.stage_native_runtime(
            tmp_path / "missing-wheels", tmp_path / "missing-target"
        )
    assert list(tmp_path.iterdir()) == []
