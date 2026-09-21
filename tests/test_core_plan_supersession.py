import json
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
LEDGER = ROOT / "model" / "emotion-personality-core-supersession-v1.json"
PARITY_LEDGER = ROOT / "model" / "emotion-matrix-capability-parity-v1.json"
PROVENANCE_LEDGER = ROOT / "model" / "emotion-matrix-provenance-v1.json"

SHA_RE = re.compile(r"^[0-9a-f]{40}$")
TASK_HEADING_RE = re.compile(r"^### Task ([0-9]+):")
INTERLEAVE_HEADING_RE = re.compile(r"^## Interleave (P[0-9]+):")

TOP_LEVEL_KEYS = {
    "schema",
    "approved_specs",
    "superseded_plans",
    "preserved_commits",
    "frozen_provenance",
    "active_plan",
    "parity_capability_count",
    "matrix_provenance_task_1",
    "old_matrix_tasks_2_to_9",
    "old_matrix_task_10",
    "old_proactive_tasks_1_to_3",
    "old_proactive_tasks_4_to_5",
    "old_proactive_tasks_6_to_7",
    "forbidden_active_capabilities",
    "forbidden_capability_categories",
}

APPROVED_SPEC_SNAPSHOTS = {
    "docs/superpowers/specs/2026-08-30-emotion-matrix-forward-port-design.md": (
        "8a8b5b233f34911e3af774a7f30d28b91a58c0b4",
        "6bd9a4756de57421807c9349fd6ce6a3cf5a8c8c",
    ),
    "docs/superpowers/specs/2026-08-31-emotion-personality-core-boundary-design.md": (
        "8a8b5b233f34911e3af774a7f30d28b91a58c0b4",
        "2e2b4f2460136c1624b7a2e654b9bf7b14fe27a5",
    ),
}

SUPERSEDED_PLAN_SNAPSHOTS = {
    "emotion-matrix-forward-port": {
        "path": "docs/superpowers/plans/2026-08-30-emotion-matrix-forward-port.md",
        "commit": "459069d28026b7d8b93a289d67c7b1b8a4b2b0c2",
        "blob": "3eec50c4a5229c97d4399e7d9a1d52e6b7b906a8",
    },
    "proactive-settings-simplification": {
        "path": "docs/superpowers/plans/2026-08-30-proactive-settings-simplification.md",
        "commit": "88a81da65cb5800e1e8fd1d78c16f43eff0792fb",
        "blob": "f683d80d921b3e3d60d67f98a78824ece1c4c48e",
    },
}

ACTIVE_PLAN_SNAPSHOT = {
    "path": "docs/superpowers/plans/2026-08-31-emotion-personality-core.md",
    "commit": "fa5c003fd437b0ab5b0e8b0d7b80066964204cde",
    "blob": "68479e6b41e99d38d279548bb658b59bd44384f4",
}

MATRIX_PROVENANCE_TASK_COMMITS = [
    "ef2a1f6df08800dcd4fee794349c6f883e409013",
    "f700f49b59b62a8461993c0e7b036793dea11288",
    "ece92b3f21c85d3ed4319283326f25f1ceb01ad0",
    "e60549f816d209e6b0d82097eb7395c74836bf7c",
]

PROACTIVE_TASKS_1_TO_3_COMMITS = [
    "3c60c06bd132cc5d1659cb02ea238317c9fb24e4",
    "39f61c9f59eed8dfba81aa2afc5cca8ad9fa2da5",
    "952798ae74b34300eca7a4bd5085c3fe92e560fc",
    "26fa25db03a1901c627ea56c76fbae9d471f30b3",
    "e2900333cf5400b1da93ddffaa8b86d4213e54fc",
    "565cd2f2725bd59acddd769df895dd12ff882464",
    "cf3cd7a96f76171732f4534fab9ffe8dd81390d5",
    "1e67b3602a15597e1500f2d358184f7b53bd2621",
    "b05d7b58eb21b91c2d931d8b7efcbcace4175381",
]

EXPECTED_TASK_DISPOSITIONS = {
    "emotion-matrix-forward-port": {
        "task-01": ("COMPLETED_PRESERVED", ["task-01"]),
        "task-02": ("ADAPTED_BY_THIS_PLAN", ["task-02"]),
        "task-03": ("ADAPTED_BY_THIS_PLAN", ["task-03"]),
        "task-04": ("ADAPTED_BY_THIS_PLAN", ["task-04"]),
        "task-05": ("ADAPTED_BY_THIS_PLAN", ["task-05"]),
        "task-06": ("ADAPTED_BY_THIS_PLAN", ["task-06"]),
        "task-07": ("ADAPTED_BY_THIS_PLAN", ["task-07"]),
        "task-08": ("ADAPTED_BY_THIS_PLAN", ["task-08"]),
        "task-09": ("ADAPTED_BY_THIS_PLAN", ["task-09"]),
        "interleave-p4": ("CANCELLED_ACTIVE_SEND", ["task-12"]),
        "task-10": ("CANCELLED_ACTIVE_SEND", ["task-12", "task-13"]),
        "interleave-p5": ("CANCELLED_ACTIVE_SEND", ["task-12"]),
        "task-11": ("ADAPTED_BY_THIS_PLAN", ["task-12", "task-14"]),
        "task-12": ("ADAPTED_BY_THIS_PLAN", ["task-15"]),
        "task-13": ("ADAPTED_BY_THIS_PLAN", ["task-16"]),
    },
    "proactive-settings-simplification": {
        "task-01": ("HISTORY_PRESERVED_RUNTIME_RETIRED", ["task-12"]),
        "task-02": ("HISTORY_PRESERVED_RUNTIME_RETIRED", ["task-12"]),
        "task-03": ("HISTORY_PRESERVED_RUNTIME_RETIRED", ["task-12"]),
        "task-04": ("CANCELLED_ACTIVE_SEND", ["task-12"]),
        "task-05": ("CANCELLED_ACTIVE_SEND", ["task-12"]),
        "task-06": ("SUPERSEDED_RELEASE_GATES", ["task-15"]),
        "task-07": ("SUPERSEDED_RELEASE_GATES", ["task-16"]),
    },
}

FORBIDDEN_CAPABILITY_CATEGORIES = {
    "supervisor": [
        "proactive.scheduler",
        "proactive.send_time",
        "host.pending_autonomy_work",
        "host.recover_autonomy",
    ],
    "recipient discovery": [
        "recipient.enumerate",
        "recipient.rank",
        "recipient.select",
    ],
    "proactive LLM generation": [
        "proactive.provider.select",
        "proactive.provider.generate",
    ],
    "proactive token budget": [
        "proactive.provider.usage_settlement",
        "proactive.provider.token_budget",
    ],
    "outbox retry": [
        "proactive.outbox.materialize",
        "proactive.outbox.recover",
        "proactive.outbox.retry",
    ],
    "platform proactive delivery": [
        "host.submit_proactive_message",
        "platform.proactive_delivery",
        "network.background_client",
    ],
    "externalization claim": [
        "proactive.externalization.claim",
        "proactive.externalization.settlement",
    ],
    "dispatch claim": [
        "proactive.dispatch.claim",
        "proactive.dispatch.settlement",
    ],
}

FORBIDDEN_CAPABILITY_IDS = [
    capability_id
    for capability_ids in FORBIDDEN_CAPABILITY_CATEGORIES.values()
    for capability_id in capability_ids
]


def _reject_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def _load_json(path):
    return json.loads(
        path.read_text(encoding="utf-8"),
        object_pairs_hook=_reject_duplicate_keys,
    )


def _git(*args):
    completed = subprocess.run(
        ["git", *args],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    )
    return completed.stdout.strip()


def _assert_exact_keys(value, expected, label):
    assert set(value) == expected, f"{label} keys drifted: {set(value) ^ expected}"


def _assert_commit_exists_and_is_ancestor(commit):
    assert SHA_RE.fullmatch(commit)
    _git("cat-file", "-e", f"{commit}^{{commit}}")
    _git("merge-base", "--is-ancestor", commit, "HEAD")


def _assert_content_addressed(path, commit, blob):
    _assert_commit_exists_and_is_ancestor(commit)
    assert SHA_RE.fullmatch(blob)
    _git("cat-file", "-e", f"{blob}^{{blob}}")
    assert _git("rev-parse", f"{commit}:{path}") == blob


def _task_universe(plan_text):
    universe = []
    for line in plan_text.splitlines():
        if match := TASK_HEADING_RE.match(line):
            universe.append(f"task-{int(match.group(1)):02d}")
        elif match := INTERLEAVE_HEADING_RE.match(line):
            universe.append(f"interleave-{match.group(1).lower()}")
    return universe


def test_old_plans_have_one_closed_disposition():
    data = _load_json(LEDGER)
    _assert_exact_keys(data, TOP_LEVEL_KEYS, "ledger")
    assert data["schema"] == "ae.emotion-personality-core-supersession.v1"

    specs = data["approved_specs"]
    assert len(specs) == len(APPROVED_SPEC_SNAPSHOTS)
    assert len({spec["path"] for spec in specs}) == len(specs)
    for spec in specs:
        _assert_exact_keys(spec, {"path", "commit", "blob"}, "approved spec")
        assert (spec["commit"], spec["blob"]) == APPROVED_SPEC_SNAPSHOTS[spec["path"]]
        _assert_content_addressed(spec["path"], spec["commit"], spec["blob"])

    active_plan = data["active_plan"]
    _assert_exact_keys(active_plan, {"path", "commit", "blob"}, "active plan")
    assert active_plan == ACTIVE_PLAN_SNAPSHOT
    _assert_content_addressed(
        active_plan["path"], active_plan["commit"], active_plan["blob"]
    )
    current_task_ids = _task_universe(
        _git("show", f'{active_plan["commit"]}:{active_plan["path"]}')
    )
    assert current_task_ids == [f"task-{task_id:02d}" for task_id in range(1, 17)]

    superseded_plans = data["superseded_plans"]
    assert len(superseded_plans) == len(SUPERSEDED_PLAN_SNAPSHOTS)
    assert len({plan["plan_id"] for plan in superseded_plans}) == len(superseded_plans)
    for plan in superseded_plans:
        _assert_exact_keys(
            plan,
            {
                "plan_id",
                "path",
                "commit",
                "blob",
                "task_universe",
                "task_dispositions",
            },
            "superseded plan",
        )
        snapshot = SUPERSEDED_PLAN_SNAPSHOTS[plan["plan_id"]]
        assert {key: plan[key] for key in ("path", "commit", "blob")} == snapshot
        _assert_content_addressed(plan["path"], plan["commit"], plan["blob"])

        derived_universe = _task_universe(
            _git("show", f'{plan["commit"]}:{plan["path"]}')
        )
        assert plan["task_universe"] == derived_universe
        assert len(derived_universe) == len(set(derived_universe))

        disposition_records = plan["task_dispositions"]
        for record in disposition_records:
            _assert_exact_keys(
                record,
                {"legacy_task_id", "disposition", "replacement_current_task_ids"},
                "task disposition",
            )
            replacements = record["replacement_current_task_ids"]
            assert replacements
            assert len(replacements) == len(set(replacements))
            assert set(replacements) <= set(current_task_ids)

        record_ids = [record["legacy_task_id"] for record in disposition_records]
        assert len(record_ids) == len(set(record_ids))
        assert set(record_ids) == set(derived_universe)
        actual_mapping = {
            record["legacy_task_id"]: (
                record["disposition"],
                record["replacement_current_task_ids"],
            )
            for record in disposition_records
        }
        assert actual_mapping == EXPECTED_TASK_DISPOSITIONS[plan["plan_id"]]

    assert data["matrix_provenance_task_1"] == "COMPLETED_PRESERVED"
    assert data["old_matrix_tasks_2_to_9"] == "ADAPTED_BY_THIS_PLAN"
    assert data["old_matrix_task_10"] == "CANCELLED_ACTIVE_SEND"
    assert data["old_proactive_tasks_1_to_3"] == "HISTORY_PRESERVED_RUNTIME_RETIRED"
    assert data["old_proactive_tasks_4_to_5"] == "CANCELLED_ACTIVE_SEND"
    assert data["old_proactive_tasks_6_to_7"] == "SUPERSEDED_RELEASE_GATES"

    preserved = data["preserved_commits"]
    _assert_exact_keys(
        preserved,
        {"matrix_provenance_task_1", "old_proactive_tasks_1_to_3"},
        "preserved commits",
    )
    assert preserved["matrix_provenance_task_1"] == MATRIX_PROVENANCE_TASK_COMMITS
    assert preserved["old_proactive_tasks_1_to_3"] == PROACTIVE_TASKS_1_TO_3_COMMITS
    all_preserved_commits = [
        *preserved["matrix_provenance_task_1"],
        *preserved["old_proactive_tasks_1_to_3"],
    ]
    assert len(all_preserved_commits) == len(set(all_preserved_commits))
    for commit in all_preserved_commits:
        _assert_commit_exists_and_is_ancestor(commit)

    frozen = data["frozen_provenance"]
    _assert_exact_keys(
        frozen,
        {"path", "frozen_at_commit", "blob", "path_commits", "task_commits"},
        "frozen provenance",
    )
    assert frozen == {
        "path": "model/emotion-matrix-provenance-v1.json",
        "frozen_at_commit": "e60549f816d209e6b0d82097eb7395c74836bf7c",
        "blob": "1747b5550aa986b55535f688c418bfe6bec8a34f",
        "path_commits": [
            "ece92b3f21c85d3ed4319283326f25f1ceb01ad0",
            "ef2a1f6df08800dcd4fee794349c6f883e409013",
        ],
        "task_commits": MATRIX_PROVENANCE_TASK_COMMITS,
    }
    _assert_content_addressed(
        frozen["path"], frozen["frozen_at_commit"], frozen["blob"]
    )
    assert _git("rev-parse", f'HEAD:{frozen["path"]}') == frozen["blob"]
    assert _git("log", "--format=%H", "--", frozen["path"]).splitlines() == frozen[
        "path_commits"
    ]
    assert frozen["task_commits"] == preserved["matrix_provenance_task_1"]

    provenance = _load_json(PROVENANCE_LEDGER)
    assert provenance["schema"] == "ae.emotion-matrix-provenance.v1"
    assert len(provenance["files"]) == 20
    assert sum(len(file_record["source_symbols"]) for file_record in provenance["files"]) == 168
    assert sum(len(file_record["target_symbols"]) for file_record in provenance["files"]) == 168

    parity = _load_json(PARITY_LEDGER)
    capability_ids = [capability["id"] for capability in parity["capabilities"]]
    assert len(capability_ids) == len(set(capability_ids)) == 43
    assert data["parity_capability_count"] == len(capability_ids)
    assert data["parity_capability_count"] == parity["release_gate"][
        "validated_capability_count"
    ]

    assert data["forbidden_active_capabilities"] == FORBIDDEN_CAPABILITY_IDS
    assert len(FORBIDDEN_CAPABILITY_IDS) == len(set(FORBIDDEN_CAPABILITY_IDS))
    categories = data["forbidden_capability_categories"]
    _assert_exact_keys(categories, set(FORBIDDEN_CAPABILITY_CATEGORIES), "forbidden categories")
    assert categories == FORBIDDEN_CAPABILITY_CATEGORIES
    categorized_ids = [
        capability_id
        for capability_ids in categories.values()
        for capability_id in capability_ids
    ]
    assert categorized_ids == data["forbidden_active_capabilities"]
    assert len(categorized_ids) == len(set(categorized_ids))
