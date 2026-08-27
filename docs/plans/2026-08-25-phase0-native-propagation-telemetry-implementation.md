# Phase 0-A Native Propagation and Telemetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现真实 sparse-edge 传播、telemetry 闭环和 append-only native compensation，兼容旧数据。

**Architecture:** Rust 独占公式、状态、digest、CAS、SQLite；PyO3 输出闭合 JSON。AESEM2 只读，新提交 AESEM3。

**Tech Stack:** Rust, FxP6/i128, ae-neurofield/runtime/store, SQLite, PyO3.

---

只修改规格 A 的 `crates/**` 和 `crates/ae-runtime/tests/phase0_native_semantic.rs`，不得改 Python。每任务前后检查 `git status --porcelain=v1`。

### Task 1: Contracts/formula digest

**Files:** Modify `crates/ae-contracts/src/lib.rs`, `crates/ae-runtime/src/lib.rs`; Create `semantic_telemetry_v1.rs`, `learning_compensation_v1.rs`.

- [ ] 新增 telemetry/proposal/compensation receipt，deny unknown，15D 固定布局。
- [ ] canonical bytes/domain hash/range/nonzero/revision/PREPARE validation；不复用 CorrectionVerdict。
- [ ] 常量/route/graph/formula 进入新 digest，移除 semantic `fixed_zero_vector()`。
- [ ] `cargo check -p ae-contracts -p ae-runtime --locked --offline`，exit 0。
- [ ] Commit `feat(native): define phase0 telemetry contracts`。

### Task 2: Graph/传播

**Files:** Modify `crates/ae-neurofield/src/lib.rs`, `crates/ae-runtime/src/semantic.rs`, `crates/ae-runtime/src/lib.rs`; Create `semantic_dynamics_v2.rs`.

- [ ] 空图 `develop_graph(...,V1)`，非空 validate/digest。
- [ ] 按规格实现 i128 FxP6、immutable Jacobi edges、八 DOF/reserve。
- [ ] prepared result 返回 next field/graph、vector digests、energy ledger、饱和计数。
- [ ] 新 perception 走 v2，旧 decode/replay 保持。
- [ ] `cargo check -p ae-neurofield -p ae-runtime --locked --offline`；Commit `feat(native): propagate semantic state over sparse edges`。

### Task 3: Telemetry/AESEM3/事务

**Files:** Modify `semantic_telemetry_v1.rs`, `semantic.rs`, runtime `lib.rs`, `crates/ae-continuum/src/lib.rs`, `crates/ae-store/src/lib.rs`.

- [ ] 计算 energy/capacity/renorm/headroom/health/gate；结构 gate 失败 Err。
- [ ] AESEM3 encode/decode；AESEM2 字节/digest 不变。
- [ ] journal/snapshot/full graph/telemetry 同事务；迁移无半状态。
- [ ] 新 dedup 复核 telemetry；旧 dedup `UNAVAILABLE_LEGACY` 且不回写。
- [ ] `cargo check -p ae-store -p ae-continuum -p ae-runtime --locked --offline`；Commit `feat(native): commit causal semantic telemetry`。

### Task 4: Job/compensation

**Files:** Modify `crates/ae-store/src/lib.rs`, `learning_compensation_v1.rs`, runtime `lib.rs`.

- [ ] text-free job/checkpoint 表、unique id/lease/terminal receipt，不存 raw text。
- [ ] enqueue/claim/abandon/reject/restart abandoned。
- [ ] native 读取 verified current telemetry，重算候选 B，CAS revision，原子写 checkpoint/receipt。
- [ ] 不改 field/graph、不返回 expression；normal perception 才读取 u。
- [ ] `cargo check -p ae-store -p ae-runtime --locked --offline`；Commit `feat(native): persist append-only learning compensation`。

### Task 5: PyO3

**Files:** Modify `crates/ae-pyo3/src/lib.rs`.

- [ ] closure v2 输出 verified telemetry 或 explicit legacy unavailable。
- [ ] job/compensation symbols 拒绝 unknown/bool/float/非法 hex/range。
- [ ] compensation `expression_projection:null`；G0 继续拒绝 correction scaffold。
- [ ] `cargo check -p astrembodiment-core --locked --offline`；Commit `feat(native): expose phase0 semantic closure abi`。

### Task 6: 最小验收

**Files:** Create `crates/ae-runtime/tests/phase0_native_semantic.rs`.

- [ ] 仅三个 focused probes：edge source→target、AESEM2 dedup、compensation append/CAS/idempotency/no-expression。
- [ ] `cargo test -p ae-runtime --test phase0_native_semantic --locked --offline`，预期 3 passed。
- [ ] 隔离编译：将 `CARGO_TARGET_DIR` 与 `CARGO_HOME` 指向由执行环境提供的本机临时目录，避免把构建缓存写入仓库；随后运行 `cargo check --workspace --locked --offline`。

- [ ] exit 0；`git diff --name-only` 仅 A 边界；Commit `test(native): verify phase0 semantic closure`。不声称 release acceptance。
