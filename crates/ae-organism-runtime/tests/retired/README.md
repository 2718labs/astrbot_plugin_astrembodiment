# Historical 1.0 test sources

These files were brought in by the master merge and are retained verbatim for
migration reference. Cargo does not discover integration tests recursively.
They are not executable acceptance targets in 1.1.0: their original source
include paths, internal macro expansion context, and retired APIs are preserved,
not adapted. Moving them does not establish equivalent test coverage.

## Preserved files

- `committed_semantic_projection_path.rs`
- `private_projection_payload_producer.rs`
- `private_projection_runtime.rs`
- `support/private_projection_runtime.rs`

## Boundary and current coverage

The frozen contract is
[retired-surface-release.md](../../../../docs/superpowers/plans/2026-09-05-task11-12-core-boundary-embodiment-clock/contracts/retired-surface-release.md).
Its 19 Native methods and typed core inbound/delivery/embodiment-clock APIs
replace the generic journal/event, R7 projection, rebirth/continuity-vault, and
native asynchronous-outbox integration surfaces used here. The unregistered
legacy Runtime/Store modules are source references, not production entrypoints.

Current runtime behavior is exercised by ae-runtime targets
`core_matrix_regression` and `core_boundary_runtime`; numerical transitions
are exercised by ae-semantic-core targets `matrix_time`,
`user_stimulus_transition`, and `affect_projection`. The standalone
`phase0_native_semantic` numerical regression remains active in ae-runtime.
Store's `legacy_semantic_upgrade_schema` retains the original historical DDL
fixture and checks rejected databases remain unchanged. Store integration
target `core_boundary_v9` checks zero-write reopen and typed SQLite revision
errors. The historical Store unit-test crate still has retired-API compile
debt; a `--lib` name filter cannot bypass its compilation. These gates do not
assert support for the historical vault/rebirth or
outbox APIs, and no historical database is opened or rewritten by this move.

Pre-existing candidate tests outside this directory are not implicitly certified;
the release workflow names its active targets explicitly instead of claiming
that all historical workspace tests pass.
