# Matrix preservation after Task 12A

Baseline: `71cbd4eefd44e827111e3a194a58edbf9d246b92`. Source-only reconciliation;
no production code, frozen vector, historical receipt or packaging code changed.

The 43-row ledger now maps 40 source capabilities, including two explicitly
retired execution capabilities. Three real artifact capabilities remain pending.
All 31 previously mapped rows retain their original 20 receipt files, hashes and
exit-zero records. They prove their historical executions, not a new full-suite run.

## Supported runtime replacements

| Retained historical target | Current `core_matrix_regression` scenario | Coverage |
| --- | --- | --- |
| `emotion_matrix_event_lane` | `nonzero_stimulus_delivery_and_reopen_preserve_matrix_lane` | Typed inbound, nonzero semantic appraisal, delivery without matrix mutation, dedupe without Provider reauthorization, persisted matrix/revision, continuing stimulus after reopen. |
| `emotion_matrix_time` | `persona_sleep_clock_projects_matrix_without_evidence_and_reopens` | Persona fixed sleep, matrix time projection, stable graph/semantic anchor, no added evidence/history, exact replay, reopen, backward-time rejection. |
| `emotion_matrix_projection` | `committed_persona_reply_affect_replays_and_corruption_is_read_only_rejected` | Valid nine-region committed ReplyAffect with nonzero delta, persisted exact replay, inspection, corruption audit/reopen rejection without repair. |

The original files remain historical fixtures under `legacy-semantic-test-api`.
Their zero-test default runs are not gates. Old contact/relation observer and
dispatch behavior is cancelled by the addendum, not restored by these replacements.
Historical relation evidence remains isolated and preserved; current appraisal and
ReplyAffect are persona-only. This migration does not claim one-to-one replacement
of every old fault-injection or observer assertion. Other legacy perception tests
and full-suite integration remain outside this bounded migration.

Existing numerical coverage is reused unchanged: contracts freeze all 15 slots,
ranges and routes; attention freezes load routing; neurofield covers nine-region
layout, capacity, graph bytes/digests; dynamics pins full 16K Jacobi/eight-DOF
rounding and invalid input; semantic-core matrix_time pins sleep relaxation,
coefficients, mixed phases and exact chunking. Runtime does not duplicate these
goldens. Personality Genesis and ordinary inbound LLM appraisal remain in the core.

## Actual verification

- New runtime target compiled first, then 3 passed, zero ignored/filtered.
- Existing contract/attention/graph/dynamics/time targets: 16 passed.
- Current 19-method source surface/manifest: 2 passed.
- `cargo check --locked --offline --workspace -j 1`: exit 0, existing dead-code warnings.
- Current ledger gate: `python -m pytest -q tests/test_core_matrix_reconciliation.py`.

Execution logs and content hashes are in `matrix-regression-evidence/`.
Rust used `CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=0`,
`PYO3_NO_PYTHON=1`, one build job and default features. No legacy feature was enabled.
The runtime test received rustfmt-only formatting after its successful run.
The local `.venv` lacks pytest; the successful Python run used the installed
`python` command. No dependency installation was needed.

`test_core_plan_supersession.py` and historical sections of
`test_emotion_matrix_provenance.py` freeze earlier active-plan/count snapshots
(including the already stale 14-unmapped snapshot). Their historical assertions
and numerical goldens are unchanged; this reconciliation uses the new narrow
ledger gate. This is not a claim that those full historical suites pass today.

## Release closure without a hash cycle

Keep Windows export, Linux export and universal package members pending in source.
After the final clean source commit, an external release sidecar must name that
SHA, actual wheel hashes, fresh platform import results, identical 19-method API,
v9 schema and tzdb identities, and the final ZIP member hashes. Do not commit the
sidecar into the source SHA it attests. A compile or a local wheel is insufficient.

Manual acceptance after build: install each wheel in a fresh platform environment,
import the 19-method module, run the existing positive/retired-operation rejection
smoke, compare schema/tzdb/API digests, and inspect the actual universal ZIP and
native members. Full historical test migration remains a separate task.
