# 1.1.0 CI and release integration

This integration adapts the master publishing state machine to the closed core.
It does not authorize a release, prove AstrBot installation, or declare all
historical tests compatible with the current 19-method contract.

## Release evidence chain

1. CI checks the production version contract and current host/core regressions.
2. Windows and actual Linux runners build from the same full checkout SHA using
   `package_plugin.py --build-native --source-sha`. Wheel compilation explicitly
   enables PyO3 extension linking in `pyproject.toml`; ordinary Cargo checks do not.
3. Each host imports its own fresh wheel and produces an identity/hash receipt.
   Wheels and receipts stay outside the checkout, preserving the clean-source gate.
4. Assembly accepts exactly both matching platform receipts and source identities.
   The production release additionally reproduces the ZIP byte-for-byte, then
   creates its SHA-256 sidecar. CI artifact tests receive the actual absolute
   wheel paths and `AE_RELEASE_ARCHIVE`; fixture tests are not artifact acceptance.
5. Each platform receives the same assembled ZIP. The verifier compares source
   bytes, loaded native identity, exact 19 APIs and binary hashes; it runs fresh
   and historical `1774023` database open/close/reopen, rejects nonempty unsafe
   upgrade evidence unchanged, and verifies that fresh stores do not create the
   retired `.native-authority` sidecar. A deliberately placed sidecar link retains
   its link identity and target, sentinel members/bytes and initialized database
   bytes/catalog across reopen. This proves preservation, not native rejection
   or absence of reads. Active `.astr-embodiment-field-migration-preimages`
   validation for authenticated migrations remains unchanged. All databases and
   sentinels are temporary fixtures.
6. Publishing depends on both archive jobs. Default permissions remain read-only;
   only the publish job obtains contents-write. Successful master push CI,
   current-master checks, annotated-tag object checks, tag-only recovery, draft
   resume, no asset replacement, remote byte checks and immutable-state reporting
   remain active. Published assets remain ZIP plus checksum; Actions artifacts
   retain both archive receipts, and the ZIP manifest retains both wheel receipts.

`.gitattributes` disables checkout newline conversion so all operating systems
receive exact Git blob bytes. The frozen tzdb manifest contains intentional CRLF;
no renormalization or historical evidence rewrite is required.

## Current regression scope

Rust checks target production libraries/binaries. Explicit integration targets are:

- `ae-runtime`: `core_boundary_runtime`, `core_matrix_regression`, `phase0_native_semantic`.
- `ae-store`: `core_boundary_v9`, `legacy_semantic_upgrade_schema`.
- `ae-semantic-core`: `matrix_time`, `user_stimulus_transition`, `affect_projection`.
- `ae-neurofield`: `graph_persistence_replay`, `structural_delta_cas`.

The obsolete alpha3/proactive/continuity/outbox APIs are not an alternative
public surface. Historical tests moved to `tests/retired/` remain source evidence;
CI does not implicitly compile them with `--all-targets` or `cargo test --workspace`.

Host CI runs `test_host_master_integration`, `test_semantic_appraisal_core`,
`test_ordinary_reply_flow`, `test_embodiment_clock` and `test_core_boundary_surface`. The runtime integration
selection covers provider preference, persona resolution, SeedCode persistence,
request injection, fixed runtime fields, native discovery/errors, schema and
commands. Auxiliary transport excludes only the three old coordinator preflight
cases (`transient_semantic_failure` and `semantic_owner` selectors), whose retired
entry point is replaced by typed appraisal tests. This is an explicit active
contract selection, not a full historical-suite claim.

Release tests cover production version/state-machine guards, wheel identity and
receipt rejection, exact archive structure, deterministic ZIP metadata, checksum
non-overwrite, archive verifier path safety and staging deprecation. The removed
staging script no longer synthesizes an unverified manifest: its CLI fails closed
and directs callers to verify the wheel before installing it in an isolated
environment.

Python E/F lint still covers all current source and tests. Formatting applies to
the release files touched here; pre-existing compact legacy formatting is not
silently rewritten by this merge. Rust formatting runs `rustfmt --edition 2021
--config skip_children=true --check` on the 18 touched production source files
and the migrated `core_boundary_v9` integration test listed explicitly in CI.
Production Clippy remains `--workspace --lib --bins --locked -- -D warnings`.
Intentional historical codec or read-only support uses local, explained lint
allowances on specific symbols; no global warning suppression is applied.

## Acceptance still required

The archive-probe correction was diagnosed using the unchanged ZIP from CI run
`35557556583` (source `fa4cd3128431411f639cb49355299fab34c95258`, SHA-256
`93f6bb7723493cbc6a63b1b96036604ecbdf0ee675eff5c878c65465f8241440`). The corrected
probe passed on Windows and existing WSL Linux without rebuilding binaries.
Those results are diagnostic evidence for that earlier artifact, not acceptance
of a new source SHA. Failed or timed-out probes expose bounded structured
diagnostics and cannot write a successful verification receipt.

Run PR CI against the final committed SHA to obtain real Windows/Linux wheel and
archive receipts. Local unit results do not establish cross-platform build,
database runtime, ZIP-size, AstrBot install/conversation/restart, or production
release acceptance. Do not dispatch the publishing workflow to test this PR.
