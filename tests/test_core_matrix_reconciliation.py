"""Current boundary ledger gate; historical count/snapshot tests remain historical."""
import hashlib
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BASE = "71cbd4eefd44e827111e3a194a58edbf9d246b92"


def test_core_matrix_mapping_preserves_history_and_defers_real_artifacts():
    ledger = json.loads((ROOT / "model/emotion-matrix-capability-parity-v1.json").read_text())
    historical = json.loads(subprocess.check_output(
        ["git", "show", f"{BASE}:model/emotion-matrix-capability-parity-v1.json"], cwd=ROOT))
    old_rows = {row["id"]: row for row in historical["capabilities"]}
    rows = {row["id"]: row for row in ledger["capabilities"]}
    assert len(rows) == len(ledger["capabilities"]) == 43
    assert rows.keys() == old_rows.keys()
    pending = {"windows-native-export", "linux-native-export", "universal-package-members"}
    assert {key for key, row in rows.items() if row["disposition"] == "UNMAPPED"} == pending
    assert ledger["release_gate"]["status"] == "NO_GO"
    assert ledger["release_gate"]["unmapped_capability_count"] == 3
    assert ledger["release_gate"]["source_mapped_capability_count"] == 40
    receipts = set()
    for key, row in rows.items():
        assert row["source_refs"] == old_rows[key]["source_refs"]
        assert row["evidence"] == old_rows[key]["evidence"]
        for evidence in row["evidence"]:
            path = ROOT / evidence["locator"]
            assert hashlib.sha256(path.read_bytes()).hexdigest() == evidence["sha256"]
            assert json.loads(path.read_text())["execution"]["exit_code"] == 0
            receipts.add(evidence["locator"])
        mapping = row["current_core_mapping"]
        if key in pending:
            assert mapping["status"] == "PENDING_REAL_ARTIFACT"
            assert mapping["execution_evidence"] == []
        else:
            assert mapping["status"] == "SOURCE_MAPPED"
    assert len(receipts) == 20
    for key in ["async-semantic-outbox", "outbox-authenticated-crypto"]:
        assert rows[key]["disposition"] == "HISTORY_PRESERVED_RUNTIME_RETIRED"
    supersession = json.loads((ROOT / "model/emotion-personality-core-supersession-v1.json").read_text())
    active = supersession["active_plan"]
    assert active["path"].endswith("2026-09-05-task11-12-core-boundary-embodiment-clock.md")
    assert subprocess.check_output(["git", "rev-parse", f'{active["commit"]}:{active["path"]}'], cwd=ROOT).decode().strip() == active["blob"]
    for task in ["task-13", "task-14"]:
        assert supersession["core_boundary_addendum"]["previous_plan_dispositions"][task] == "CANCELLED_CONTACT_INTENT_PROJECTION"
    receipt = json.loads((ROOT / ledger["core_boundary_reconciliation"]["execution_receipt"]).read_text())
    for run in receipt["runs"]:
        assert run["exit_code"] == 0
        assert hashlib.sha256((ROOT / run["log_path"]).read_bytes()).hexdigest() == run["log_sha256"]
    for source in receipt["source_files"]:
        assert hashlib.sha256((ROOT / source["path"]).read_bytes()).hexdigest() == source["sha256"]
