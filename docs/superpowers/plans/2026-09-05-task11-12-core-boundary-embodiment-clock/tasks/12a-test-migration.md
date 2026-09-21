# Task 12A Test Migration Handoff

2026-09-08 follow-up: the three matrix runtime groups now have supported aggregate
core replacements and fresh focused execution evidence in
[matrix-regression.md](matrix-regression.md). The retained legacy entry points
remain historical; other migration items below are still outside that bounded run.

Recorded: 2026-09-08.
Implementation reviewed: `0784ecebbc3878a8e7231a3666c38526cd95676a`.
This handoff changes documentation only. It does not modify, disable or delete
the retained matrix/perception tests, change algorithms, or report a full-suite
pass. The coordinator accepted the execution boundary after independent review
reported APPROVE with no P0/P1 findings. Historical codec visibility follows
[the controlled exception](../contracts/retired-surface-release.md#811-controlled-task-12a-historical-codec-exception).

## Evidence already obtained

- Four-library check passed: `ae-contracts`, `ae-store`, `ae-runtime`,
  `astrembodiment-core` with one build job.
- A fresh local Windows Native build from the implementation source imported
  through `target/task12a/python`; `astrembodiment_core`, `astr_embodiment` and
  `main` imported successfully.
- `tests/test_core_boundary_surface.py`, `tests/test_ordinary_reply_flow.py`
  and `tests/test_semantic_appraisal_core.py`: 14 passed. The Native callable set
  was exactly the 19-method manifest; retired operation names were rejected
  before malformed/oversized body parsing and unknown names remained distinct.
- Contract classifier/manifest unit tests: 2 passed in the Task 12A run.
- Five checked-in neurofield vectors, four tzdb assets, the v9 implementation,
  its test file and the v9 schema-manifest document were byte-identical to
  `f3a6a6a490492c5f7013a66535c22e6ed9404206`.

The local Native reports `source_sha: null` and `release_verified: false`.
It is local runtime evidence, not Task 12B clean-source/cross-platform release
provenance. This documentation commit requires no Native rebuild; artifacts
for release must still be built and attested from the final clean source SHA.

## Retained targets requiring migration

The following inventory is based on source references to retired symbols.
These targets were not compiled or run as a complete set after retirement;
the inventory is not a test-failure transcript or an exhaustive repository scan.

| Target or file | Follow-up work |
| --- | --- |
| `crates/ae-runtime/tests/emotion_matrix_event_lane.rs` | Reconnect event setup to typed core inbound/delivery. Preserve matrix-state, revision and event-lane assertions. |
| `crates/ae-runtime/tests/emotion_matrix_projection.rs` | Replace retired wake/observation setup with typed inbound/clock and supported reply-affect or internal attested projection fixtures. Preserve projection, 16K reduction and digest assertions. |
| `crates/ae-runtime/tests/emotion_matrix_time.rs` | Replace old time submission with typed embodiment-clock setup. Preserve time evolution, anchor/restart/replay and state/graph assertions. |
| `crates/ae-runtime/tests/perception_origin_authority.rs` | Reconnect challenge creation to committed typed inbound. Preserve source-origin, causality, duplicate and authority rejection checks. |
| `crates/ae-runtime/tests/perception_proposal_authority.rs` | Reconnect reservations/challenges and settlement to typed core APIs. Preserve proposal-authority and semantic mutation checks. |
| `crates/ae-runtime/tests/alpha3_contact.rs` | Separate obsolete contact execution expectations from any reusable historical-data assertions; active contact behavior must become absence/rejection coverage. |
| `crates/ae-runtime/tests/alpha3_headless_boundary.rs` | Replace removed runtime wake/generic entry points with current closed-surface checks. Keep headless boundary assertions. |
| `crates/ae-runtime/tests/alpha3_projection.rs` | Retire old dispatch/contact projection behavior; preserve any useful historical verification assertions through read-only fixtures. |
| `crates/ae-runtime/tests/proactive_outbox.rs` | Old gate/target/dispatch imports are retired. Replace execution expectations with no-reachability coverage without restoring their APIs. |
| `crates/ae-runtime/src/lib.rs` test module | Review helper/test references to deleted bootstrap, wake, generic event and projection methods; retain Genesis and current runtime assertions. |
| `crates/ae-store/tests/autonomy_migration.rs` | The `migration-test-hooks` target references removed `commit_journal` and externalization methods. Use frozen historical fixtures/read-only migration verification; keep row-preservation/fence assertions. |
| `tests/test_runtime_integration.py` | Remove the retired `build_interaction_batch_v1` import and migrate relevant ordinary/semantic fixtures to typed inbound/delivery. Preserve Genesis, Native loading and live core integration coverage. |
| `tests/test_relation_binding.py` | Remove the import and two tests of the deleted secret-store implementation. Preserve the two relation-identity tests. No installed key, config or ciphertext is a cleanup target. |
| `tests/test_static_contracts.py` | Audit static expectations for retired files/symbols and align them with the explicit manifest. A banned-word scan must not reject the approved historical codec role. |

The three `emotion_matrix_*` targets and
`perception_proposal_authority.rs` currently use the
`legacy-semantic-test-api` feature. Their mere absence from a default test run
is not regression evidence. Reconnection must preserve the algorithm and
authority assertions and run them through an appropriate supported test setup;
removing a feature flag or adding blanket skips is not acceptance.

The retired-only Python suites `test_autonomous_supervisor.py`,
`test_proactive_adapter.py`, `test_proactive_settings.py` and
`test_alpha3_host_boundaries.py` were removed in Task 12A. Current surface
absence/rejection and ordinary reply behavior are covered by the targeted core
tests above. This does not imply that every assertion in those old suites has
a one-to-one replacement.

## Follow-up acceptance

At the next matrix-regression wiring task, first compile the changed targets,
then run the retained assertions using typed inbound, semantic settlement and
embodiment-clock fixtures. Report the exact feature set, targets, source SHA and
results. Keep frozen numerical vectors, matrix dimensions, 15D semantics,
Genesis identity and historical schema/digests unchanged. Do not reintroduce a
public old API as a test convenience, and do not delete matrix algorithm
coverage to make compilation pass.

Full-suite/CI status remains unverified until that migration runs. Task 12B may
prepare packaging independently, but must label any excluded targets and cannot
represent packaging checks or platform imports as a full regression pass.
