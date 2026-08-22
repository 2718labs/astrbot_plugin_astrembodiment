# RC2 原生表达与同轮注入实施计划

> **给执行代理：** 必须逐任务使用 executing-plans 与 test-driven-development；所有步骤使用复选框跟踪。

**目标：** 让 15 维闭合语义证据改变持久化原生神经场，并在确认提交后用无内容表达档案影响同一轮回复。

**架构：** attention crate 产生九区域、逐节点的确定性负载；runtime 从已提交场导出六值表达档案，并由 PyO3 与 Python 闭合校验后交给请求注入器。Python 只接受已确认 revision 的原生档案，失败时保留原 G0 请求。

**技术栈：** Rust 2021、serde/serde_json、PyO3、Python 3.12、pytest、ruff、Cargo。

---

## 文件职责和写入边界

| 文件 | 职责 |
| --- | --- |
| crates/ae-attention/Cargo.toml | 允许 R7 attention 使用标准 neurofield 区域布局 |
| crates/ae-attention/src/r7.rs | 15 维到九区域、区域到节点的固定负载路由 |
| crates/ae-attention/tests/rc2_semantic_routes.rs | 每一个维度的路由与节点覆盖回归 |
| crates/ae-runtime/src/lib.rs | 表达档案数据类型、字段均值、原子提交和去重返回 |
| crates/ae-runtime/src/r7.rs | 用逐节点负载更新 next field |
| crates/ae-runtime/tests/durable_semantic_authority.rs | 持久化、去重、重载和档案变化回归 |
| crates/ae-pyo3/src/lib.rs | 把白名单 expression_projection 放入原生 JSON 边界 |
| astr_embodiment/bridge.py | 严格验证并重建 expression_projection |
| main.py | 同轮 affect context、v2 observatory 与失败隔离 |
| tests/test_semantic_bridge.py | Python bridge 的闭合 JSON 回归 |
| tests/test_runtime_integration.py | 注入顺序、一次性、日志和隐私回归 |

本计划不修改版本号、README、CHANGELOG、GitHub workflow 或发布脚本；这些只属于 2026-08-23-rc2-release-automation.md。

### Task 1：写出 15 维区域路由的 RED 测试

**文件：**

- 修改：crates/ae-attention/Cargo.toml
- 创建：crates/ae-attention/tests/rc2_semantic_routes.rs
- 修改：crates/ae-attention/src/r7.rs

- [ ] **步骤 1：添加期望的 15 维表驱动集成测试**

测试为每个 EvidenceVector 字段单独置 1000000，调用 r7::assemble_load，并验证主区域非零、次区域非零或为空、非目标区域为零、active_nodes 覆盖对应的 REGION_LAYOUT 切片。

~~~rust
#[test]
fn every_semantic_dimension_has_only_its_declared_region_loads() {
    for case in CASES {
        let mut evidence = EvidenceVector::default();
        case.set(&mut evidence, Fixed::ONE);
        let load = assemble_load(&evidence, NEURON_SLOTS as u32);
        assert_eq!(load.regional_loads.len(), REGION_LAYOUT.len());
        assert_eq!(load.regional_loads[case.primary], Fixed::ONE);
        assert_eq!(load.regional_loads[case.secondary], Fixed::from_raw(500_000));
        assert!(load.active_nodes.iter().all(|node| case.contains(*node)));
        assert_eq!(load.active_nodes.len(), case.expected_node_count());
    }
}
~~~

同文件另加全零证据测试，断言 active_nodes 和 node_loads 皆为空，九个 regional_loads 全为 Fixed::ZERO。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

~~~text
cargo test -p ae-attention --test rc2_semantic_routes --locked --offline
~~~

预期：因 node_loads、区域路由或依赖尚不存在而编译失败，不能是测试夹具拼写错误。

- [ ] **步骤 3：以最小实现加入固定路由**

在 crates/ae-attention/Cargo.toml 增加 ae-neurofield.workspace = true。

在 r7.rs 用九个固定区域索引和 15 条不可变 RouteRule 替代四维 intensity。LoadCandidate 保留 active_nodes 与 regional_loads，并新增与 active_nodes 等长的 node_loads。区域非零时，遍历 ae_neurofield::REGION_LAYOUT 对应的完整范围，向 active_nodes 追加每个节点并向 node_loads 追加该区域信号。

~~~rust
pub struct LoadCandidate {
    pub active_nodes: Vec<u32>,
    pub node_loads: Vec<Fixed>,
    pub regional_loads: Vec<Fixed>,
    pub route_digest: [u8; 32],
}

for (region, &(start, count)) in REGION_LAYOUT.iter().enumerate() {
    let signal = regional_loads[region];
    if signal == Fixed::ZERO {
        continue;
    }
    for node in start..start + count {
        active_nodes.push(u32::try_from(node).expect("canonical node count"));
        node_loads.push(signal);
    }
}
~~~

每个维度的主系数是 Fixed::ONE，次系数是 Fixed::from_raw(500_000)。区域总和使用 saturating_add，且在本函数中不乘 confidence。

- [ ] **步骤 4：运行 RED 测试并确认 GREEN**

运行：

~~~text
cargo test -p ae-attention --test rc2_semantic_routes --locked --offline
~~~

预期：全部通过；每个非零区域的 active_nodes 与 node_loads 长度相同，未发生重复节点。

- [ ] **步骤 5：提交路由变化**

~~~text
git add crates/ae-attention/Cargo.toml crates/ae-attention/src/r7.rs crates/ae-attention/tests/rc2_semantic_routes.rs
git commit -m "feat: route all semantic dimensions natively"
~~~

### Task 2：让 runtime 原子提交表达档案

**文件：**

- 修改：crates/ae-runtime/src/lib.rs
- 修改：crates/ae-runtime/src/r7.rs
- 修改：crates/ae-runtime/tests/durable_semantic_authority.rs

- [ ] **步骤 1：为确认的 decision 写 RED 测试**

在 durable_semantic_authority.rs 使用现有 genesis/scope/proposal fixture，添加三类测试：

~~~rust
#[test]
fn confirmed_semantic_commit_returns_closed_expression_for_its_revision() {
    let decision = runtime.apply_perception_proposal_v1(&scope, &proposal).unwrap();
    assert_eq!(decision.expression_projection.revision, decision.revision);
    assert!(decision.expression_projection.profile_fxp6.values().into_iter()
        .all(|value| (0..=1_000_000).contains(&value)));
}
~~~

第二个测试按 positive、affiliation、engagement 序列和 harm、boundary、hostility、rejection 序列分别提交，断言 warmth 或 engagement 不相同，且 sensitivity 或 guardedness 不相同。第三个测试 close、reopen、再提交新事件，断言重载路径返回的档案由恢复 field 推导而非 Python 缓存。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

~~~text
cargo test -p ae-runtime --test durable_semantic_authority --locked --offline
~~~

预期：因为 PerceptionProposalDecisionV1 尚无 expression_projection 字段而失败。

- [ ] **步骤 3：先修正逐节点场更新**

在 r7.rs 的 prepare_production_user_stimulus_transition_v1 中，将位置取模 regional_loads 的循环替换为成对遍历 active_nodes 与 node_loads：

~~~rust
for (&node, &load) in prepared_load.active_nodes.iter().zip(&prepared_load.node_loads) {
    let regional_signal = load.checked_mul(stimulus.evidence.estimator_confidence)
        .ok_or(RuntimeError::InvalidSemanticEstimate)?;
    let index = usize::try_from(node).map_err(|_| RuntimeError::InvalidNeuralField)?;
    next_field.potential[index] = next_field.potential[index].saturating_add(regional_signal);
    next_field.excitation[index] = next_field.excitation[index].saturating_add(regional_signal);
}
~~~

在更新前拒绝 active_nodes/node_loads 长度不等、节点越界或重复节点的 LoadCandidate。

- [ ] **步骤 4：实现闭合 expression_projection**

在 lib.rs 定义仅含 revision 与六个 u32 fxp6 数值的 ExpressionProjectionV1，以及固定 schema 常量。用 i128 累加区域 potential 和 excitation raw 值，除以区域节点数与两个通道数，再 clamp 到 0..1000000，避免定点累加溢出。

~~~rust
pub struct ExpressionProfileFxP6 {
    pub warmth: u32,
    pub sensitivity: u32,
    pub guardedness: u32,
    pub repair_orientation: u32,
    pub engagement: u32,
    pub epistemic_caution: u32,
}

impl ExpressionProfileFxP6 {
    pub fn values(&self) -> [u32; 6] {
        [self.warmth, self.sensitivity, self.guardedness,
         self.repair_orientation, self.engagement, self.epistemic_caution]
    }
}

pub struct ExpressionProjectionV1 {
    pub revision: u64,
    pub profile_fxp6: ExpressionProfileFxP6,
}
~~~

将其加入 PerceptionProposalDecisionV1。新提交路径从 prepared.next_field 和 next_revision 生成它；重复路径先 bind_hot，再从 hot.field 和 row.revision 生成它。任何 store 错误在档案生成前返回，任何无效 field 返回现有 InvalidNeuralState。

同时把 native_formula_digest 的 attention 输入说明从四维改为 all-15 regional routing，使规则变更能改变公式身份。

- [ ] **步骤 5：运行 runtime 测试并确认 GREEN**

运行：

~~~text
cargo test -p ae-runtime --test durable_semantic_authority --locked --offline
~~~

预期：新测试与既有 durable semantic tests 全部通过；重复提交不改变 revision。

- [ ] **步骤 6：提交原子档案变化**

~~~text
git add crates/ae-runtime/src/lib.rs crates/ae-runtime/src/r7.rs crates/ae-runtime/tests/durable_semantic_authority.rs
git commit -m "feat: derive expression from committed semantic field"
~~~

### Task 3：把档案通过 PyO3 和 bridge 保持闭合

**文件：**

- 修改：crates/ae-pyo3/src/lib.rs
- 修改：astr_embodiment/bridge.py
- 修改：tests/test_semantic_bridge.py

- [ ] **步骤 1：写 PyO3 与 bridge 的 RED 测试**

在 ae-pyo3 的现有 semantic_perception_payload 单元测试中要求精确的顶层字段 expression_projection。tests/test_semantic_bridge.py 中增加一个有效 payload 与以下无效变体：额外键、未知 schema、bool 数值、越界值、漏键、乱序键、revision 不匹配。

~~~python
def test_bridge_rejects_expression_profile_with_mismatched_revision() -> None:
    payload = confirmed_payload()
    payload["expression_projection"]["revision"] += 1
    assert bridge.apply_perception_proposal_v1(scope(), proposal()) == {
        "status": "DEGRADED",
        "code": "NATIVE_MALFORMED",
    }
~~~

- [ ] **步骤 2：运行专项测试并确认 RED**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_semantic_bridge.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-expression
~~~

预期：缺少 expression_projection 或缺少严格校验导致失败。

- [ ] **步骤 3：实现白名单 JSON 边界**

在 PyO3 semantic_perception_payload 中创建固定顺序 JSON：

~~~rust
"expression_projection": {
    "schema": "astr-embodiment.expression-projection.v1",
    "revision": decision.expression_projection.revision,
    "profile_fxp6": {
        "warmth": decision.expression_projection.profile_fxp6.warmth,
        "sensitivity": decision.expression_projection.profile_fxp6.sensitivity,
        "guardedness": decision.expression_projection.profile_fxp6.guardedness,
        "repair_orientation": decision.expression_projection.profile_fxp6.repair_orientation,
        "engagement": decision.expression_projection.profile_fxp6.engagement,
        "epistemic_caution": decision.expression_projection.profile_fxp6.epistemic_caution,
    }
}
~~~

在 bridge.py 扩展 _SEMANTIC_RESULT_FIELDS，新增私有 _validate_expression_projection，按规格重建纯 dict。_validate_semantic_result 只在回执确认、revision 相等、六键顺序完全正确、每值为非 bool 整数且在范围内时返回 expression_projection。任何不合格原生输出走既有 NATIVE_MALFORMED 降级路径。

- [ ] **步骤 4：运行 Python 与 PyO3 GREEN**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_semantic_bridge.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-expression
cargo test -p astrembodiment-core --locked --offline
~~~

预期：所有闭合 schema 变体按预期通过或降级，没有原生私有字段透传。

- [ ] **步骤 5：提交跨语言闭合边界**

~~~text
git add crates/ae-pyo3/src/lib.rs astr_embodiment/bridge.py tests/test_semantic_bridge.py
git commit -m "feat: expose closed expression projection"
~~~

### Task 4：同轮注入、v2 observatory 和失败隔离

**文件：**

- 修改：main.py
- 修改：tests/test_runtime_integration.py

- [ ] **步骤 1：写注入与日志的 RED 测试**

在 test_runtime_integration.py 的 FakeRequest/FakeContext 测试域添加：

~~~python
def test_confirmed_expression_is_injected_after_g0_before_host_call() -> None:
    instance, request = run_hook_with(confirmed_outcome(profile=profile(700_000)))
    assert request.system_prompt.index("[AE Runtime Context / v1]") < request.system_prompt.index("[AE Affect Expression Context / v1]")
    assert request.system_prompt.count("[AE Affect Expression Context / v1]") == 1
    assert "warmth=700000" in request.system_prompt

def test_rejected_expression_leaves_only_g0_and_warns() -> None:
    instance, request, record = run_hook_with(confirmed_outcome(profile={"warmth": True}))
    assert "[AE Affect Expression Context / v1]" not in request.system_prompt
    assert record["expression_state"] == "REJECTED"
~~~

再加入 at-most-once、request 属性写入失败回滚、语义 DEGRADED、raw text/digest/node sentinel 不泄露、SUCCESS 日志为 APPLIED 的测试。

- [ ] **步骤 2：运行测试并确认 RED**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_runtime_integration.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-runtime
~~~

预期：因为 affect marker、v2 字段和 helper 尚不存在而失败。

- [ ] **步骤 3：实现最小安全注入**

在 main.py 增加固定 profile 名称、独立 request marker 和 _inject_expression_projection。该 helper 只接收已由 bridge 重建的六整数 dict，先保存原 system_prompt，追加固定英文模板，再设置 marker；任一 BaseException 回滚 prompt 并返回 INJECTION_FAILED。

在 on_llm_request 中保留既有 G0 注入位置。preflight 返回后调用校验与注入 helper，再调用 _emit_semantic_observatory。不得把 raw_outcome 的任意未校验字段写入 request。

将 _SPC1_OBSERVATORY_SCHEMA 更新为 v2，固定新增 expression_state 与 expression_profile_fxp6。_semantic_observatory_record 对 APPLIED、NOT_ATTEMPTED、UNAVAILABLE、REJECTED、INJECTION_FAILED 逐一白名单校验；INFO 仅用于 SUCCESS/NOOP 且无表达错误，表达 REJECTED 或 INJECTION_FAILED 使用 WARNING。

- [ ] **步骤 4：运行专项测试并确认 GREEN**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_runtime_integration.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-runtime
~~~

预期：所有新旧 runtime integration 测试通过；语义失败不会停止已有效的 G0 请求。

- [ ] **步骤 5：提交同轮行为**

~~~text
git add main.py tests/test_runtime_integration.py
git commit -m "feat: inject confirmed expression in current turn"
~~~

### Task 5：跨层回归、格式化与小范围重构

**文件：**

- 修改：仅 Task 1 至 Task 4 为修复格式或测试失败所必需的文件

- [ ] **步骤 1：执行受影响的完整回归**

运行：

~~~text
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.venv\Scripts\python.exe -m pytest tests/test_semantic_bridge.py tests/test_runtime_integration.py tests/test_semantic_estimator.py -q -o cache_dir=G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\pytest-rc2-expression-full
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
ruff format --check main.py astr_embodiment tests
ruff check main.py astr_embodiment tests
git diff --check
~~~

预期：全绿。若格式工具提出纯格式变更，先应用格式化，再重新运行同一命令。

- [ ] **步骤 2：确认提交边界**

运行：

~~~text
git status --short
git log --oneline --max-count=5
~~~

预期：只出现本计划的 native/Python 变化；版本与 GitHub workflow 尚未在本计划中触碰。

- [ ] **步骤 3：提交仅为回归所需的修复**

~~~text
git add crates/ae-attention crates/ae-runtime crates/ae-pyo3 astr_embodiment main.py tests
git commit -m "test: cover rc2 expression integration"
~~~

若没有未提交的回归修复，不创建空提交。
