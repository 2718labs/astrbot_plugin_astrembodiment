# Emotion Personality Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore AstrEmbodiment's original hybrid emotion/personality architecture—authorized inbound LLM semantic appraisal feeding the deterministic 15D→9-region→16K×8 Native matrix and ordinary AstrBot replies—while removing every active proactive-chat/send surface and replacing it with a least-authority, read-only contact-intent projection for optional external plugins.

**Architecture:** Preserve the completed provenance custody at `e60549f` and selectively forward-port the audited v1.0.0 matrix through pure contracts, graph, dynamics, codec, atomic store, event, time and projection layers. Retain the event-driven semantic estimator, Persona Genesis Provider path and ordinary main-LLM response flow; idle/clock/sleep/matrix/intent projection remain deterministic and make zero Provider/network calls. Quarantine historical proactive data with an idempotent boundary migration, remove proactive Host/UI/Native/package entry points, then expose a nonce-bound one-relation projection whose five authority flags are always false.

**Tech Stack:** Rust 2021 workspace, FXP6 `ae-fixed`, SQLite/rusqlite immediate transactions, serde canonical wire, PyO3, Python 3, AstrBot Provider and ordinary reply hooks, pytest, Windows x64 and Linux x86_64 wheels, universal ZIP.

---

## Frozen baselines and supersession rules

- Target worktree: `G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-worktree`.
- Read-only source worktree: `G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\release-1.0.0-integration` at `710829ae5d3bef82ce818754354272517cb28056`.
- Shared ancestor: `8c4a606e63888351b3d8854e9006b89aa6623f07`.
- Approved specs: `docs/superpowers/specs/2026-08-30-emotion-matrix-forward-port-design.md` and `docs/superpowers/specs/2026-08-31-emotion-personality-core-boundary-design.md` at `8a8b5b233f34911e3af774a7f30d28b91a58c0b4`.
- Matrix provenance Task 1 is complete through `e60549f`: 20 files, 168 source/target symbol references and 43 capability rows, all currently `UNMAPPED`/release `NO_GO`. Do not repeat or rewrite that task.
- This plan supersedes `docs/superpowers/plans/2026-08-30-emotion-matrix-forward-port.md` after completed Task 1. Its Tasks 2–9 are retained below; its proactive interleaves, old Task 10 and active async-outbox direction are cancelled.
- This plan supersedes `docs/superpowers/plans/2026-08-30-proactive-settings-simplification.md` after completed Tasks 1–3. Tasks 4–5 are cancelled; Tasks 6–7 are replaced by this plan's release gates.
- Commits `3c60c06..b05d7b5` remain in history. Saved proactive configuration and runtime rows remain available for downgrade/export, but the resolver/frequency machinery leaves the active import graph, runtime digest, visible UI and release package.
- Before every task run `git status --short`; stop on unrelated changes. Never modify, reset or clean the source worktree.
- Run only the named focused RED/GREEN nodes plus the task compile command. Missing offline cache is `ENVIRONMENTAL_HARNESS_FAILURE`, never behavioral PASS.

## Final data flow and ownership

```text
authorized inbound text / explicitly authorized stimulus
  -> auxiliary LLM strict semantic appraisal (one request, bounded semantic budget)
  -> validated EvidenceVector[15] + confidence/model/source commitments
  -> deterministic Native routing/graph/FXP6 matrix/personality
  -> bounded affect/expression projection
  -> AstrBot main LLM ordinary reply to the current inbound request
  -> ordinary host delivery fact

frozen local time / sleep
  -> deterministic matrix/allostasis/intent evolution
  -> no LLM, no network, no recipient, no send

external plugin with one relation capability + projection-read consent
  -> one-use AffectContactIntentProjectionV1
  -> external plugin owns message consent/schedule/recipient/LLM/budget/send
```

## File responsibility map

- `model/emotion-matrix-provenance-v1.json`: immutable completed source custody; verification only.
- `model/emotion-matrix-capability-parity-v1.json`: 43-row release ledger; each task closes only capabilities it proves.
- `model/emotion-personality-core-supersession-v1.json`: old-plan disposition and active/retired boundary ledger.
- `crates/ae-contracts/src/emotion_matrix.rs`: evidence, route, formula, time and contact-intent projection contracts.
- `crates/ae-attention/src/emotion_matrix.rs`: pure 15D-to-nine-region routing.
- `crates/ae-neurofield/src/{graph_development,graph_replay,structural_delta}.rs`: deterministic sparse graph lifecycle.
- `crates/ae-runtime/src/{semantic,semantic_dynamics_v2,semantic_telemetry_v1,matrix_time,intent_projection}.rs`: pure matrix, local time and read-only intent calculation.
- `crates/ae-store/src/{semantic,semantic_field_attestation,core_boundary,intent_projection}.rs`: semantic single writer, historical proactive sealing and nonce audit.
- `astr_embodiment/{semantic_contract,semantic_estimator}.py`: closed semantic Provider request/response and proposal validation.
- `astr_embodiment/persona_genesis.py`: one-time authorized Persona Genesis compilation.
- `astr_embodiment/autonomy.py`: local-only `EmbodimentClock`; no relation scan or externalization callback.
- `main.py`: inbound appraisal, Genesis, ordinary reply injection and clock lifecycle only.
- `crates/ae-pyo3/src/lib.rs` and `astr_embodiment/bridge.py`: allowlisted core APIs; no proactive dispatch/recovery.
- `scripts/verify_core_package.py`: real wheel/ZIP matrix parity and prohibited active-surface scanner.

### Task 1: Record supersession without repeating provenance custody

**Files:**
- Create: `model/emotion-personality-core-supersession-v1.json`
- Create: `tests/test_core_plan_supersession.py`

- [ ] **Step 1: Write the RED supersession test**

```python
def test_old_plans_have_one_closed_disposition():
    data = json.loads(Path("model/emotion-personality-core-supersession-v1.json").read_text("utf-8"))
    assert data["matrix_provenance_task_1"] == "COMPLETED_PRESERVED"
    assert data["old_matrix_tasks_2_to_9"] == "ADAPTED_BY_THIS_PLAN"
    assert data["old_matrix_task_10"] == "CANCELLED_ACTIVE_SEND"
    assert data["old_proactive_tasks_1_to_3"] == "HISTORY_PRESERVED_RUNTIME_RETIRED"
    assert data["old_proactive_tasks_4_to_5"] == "CANCELLED_ACTIVE_SEND"
    assert data["old_proactive_tasks_6_to_7"] == "SUPERSEDED_RELEASE_GATES"
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_core_plan_supersession.py::test_old_plans_have_one_closed_disposition -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-01-red'
```

Expected: FAIL because the supersession ledger does not exist.

- [ ] **Step 3: Create the closed ledger**

The JSON includes `schema`, `approved_specs`, `preserved_commits`, `active_plan`, `parity_capability_count: 43`, the six dispositions above, and exactly these forbidden active capabilities: supervisor, recipient discovery, proactive LLM generation, proactive token budget, outbox retry, platform proactive delivery, externalization claim and dispatch claim. Do not modify either existing matrix manifest.

- [ ] **Step 4: Run GREEN and commit**

```powershell
python -m pytest -q tests/test_core_plan_supersession.py::test_old_plans_have_one_closed_disposition -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-01-green'
git add model/emotion-personality-core-supersession-v1.json tests/test_core_plan_supersession.py
git commit -m "docs: record emotion core plan supersession"
```

### Task 2: Port frozen contracts and all-fifteen routing

**Files:**
- Create: `crates/ae-contracts/src/emotion_matrix.rs`
- Modify: `crates/ae-contracts/src/lib.rs`
- Create: `crates/ae-contracts/tests/emotion_matrix_contract.rs`
- Create: `crates/ae-attention/src/emotion_matrix.rs`
- Modify: `crates/ae-attention/src/lib.rs`
- Modify: `crates/ae-attention/Cargo.toml`
- Create: `crates/ae-attention/tests/emotion_matrix_route.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add contract/route RED tests**

The contract test asserts the exact 15-field order returned by `perception_dimension_values`, range rejection, 15 frozen primary/secondary routes, coefficients 1,000,000/500,000 and the frozen route digest. The attention test uses fifteen distinct FXP6 values and asserts `evaluated_dimension_count == injected_dimension_count == 15`, all nine regions are covered, zero is a valid observed neutral value, and an out-of-range dimension returns `InvalidDimension`.

```rust
#[test]
fn all_fifteen_dimensions_route_through_one_frozen_contract() {
    let evidence = evidence_vector_from_values(std::array::from_fn(|i| Fixed::from_raw((i as i64 + 1) * 10_000)));
    assert_eq!(perception_dimension_values(&evidence)[14], Fixed::from_raw(150_000));
    assert_eq!(PHASE0_SEMANTIC_ROUTE_RULES_V1.len(), 15);
    let load = assemble_full_vector_load(&evidence).unwrap();
    assert_eq!((load.evaluated_dimension_count, load.injected_dimension_count), (15, 15));
    assert_eq!(load.route_digest, phase0_semantic_route_digest_v1());
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-02-red'
cargo test --locked --offline -p ae-contracts --test emotion_matrix_contract all_fifteen_dimensions_have_frozen_order_and_routes -- --exact
cargo test --locked --offline -p ae-attention --test emotion_matrix_route all_fifteen_dimensions_route_through_one_frozen_contract -- --exact
```

Expected: FAIL because the modules and exports do not exist.

- [ ] **Step 3: Selectively port the exact route/formula surface**

Move the source symbols `StateSubcodeV1`, Phase-0 formula constants, `Phase0SemanticRouteRuleV1`, `PHASE0_SEMANTIC_ROUTE_RULES_V1`, `phase0_semantic_route_digest_v1`, `phase0_canonical_formula_digest_v1`, `perception_dimension_values`, `evidence_vector_from_values`, semantic receipt v2, native telemetry v1 and node-observability v2 into `emotion_matrix.rs`; re-export them from `lib.rs`. Keep current alpha3 `EvidenceVector`, `SemanticEstimate`, `UserStimulus`, authority and wire definitions as the single public types. Port source `ae-attention/src/r7.rs::{FullVectorLoad,FullVectorLoadError,assemble_full_vector_load}` into the new module, importing the current public evidence type. No private R7 `EvidenceVector` becomes a second writer.

```rust
pub mod emotion_matrix;
pub use emotion_matrix::{
    evidence_vector_from_values, perception_dimension_values,
    phase0_canonical_formula_digest_v1, phase0_semantic_route_digest_v1,
    NativeTelemetryReceiptV1, NodeObservabilityProjectionWireV2,
    Phase0SemanticRouteRuleV1, SemanticVectorReceiptV2,
    PHASE0_SEMANTIC_ROUTE_RULES_V1,
};
```

- [ ] **Step 4: Mark only proven contract/route capabilities**

Set corresponding ledger entries to `EXACT_PORT` with the two exact test nodes. Keep all other entries `UNMAPPED`; do not mark a file complete merely because it compiles. Re-read but never edit the provenance manifest.

- [ ] **Step 5: Run GREEN, compile, and commit**

```powershell
cargo test --locked --offline -p ae-contracts --test emotion_matrix_contract all_fifteen_dimensions_have_frozen_order_and_routes -- --exact
cargo test --locked --offline -p ae-attention --test emotion_matrix_route all_fifteen_dimensions_route_through_one_frozen_contract -- --exact
cargo check --locked --offline -p ae-contracts -p ae-attention
git add crates/ae-contracts crates/ae-attention model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: restore frozen emotion evidence routes"
```

Expected: both named tests and check PASS; all fifteen dimensions are consumed and no Host/store code changes.

### Task 3: Restore deterministic graph development, replay, and structural CAS

**Files:**
- Create: `crates/ae-neurofield/src/graph_development.rs`
- Create: `crates/ae-neurofield/src/graph_replay.rs`
- Create: `crates/ae-neurofield/src/structural_delta.rs`
- Modify: `crates/ae-neurofield/src/lib.rs`
- Create: `crates/ae-neurofield/tests/deterministic_graph_development.rs`
- Create: `crates/ae-neurofield/tests/graph_persistence_replay.rs`
- Create: `crates/ae-neurofield/tests/structural_delta_cas.rs`
- Create: `crates/ae-neurofield/tests/vectors/graph-development-v1.json`
- Create: `crates/ae-neurofield/tests/vectors/graph-replay-v1.bin`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Port the three source tests first and run one RED node per boundary**

Copy the final source-baseline fixtures/tests, retaining these exact names: `v1_golden_vector_is_cross_process_and_cross_platform_stable`, `close_reopen_replays_to_the_authoritative_non_genesis_graph`, and `canonical_add_update_remove_is_a_digest_checked_compare_and_swap`.

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-03-red'
cargo test --locked --offline -p ae-neurofield --test deterministic_graph_development v1_golden_vector_is_cross_process_and_cross_platform_stable -- --exact
```

Expected: compile FAIL because `develop_graph`/`GraphFormula` are absent.

- [ ] **Step 2: Port final source modules without changing constants**

Port the source-baseline files byte-for-byte where their imports match. In `lib.rs`, expose the modules and retain current constants `NEURON_SLOTS=16_384`, `NODE_DOF=8`, `EDGE_CAPACITY=524_288`, nine-region layout and current initial field. `GraphFormula::V1` must produce exactly 262,144 canonical edges with source offsets sorted and edge rows canonical.

```rust
pub mod graph_development;
pub mod graph_replay;
pub mod structural_delta;
pub use graph_development::{develop_graph, GraphDevelopmentError, GraphFormula};
pub use graph_replay::{GraphReplayError, GraphReplayV1, GraphSnapshotV1};
pub use structural_delta::{apply_delta, DeltaError, EdgeOperationV1, StructuralDeltaV1};
```

- [ ] **Step 3: Run focused GREEN nodes and compile**

```powershell
cargo test --locked --offline -p ae-neurofield --test deterministic_graph_development v1_golden_vector_is_cross_process_and_cross_platform_stable -- --exact
cargo test --locked --offline -p ae-neurofield --test graph_persistence_replay close_reopen_replays_to_the_authoritative_non_genesis_graph -- --exact
cargo test --locked --offline -p ae-neurofield --test structural_delta_cas canonical_add_update_remove_is_a_digest_checked_compare_and_swap -- --exact
cargo check --locked --offline -p ae-neurofield
```

Expected: fixtures match source bytes/digests; stale delta and tampered replay do not mutate caller graph.

- [ ] **Step 4: Update exact parity dispositions and commit**

Mark graph development/replay/delta entries `EXACT_PORT` only after all three nodes pass.

```powershell
git add crates/ae-neurofield model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: restore deterministic emotion graph"
```

### Task 4: Restore FXP6 Jacobi semantic dynamics and telemetry as pure computation

**Files:**
- Create: `crates/ae-runtime/src/semantic_dynamics_v2.rs`
- Create: `crates/ae-runtime/src/semantic_telemetry_v1.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Modify: `crates/ae-runtime/Cargo.toml`
- Create: `crates/ae-runtime/tests/emotion_matrix_dynamics.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Write a RED golden dynamics test**

Use a two-edge sparse fixture plus a full 16K field. Assert immutable-before Jacobi semantics, all eight output vectors have 16,384 entries, state changes for non-zero evidence, input remains byte-identical, and energy/capacity/renormalization residuals equal the source golden values.

```rust
#[test]
fn phase0_dynamics_change_all_bounded_state_from_immutable_before() {
    let before = fixture_field();
    let before_digest = state_digest(&before, &fixture_formula());
    let prepared = propagate_semantic_dynamics_v2(DynamicsInputV2 {
        field: &before, baseline: &fixture_baseline(), graph: &fixture_graph(),
        local_by_region: [Fixed::ONE; 9],
        local_confidence_by_region: [Fixed::ONE; 9],
    }).unwrap();
    assert_eq!(state_digest(&before, &fixture_formula()), before_digest);
    assert_ne!(prepared.next_field, before);
    assert!(prepared.next_field.validate());
    assert!(prepared.energy.spent_mean > Fixed::ZERO);
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-04-red'
cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics phase0_dynamics_change_all_bounded_state_from_immutable_before -- --exact
```

Expected: compile FAIL because dynamics types/functions are absent.

- [ ] **Step 3: Port the final pure modules**

Port source `semantic_dynamics_v2.rs` and `semantic_telemetry_v1.rs`, preserving checked i128 intermediates, signed half-away rounding, `[0,1]`/`[-1,1]` clamps, before-state reads, graph validation and typed mapping to `INVALID_NEURAL_STATE`. Do not add Store, wall clock, threads or LLM calls. Expose the smallest public test surface; runtime orchestration stays private.

- [ ] **Step 4: Run focused tests and compile**

```powershell
cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics phase0_dynamics_change_all_bounded_state_from_immutable_before -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics overflow_and_invalid_shape_fail_without_partial_field -- --exact
cargo check --locked --offline -p ae-runtime
```

Expected: two tests PASS; the failure case returns a typed subcode and the before field digest is unchanged.

- [ ] **Step 5: Update parity and commit**

Mark FXP6, Jacobi, eight DOFs, energy/capacity and internal renormalization residual `EXACT_PORT` with the named tests.

```powershell
git add crates/ae-runtime model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: restore fixed point emotion dynamics"
```

### Task 5: Restore semantic prepare, AESEM3 writes, and authenticated AESEM2 decode/migration

**Files:**
- Create: `crates/ae-runtime/src/semantic.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Test: `crates/ae-runtime/src/semantic.rs` (`#[cfg(test)] mod tests`)
- Create: `crates/ae-store/src/semantic_field_attestation.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Create: `crates/ae-store/tests/emotion_matrix_migration.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add RED codec and migration fixtures**

Port the authenticated source fixtures for AESEM2, AESEM3, retired-compensation rejection and the one finite legacy overflow normalization. Add a test proving current writes start `AESEM3\0`, AESEM2 is read-only, arbitrary overflow is rejected, and failed copy/verify leaves original bytes and cursor unchanged.

```rust
#[test]
fn aesem2_is_authenticated_read_only_and_aesem3_is_current_write() {
    let fixture = frozen_aesem2_fixture();
    let legacy = fixture.bytes.clone();
    let (field, graph, _) = decode_semantic_snapshot_v2(
        &legacy, &fixture.formula_digest, &fixture.state_digest,
        &fixture.graph_digest, &fixture.legacy_receipt,
    ).unwrap();
    let current = encode_semantic_snapshot_v3(
        &fixture.formula_digest, &field, &graph, &fixture.current_telemetry,
    ).unwrap();
    assert!(current.starts_with(b"AESEM3\0"));
    assert_eq!(legacy, fixture.bytes);
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-05-red'
cargo test --locked --offline -p ae-runtime semantic::tests::aesem2_is_authenticated_read_only_and_aesem3_is_current_write -- --exact
```

Expected: compile FAIL because semantic codec functions are absent.

- [ ] **Step 3: Port semantic preparation and codecs**

Port final source symbols `PreparedSemanticTransitionV2`, `prepare_semantic_transition_v2`, `semantic_vector_receipt_v2`, `node_observability_projection_v2`, `expression_projection_from_field_v1`, field/graph canonical codecs and AESEM2/3 functions. Adapt imports to current alpha3 contracts only. Preserve `AESEM2\0` schema 2 and `AESEM3\0` schema 3 bytes and all size/count/trailing-byte guards.

- [ ] **Step 4: Port store-local attestation and finite migration**

Port `semantic_field_attestation.rs` and only the source migration types/functions needed for `LegacySemanticFormulaUpgradeReceiptV1`, `LegacySemanticFieldDomainUpgradeV1`, preimage backup manifest, copy/verify/commit and typed SQLite revision errors. Migration accepts only the authenticated finite-domain predecessor; every other mismatch returns closed typed error before write.

```rust
pub enum SemanticMigrationOutcomeV1 {
    NotRequired,
    Migrated { from_revision: u64, to_revision: u64, backup_digest: Digest },
}
```

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test --locked --offline -p ae-runtime semantic::tests::aesem2_is_authenticated_read_only_and_aesem3_is_current_write -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration finite_aesem2_migration_is_copy_verify_commit_and_idempotent -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration corrupt_or_unrelated_legacy_state_causes_zero_writes -- --exact
cargo check --locked --offline -p ae-runtime -p ae-store
git add crates/ae-runtime crates/ae-store model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: restore semantic snapshots and migration"
```

Expected: named tests/check PASS; AESEM2 bytes remain available for replay; no automatic repair of untrusted state.

### Task 6: Add persona semantic namespace and paired atomic single-writer commit

**Files:**
- Create: `crates/ae-store/src/semantic.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Create: `crates/ae-store/tests/emotion_matrix_atomic_commit.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Write RED atomicity/identity tests**

Test an immediate transaction that inserts one current journal event plus one semantic revision, snapshot, graph/receipt/telemetry and relation-private evidence binding. Fault-inject after journal insert and before semantic insert, then after semantic insert and before commit; both leave every table count/cursor/hot state unchanged. Also assert duplicate event returns original receipt, same event ID with a different digest is identity conflict, and stale semantic base writes nothing.

```rust
#[test]
fn canonical_event_and_semantic_sidecar_commit_together_or_not_at_all() {
    let mut fixture = Fixture::new();
    fixture.store.set_semantic_fault(SemanticFaultPoint::AfterJournalInsert);
    assert!(fixture.commit().is_err());
    assert_eq!(fixture.store.semantic_counts().unwrap(), (0, 0, 0, 0));
    assert_eq!(fixture.store.count_journal().unwrap(), 0);
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-06-red'
cargo test --locked --offline -p ae-store --test emotion_matrix_atomic_commit canonical_event_and_semantic_sidecar_commit_together_or_not_at_all -- --exact
```

Expected: compile FAIL because paired semantic commit API/schema does not exist.

- [ ] **Step 3: Add one closed semantic Store module and schema**

Create persona-keyed `semantic_cursor`, `semantic_snapshots`, `semantic_graphs`, `semantic_receipts`, and relation-keyed `semantic_evidence_authority`. Every row binds persona scope, semantic revision, incarnation/manifest, route/formula/graph/state/evidence/estimator digests and main event digest. Do not reuse alpha3 relation contact revision or autonomy revision.

```rust
pub struct PairedSemanticCommitV1 {
    pub journal: CommitEnvelope,
    pub persona_scope: Digest,
    pub relation_scope: Option<Digest>,
    pub semantic_base_revision: u64,
    pub event_id: Id128,
    pub event_digest: Digest,
    pub evidence_digest: Digest,
    pub estimator_digest: Digest,
    pub snapshot_bytes: Vec<u8>,
    pub graph_digest: Digest,
    pub receipt_bytes: Vec<u8>,
    pub telemetry_bytes: Vec<u8>,
}
```

- [ ] **Step 4: Implement a single `BEGIN IMMEDIATE` commit path**

Refactor existing journal insertion into a transaction-local helper used by both `commit_journal` and `commit_event_with_semantic_v1`. Validate all bytes/digests/identity before first INSERT; recheck journal and semantic CAS in the same transaction; insert all rows; commit; only then return the persisted row. Test-only fault points live under `#[cfg(test)]` and never ship as ABI.

The test-only fault enum is closed and contains exactly `AfterJournalInsert` and `AfterSemanticInsert`; `set_semantic_fault(None)` clears it. Production builds contain neither enum nor branch.

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test --locked --offline -p ae-store --test emotion_matrix_atomic_commit canonical_event_and_semantic_sidecar_commit_together_or_not_at_all -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_atomic_commit duplicate_stale_and_identity_conflict_never_split_cursors -- --exact
cargo check --locked --offline -p ae-store
git add crates/ae-store model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: commit emotion state with canonical events"
```

Expected: tests/check PASS; SQLite uses one writer and one immediate transaction; existing no-semantic `DeliveryOutcome` commit remains valid.

### Task 7: Replace UserStimulus no-op with authority-checked semantic evolution

**Files:**
- Modify: `crates/ae-runtime/src/lib.rs`
- Create: `crates/ae-runtime/tests/emotion_matrix_event_lane.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Write RED event-lane tests**

Given authenticated non-zero 15D evidence, assert `state_after != state_before`, semantic revision 0→1, all commitments agree and reopen restores identical bytes. Explicit valid all-zero evidence with non-zero estimator confidence commits as an observed neutral proposal. Zero confidence/zero estimator digest, missing/invalid dimension, wrong `SourceAuthority`, relation/persona scope, stale base, request nonce/incarnation mismatch and estimator digest mismatch all produce zero semantic writes. `DeliveryOutcome` never manufactures evidence.

```rust
#[test]
fn authenticated_user_stimulus_changes_and_reopens_persona_emotion_state() {
    let mut fixture = Fixture::new();
    let decision = fixture.runtime.apply_event(&fixture.scope, &fixture.nonzero_stimulus()).unwrap();
    assert_ne!(decision.receipt.state_before, decision.receipt.state_after);
    drop(fixture.runtime);
    let reopened = fixture.reopen();
    assert_eq!(reopened.semantic_revision_v1(&fixture.scope).unwrap(), 1);
    assert_eq!(reopened.inspect(&fixture.scope).unwrap().state_digest, decision.receipt.state_after);
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-07-red'
cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane authenticated_user_stimulus_changes_and_reopens_persona_emotion_state -- --exact
```

Expected: FAIL because current `apply_event` sets `state_after == state_before`.

- [ ] **Step 3: Adapt current authority to the pure proposal path**

In `AstrRuntime::apply_event`, preserve current event support, `SourceAuthority` projection, persona hot binding, event dedupe and causal checks. For `UserStimulus`, require schema 1, all 15 dimensions in `[0,1]`, confidence in `(0,1]`, non-zero estimator digest, matching event scope/persona/relation/incarnation and a digest bound to canonical evidence bytes, event/request nonce and estimator identity. Assemble route load, prepare dynamics/telemetry/AESEM3, then call only `commit_event_with_semantic_v1`. For `DeliveryOutcome`, retain the current journal behavior and unchanged field.

```rust
match event {
    CanonicalEvent::UserStimulus(stimulus) => self.prepare_and_commit_semantic_v1(scope, stimulus),
    CanonicalEvent::DeliveryOutcome(_) => self.commit_non_semantic_event(scope, event),
    _ => Err(RuntimeError::UnsupportedEvent(wire::event_kind_name(event))),
}
```

Split the hot cursor explicitly: `canonical_revision` tracks the main journal and `semantic_revision` tracks the persona matrix. Do not replace in-memory field/graph/either revision until Store commit succeeds. On reopen, hydrate only an attested latest semantic snapshot whose identity/formula/graph/state chain closes; alpha3 no-op history starts semantic revision 0 from Genesis and is not rewritten.

- [ ] **Step 4: Prove relation-private input and persona-global state separation**

Apply two relations to one persona: both advance the same persona semantic cursor, but their evidence authority rows remain independently keyed and neither can dedupe/settle the other. Apply another persona and prove its field/cursor do not change.

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane authenticated_user_stimulus_changes_and_reopens_persona_emotion_state -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane relation_evidence_is_private_while_persona_mood_is_continuous -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane invalid_evidence_and_commit_fault_leave_hot_and_store_unchanged -- --exact
cargo check --locked --offline -p ae-runtime
git add crates/ae-runtime model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: evolve emotion matrix from trusted stimuli"
```

Expected: tests/check PASS; event+semantic commit is paired; no source worktree changes.

### Task 8: Add deterministic sleep-aware matrix time advancement

**Files:**
- Modify: `crates/ae-contracts/src/emotion_matrix.rs`
- Create: `crates/ae-runtime/src/matrix_time.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs`
- Modify: `crates/ae-store/src/semantic.rs`
- Modify: `crates/ae-store/src/alpha3/ecosystem.rs`
- Create: `crates/ae-runtime/tests/emotion_matrix_time.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Write RED time invariants**

Test frozen durations at awake/drowsy/asleep, chunk equivalence, zero-duration no-op, maximum-step bound, backward/future time rejection, deterministic baseline relaxation/adaptation release/reserve recovery, unchanged graph and no semantic evidence receipt. Asleep must relax/recover faster than awake under frozen constants.

Also freeze the existing dream-residue and endogenous-intent rules: a local time step may form/decay bounded internal intent and update versioned dream residue, but neither result contains a recipient, message, Provider request or send authority.

```rust
#[test]
fn sleep_time_advance_relaxes_without_inventing_evidence() {
    let asleep = advance_matrix_time_v1(&before(), 3_600_000, MatrixSleepPhaseV1::Asleep).unwrap();
    let awake = advance_matrix_time_v1(&before(), 3_600_000, MatrixSleepPhaseV1::Awake).unwrap();
    assert!(distance_to_baseline(&asleep.field) < distance_to_baseline(&awake.field));
    assert_eq!(asleep.graph_digest, awake.graph_digest);
    assert!(asleep.evidence_digest.is_none());
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-08-red'
cargo test --locked --offline -p ae-runtime --test emotion_matrix_time sleep_time_advance_relaxes_without_inventing_evidence -- --exact
```

Expected: compile FAIL because `MatrixTimeAdvanceV1` is absent.

- [ ] **Step 3: Define the independent formula contract and pure function**

```rust
pub const MATRIX_TIME_FORMULA_V1: &str = "matrix-time-advance-fxp6-v1";

pub enum MatrixSleepPhaseV1 { Awake, Drowsy, Asleep }

pub struct MatrixTimeAdvanceV1 {
    pub elapsed_ms: u64,
    pub sleep_phase: MatrixSleepPhaseV1,
    pub prior_semantic_revision: u64,
    pub prior_state_digest: Digest,
    pub formula_digest: Digest,
}
```

Use checked integer exponentiation-by-squaring over bounded fixed-step coefficients so a single 60-minute step equals six 10-minute steps at the frozen granularity. Only potential/excitation/inhibition/adaptation/precision/prediction-error/eligibility relax toward Genesis baseline and metabolic reserve toward one; graph, route and identity are immutable.

- [ ] **Step 4: Pair with the current wake settlement**

Prepare time advancement from the committed `TimeAdvanceV1` frozen timestamp/sleep state. Commit autonomous wake state plus semantic time sidecar in one immediate transaction or neither. Emergency wake changes only subsequent phase selection; it does not skip matrix CAS or backdate state.

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test --locked --offline -p ae-runtime --test emotion_matrix_time sleep_time_advance_relaxes_without_inventing_evidence -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_time frozen_chunking_is_deterministic_and_backward_time_fails_closed -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_time dream_residue_and_endogenous_intent_remain_local_and_deterministic -- --exact
cargo check --locked --offline -p ae-runtime -p ae-store
git add crates/ae-contracts crates/ae-runtime crates/ae-store model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: advance emotion state through sleep time"
```

### Task 9: Publish committed affect/personality observer projection and keep renorm read-only

**Files:**
- Modify: `crates/ae-contracts/src/alpha3.rs`
- Modify: `crates/ae-store/src/alpha3/projection.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs`
- Create: `crates/ae-runtime/tests/emotion_matrix_projection.rs`
- Modify: `crates/ae-runtime/tests/alpha3_projection.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Write RED privacy/renorm/projection tests**

Assert Experience exposes only persona-level region means/deltas, confidence, revision, state/formula digests and coarse trend; serialized projection contains no relation/session/user token, event ID, raw evidence, source text or node vector. Private projection retains existing relation consent/contact/budget/cause only. Developer projection may include typed telemetry/counts but no private evidence. Corrupt/uncommitted state and renorm residual above threshold return `Inconsistent`/`matrix_unavailable` without replacing committed state.

```rust
#[test]
fn committed_affect_projection_is_persona_global_and_non_identifying() {
    let projection = fixture.experience_projection();
    let json = serde_json::to_string(&projection).unwrap();
    assert!(json.contains("semantic_revision"));
    for forbidden in [fixture.relation_hex(), fixture.session_hex(), fixture.event_hex(), "dimensions"] {
        assert!(!json.contains(&forbidden));
    }
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-09-red'
cargo test --locked --offline -p ae-runtime --test emotion_matrix_projection committed_affect_projection_is_persona_global_and_non_identifying -- --exact
```

Expected: compile FAIL because affect projection is absent.

- [ ] **Step 3: Add a closed minimal projection contract**

```rust
pub enum AffectTrendV1 { Falling, Stable, Rising }

pub struct AffectProjectionV1 {
    pub semantic_revision: u64,
    pub personality_revision: u64,
    pub state_digest: Digest,
    pub formula_digest: Digest,
    pub confidence_fxp6: u32,
    pub region_mean_fxp6: [i64; 9],
    pub region_delta_fxp6: [i64; 9],
    pub trend: AffectTrendV1,
}
```

Add `affect: ProjectionFieldV1<AffectProjectionV1>` to `ExperienceProjectionV2` with a serde default of `NotInitialized` for old payloads. Add typed semantic health/telemetry to Developer only. Do not add affect/evidence to `PrivateProjectionV2`.

- [ ] **Step 4: Derive only from committed state and use `ae-renorm` downstream**

Load latest attested semantic snapshot and receipt in one read transaction. Build region means/deltas and source `node_observability_projection_v2`; pass the committed field to `ae_renorm::restrict`. Keep semantic `renormalization_residual` and alpha3 renorm pyramid under distinct names/domains. Residual failure suppresses downstream affect authority but does not roll back or mutate semantic journal.

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test --locked --offline -p ae-runtime --test emotion_matrix_projection committed_affect_projection_is_persona_global_and_non_identifying -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_projection uncommitted_corrupt_or_renorm_failed_projection_is_unavailable -- --exact
cargo test --locked --offline -p ae-runtime --test alpha3_projection projection_layers_remain_relation_private -- --exact
cargo check --locked --offline -p ae-contracts -p ae-store -p ae-runtime
git add crates/ae-contracts crates/ae-store crates/ae-runtime model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: expose committed affect projection"
```

Expected: tests/check PASS; projection is a bounded read model, not a second writer.


### Task 10: Restore authorized semantic LLM appraisal, Persona Genesis and ordinary replies

> **Task 10B authority:** [2026-09-05 semantic-appraisal retention/capacity addendum](2026-09-05-task10b-semantic-appraisal-retention-capacity-addendum.md).

**Files:**
- Create: `astr_embodiment/semantic_contract.py`
- Create: `astr_embodiment/semantic_estimator.py`
- Modify: `astr_embodiment/persona_genesis.py`
- Modify: `astr_embodiment/coordinator.py`
- Modify: `main.py`
- Modify: `_conf_schema.json`
- Create: `tests/test_semantic_appraisal_core.py`
- Create: `tests/test_persona_genesis_compatibility.py`
- Create: `tests/test_ordinary_reply_flow.py`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add RED hybrid-architecture tests**

```python
@pytest.mark.asyncio
async def test_authorized_inbound_appraisal_calls_provider_once_then_commits_closed_15d(fake_host, plugin):
    await plugin.on_llm_request(fake_host.authorized_inbound_event("I missed you"), fake_host.request)
    assert fake_host.semantic_provider_calls == 1
    assert fake_host.proactive_provider_calls == 0
    assert fake_host.native_proposals[0].keys() == VALID_PROPOSAL_KEYS
    assert len(fake_host.native_proposals[0]["dimensions"]) == 15

@pytest.mark.asyncio
async def test_ordinary_reply_uses_main_llm_path_without_becoming_proactive(fake_host, plugin):
    await plugin.on_llm_request(fake_host.authorized_inbound_event("hello"), fake_host.request)
    assert fake_host.ordinary_reply_requests == 1
    assert fake_host.send_calls == 0
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_runtime_integration.py::test_semantic_inbound_calls_provider_once_and_submits_exact_six_key_proposal tests/test_persona_genesis_compatibility.py::test_genesis_provider_output_still_requires_closed_validation_and_native_authority tests/test_ordinary_reply_flow.py::test_ordinary_reply_uses_main_llm_path_without_becoming_proactive -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-10-red'
```

Expected: FAIL because the target lacks the complete v1.0.0 semantic estimator integration.

- [ ] **Step 3: Port the closed estimator and separate Provider purposes**

Port source `semantic_contract.py` and final `semantic_estimator.py` symbols `SemanticEstimateV3`, `parse_estimator_output_v3`, `make_request_nonce_digest`, `validate_perception_proposal`, `build_perception_proposal_v3`, `build_contextual_estimator_request`, and `estimate_context_bound`. The sole Provider call accepts the frozen current authorized turn and returns exactly 15 closed state/intensity/confidence slots. Bind model/source digests, scope, event, turn, semantic base revision and nonce. Never persist prompts, Provider transcript or hidden reasoning.

```python
async def _semantic_estimate_v3(self, *, scope, turn, event_digest, observed_at_utc_ms):
    estimate = await estimate_context_bound(
        self._semantic_provider(), scope, turn,
        semantic_base_revision=self._native_revision(scope),
        observed_at_utc_ms=observed_at_utc_ms,
    )
    return build_perception_proposal_v3(
        scope=scope, turn=turn, estimate=estimate, event_digest=event_digest,
        observed_at_utc_ms=observed_at_utc_ms,
    )
```

- [ ] **Step 4: Retain Genesis and ordinary request/response behavior**

Keep `compile_with_provider`/`compile_with_current_chat_model`, `validate_proposal` and Native Genesis authority. Keep `_llm_generate` reachable only from authorized appraisal/Genesis and AstrBot's ordinary inbound reply lifecycle; no intent/clock path may call it. Preserve bounded affect/personality injection and ordinary `on_llm_response`/`after_message_sent` delivery facts. Expose `semantic_estimator_provider_id`, `semantic_appraisal_token_daily_max` and typed semantic telemetry; preserve a prior mixed Provider value as migration input without making it proactive.

- [ ] **Step 5: Run GREEN, compile, close only Host/semantic rows, and commit**

```powershell
python -m pytest -q tests/test_runtime_integration.py::test_semantic_inbound_calls_provider_once_and_submits_exact_six_key_proposal tests/test_runtime_integration.py::test_semantic_budget_exhaustion_skips_provider_and_keeps_normal_reply_open tests/test_persona_genesis_compatibility.py::test_genesis_provider_output_still_requires_closed_validation_and_native_authority tests/test_ordinary_reply_flow.py::test_ordinary_reply_uses_main_llm_path_without_becoming_proactive -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-10-green'
python -m py_compile main.py astr_embodiment/semantic_contract.py astr_embodiment/semantic_estimator.py astr_embodiment/persona_genesis.py astr_embodiment/coordinator.py
git add main.py _conf_schema.json astr_embodiment tests/test_semantic_appraisal_core.py tests/test_persona_genesis_compatibility.py tests/test_ordinary_reply_flow.py model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: restore semantic appraisal and ordinary replies"
```

> **Task 11/12 authority:** [2026-09-05 core-boundary and EmbodimentClock addendum](2026-09-05-task11-12-core-boundary-embodiment-clock.md). It supersedes the Task 11/12 details below and cancels the later active contact-intent projection direction.

### Task 11: Replace the proactive supervisor with local-only EmbodimentClock

**Files:**
- Modify: `astr_embodiment/autonomy.py`
- Modify: `main.py`
- Create: `tests/test_embodiment_clock.py`
- Modify: `tests/test_plugin_lifecycle.py`

- [ ] **Step 1: Write RED clock and zero-side-effect tests**

```python
@pytest.mark.asyncio
async def test_clock_advances_one_persona_without_relations_or_externalization(fake_bridge):
    clock = EmbodimentClock(fake_bridge, persona_scope=PERSONA, frozen_time_source=TIME)
    await clock.tick_once()
    assert fake_bridge.calls == [("advance_embodiment_time_v1", PERSONA)]
    assert fake_bridge.relation_queries == []
    assert fake_bridge.provider_calls == fake_bridge.send_calls == 0
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_embodiment_clock.py::test_clock_advances_one_persona_without_relations_or_externalization tests/test_plugin_lifecycle.py::test_initialize_starts_clock_not_proactive_supervisor -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-11-red'
```

Expected: FAIL because `AutonomousSupervisor` still scans pending autonomy work.

- [ ] **Step 3: Implement the local clock and remove active Host wiring**

```python
class EmbodimentClock:
    def __init__(self, bridge, *, persona_scope, frozen_time_source):
        self._bridge = bridge
        self._persona_scope = dict(persona_scope)
        self._frozen_time_source = frozen_time_source

    async def tick_once(self) -> dict[str, object]:
        frozen = self._frozen_time_source.freeze(self._persona_scope)
        return self._bridge.advance_embodiment_time_v1(self._persona_scope, frozen)
```

Remove `on_externalization`, relation enumeration, pending/recovery loops and Provider/send callbacks. In `main.py`, remove `AutonomousSupervisor`, `_on_autonomous_externalization`, proactive imports/state/config preparation and lifecycle. Start/stop only `EmbodimentClock`. `ae_wake` remains a body wake; stop registering `ae_contact_pause` and `ae_contact_end`.

- [ ] **Step 4: Run GREEN and commit**

```powershell
python -m pytest -q tests/test_embodiment_clock.py::test_clock_advances_one_persona_without_relations_or_externalization tests/test_embodiment_clock.py::test_idle_sleep_intent_ticks_make_zero_provider_network_token_or_send_calls tests/test_plugin_lifecycle.py::test_initialize_starts_clock_not_proactive_supervisor -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-11-green'
python -m py_compile main.py astr_embodiment/autonomy.py
git add main.py astr_embodiment/autonomy.py tests/test_embodiment_clock.py tests/test_plugin_lifecycle.py
git commit -m "refactor: reduce autonomy to embodiment clock"
```

### Task 12: Seal historical proactive state and retire active Host, UI and Native surfaces

**Files:**
- Create: `crates/ae-store/src/core_boundary.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `astr_embodiment/bridge.py`
- Delete: `astr_embodiment/proactive.py`
- Modify: `_conf_schema.json`
- Create: `crates/ae-store/tests/core_boundary_upgrade.rs`
- Create: `crates/ae-pyo3/tests/core_boundary_exports.rs`
- Create: `tests/test_core_boundary_surface.py`

- [ ] **Step 1: Add RED migration and negative-surface tests**

```rust
#[test]
fn upgrade_preserves_rows_but_makes_every_old_work_item_non_executable() {
    let mut store = legacy_fixture_with_all_work_states();
    let before = store.historical_digest().unwrap();
    let receipt = store.apply_core_boundary_upgrade_v1().unwrap();
    assert_eq!(receipt.source_digest, before);
    assert!(store.pending_executable_autonomy().unwrap().is_empty());
    assert_eq!(store.dispatch_unknown_count().unwrap(), 1);
}
```

```python
def test_core_has_no_proactive_bridge_or_visible_settings(plugin_archive_tree):
    assert not hasattr(NativeBridge, "pending_autonomy_work")
    assert "astr_embodiment/proactive.py" not in plugin_archive_tree
    assert PROACTIVE_UI_KEYS.isdisjoint(json.loads(Path("_conf_schema.json").read_text())["properties"])
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-core-cargo\task-12-red'
cargo test --locked --offline -p ae-store --test core_boundary_upgrade upgrade_preserves_rows_but_makes_every_old_work_item_non_executable -- --exact
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_core_boundary_surface.py::test_core_has_no_proactive_bridge_or_visible_settings -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-12-red'
```

Expected: FAIL because active proactive methods, module and UI keys remain.

- [ ] **Step 3: Add idempotent historical sealing**

Add hidden `core_boundary_revision=1`. In one immediate transaction: Ready/Deferred/Externalizing become `Suppressed(core_boundary_upgrade)`; unstarted outbox becomes `Terminal(core_boundary_upgrade)`; started/unproven calls remain `DispatchUnknown`; claims expire; reservations follow the prior settlement contract or freeze. Preserve every historical row and old config byte. Failure sets `externalization_disabled=true` and never starts recovery.

```rust
pub struct CoreBoundaryUpgradeReceiptV1 {
    pub source_digest: Digest,
    pub target_digest: Digest,
    pub suppressed_intentions: u64,
    pub terminal_outbox: u64,
    pub dispatch_unknown: u64,
}
```

- [ ] **Step 4: Remove active entry points, not historical readers**

Delete bridge/PyO3/allowlist operations for autonomy recovery/pending, externalization/dispatch gate-and-claim/settle and proactive Provider/token settlement. Old `alpha3_call` names return `UNSUPPORTED_CORE_BOUNDARY`. Remove proactive UI keys, resolver import and active digest contribution; preserve stored values as `legacy_preserved_ignored`. Keep semantic Provider/budget/telemetry. Delete `astr_embodiment/proactive.py` from source/package.

- [ ] **Step 5: Run GREEN, compile, and commit**

```powershell
cargo test --locked --offline -p ae-store --test core_boundary_upgrade upgrade_preserves_rows_but_makes_every_old_work_item_non_executable -- --exact
cargo test --locked --offline -p ae-store --test core_boundary_upgrade upgrade_is_idempotent_and_failure_never_reenables_externalization -- --exact
cargo test --locked --offline -p ae-pyo3 --test core_boundary_exports old_proactive_operations_are_absent_or_unsupported -- --exact
python -m pytest -q tests/test_core_boundary_surface.py::test_core_has_no_proactive_bridge_or_visible_settings tests/test_core_boundary_surface.py::test_legacy_config_is_preserved_but_excluded_from_active_digest -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-12-green'
cargo check --locked --offline -p ae-store -p ae-runtime -p ae-pyo3
python -m py_compile main.py astr_embodiment/bridge.py
git add crates/ae-store crates/ae-runtime crates/ae-pyo3 astr_embodiment/bridge.py _conf_schema.json tests/test_core_boundary_surface.py
git rm astr_embodiment/proactive.py
git commit -m "refactor: retire proactive delivery from core"
```

### Task 13: Add one-use least-authority contact-intent projection

**Files:**
- Modify: `crates/ae-contracts/src/emotion_matrix.rs`
- Create: `crates/ae-runtime/src/intent_projection.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Create: `crates/ae-store/src/intent_projection.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Create: `crates/ae-runtime/tests/affect_contact_intent_projection.rs`
- Create: `crates/ae-store/tests/intent_projection_nonce.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Write RED contract, privacy and replay tests**

```rust
#[test]
fn projection_is_one_relation_read_only_and_has_no_action_authority() {
    let before = fixture.state_revisions();
    let value = fixture.project(valid_request()).unwrap();
    assert_eq!(value.intent_state, IntentStateV1::Present);
    assert!(!value.send_authority && !value.recipient_authority);
    assert!(!value.schedule_authority && !value.generation_authority && !value.delivery_authority);
    assert_eq!(fixture.state_revisions(), before);
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-core-cargo\task-13-red'
cargo test --locked --offline -p ae-runtime --test affect_contact_intent_projection projection_is_one_relation_read_only_and_has_no_action_authority -- --exact
```

Expected: compile FAIL because request/response contracts are absent.

- [ ] **Step 3: Define the exact closed request and response**

`AffectContactIntentProjectionRequestV1` contains exactly schema version, caller digest, grant ID/revision, persona scope, one relation scope, purpose `contact_consideration`, nonce, frozen observed time, `max_age_ms` in 1..300000, and consent epoch/revision. `AffectContactIntentProjectionV1` contains projection ID, the same scopes/nonce, issue/expiry, matrix/personality/relationship/sleep revisions, state/evidence commitments, closed intent state, seven FXP6 axes, closed inhibition reasons, projection digest and these constants:

```rust
pub const SEND_AUTHORITY: bool = false;
pub const RECIPIENT_AUTHORITY: bool = false;
pub const SCHEDULE_AUTHORITY: bool = false;
pub const GENERATION_AUTHORITY: bool = false;
pub const DELIVERY_AUTHORITY: bool = false;
```

The seven axes are approach, care, unresolved tension, novelty seeking, withdrawal, urgency and confidence. No raw evidence, nodes, text, prompt, Provider, platform identity, relation list or recipient choice is serializable.

- [ ] **Step 4: Implement a read-only snapshot plus independent nonce receipt**

Validate caller/persona/relation/purpose/capability and projection-read consent separately from message consent. Bind request digest, four state revisions, state/evidence commitments and frozen time under `ae.affect-contact-intent-projection.v1`. Atomically consume nonce in an audit table whose revision is outside matrix/personality/relation/sleep. Reuse, revocation, stale consent, scope mismatch, rollback, corrupt state or expiry returns a non-enumerating closed error/unavailable result.

- [ ] **Step 5: Run GREEN, close intent rows, and commit**

```powershell
cargo test --locked --offline -p ae-runtime --test affect_contact_intent_projection projection_is_one_relation_read_only_and_has_no_action_authority -- --exact
cargo test --locked --offline -p ae-runtime --test affect_contact_intent_projection invalid_capability_is_non_enumerating_unavailable -- --exact
cargo test --locked --offline -p ae-store --test intent_projection_nonce nonce_is_one_use_without_advancing_embodiment_state -- --exact
cargo check --locked --offline -p ae-contracts -p ae-runtime -p ae-store
git add crates/ae-contracts crates/ae-runtime crates/ae-store model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: expose least authority contact intent"
```

### Task 14: Expose only core Host APIs and a read-only consumer boundary

**Files:**
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `crates/ae-pyo3/Cargo.toml`
- Modify: `astr_embodiment/bridge.py`
- Modify: `astr_embodiment/coordinator.py`
- Create: `tests/test_emotion_core_host.py`
- Create: `tests/test_intent_projection_consumer.py`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add RED allowlist and consumer tests**

```python
def test_host_surface_is_core_only(native_bridge):
    allowed = set(native_bridge.contract_info()["methods"])
    assert {"apply_perception_proposal_v1", "semantic_revision_v1",
            "project_affect_contact_intent_v1", "advance_embodiment_time_v1"} <= allowed
    assert allowed.isdisjoint(PROACTIVE_METHODS)

def test_projection_consumer_cannot_query_all_relations(native_bridge):
    assert not hasattr(native_bridge, "list_contact_intents")
    assert not hasattr(native_bridge, "next_contact_recipient")
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_emotion_core_host.py::test_host_surface_is_core_only tests/test_intent_projection_consumer.py::test_projection_consumer_cannot_query_all_relations -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-14-red'
```

Expected: FAIL because final core-only ABI and projection method are not registered.

- [ ] **Step 3: Register exact bounded APIs**

Expose Genesis create/read/verify, `apply_perception_proposal_v1`, `semantic_revision_v1`, committed Experience/Developer projection, `advance_embodiment_time_v1`, `project_affect_contact_intent_v1`, `flush_and_close`, integrity check and boundary-migration status. Reject unknown/oversized fields, raw text in proposal/projection requests and old proactive operation names. The coordinator exposes only a caller-supplied single-relation request; no polling loop, relation list or callback.

```python
def project_affect_contact_intent_v1(self, request: dict[str, Any]) -> dict[str, Any]:
    return _parse_payload(self._require().project_affect_contact_intent_v1(_canonical_json(request)))
```

- [ ] **Step 4: Run GREEN, close Host ABI rows, and commit**

```powershell
python -m pytest -q tests/test_emotion_core_host.py::test_host_surface_is_core_only tests/test_emotion_core_host.py::test_closed_proposal_and_projection_payloads_leak_no_private_material tests/test_intent_projection_consumer.py::test_projection_consumer_cannot_query_all_relations -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-14-green'
python -m py_compile astr_embodiment/bridge.py astr_embodiment/coordinator.py
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-core-cargo\task-14-green'
cargo check --locked --offline -p ae-pyo3
git add crates/ae-pyo3 astr_embodiment/bridge.py astr_embodiment/coordinator.py tests/test_emotion_core_host.py tests/test_intent_projection_consumer.py model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: expose emotion personality core bridge"
```

### Task 15: Version, document and build dual-platform core-only packages

**Files:**
- Create: `scripts/verify_core_package.py`
- Create: `tests/test_core_package_parity.py`
- Modify: `scripts/package_plugin.py`
- Modify: `tests/test_release_contracts.py`
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `pyproject.toml`
- Modify: `uv.lock`
- Modify: `metadata.yaml`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Verify: `model/emotion-matrix-provenance-v1.json`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add RED version/package tests**

```python
REQUIRED_DOMAINS = {
    b"ae.phase0.semantic-route-rules.v1", b"phase0-native-propagation-fxp6-v1",
    b"matrix-time-advance-fxp6-v1", b"ae.affect-contact-intent-projection.v1",
    b"AESEM2\0", b"AESEM3\0", b"astr-embodiment.node-observability.v2",
}
FORBIDDEN_MEMBERS = {"astr_embodiment/proactive.py"}
FORBIDDEN_METHODS = {"pending_autonomy_work", "recover_autonomy",
    "gate_and_claim_externalization_v2", "gate_and_claim_dispatch_v2"}
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_release_contracts.py::test_release_versions_and_required_files_are_present tests/test_core_package_parity.py::test_dual_platform_packages_are_matrix_complete_and_proactive_free -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-15-red'
```

Expected: version test FAIL at the previous version and artifact test FAIL with explicit missing fresh artifacts.

- [ ] **Step 3: Set the next version once and document truthful scope**

Set the authoritative release version to Rust/native/metadata `1.1.0-alpha4` and Python/wheel `1.1.0a4`. Preserve protocol identifiers and historical changelog text. Document matrix/personality, semantic appraisal Provider/budget/telemetry, Genesis, ordinary replies, deterministic sleep/time, external intent projection, preserved legacy state and absence of active proactive send.

- [ ] **Step 4: Verify positive and negative package contracts**

`verify_core_package.py` opens both wheels and the ZIP, verifies member hashes/manifests, imports each build in its platform job, compares canonical `contract_info`, requires `PyInit__native` and all required domains/methods, and rejects forbidden members/methods/UI keys. Its call-graph receipt proves `llm_generate`/`get_provider_by_id` are reachable only from appraisal/Genesis/ordinary inbound flow and `send_message` is absent.

- [ ] **Step 5: Close all 43 parity rows only with named evidence and commit**

Every row becomes `EXACT_PORT`, `ADAPTED_WITH_EQUIVALENCE` or `EXPLICITLY_SUPERSEDED`; only the alpha3 no-op lane and active proactive/outbox transport may be superseded, citing both approved specs and replacement/negative tests. Release mode rejects open status, empty test evidence, missing artifact hash or unapproved supersession.

```powershell
python -m pytest -q tests/test_release_contracts.py::test_release_versions_and_required_files_are_present tests/test_emotion_matrix_provenance.py::test_release_parity_has_no_unmapped_unknown_or_empty_evidence -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\task-15-green'
python -m py_compile scripts/package_plugin.py scripts/verify_core_package.py
git add README.md CHANGELOG.md Cargo.toml Cargo.lock pyproject.toml uv.lock metadata.yaml .github/workflows scripts tests/test_release_contracts.py tests/test_core_package_parity.py model/emotion-matrix-capability-parity-v1.json
git commit -m "chore: prepare emotion personality core release"
```

Expected: source version/parity tests PASS; package parity remains unearned until fresh artifacts exist.

### Task 16: Run bounded acceptance and retain NO-GO until every gate is evidenced

**Files:**
- Verify only; do not create an empty commit.

- [ ] **Step 1: Collect exact Python nodes**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
$pytestCache='G:\AstrEmbodiment\.codex-task-temp\emotion-core-pytest\final'
python -m pytest --collect-only -q tests/test_emotion_matrix_provenance.py::test_release_parity_has_no_unmapped_unknown_or_empty_evidence tests/test_runtime_integration.py::test_semantic_inbound_calls_provider_once_and_submits_exact_six_key_proposal tests/test_embodiment_clock.py::test_idle_sleep_intent_ticks_make_zero_provider_network_token_or_send_calls tests/test_core_boundary_surface.py::test_core_has_no_proactive_bridge_or_visible_settings tests/test_emotion_core_host.py::test_host_surface_is_core_only tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir=$pytestCache
```

Expected: six nodes collect with exit 0. Collection failure is not a test result.

- [ ] **Step 2: Run minimum Python/compile gates**

```powershell
python -m pytest -q tests/test_emotion_matrix_provenance.py::test_release_parity_has_no_unmapped_unknown_or_empty_evidence tests/test_runtime_integration.py::test_semantic_inbound_calls_provider_once_and_submits_exact_six_key_proposal tests/test_runtime_integration.py::test_semantic_budget_exhaustion_skips_provider_and_keeps_normal_reply_open tests/test_embodiment_clock.py::test_idle_sleep_intent_ticks_make_zero_provider_network_token_or_send_calls tests/test_core_boundary_surface.py::test_core_has_no_proactive_bridge_or_visible_settings tests/test_emotion_core_host.py::test_host_surface_is_core_only tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir=$pytestCache
python -m py_compile main.py astr_embodiment/bridge.py astr_embodiment/coordinator.py astr_embodiment/semantic_contract.py astr_embodiment/semantic_estimator.py astr_embodiment/persona_genesis.py astr_embodiment/autonomy.py scripts/package_plugin.py scripts/verify_core_package.py
```

- [ ] **Step 3: Run one high-value Rust node per boundary and workspace compile**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-core-cargo\final'
cargo fmt --all -- --check
cargo test --locked --offline -p ae-neurofield --test deterministic_graph_development v1_golden_vector_is_cross_process_and_cross_platform_stable -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics phase0_dynamics_change_all_bounded_state_from_immutable_before -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_atomic_commit canonical_event_and_semantic_sidecar_commit_together_or_not_at_all -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane authenticated_user_stimulus_changes_and_reopens_persona_emotion_state -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_time sleep_time_advance_relaxes_without_inventing_evidence -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_projection committed_affect_projection_is_persona_global_and_non_identifying -- --exact
cargo test --locked --offline -p ae-store --test core_boundary_upgrade upgrade_preserves_rows_but_makes_every_old_work_item_non_executable -- --exact
cargo test --locked --offline -p ae-runtime --test affect_contact_intent_projection projection_is_one_relation_read_only_and_has_no_action_authority -- --exact
cargo check --locked --offline --workspace
```

Expected: format, eight behavior nodes and workspace check PASS.

- [ ] **Step 4: Verify custody, scope and absence of active send**

```powershell
$src='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\release-1.0.0-integration'
git -C $src status --short
git status --short
git diff 8a8b5b233f34911e3af774a7f30d28b91a58c0b4..HEAD --check
(git log --format='%H' -- model/emotion-matrix-provenance-v1.json).Count
Select-String -LiteralPath 'model\emotion-matrix-capability-parity-v1.json' -Pattern 'UNMAPPED|UNKNOWN'
Select-String -LiteralPath 'main.py','astr_embodiment\bridge.py','crates\ae-pyo3\src\lib.rs' -Pattern 'pending_autonomy_work|recover_autonomy|gate_and_claim_externalization|gate_and_claim_dispatch|execute_proactive_intention|submit_proactive_message'
Test-Path -LiteralPath 'astr_embodiment\proactive.py'
```

Expected: source/target clean; provenance has one creation commit; ledger/symbol scans have no matches; proactive module path is false.

- [ ] **Step 5: Build and verify fresh dual-platform artifacts**

Produce Windows/Linux wheels and platform import receipts from the same HEAD plus a new universal ZIP under `G:\AstrEmbodiment\.codex-task-temp\emotion-personality-core-release\`.

```powershell
python scripts/verify_core_package.py --artifact-root 'G:\AstrEmbodiment\.codex-task-temp\emotion-personality-core-release'
python -m pytest -q tests/test_core_package_parity.py::test_dual_platform_packages_are_matrix_complete_and_proactive_free -o cache_dir=$pytestCache
```

Expected: identical core method/domain contracts, correct native hashes, no proactive member/method/string/UI key. Either platform missing is `NO_GO_PACKAGE_PARITY`.

- [ ] **Step 6: Perform disposable AstrBot manual acceptance**

1. Fresh install: Genesis, one authorized inbound appraisal and ordinary reply work; one semantic Provider call is permitted and no proactive send occurs.
2. Malformed/over-budget appraisal: zero semantic mutation, stable reason, no retry.
3. Awake→drowsy→asleep→awake with high urgency: local evolution and zero Provider/token reservation/network/send.
4. Upgrade old `proactive_enabled=true` config: values remain stored but hidden/ignored; no supervisor starts.
5. Upgrade DB fixtures with Ready/Deferred/Externalizing/DispatchPending/DispatchUnknown: history remains and none retries.
6. Upgrade alpha3 no-op and authenticated AESEM2/3 copies; corrupt copies produce zero write.
7. Grant one relation projection capability: one nonce returns the closed projection; replay/revoke/expiry/scope mismatch fail without enumeration or mutation.
8. Verify persona mood continuity without relation evidence/consent/cause/target/platform identity leakage.

Retain config/DB hashes, Provider/send counters, state/formula/graph/projection digests and package receipts.

- [ ] **Step 7: Classify honestly**

- `PASS_SOURCE_FOCUSED` requires Steps 1–4.
- `PASS_PACKAGE` additionally requires Step 5 for both platforms from one HEAD.
- `PASS_HOST_MANUAL` additionally requires Step 6 receipts.
- Any one of 43 capabilities open/unevidenced, appraisal loss, active proactive export/call path, old-work retry, relation enumeration/private leak, projection mutation/authority escalation, or Provider/network call from idle/sleep/time/intent/projection is release `NO_GO`.
- Missing offline cache, platform builder or Host access is an environmental/partial result, not full acceptance.

## Spec coverage audit

- [ ] Completed provenance custody is referenced once and not rewritten or repeated.
- [ ] All 43 old capabilities remain release-blocking until named evidence changes their disposition.
- [ ] 15 slots/routes, nine regions, 16,384 nodes, eight DOFs, graph lifecycle and FXP6 Jacobi dynamics are preserved.
- [ ] AESEM2/3, migration backup, semantic cursor, atomicity, dedupe, stale and identity conflict are covered.
- [ ] Authorized inbound appraisal retains Provider/budget/telemetry; malformed or exhausted appraisal is zero mutation/no retry.
- [ ] Persona Genesis remains behind closed validation and Native authority.
- [ ] Ordinary main-LLM reply/delivery fact remains; proactive generation/send does not.
- [ ] Matrix, sleep, time, intent and projection are deterministic zero-LLM/network paths.
- [ ] Old proactive Tasks 1–3/data remain in history/storage; Tasks 4–5 and old matrix Task 10 are cancelled.
- [ ] Supervisor, recipient discovery, proactive Provider/token/outbox/retry/dispatch/send, related UI and exports leave the package.
- [ ] Historical work is terminalized/frozen; DispatchUnknown never retries.
- [ ] Contact intent is one relation, nonce-bound, expiring, non-enumerating, read-only and has five false authority flags.
- [ ] Projection-read consent is distinct from message consent and defaults closed.
- [ ] Semantic settings remain visible/purpose-specific; proactive-expression settings are preserved but hidden/ignored.
- [ ] Windows/Linux wheels and ZIP prove matrix parity and absence of active-send surfaces.

## Plan self-review commands

```powershell
$plan='docs\superpowers\plans\2026-08-31-emotion-personality-core.md'
$forbidden=@(('TO'+'DO'),('TB'+'D'),('PLACE'+'HOLDER'),('fill'+' in'),('as '+'appropriate'))
foreach($marker in $forbidden){ Select-String -LiteralPath $plan -SimpleMatch $marker }
(Select-String -LiteralPath $plan -Pattern '^### Task [0-9]+:').Count
$cancelled=@(('Interleave '+'P4'),('Interleave '+'P5'),('Let proactive '+'gating consume'))
foreach($marker in $cancelled){ Select-String -LiteralPath $plan -SimpleMatch $marker }
git diff --check -- $plan
```

Expected: no forbidden/cancelled-route matches, exactly sixteen task headings and clean diff. Type review confirms `EvidenceVector`, `PairedSemanticCommitV1`, `MatrixTimeAdvanceV1`, `AffectProjectionV1`, `AffectContactIntentProjectionRequestV1`, `AffectContactIntentProjectionV1` and `CoreBoundaryUpgradeReceiptV1` match their first definitions.
