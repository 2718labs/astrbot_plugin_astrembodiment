# Emotion Matrix Forward Port Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在不丢失旧能力、不覆盖源历史的前提下，把冻结的 15 维情感证据、九区路由、16,384×8 神经场、确定性稀疏图、FXP6 动力学、语义持久化和可验证投影选择性前移到当前 alpha3 发布线，并让睡眠、观察与主动联系只消费已提交的最小投影。

**Architecture:** 先冻结来源与能力平价账本，再按“纯契约/路由 → 图 → 动力学 → codec/迁移 → 单写入者原子提交 → 睡眠时间推进 → 投影/renorm → 主动联系 → Host/包体”逐层接入。persona 级情感场与 relation 级私有证据使用不同持久化键；只有 runtime/store 可写矩阵，Host、观察台和主动联系只能提交闭合输入或读取已验证投影。原 `FullVectorRouteNeutralRelaxationV1` 保持字节兼容，新增的 `MatrixTimeAdvanceV1` 使用独立公式承诺。

**Tech Stack:** Rust 2021 workspace、FXP6 `ae-fixed`、SQLite/rusqlite immediate transaction、serde canonical wire、PyO3、Python 3、pytest、AstrBot 插件打包脚本、Windows x64 与 Linux x86_64 native wheel。

---

## Frozen baselines and execution order

- 只读源工作树：`G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\release-1.0.0-integration`
- 源 commit：`710829ae5d3bef82ce818754354272517cb28056`
- 目标实施工作树：`G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-worktree`
- 目标设计基线：`e8cf2794ee45a9fad6e54125ea282446c8463ccf`
- 两线共同祖先：`8c4a606e63888351b3d8854e9006b89aa6623f07`
- 禁止在源工作树提交、reset、删除或覆盖；只允许 `git show <source>:<path>`、`Get-FileHash` 和只读比较。
- 每项任务开始前运行 `git status --short`。若出现不属于当前任务的修改，停止并交回主代理，不清理用户工作。

现有计划 `docs/superpowers/plans/2026-08-30-proactive-settings-simplification.md` 的 Tasks 1–3 已由 `3c60c06..b05d7b5` 完成，不再执行。其 Task 4 和 Task 5 保持原文为唯一实现说明，并按以下顺序交错；其 Tasks 6–7 被本计划 Tasks 12–13 的合并版本/包体验收取代，不能独立执行：

1. 本计划 Tasks 1–9：先得到已提交、可验证的 persona affect projection。
2. 原主动设置计划 Task 4：绑定 effective frequency 和正预算到现有 Native gate。
3. 本计划 Task 10：在同一 gate authority 中接入 verified affect projection；矩阵不可用时显式安全降级。
4. 原主动设置计划 Task 5：在 recovery/Provider 前准备每个 relation policy/budget。
5. 本计划 Tasks 11–13：Host/兼容 outbox、统一 alpha4 版本、双平台包与最终 NO-GO 门禁。

## Frozen source provenance

Task 1 写入的 manifest 必须逐字包含下表；SHA-256 来自 `git show 710829a:<path>` 的 blob 内容，不依赖源工作树 checkout 状态。

| Source path | SHA-256 |
|---|---|
| `crates/ae-contracts/src/lib.rs` | `4b42175c17cdf4a4e5e982bea60f4f6c63c30323c0af7969e99fc82d90f32b70` |
| `crates/ae-attention/src/lib.rs` | `1b7c5141f01f4f910ee60ba1933ea6645fcc410bafdef7615ab4d00591c67c52` |
| `crates/ae-attention/src/r7.rs` | `efaa208c170d5c950340ed6b997028b7e2f7dacfb402a95030da051c90f6bce4` |
| `crates/ae-neurofield/src/lib.rs` | `c0ab2ed01917564d95865065b5b51f6f86a3e9a1eaed4a4fa5cd793f41587a16` |
| `crates/ae-neurofield/src/graph_development.rs` | `d014151e501e55938200b2773e94a5a2f290ad09cda5c615fff5a6f9969ed00e` |
| `crates/ae-neurofield/src/graph_replay.rs` | `a3b7355d6f35ba04ea42c0959f1b96793026104ae504244cd20a9dc96f040ddf` |
| `crates/ae-neurofield/src/structural_delta.rs` | `4121208c9d6ecf9fd207616a6e95c0c65a4f73d9d24867131195ae92b40a5891` |
| `crates/ae-runtime/src/semantic.rs` | `63ea4b3ac75fdd5703fd2a8cd753a6d677b7eb6d8250b26f3b81e03372864649` |
| `crates/ae-runtime/src/semantic_dynamics_v2.rs` | `9162735da896c82bc07fbebdcc446354bbc12138efebd05e2d8cc89566f9f91d` |
| `crates/ae-runtime/src/semantic_telemetry_v1.rs` | `737a1ee8d8341586f46a552ea5de0e36ca41d50381e332e9092b2562c3616a7f` |
| `crates/ae-runtime/src/n2_native_assembly.rs` | `47031415ea7fd2966b149ecc7b9b4359b4476f74a78c051c9c315ce6ec25086f` |
| `crates/ae-runtime/src/lib.rs` | `930bfe25565ae221cd05a8b0d81d4a8a5318e3dbe7ec855e3dc38b290e64a0ad` |
| `crates/ae-store/src/lib.rs` | `a629ff3db836d855e9392eaa940808ba26153124c7a8610a654d7b6b1265b2b7` |
| `crates/ae-store/src/semantic_field_attestation.rs` | `05acadb0c7d5195f1497ae3673169ef79926399a86c5c5b96a46ade534564fcd` |
| `crates/ae-store/src/semantic_outbox_crypto.rs` | `40a0d21efaf6ce9a52e59e71eeb02a806571ffa0c67158b1891c648d461c38f8` |
| `crates/ae-pyo3/src/lib.rs` | `792fe403e85d7cf599a2df48ec9888fb1c47e045e251ce13274c819a079553c6` |
| `astr_embodiment/bridge.py` | `23afe048cc9865544bf9cda67a501c0dea769ff480017865f952d5ded7ab69e2` |
| `astr_embodiment/coordinator.py` | `9a236b49bd13753f7a349f541179dedbe8e235154a2b210b16319c40e5d68577` |
| `astr_embodiment/semantic_outbox.py` | `92cdb95127f0bb9074e10aea1b3192964695f71a5ded83fd85ac45c4a27d7014` |
| `model/regions-v1.toml` | `6a2af87fe65e01b17a512b9796bee011e6ea319b1362467d426fb685e887383a` |

源提交审计集合固定为 `a036391`、`dac330e`、`15a2da5`、`d6cfbe5`、`d8bfc7c`、`867eec6`、`8209a2b`、`1774023`、`3984ffb`、`cc725f2`、`9d373c4`、`ca8d719`、`710829a`。实施者必须审查最终源头而非只复制最早 feature commit。

## Target file map

- `model/emotion-matrix-provenance-v1.json`：不可变源 commit/path/hash/symbol/target/disposition 清单。
- `model/emotion-matrix-capability-parity-v1.json`：机器可读能力平价、状态、验证 node 和工件证据；任何 `UNMAPPED`/`UNKNOWN` 都是发布 NO-GO。
- `crates/ae-contracts/src/emotion_matrix.rs`：冻结的 route/formula/time-advance/affect-projection 契约。
- `crates/ae-attention/src/emotion_matrix.rs`：15 维证据到九区负载的纯函数。
- `crates/ae-neurofield/src/{graph_development,graph_replay,structural_delta}.rs`：确定性图生成、回放和 CAS 增量。
- `crates/ae-runtime/src/{semantic,semantic_dynamics_v2,semantic_telemetry_v1,matrix_time}.rs`：纯语义计算、AESEM codec、时间推进和投影。
- `crates/ae-store/src/{semantic,semantic_field_attestation,semantic_outbox_crypto}.rs`：独立 semantic namespace、原子提交、迁移、恢复和兼容 crypto surface。
- `crates/ae-runtime/src/lib.rs`：当前 `CanonicalEvent` authority 到纯计算/Store 单写入者的适配。
- `crates/ae-store/src/alpha3/projection.rs`：committed affect projection、renorm 读侧和 proactive gate authority。
- `crates/ae-contracts/src/alpha3.rs`：Experience/Developer 的最小投影字段；Private 保持 relation-local。
- `crates/ae-pyo3/src/lib.rs`、`astr_embodiment/bridge.py`：闭合 Host ABI；不暴露节点或原始证据。
- `scripts/verify_semantic_package_parity.py`：扫描实际 Windows/Linux wheel 和 universal ZIP 的 domain/export/manifest 平价。

### Task 1: Freeze provenance and capability parity before production edits

**Files:**
- Create: `model/emotion-matrix-provenance-v1.json`
- Create: `model/emotion-matrix-capability-parity-v1.json`
- Create: `tests/test_emotion_matrix_provenance.py`

- [ ] **Step 1: Write the failing provenance test**

Add a test which requires the two manifests, exact baselines, all twenty source records above, all thirteen audited commits, unique capability IDs and closed dispositions:

```python
ALLOWED = {"EXACT_PORT", "ADAPTED_WITH_EQUIVALENCE", "EXPLICITLY_SUPERSEDED", "UNMAPPED"}

def test_emotion_matrix_provenance_and_parity_are_closed():
    provenance = json.loads(Path("model/emotion-matrix-provenance-v1.json").read_text("utf-8"))
    parity = json.loads(Path("model/emotion-matrix-capability-parity-v1.json").read_text("utf-8"))
    assert provenance["schema"] == "ae.emotion-matrix-provenance.v1"
    assert provenance["source_commit"] == "710829ae5d3bef82ce818754354272517cb28056"
    assert provenance["target_baseline"] == "e8cf2794ee45a9fad6e54125ea282446c8463ccf"
    assert provenance["merge_base"] == "8c4a606e63888351b3d8854e9006b89aa6623f07"
    assert len(provenance["files"]) == 20
    assert len({row["source_path"] for row in provenance["files"]}) == 20
    assert all(len(row["sha256"]) == 64 for row in provenance["files"])
    assert all(set(row) == {"source_commit", "source_path", "sha256", "source_symbols", "target_path", "target_symbols"} for row in provenance["files"])
    assert parity["schema"] == "ae.emotion-matrix-capability-parity.v1"
    assert len({row["id"] for row in parity["capabilities"]}) == len(parity["capabilities"])
    assert all(row["disposition"] in ALLOWED for row in parity["capabilities"])
    assert all(row["source_symbols"] and row["target_symbols"] for row in parity["capabilities"])
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_emotion_matrix_provenance.py::test_emotion_matrix_provenance_and_parity_are_closed -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\task-01-red'
```

Expected: FAIL because the manifests do not exist.

- [ ] **Step 3: Write both closed manifests**

Use the exact table in this plan. Every provenance file row has this complete shape:

```json
{
  "source_commit": "710829ae5d3bef82ce818754354272517cb28056",
  "source_path": "crates/ae-runtime/src/semantic.rs",
  "sha256": "63ea4b3ac75fdd5703fd2a8cd753a6d677b7eb6d8250b26f3b81e03372864649",
  "source_symbols": ["prepare_semantic_transition_v2", "decode_semantic_snapshot_v2", "encode_semantic_snapshot_v3", "decode_semantic_snapshot_v3"],
  "target_path": "crates/ae-runtime/src/semantic.rs",
  "target_symbols": ["prepare_semantic_transition_v2", "decode_semantic_snapshot_v2", "encode_semantic_snapshot_v3", "decode_semantic_snapshot_v3"]
}
```

The provenance file is immutable after this commit: later tasks may only verify its source commit/path/hash/symbol/target mapping. The parity file must contain separate IDs for evidence codec/order/range, 15 routes, route digest, 9-region layout, 16K×8 field, 262,144-edge graph, graph replay/delta, FXP6/Jacobi dynamics, eight DOFs, energy/capacity/renorm telemetry, semantic receipt v2, node observability, expression projection, independent cursor, atomic/dedupe/stale/identity conflict, AESEM2, AESEM3, finite-domain migration/backup/formula upgrade, async outbox/crypto, relation privacy/persona mood, time decay/sleep, Host ABI, Windows/Linux exports and package members. Each starts `UNMAPPED`, cites exact source symbols and names its future focused test node.

- [ ] **Step 4: Verify source blobs without changing the source worktree**

```powershell
$src='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\release-1.0.0-integration'
$manifest=Get-Content -LiteralPath 'model\emotion-matrix-provenance-v1.json' -Raw | ConvertFrom-Json
foreach($row in $manifest.files){
  $actual=(git -C $src show "710829ae5d3bef82ce818754354272517cb28056`:$($row.source_path)" | openssl dgst -sha256) -replace '^SHA2-256\(stdin\)= ',''
  if($actual -ne $row.sha256){ throw "source hash mismatch: $($row.source_path)" }
}
git -C $src status --short
```

Expected: no hash mismatch and no source worktree changes.

- [ ] **Step 5: Run GREEN and commit**

```powershell
python -m pytest -q tests/test_emotion_matrix_provenance.py::test_emotion_matrix_provenance_and_parity_are_closed -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\task-01-green'
git add model/emotion-matrix-provenance-v1.json model/emotion-matrix-capability-parity-v1.json tests/test_emotion_matrix_provenance.py
git commit -m "chore: freeze emotion matrix provenance"
```

Expected: PASS; production crates are unchanged; ledger intentionally remains release-NO-GO until later tasks replace every `UNMAPPED`.

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

### Task 5: Restore authority-owned semantic codecs and crash-recoverable finite migration

**Files:**
- Modify: `crates/ae-runtime/src/semantic.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Modify: `crates/ae-store/src/semantic_field_attestation.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Replace: `crates/ae-store/tests/emotion_matrix_migration.rs`
- Create: `crates/ae-store/tests/emotion_matrix_migration_v2.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`
- Modify: `tests/test_emotion_matrix_provenance.py`

Task 5 owns only authenticated read compatibility and the unique finite legacy migration. Task 7 remains the sole owner of non-zero semantic event application. A successful codec round trip, a caller-supplied self-consistent receipt, or a public `seal` method is never migration authority.

#### Phase 0: Safety rollback before production edits

- [ ] Mark `aesem2-read-compatibility`, `aesem3-current-write`, `finite-domain-migration`, `migration-preimage-backup`, and `formula-upgrade-proof` `UNMAPPED`; set every affected target commit to `null`, empty their evidence, preserve `NO_GO`, and remove obsolete Task 5 store receipts/logs.
- [ ] Add a focused ledger test which requires those five rows to remain unearned until new receipts prove this revised acceptance contract.
- [ ] Commit this docs/ledger/test-only rollback before changing runtime or store production code. The immutable provenance manifest must not change.

#### Phase A: Checked storage primitives and bounded reads

- [ ] Add focused RED tests for negative SQLite revisions, `u64 > i64::MAX`, `i64::MAX` checked-next overflow, malformed digest widths, oversized rows, oversized databases, and oversized backup manifests. These tests exercise the public Store paths and migration paths, not duplicate test-only parsers.
- [ ] Introduce one shared checked revision boundary (`JournalRevision`/`SqliteRevision` or an equivalently closed type) used by public Store and migration. No `as i64`, unchecked `+ 1`, or negative-to-unsigned conversion is permitted at a SQL boundary.
- [ ] Length-gate blobs in SQL before materializing them; stream or incrementally hash bounded snapshot/backup data. Read the backup manifest at one exact fixed length and enforce explicit database, row-count, and aggregate-byte budgets.

Focused node:

```powershell
cargo test --locked --offline -p ae-store --lib --jobs 1 semantic_field_attestation::tests::storage_primitives_reject_unrepresentable_revisions_and_oversized_rows -- --exact
```

Commit checkpoint: `fix: bound semantic migration storage primitives`.

#### Phase B: One strict AESEM decoder and exactly four AESEM3 blocks

- [ ] Add RED fixtures for a valid four-block AESEM3, three-block truncation, fifth/trailing block, wrong schema/magic, oversized block length/count, graph-width boundary, and retired non-zero compensation.
- [ ] Use one strict bounded decoder where runtime/store contracts permit sharing. `AESEM3\0` schema 3 contains exactly four length-delimited blocks in canonical order: field, graph, telemetry, retired-compensation. The fourth block must be present and canonically zero; three blocks are truncation, and any fifth/trailing byte is invalid.
- [ ] Preserve `AESEM2\0` schema 2 as read-only bytes. Do not treat codec authentication alone as full history or migration compatibility.

Focused nodes:

```powershell
cargo test --locked --offline -p ae-runtime --lib --jobs 1 semantic::tests::aesem3_decoder_requires_exactly_four_bounded_blocks -- --exact
cargo test --locked --offline -p ae-runtime --lib --jobs 1 semantic::tests::aesem2_history_codec_is_bounded_and_authority_authenticated -- --exact
```

Commit checkpoint: `fix: enforce strict semantic snapshot framing`.

#### Phase C: Store-private migration authority construction

- [ ] Replace the public raw migration envelope plus `target_state_bytes` request with a selector/CAS-only V2 request. The caller may select an authenticated legacy scope/revision and expected source digest; only Store privately derives the migration event, normalized field, semantic transition commitment, telemetry, AESEM3 bytes, transition/formula/domain receipts, graph/context sidecars, and authority digests.
- [ ] If a compatibility request still carries stimulus bytes, require the exact canonical all-zero migration stimulus. Any non-zero, non-canonical, unknown, or trailing input fails before backup creation or database write.
- [ ] Bind telemetry to a private `SemanticTransitionCommitment` that includes source/target state, graph, snapshot, formula, migration event, and authority digests. Public construction or self-sealing of telemetry/receipts never grants write authority.

Focused nodes:

```powershell
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 public_selector_cannot_supply_migration_authority -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 authority_binds_formula_upgrade_and_rejects_public_raw_envelopes -- --exact
```

Commit checkpoint: `fix: make semantic migration authority store-owned`.

#### Phase D: Full history, set, identity, and genesis closure

- [ ] Authenticate every legacy revision from the manifest baseline. Require exact set equality, not just latest-row or per-row membership, across journal, applied-events, snapshot, graph, and context histories: no gaps, extras, duplicates, wrong scope, or orphan sidecars.
- [ ] Close the complete identity chain: manifest canonical bytes, persona/scope binding, incarnation, namespace/root digests, genesis field/graph/formula/development seed, source metadata, every event/receipt/authority/chain link, state-before/after, graph/context sidecars, and final snapshot/cursor.
- [ ] Run the same full closure again against the source connection immediately before backup and against the migrated connection before commit/return. Any discrepancy is a typed closed error with zero writes.

Focused node:

```powershell
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 history_identity_requires_exact_set_and_root_closure -- --exact
```

Commit checkpoint: `fix: close complete semantic migration history`.

#### Phase E: Same-connection crash-recoverable backup

- [ ] Add failpoint/reopen RED coverage before/after snapshot, database sync, manifest sync, directory sync, atomic publish, parent sync, transaction begin, row insertion, and commit. Each case must classify clean retry, valid-final recovery, stage cleanup, or fail-closed mismatch without changing source bytes/cursor.
- [ ] Use rusqlite/SQLite's same-connection snapshot or backup API. Never reopen the authoritative source by filesystem path. If the required API is unavailable under the current dependency features, stop and report the exact feature/API blocker before proposing an alternative.
- [ ] Stage exactly one sibling directory containing database plus fixed-length manifest. Validate both, compare a source fingerprint and `PRAGMA data_version` CAS, fsync both files and the staged directory, atomically rename the directory, then fsync its parent. A valid final directory with no migration row is recoverable and retryable; stale stage debris is cleaned or replaced safely and never hard-wedges; a mismatched final directory fails closed.

Focused node:

```powershell
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 backup_recovers_every_failpoint_without_source_mutation -- --exact
```

Commit checkpoint: `fix: make semantic migration backup crash recoverable`.

#### Phase F: Atomic migration, retry, and final proof

- [ ] Commit normalized state, strict four-block AESEM3, receipts, graph/context sidecars, cursor, applied event, and migration record in one checked transaction. Concurrent CAS loss performs zero writes. Exact retry returns the persisted outcome only after complete post-commit closure; conflicting retry fails closed.
- [ ] The immutable two-or-more revision source fixture must tamper every authority/chain/state/graph/context link and add gap/extra/orphan/malformed/budget cases. Copy/verify failures and every failpoint must preserve source bytes, cursors, and authoritative row sets.
- [ ] Restore parity mappings only after new commit-pinned receipts prove the complete revised contract. Keep the overall release gate `NO_GO`; provenance remains byte-identical.

Final focused nodes:

```powershell
cargo test --locked --offline -p ae-runtime --lib --jobs 1 semantic::tests::aesem2_history_codec_is_bounded_and_authority_authenticated -- --exact
cargo test --locked --offline -p ae-runtime --lib --jobs 1 semantic::tests::aesem3_decoder_requires_exactly_four_bounded_blocks -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 authority_constructs_finite_migration_and_closes_all_history_sets -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 corrupt_history_and_budget_violations_cause_zero_writes -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 backup_recovers_every_failpoint_without_source_mutation -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_migration_v2 --jobs 1 concurrent_cas_and_exact_retry_are_zero_write_or_identical -- --exact
cargo check --locked --offline -p ae-runtime -p ae-store --jobs 1
```

Expected: all named nodes/check pass; AESEM2 remains bounded read-only history; all current migration writes are authority-owned strict AESEM3; backup is same-connection, single-directory and crash-recoverable; only the one authenticated finite predecessor migrates; every mismatch is zero-write. Commit each phase separately and pin new evidence to the final production commit.

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
cargo check --locked --offline -p ae-runtime -p ae-store
git add crates/ae-contracts crates/ae-runtime crates/ae-store model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: advance emotion state through sleep time"
```

### Task 9: Publish committed affect/observer projection and keep renorm read-only

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

## Interleave P4: Execute the existing proactive plan Task 4

After Task 9 and before Task 10, execute exactly `docs/superpowers/plans/2026-08-30-proactive-settings-simplification.md` Task 4, including its RED/GREEN node `auto_frequency_and_budget_policy_share_gate_authority` and commit `feat: enforce proactive frequency in native gate`. Do not edit that task's contract to consume affect yet; this separation proves frequency/budget authority independently before Task 10 adds one optional verified input.

### Task 10: Let proactive gating consume only verified affect authority

**Files:**
- Modify: `crates/ae-contracts/src/alpha3.rs`
- Modify: `crates/ae-store/src/alpha3/projection.rs`
- Modify: `crates/ae-runtime/tests/alpha3_projection.rs`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add one RED gate-authority test**

Extend the now-passing frequency/budget fixture. A current attested projection may increase only candidate endogenous strength within the already selected frequency pair; it cannot change daily max/cooldown, bypass sleep/quiet/consent/unanswered/budget/target/readiness, or create an intention without relation-local authorized cause. Missing, stale, future, digest-mismatched, inconsistent or renorm-failed projection records `matrix_unavailable` and uses the current restrained/non-matrix policy without claiming matrix evidence.

```rust
#[test]
fn proactive_gate_uses_verified_affect_without_bypassing_any_hard_gate() {
    let verified = fixture.committed_affect_authority();
    let allowed = fixture.gate_with_affect(verified.clone()).unwrap();
    assert_eq!(allowed.effective_frequency, Some(fixture.expected_frequency()));
    assert_eq!(allowed.affect_authority.unwrap().semantic_revision, verified.semantic_revision);
    for suppressed in fixture.every_hard_suppression_with(verified) {
        assert!(suppressed.claim.is_none());
    }
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-10-red'
cargo test --locked --offline -p ae-runtime --test alpha3_projection proactive_gate_uses_verified_affect_without_bypassing_any_hard_gate -- --exact
```

Expected: compile FAIL because `GateAuthoritySnapshotV1` has no affect authority.

- [ ] **Step 3: Bind projection evidence into the existing authority snapshot**

Add this optional closed authority to the same `GateAuthoritySnapshotV1` commitment and `GateDecisionV2`:

```rust
pub struct VerifiedAffectAuthorityV1 {
    pub semantic_revision: u64,
    pub state_digest: Digest,
    pub formula_digest: Digest,
    pub projection_digest: Digest,
    pub confidence_fxp6: u32,
}
```

Load it in the same transaction as contact/budget/time/count. Accept only latest committed persona projection whose digest chain closes and whose revision is not future/regressing. Apply affect after all existing hard suppressions and before intention score threshold; never alter `EffectiveFrequencyV1` or reservation.

- [ ] **Step 4: Make unavailability explicit and safe**

Use these closed diagnostics in developer/gate output:

```rust
pub enum MatrixUnavailableReasonV1 {
    NotInitialized,
    Stale,
    FutureRevision,
    DigestMismatch,
    InvalidProjection,
    RenormResidualExceeded,
}

pub enum AffectGateStatusV1 {
    Verified(VerifiedAffectAuthorityV1),
    Unavailable(MatrixUnavailableReasonV1),
}
```

Existing non-matrix candidate behavior remains bounded; diagnostics must not say affect-backed when unavailable. Matrix computation remains local even when proactive is disabled or token budget is zero.

- [ ] **Step 5: Run GREEN and commit**

```powershell
cargo test --locked --offline -p ae-runtime --test alpha3_projection proactive_gate_uses_verified_affect_without_bypassing_any_hard_gate -- --exact
cargo test --locked --offline -p ae-runtime --test alpha3_projection matrix_unavailable_is_explicit_and_never_claims_affect_authority -- --exact
cargo check --locked --offline -p ae-store -p ae-runtime
git add crates/ae-contracts crates/ae-store crates/ae-runtime/tests model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: bind verified affect to proactive authority"
```

## Interleave P5: Execute the existing proactive plan Task 5

After Task 10 and before Task 11, execute exactly `docs/superpowers/plans/2026-08-30-proactive-settings-simplification.md` Task 5, including its Host RED/GREEN node `test_proactive_budget_is_prepared_before_recovery_or_provider` and commit `feat: wire proactive budget before autonomous work`. Its preparation callback remains responsible for relation frequency/budget closure; it must not synthesize or cache affect projection.

### Task 11: Expose closed Host semantic APIs and preserve async outbox compatibility

**Files:**
- Create: `crates/ae-store/src/semantic_outbox_crypto.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Modify: `crates/ae-store/Cargo.toml`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `crates/ae-pyo3/Cargo.toml`
- Modify: `astr_embodiment/bridge.py`
- Create: `astr_embodiment/semantic_outbox.py`
- Modify: `astr_embodiment/coordinator.py`
- Create: `tests/test_emotion_matrix_host.py`
- Create: `tests/test_semantic_async_outbox.py`
- Modify: `model/emotion-matrix-capability-parity-v1.json`

- [ ] **Step 1: Add RED Host ABI and compatibility tests**

The recording fake requires `semantic_revision_v1`, `apply_perception_proposal_v1`, committed `inspect` projection and semantic outbox status/seal/open. It rejects unknown fields, malformed/oversized JSON, raw text, zero confidence/digest, cross-scope proposal, request nonce/incarnation mismatch and any response containing raw node arrays/evidence. The outbox test requires source-compatible crypto envelope/status and fail-closed tamper/size behavior, but does not enable background transport by default.

```python
def test_host_semantic_bridge_is_closed_and_projection_only(native_bridge):
    result = native_bridge.apply_perception_proposal_v1(SCOPE, VALID_PROPOSAL)
    assert result["semantic_revision"] == 1
    encoded = json.dumps(result, sort_keys=True)
    assert "node" not in encoded and "dimensions" not in encoded and RAW_TEXT not in encoded
    with pytest.raises(ClosedSchemaViolation):
        native_bridge.apply_perception_proposal_v1(SCOPE, {**VALID_PROPOSAL, "raw_text": RAW_TEXT})
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_emotion_matrix_host.py::test_host_semantic_bridge_is_closed_and_projection_only -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\task-11-red'
```

Expected: FAIL because bridge/native methods are absent.

- [ ] **Step 3: Port/adapt the PyO3 semantic surface**

Port source error mapping, bounded parsers and payload builders for `semantic_revision_v1`, `apply_perception_proposal_v1`, expression/node-observability/native-telemetry projections, `inspect`, replay and outbox crypto. Register exact methods in `_native`; add their schemas/domains to `contract_info`. Adapt result JSON to the bounded `AffectProjectionV1`; do not export raw 16K vectors.

- [ ] **Step 4: Add thin Python bridge methods and dormant compatibility outbox**

```python
def semantic_revision_v1(self, scope: dict[str, Any]) -> dict[str, Any]:
    return _parse_payload(self._require().semantic_revision_v1(_canonical_json(scope)))

def apply_perception_proposal_v1(self, scope: dict[str, Any], proposal: dict[str, Any]) -> dict[str, Any]:
    return _parse_payload(self._require().apply_perception_proposal_v1(
        _canonical_json(scope), _canonical_json(proposal)
    ))
```

Port source crypto/store compatibility and a bounded queue codec so existing semantic envelopes can authenticate/reopen. Keep scheduling disabled unless a current explicit auxiliary transport config enables it; no new thread, network call or Provider call is created by matrix installation alone.

- [ ] **Step 5: Keep current zero-confidence Host input honest**

`build_user_stimulus_json` remains explicit zero-confidence when no estimator exists. Coordinator must classify that as `matrix_input_unavailable`, use the non-semantic event path, and never claim an emotion update. A future/other plugin may call the closed proposal API with attested evidence; AstrEmbodiment itself still works without it and sleep/time evolution remains local.

- [ ] **Step 6: Run GREEN, compile, and commit**

```powershell
python -m pytest -q tests/test_emotion_matrix_host.py::test_host_semantic_bridge_is_closed_and_projection_only tests/test_semantic_async_outbox.py::test_compatibility_outbox_is_dormant_and_tamper_closed -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\task-11-green'
python -m py_compile astr_embodiment/bridge.py astr_embodiment/coordinator.py astr_embodiment/semantic_outbox.py
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\task-11-green'
cargo check --locked --offline -p ae-store -p ae-pyo3
git add crates/ae-store crates/ae-pyo3 astr_embodiment tests/test_emotion_matrix_host.py tests/test_semantic_async_outbox.py model/emotion-matrix-capability-parity-v1.json
git commit -m "feat: expose bounded emotion matrix bridge"
```

### Task 12: Consolidate alpha4 version, docs, and dual-platform package parity

**Files:**
- Create: `scripts/verify_semantic_package_parity.py`
- Create: `tests/test_semantic_package_parity.py`
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

- [ ] **Step 1: Add RED release and package-domain tests**

Set authoritative expectations to Rust/native/metadata `1.1.0-alpha4`, Python/wheel `1.1.0a4`. The package scanner must open both wheels and universal ZIP, hash native members, extract strings/OS exports, and require the same semantic route/formula/AESEM/node-observability/outbox domains on Windows/Linux. OS export parity requires `PyInit__native`; PyO3 callable-method parity is proven by a platform-local offline import plus canonical `contract_info` receipt, not by pretending Rust functions are exported C symbols. It rejects alpha3 members, missing platform, duplicate native member, manifest/hash mismatch or an `UNMAPPED`/`UNKNOWN` ledger entry.

```python
REQUIRED_DOMAINS = {
    b"ae.phase0.semantic-route-rules.v1",
    b"phase0-native-propagation-fxp6-v1",
    b"matrix-time-advance-fxp6-v1",
    b"AESEM2\0", b"AESEM3\0",
    b"astr-embodiment.node-observability.v2",
}
REQUIRED_OS_EXPORTS = {
    "PyInit__native",
}
REQUIRED_PY_METHODS = {
    "apply_perception_proposal_v1", "semantic_revision_v1", "inspect", "verify_replay"
}
```

- [ ] **Step 2: Run RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_release_contracts.py::test_release_versions_and_required_files_are_present tests/test_semantic_package_parity.py::test_dual_platform_semantic_domains_and_exports_match -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\task-12-red'
```

Expected: version test FAIL at alpha3 and package test SKIP/FAIL with explicit missing fresh alpha4 artifacts; no old artifact counts as PASS.

- [ ] **Step 3: Bump all authoritative versions once**

Apply the version changes described above, refresh locks using repository-established locked commands, and retain protocol/module identifiers such as `alpha3_call`, alpha3 DB contracts and Rust module paths. Historical alpha3 changelog text remains unchanged.

- [ ] **Step 4: Document actual alpha4 scope without overstating acceptance**

README/CHANGELOG must distinguish: built-in emotion matrix versus optional cyber-person plugin; relation-private evidence versus persona-global mood; explicit zero-confidence Host input; sleep/time evolution; verified-only proactive input; Auto/frequency/budget work; migration/rollback and package gates. State source-focused, Host-manual and package acceptance separately.

- [ ] **Step 5: Update package builder and CI**

Require fresh `1.1.0a4` wheels for `win_amd64` and `manylinux_2_17_x86_64`, verify `_bundled/manifest.json` member hashes/API/ABI, and emit a canonical per-platform receipt containing wheel SHA-256, native member SHA-256, `PyInit__native`, imported method list and `contract_info` digest. Run `verify_semantic_package_parity.py` over both wheels and receipts, then build a new `astrbot_plugin_astrembodiment-1.1.0-alpha4-universal.zip`. Never reuse/overwrite alpha3 ZIP or wheel members. CI runs bounded contract/dynamics/atomic/projection nodes plus both platform builds; a platform artifact failure is package NO-GO.

- [ ] **Step 6: Close provenance and parity ledger**

Every file row becomes `EXACT_PORT`, `ADAPTED_WITH_EQUIVALENCE`, or `EXPLICITLY_SUPERSEDED`; every capability has at least one passing named test or inspected artifact hash. `EXPLICITLY_SUPERSEDED` is allowed only for current alpha3 no-op semantic lane and must cite this approved spec plus the non-zero event-lane test. Add a test assertion that release mode rejects any other disposition or empty evidence.

- [ ] **Step 7: Run source GREEN and commit**

```powershell
python -m pytest -q tests/test_release_contracts.py::test_release_versions_and_required_files_are_present tests/test_emotion_matrix_provenance.py::test_release_parity_has_no_unmapped_unknown_or_empty_evidence -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\task-12-green'
python -m py_compile scripts/package_plugin.py scripts/verify_semantic_package_parity.py
git add README.md CHANGELOG.md Cargo.toml Cargo.lock pyproject.toml uv.lock metadata.yaml .github/workflows scripts tests/test_release_contracts.py tests/test_semantic_package_parity.py model/emotion-matrix-capability-parity-v1.json
git commit -m "chore: prepare emotion matrix alpha4 release"
```

Expected: source version/parity tests PASS; actual dual-platform package test remains unearned until fresh artifacts are supplied in Task 13.

### Task 13: Run bounded acceptance and enforce no-old-capability-loss NO-GO

**Files:**
- Verify only; do not create an empty commit.

- [ ] **Step 1: Collect exact Python nodes before execution**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
$pytestCache='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-pytest\final'
python -m pytest --collect-only -q tests/test_emotion_matrix_provenance.py::test_release_parity_has_no_unmapped_unknown_or_empty_evidence tests/test_emotion_matrix_host.py::test_host_semantic_bridge_is_closed_and_projection_only tests/test_runtime_integration.py::test_proactive_budget_is_prepared_before_recovery_or_provider tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir=$pytestCache
```

Expected: four nodes collect with exit 0. Collection failure is not a test result.

- [ ] **Step 2: Run minimum Python/compile checks**

```powershell
python -m pytest -q tests/test_emotion_matrix_provenance.py::test_release_parity_has_no_unmapped_unknown_or_empty_evidence tests/test_emotion_matrix_host.py::test_host_semantic_bridge_is_closed_and_projection_only tests/test_runtime_integration.py::test_proactive_budget_is_prepared_before_recovery_or_provider tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir=$pytestCache
python -m py_compile main.py astr_embodiment/bridge.py astr_embodiment/coordinator.py astr_embodiment/semantic_outbox.py scripts/package_plugin.py scripts/verify_semantic_package_parity.py
```

- [ ] **Step 3: Run one high-value node per Rust boundary and workspace compile**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-cargo\final'
cargo fmt --all -- --check
cargo test --locked --offline -p ae-neurofield --test deterministic_graph_development v1_golden_vector_is_cross_process_and_cross_platform_stable -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_dynamics phase0_dynamics_change_all_bounded_state_from_immutable_before -- --exact
cargo test --locked --offline -p ae-store --test emotion_matrix_atomic_commit canonical_event_and_semantic_sidecar_commit_together_or_not_at_all -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_event_lane authenticated_user_stimulus_changes_and_reopens_persona_emotion_state -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_time sleep_time_advance_relaxes_without_inventing_evidence -- --exact
cargo test --locked --offline -p ae-runtime --test emotion_matrix_projection committed_affect_projection_is_persona_global_and_non_identifying -- --exact
cargo test --locked --offline -p ae-runtime --test alpha3_projection proactive_gate_uses_verified_affect_without_bypassing_any_hard_gate -- --exact
cargo check --locked --offline --workspace
```

Expected: format, seven behavior nodes and workspace check PASS. Offline cache absence is `ENVIRONMENTAL_HARNESS_FAILURE`, never behavioral PASS and never permission for an unplanned network fetch.

- [ ] **Step 4: Verify preservation, scope, and source immutability**

```powershell
$src='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\worktrees\release-1.0.0-integration'
git -C $src status --short
git status --short
git diff e8cf2794ee45a9fad6e54125ea282446c8463ccf..HEAD --check
(git log --format='%H' -- model/emotion-matrix-provenance-v1.json).Count
Select-String -LiteralPath 'model\emotion-matrix-capability-parity-v1.json' -Pattern 'UNMAPPED|UNKNOWN'
git grep -n -E '1\.1\.0-alpha3|1\.1\.0a3' -- ':!CHANGELOG.md' ':!docs/superpowers/**'
git grep -n -F 'state_after: state_before' -- 'crates/ae-runtime/src/lib.rs'
```

Expected: source and target worktrees clean; diff check clean; provenance has exactly one creation commit and no later modification; no open ledger status; old versions only in history; semantic `UserStimulus` no longer uses no-op while `DeliveryOutcome` may remain unchanged.

- [ ] **Step 5: Build and verify fresh dual-platform package artifacts**

Use CI or controlled builders to produce both wheels from the same accepted HEAD. Place artifacts under `G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-alpha4\`, then run:

```powershell
python scripts/verify_semantic_package_parity.py --windows-wheel 'G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-alpha4\astrbot_plugin_astrembodiment-1.1.0a4-cp311-abi3-win_amd64.whl' --windows-receipt 'G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-alpha4\windows-contract-receipt.json' --linux-wheel 'G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-alpha4\astrbot_plugin_astrembodiment-1.1.0a4-cp311-abi3-manylinux_2_17_x86_64.whl' --linux-receipt 'G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-alpha4\linux-contract-receipt.json' --zip 'G:\AstrEmbodiment\.codex-task-temp\emotion-matrix-alpha4\astrbot_plugin_astrembodiment-1.1.0-alpha4-universal.zip'
python -m pytest -q tests/test_semantic_package_parity.py::test_dual_platform_semantic_domains_and_exports_match -o cache_dir=$pytestCache
```

Expected: both wheel hashes/member manifests/domain strings/exports match their declared platform and the ZIP contains exactly those verified members. Missing Linux or Windows artifact is `NO_GO_PACKAGE_PARITY`.

- [ ] **Step 6: Perform disposable Host manual acceptance**

On supported AstrBot >=4.16,<5 with a backed-up disposable data directory:

1. Fresh install alpha4, create persona, submit one attested non-zero proposal through the closed bridge, verify `state_after != state_before`, then restart and verify identical revision/state digest.
2. Upgrade a copy of an alpha3 no-op database: confirm old journal is unchanged and first semantic revision starts from authenticated Genesis.
3. Upgrade a copy of source AESEM2/AESEM3 data: confirm migration/reopen or exact closed NO-GO; corrupt a copy and confirm zero write/no deletion.
4. Use two relations/one persona and two personas: verify shared persona mood but no relation evidence/consent/cause/target leakage.
5. Advance awake/drowsy/asleep time and verify deterministic decay/recovery with no message or Provider call.
6. Disable proactive and set budget zero: matrix continues local time evolution, while messages/Provider calls remain zero.
7. Enable explicit relation consent/cause and positive budget: verify matrix signal never bypasses sleep, quiet, frequency, unanswered hard stop, target, readiness or claim/settle.
8. Observe Experience/Private/Developer payloads and logs: no raw text, evidence vector, relation token in persona projection, or 16K node dump.

Expected: retain receipts, DB-copy hashes before/after, state/formula/graph/projection digests, native manifest hashes and adapter/platform delivery distinctions.

- [ ] **Step 7: Classify honestly**

- `PASS_SOURCE_FOCUSED` only if Steps 1–4 pass.
- Add `PASS_PACKAGE` only if Step 5 passes for both platforms from one HEAD.
- Add `PASS_HOST_MANUAL` only with Step 6 receipts.
- Any parity ledger gap, source hash mismatch, missing old fixture/codec/export/domain, migration write-on-failure, half commit, privacy leak, or proactive bypass is `NO_GO_OLD_CAPABILITY_LOSS` and blocks release/package publication.
- Test timeout, missing offline cache or unavailable platform builder is an exact environmental/partial result, not permission to delete evidence or claim full acceptance.

## Spec coverage audit

- [ ] Source commit, merge-base, 13 audit commits and 20 exact file hashes are frozen before implementation.
- [ ] Fifteen evidence slots/order/range, fifteen route rules, coefficients and route digest have one public contract.
- [ ] Nine regions, 16,384 nodes, eight DOFs, 524,288 capacity and 262,144 V1 edges are preserved.
- [ ] Graph development/replay/delta canonical bytes, digest, CAS and tamper behavior are covered.
- [ ] FXP6 checked arithmetic, immutable-before Jacobi, neutral/adaptation/energy/reserve formula and telemetry are covered.
- [ ] AESEM2 read/authentication, AESEM3 current writes, finite migration, preimage backup, formula upgrade and corrupt-state zero-write are covered.
- [ ] Persona semantic cursor is independent; main event + semantic sidecar is atomic, single-writer, deduplicated and stale/identity closed.
- [ ] Relation evidence/consent/cause/target stays private while persona mood continuity is explicit and tested.
- [ ] Sleep/time decay is independently versioned, deterministic, graph-preserving and evidence-free.
- [ ] Internal renormalization residual and downstream `ae-renorm` are distinct; only committed state is projected.
- [ ] Experience/Developer expose bounded observability; no raw node/evidence/text leaks.
- [ ] Proactive uses verified projection only after all hard gates and is explicit/safe when matrix is unavailable.
- [ ] Host ABI, outbox/crypto compatibility, Windows/Linux native exports/domains/manifests and universal ZIP are gated.
- [ ] Existing proactive Tasks 1–3 are not repeated; Tasks 4/5 are interleaved after matrix authority; consolidated release tasks supersede old Tasks 6/7.
- [ ] Any old capability without mapping/evidence forces NO-GO.

## Plan self-review commands

```powershell
$plan='docs\superpowers\plans\2026-08-30-emotion-matrix-forward-port.md'
$forbidden=@(('TO'+'DO'),('TB'+'D'),('PLACE'+'HOLDER'),('fill'+' in'),('as '+'appropriate'))
foreach($marker in $forbidden){ Select-String -LiteralPath $plan -SimpleMatch $marker }
(Select-String -LiteralPath $plan -Pattern '^### Task [0-9]+:').Count
git diff --check -- $plan
```

Expected: no forbidden-marker matches, exactly thirteen task headings, and clean diff. Re-read the approved spec and verify every listed type/function name is defined before later use, especially `AffectProjectionV1`, `MatrixTimeAdvanceV1`, `PairedSemanticCommitV1`, `VerifiedAffectAuthorityV1`, `AffectGateStatusV1` and `MatrixUnavailableReasonV1`.
