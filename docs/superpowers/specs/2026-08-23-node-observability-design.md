# RC2 节点计算可观察性设计规格

日期：2026-08-23
任务：`AE-RC2-NODE-OBSERVABILITY-SPEC-20260823`
基线：`80cec9c829ad4606bc4cd77f88b5c4adc44b730d`
状态：方向已批准，本文仅定义设计，不授权实现或发布

## 1. 结论

唯一推荐是：**保持 canonical `TransitionReceipt` 的 `schema_version=1` 和二进制编码完全不变，在 `apply_perception_proposal_v1` 的外层成功结果中增加一个兼容可选、带独立版本的 `node_observability` native projection；同时把生产 observatory 日志从 v2 升为 v3。**

具体边界如下：

- `TransitionReceipt.active_nodes` 保留现名、现值和现有 wire 语义；它准确表示本次 attention/load assembler 选出的节点索引数量，不表示实际改变、变化后非零或全场容量。
- 新投影 schema 固定为 `astr-embodiment.node-observability.v1`。新 native 对成功的新提交和去重命中都必须给出同一份可重建结果；旧 native 可省略该字段。
- Python 对“字段缺失”和“字段存在但非法”分别记录 `UNAVAILABLE` 与 `REJECTED`，绝不补零。
- observatory 日志 v3 明确输出 `selected_node_count`、`activated_node_count`、`changed_node_count`、变化后非零节点计数、九个 region 的 `potential`/`excitation` 聚合与 delta，以及 residual 的计算状态。
- 新增配置 `node_observability_detailed_logging`，类型为 bool、默认 `false`。关闭时成功只输出包含严格 15 维值的简洁中文单行；开启时才输出完整 observatory v3 JSON。失败在任一模式下都必须记录固定失败码与阶段。
- 当前 RC2 路径没有计算五项 invariant residual，因此日志必须输出 `residuals.state="NOT_COMPUTED"`、`formula=null`、`values_fxp6=null`。receipt 中现有五个默认零不得继续被解释或展示为实际 residual 计算结果。
- 禁止逐节点输出。计算只允许固定一次 16,384 节点扫描，输出固定九个 region，单个 projection 的 canonical compact JSON 上限为 16,384 bytes。

## 2. 已核实的当前语义

### 2.1 `active_nodes` 的准确含义

当前语义提交在 `crates/ae-runtime/src/r7.rs` 的 `prepare_production_user_stimulus_transition_v1` 中调用 `assemble_load`。assembler 对 15 维证据做固定 region 路由，只把 regional load 非零且位于 `node_limit` 内的 region 节点放入 `load.active_nodes`。运行时校验索引不越界、不重复，并逐一尝试更新其 `potential` 和 `excitation`。随后：

```text
TransitionReceipt.active_nodes = len(load.active_nodes)
```

因此，本规格采用以下规范定义：

> `active_nodes` 是本次 transition 中经 attention/load assembler 选出、通过唯一性和边界校验、进入写入循环的节点索引数。

它不是以下任何含义：

- 不是 `NEURON_SLOTS=16_384` 的总容量；
- 不是本次实际发生数值变化的节点数；
- 不是变化后值非零的节点数；
- 不是 potential、excitation、能量或 residual 的总和；
- 不是累计计数；
- 不是“15 个维度都非零”的证据。

当前审计样例中，`positive`、`affiliation`、`engagement` 三个非零证据经固定路由命中 region 1、7、8；其容量分别为 2,048、1,024、1,024，所以 receipt 的 `active_nodes=4096` 与稀疏输入完全一致。其余 12 维为零表示当前消息未提供相应证据，不能为了让日志“看起来完整”而填成非零。

### 2.2 16k、状态变化与非零值不是同一概念

`NEURON_SLOTS=16_384` 是九个固定 region 的总容量。神经场在 Genesis 后并非简单的全零数组，且历史 transition 可以使本次未选中的节点继续保持非零。因此：

- `selected_node_count` 只描述本次候选写集；
- `changed_node_count` 只描述本次 before/after 的真实变化；
- `signal_nonzero_after_count` 描述 transition 后全场 `potential != 0 || excitation != 0` 的节点数，可能包含大量本次未选中节点；
- 三者不得互相替代。

### 2.3 residual 当前没有被计算

`InvariantResiduals` 的五个字段是 `authority`、`continuity`、`energy`、`renormalization`、`capacity`。当前 semantic commit 直接写入 `InvariantResiduals::default()`，即五个固定零；这只是路径尚未计算/输出 residual 的事实，不证明 invariant 已以零残差通过，也不证明节点值为零。

## 3. 选型与兼容策略

### 3.1 不选择 receipt v2

`TransitionReceipt` 是固定顺序的 canonical binary wire，参与持久化、digest、replay 和 Python 闭合字段校验。把九个 region 的投影直接加入 receipt 会同时要求：

- 升级 canonical codec 与 domain/version；
- 迁移历史 journal/receipt 解码；
- 修改 receipt identity 与链式摘要；
- 扩大所有非 semantic lane 的公共合同；
- 为纯可观察性承担不必要的持久化兼容风险。

本次需求只要求真实、受限的日志可观察性，不要求节点统计成为 transition 的 canonical identity。因此不升 receipt schema。

### 3.2 推荐的兼容 optional projection

外层 `astrembodiment.semantic-perception-closure.v1` 已采用可选 projection 模式。新增字段：

```json
{
  "node_observability": {
    "schema": "astr-embodiment.node-observability.v1"
  }
}
```

兼容规则固定为：

1. 新 native 在 `SUCCESS / SEMANTIC_COMMITTED` 时必须输出合法 projection。
2. 旧 native 可以省略该字段；Python 保留成功 commit，但标记 `node_observability_state="UNAVAILABLE"`。
3. 字段存在但 schema、字段集合、顺序相关数组、数值范围、revision 或不变量非法时，Python 丢弃整个 projection，标记 `REJECTED`，不得透传部分字段或原始对象。
4. projection 不参与 receipt digest，不改变 journal identity，不改变 semantic revision。
5. observatory 日志字段集合发生变化，所以日志 schema 必须由 `astr-embodiment.observatory.semantic-injection.v2` 升为 `astr-embodiment.observatory.semantic-injection.v3`；不得在 v2 名义下静默增字段。

### 3.3 新提交与 dedup 必须一致

新提交路径已有 before field、prepared after field、事件 load 和最终 revision，可在 commit 成功后返回预先计算并校验过的 projection。

去重路径不得把 after-only 状态冒充本次 delta。它必须重建同一对状态：

- `base_revision > 0`：读取 exact base revision snapshot；
- `base_revision == 0`：通过现有 Genesis/semantic hydration 规则重建 initial semantic field；
- 读取 `next_revision` 的 exact committed snapshot 并核对 `state_after`；
- 用已核对 event/proposal 重新组装同一 selected set；
- 对 before/after 运行同一个纯 projection helper。

若任一 exact snapshot、digest 或 revision 绑定不可用，native 必须返回固定错误并由现有语义路径降级；不得用当前 latest field、全零 field 或估算值替代。第一次提交、同进程 dedup、重启后 dedup 的 canonical projection 必须字节一致。

## 4. `node_observability.v1` 数据合同

### 4.1 顶层字段

字段集合固定且拒绝未知字段：

```json
{
  "schema": "astr-embodiment.node-observability.v1",
  "formula": "spc1-node-observability-v1",
  "revision": 3,
  "field_node_capacity": 16384,
  "region_layout": "regions-v1",
  "counts": {},
  "residuals": {},
  "regions": []
}
```

- `revision` 必须等于外层 result revision 和 receipt `next_revision`。
- `field_node_capacity` 在 v1 中必须精确等于 `16384`。
- `region_layout` 在 v1 中必须精确等于 `regions-v1`。
- `regions` 必须恰有九项，按 `region_id=0..8` 升序；不得缺项、重复或重排。

### 4.2 全局节点计数

`counts` 固定包含：

```json
{
  "selected_node_count": 4096,
  "activated_node_count": 4096,
  "changed_node_count": 4096,
  "potential_nonzero_after_count": 16384,
  "excitation_nonzero_after_count": 16384,
  "signal_nonzero_after_count": 16384
}
```

规范语义：

- `selected_node_count`：assembler 选出的唯一节点数；必须等于 receipt `active_nodes`。
- `activated_node_count`：selected 节点中，乘以 `estimator_confidence` 后的有效 write drive 严格大于零的节点数。它描述“本次收到了正向激活输入”，不描述 after 值是否非零。
- `changed_node_count`：全场节点中，`potential_before != potential_after` 或 `excitation_before != excitation_after` 的唯一节点数。当前 transition 只准修改 selected set，因此必须满足 `changed_node_count <= activated_node_count <= selected_node_count`；发现 selected set 外变化时 projection 生成失败。
- `potential_nonzero_after_count`：全场 after state 中 `potential != 0` 的节点数。
- `excitation_nonzero_after_count`：全场 after state 中 `excitation != 0` 的节点数。
- `signal_nonzero_after_count`：全场 after state 中 `potential != 0 || excitation != 0` 的节点并集数。

所有计数均为 JSON integer，范围固定在 `0..16384`。当前实现通常有 `activated_node_count == selected_node_count`，但饱和节点可能满足“收到正向 drive、实际未变化”，所以不得把 activated 与 changed 合并。

### 4.3 region 聚合与 delta

九个 region 的名称和容量固定为：

| `region_id` | `region_name` | `node_capacity` |
|---:|---|---:|
| 0 | `interoception_allostasis` | 2048 |
| 1 | `affective_valuation` | 2048 |
| 2 | `salience` | 1024 |
| 3 | `epistemic_fallibility` | 2048 |
| 4 | `social_boundary` | 2048 |
| 5 | `temper_inhibitory` | 1024 |
| 6 | `world_model_imagination` | 4096 |
| 7 | `global_workspace` | 1024 |
| 8 | `action_expression` | 1024 |

每个 region 固定为：

```json
{
  "region_id": 1,
  "region_name": "affective_valuation",
  "node_capacity": 2048,
  "selected_node_count": 2048,
  "activated_node_count": 2048,
  "changed_node_count": 2048,
  "potential": {
    "before_mean_fxp6": 100,
    "after_mean_fxp6": 110,
    "delta_mean_fxp6": 10,
    "changed_node_count": 2048,
    "nonzero_after_count": 2048
  },
  "excitation": {
    "before_mean_fxp6": 100,
    "after_mean_fxp6": 110,
    "delta_mean_fxp6": 10,
    "changed_node_count": 2048,
    "nonzero_after_count": 2048
  }
}
```

聚合规则：

- mean 覆盖该 region 的全部节点，不只覆盖 selected 节点。
- `before_mean_fxp6` 和 `after_mean_fxp6` 分别用 signed i128 累加 raw fixed-point 值，再按节点数做向零截断的整数除法；输出必须落入 signed i64。
- `delta_mean_fxp6` 直接对每个节点的 `(after - before)` 用 signed i128 求和后取均值，不得用两个已截断 mean 相减代替。
- 内层 `changed_node_count` 分别表示该分量发生变化的节点数；region 顶层 `changed_node_count` 是两分量变化节点的并集。
- 所有 region 的 selected、activated、changed 计数之和必须分别等于全局同名计数；所有 `node_capacity` 之和必须为 16,384。
- 当前 RC2 transition 只改变 potential 和 excitation；若任何其他 NeuralField 分量在 before/after 间改变，本投影必须拒绝生成，而不是静默忽略。

### 4.4 residual 状态

`residuals` 固定为：

```json
{
  "state": "NOT_COMPUTED",
  "formula": null,
  "values_fxp6": null
}
```

v1 的当前 RC2 实现只允许上述形态。它明确表示 receipt 内的默认零没有物理计算语义。未来若确实实现 residual 公式，必须使用新的 projection schema 版本，并在新版本中同时绑定非空 formula 标识和五个闭合值；不得在 v1 下把 `state` 改成 `COMPUTED`。

## 5. 配置合同

### 5.1 新配置及中文 Schema

配置名固定为 `node_observability_detailed_logging`。未来 `_conf_schema.json` 中必须按现有 AstrBot 配置格式增加：

```json
{
  "node_observability_detailed_logging": {
    "description": "输出完整节点可观察性日志",
    "hint": "默认关闭；关闭时仅记录简洁中文结果，开启时记录包含节点计数与区域聚合的完整 JSON。失败始终记录失败码与阶段。",
    "type": "bool",
    "default": false
  }
}
```

解析必须严格：仅 Python 原生 bool 有效；缺失或任何非 bool 值都按 `false` 处理。环境变量、字符串 `"true"`、整数 `1` 和真值对象都不得隐式开启详细日志。

### 5.2 与现有 `observatory_enabled` 的兼容和优先级

现有 `observatory_enabled` 保留，不删除、不改名，避免旧配置加载失败。两个配置只控制日志可见性/格式，不控制 native 是否计算：

| `observatory_enabled` | `node_observability_detailed_logging` | 成功与 NOOP | 失败 |
|---|---|---|---|
| `true` 或缺失默认 | `false` 或非法 | 简洁中文 | 简洁中文 WARNING |
| `false` 或非法 | `false` 或非法 | 保持旧行为，不输出例行成功/NOOP | 简洁中文 WARNING，不能被抑制 |
| 任意值 | `true` | 完整 v3 JSON | 完整 v3 JSON WARNING |

因此，新配置显式 `true` 优先于旧 master switch，并完整开启本规格的 observatory；新配置默认 `false` 不会让现有默认安装继续打印大 JSON。旧配置显式 `false` 仍能抑制例行成功/NOOP，但基于新增审计要求，不再允许抑制失败记录。

### 5.3 关闭详细日志时的固定中文格式

成功必须恰为一条单行 INFO，固定前缀和字段顺序如下：

```text
AstrEmbodiment：运算已完成｜十五维：positive=350000,affiliation=250000,harm=0,boundary=0,repair=0,repetition=0,new_information=0,constraint_instability=0,epistemic_conflict=0,self_responsibility=0,other_responsibility=0,hostility=0,publicness=0,engagement=600000,rejection=0
```

冒号后必须按 `DIMENSION_NAMES` 的固定顺序写满 15 个 `name=raw_fxp6_integer`，不得省略零值、改为浮点、只写非零维、折行或附加 user 原文。上例对应已审计的稀疏向量，只展示格式，不要求其他消息产生相同数值。新提交和 dedup 都使用“运算已完成”，不在简洁行中伪造节点细节。

失败必须是一条单行 WARNING：

```text
AstrEmbodiment：运算失败｜失败码=NATIVE_ERROR｜阶段=NATIVE_APPLY
```

`失败码` 与 `阶段` 只能来自现有固定 allowlist。不得追加异常文本、原始 Provider 输出或任意上游对象。即使 `observatory_enabled=false`，该失败行也必须输出。

NOOP 不是失败，固定写为：

```text
AstrEmbodiment：未执行运算｜原因=ZERO_LOAD｜十五维：positive=0,affiliation=0,harm=0,boundary=0,repair=0,repetition=0,new_information=0,constraint_instability=0,epistemic_conflict=0,self_responsibility=0,other_responsibility=0,hostility=0,publicness=0,engagement=1,rejection=0
```

ZERO_LOAD 在 estimate 可用时同样写满 15 维；其含义仍是 load 维度没有形成 native write set，而不是“节点算出了零”。`EMPTY_REQUEST` 没有合法 estimate，固定写 `十五维：不可用`。NOOP 是否输出继续服从上一节的兼容矩阵。

## 6. 日志语义

### 6.1 固定三态

observatory v3 的 `calculation_state` 只允许：

| 值 | 含义 | 数值字段规则 |
|---|---|---|
| `SUCCEEDED` | validated receipt 证明 semantic calculation 已成功产生状态变化并完成新提交或命中同一提交 | receipt 粗粒度结果可用；新 native 的 node projection 必须为 `CONFIRMED` |
| `FAILED` | 端到端计算管线未产生可验证的成功结果 | 所有节点统计和 region 聚合必须为 `null`；它不等于“算出零” |
| `NOT_EXECUTED` | 输入为空、ZERO_LOAD 或 native apply 之前已停止，计算没有执行 | 所有节点统计和 region 聚合必须为 `null` |

`FAILED` 不自动证明 native 未写入：在 `stage=NATIVE_APPLY|RECEIPT|INTERNAL` 时继续用现有 `commit_state="UNKNOWN"` 表达持久化不确定性。失败原因只允许现有固定 code，不记录异常文本。

### 6.2 node projection 状态

`node_observability_state` 固定为：

- `CONFIRMED`：projection 存在并通过完整闭合校验。
- `UNAVAILABLE`：成功 result 来自兼容旧 native，字段缺失。
- `REJECTED`：字段存在但非法，原始内容已丢弃。
- `NOT_APPLICABLE`：`calculation_state` 为 `FAILED` 或 `NOT_EXECUTED`。

只有 `CONFIRMED` 可以伴随非空 `node_observability`。其他三态必须为 `null`，绝不使用全零对象占位。

### 6.3 详细模式的 v3 日志结构与级别

只有 `node_observability_detailed_logging=true` 时才构造并输出单行 compact JSON，前缀仍为：

```text
AstrEmbodiment SPC1 observatory:
```

现有 status、code、stage、commit/value/revision、15 维证据、confidence、expression 字段继续保留。计算相关字段固定为：

- `schema="astr-embodiment.observatory.semantic-injection.v3"`；
- `calculation_state="SUCCEEDED"`；
- `native_calculation={"state_changed":true,"receipt_active_nodes":4096,"active_edges":0}`；
- `node_observability_state="CONFIRMED"`；
- `node_observability` 必须是第 4 节定义的完整对象，不允许空对象、字段子集或字符串占位。

- `receipt_active_nodes` 是现有 receipt 值的显式改名投影，避免继续让日志读者把 `active_nodes` 误认为 changed 或 nonzero。
- v3 不再输出当前 `residuals_fxp6` 五个默认零；residual 只出现在带状态的 `node_observability.residuals` 中。
- `SUCCESS` 且 projection `CONFIRMED`：INFO。
- `NOOP`：INFO。
- `DEGRADED`：WARNING。
- commit 成功但 projection 为 `UNAVAILABLE` 或 `REJECTED`：WARNING；主 status 仍保持 `SUCCESS`，不得伪造 commit 失败。
- 详细模式的失败记录必须包含固定 `code`、`stage`、`commit_state` 与 calculation/node 状态；节点结果不可用时保持 null，不得用简洁错误行取代完整诊断。

## 7. 成功、失败、未执行验收样例

### 7.1 成功：4096 个节点被选中并真实变化

使用一个闭合 native 测试 fixture：before state 的 16,384 个节点在 potential/excitation 上均为 `100`；region 1、7、8 的全部节点分别收到 `+10`，其余节点不变。应断言：

```json
{
  "status": "SUCCESS",
  "code": "SEMANTIC_COMMITTED",
  "calculation_state": "SUCCEEDED",
  "node_observability_state": "CONFIRMED",
  "node_observability": {
    "schema": "astr-embodiment.node-observability.v1",
    "formula": "spc1-node-observability-v1",
    "revision": 1,
    "field_node_capacity": 16384,
    "region_layout": "regions-v1",
    "counts": {
      "selected_node_count": 4096,
      "activated_node_count": 4096,
      "changed_node_count": 4096,
      "potential_nonzero_after_count": 16384,
      "excitation_nonzero_after_count": 16384,
      "signal_nonzero_after_count": 16384
    },
    "residuals": {
      "state": "NOT_COMPUTED",
      "formula": null,
      "values_fxp6": null
    }
  }
}
```

上方是完整日志中的关键字段断言子集；实际 `node_observability` 还必须带第 4.3 节规定的九项 `regions`，不能直接按该子集发日志。region 1、7、8 的 potential/excitation `before_mean_fxp6=100`、`after_mean_fxp6=110`、`delta_mean_fxp6=10`；其余六个 region 的 before/after mean 均为 `100`、delta mean 为 `0`。该 fixture 同时证明：16k 的 nonzero-after 可以与 4096 的 selected/changed 同时成立，二者不矛盾。

### 7.2 失败：native/receipt 路径没有可验证结果

以下是日志关键字段断言子集，不是独立 wire 对象：

```json
{
  "status": "DEGRADED",
  "stage": "RECEIPT",
  "commit_state": "UNKNOWN",
  "calculation_state": "FAILED",
  "native_calculation": null,
  "node_observability_state": "NOT_APPLICABLE",
  "node_observability": null
}
```

不得出现 selected、activated、changed、nonzero、region 或 residual 零值对象。`FAILED` 只表示未获得可验证成功结果；`commit_state="UNKNOWN"` 继续警告可能已经发生持久化。

### 7.3 未执行：EMPTY_REQUEST 或 ZERO_LOAD

```json
{
  "status": "NOOP",
  "commit_state": "NOT_ATTEMPTED",
  "calculation_state": "NOT_EXECUTED",
  "native_calculation": null,
  "node_observability_state": "NOT_APPLICABLE",
  "node_observability": null
}
```

ZERO_LOAD 可以继续记录经验证的 15 维 estimate，但必须清楚标为 `ESTIMATED_NOT_COMMITTED`；estimate 中的零是“无该维证据”，不是节点计算结果。

### 7.4 旧 native 兼容

旧 native 成功 result 缺少 optional projection 时：

```json
{
  "status": "SUCCESS",
  "calculation_state": "SUCCEEDED",
  "node_observability_state": "UNAVAILABLE",
  "node_observability": null
}
```

日志为 WARNING。它可以继续展示 validated receipt 的 `receipt_active_nodes`，但不得声称 activated、changed、nonzero、region delta 或 residual 结果已知。

## 8. 性能、尺寸与隐私边界

### 8.1 固定性能预算

- projection helper 对 before/after field 做一次顺序扫描，时间复杂度固定为 `O(NEURON_SLOTS)`，当前精确为 16,384 节点。
- 不得对每个 region 重扫全场；region 由节点索引在单次扫描中定位。
- 额外输出只允许九个固定 region 对象；不得分配或返回节点 ID、节点值或节点 delta 数组。
- canonical compact JSON 编码后必须 `<= 16_384 bytes`；超过上限视为 projection `REJECTED`，不得截断后输出。
- `regions.len()` 必须为 9，`field_node_capacity` 必须为 16,384；不允许配置扩大日志基数。
- 统计只用 deterministic integer/fixed-point 运算，不引入浮点、随机采样或近似直方图。
- 详细配置关闭时，logger 不得序列化完整 projection；只从已闭合 estimate 构造固定 15 维中文行，以避免无意义的大 JSON 开销。

### 8.2 明确禁止的内容

native projection、Python diagnostic 和最终日志都不得包含：

- 16,384 个节点的逐节点值、索引、排序或采样；
- user 原文、Provider 原始输出、prompt/history、系统提示、工具输入；
- bot/persona/session/relation/event/turn token；
- SeedCode、incarnation、nonce、formula/scope/event/state/graph digest；
- exception 文本、任意上游 dict、未闭合 extension 对象；
- 可由多次日志拼接恢复用户原文的 token、n-gram、embedding 或向量。

只允许固定 schema 名、formula 名、revision、固定 region 名、整数计数和 fixed-point 聚合。该数据是运行时状态的有损、固定基数统计，不得进入 LLM prompt 或表达策略。

## 9. Python/native 接口边界与最小改动路径

### 9.1 Native owner

Rust 是节点状态、selected set、before/after 差值和 region 聚合的唯一 owner。Python 不得读取 snapshot 后自行计算，也不得从 15 维 estimate 推断节点数。

最小 native 改动集中于：

- `crates/ae-runtime/src/lib.rs`
  - 定义私有/公开只读的 `NodeObservabilityProjectionV1` 闭合类型与纯计算 helper；
  - 在 `PerceptionProposalDecisionV1` 增加 projection；
  - 新提交用真实 before/after 与 selected load 计算；
  - dedup 用 exact base/next snapshot 重建并产生相同结果；
  - 不修改 neural update、attention route、state digest、store schema 或 receipt。
- `crates/ae-pyo3/src/lib.rs`
  - 仅在 semantic perception 成功 payload 中投影闭合 JSON；
  - 不增加新 Python callable，不暴露 NeuralField getter。

明确不修改 `crates/ae-contracts/src/lib.rs` 的 `TransitionReceipt`、wire/domain、`ae-store` schema 或 journal 格式。

### 9.2 Python validator、coordinator 与配置

- `astr_embodiment/bridge.py`
  - 接受旧 payload 和带 `node_observability` 的新 payload 两种闭合字段集合；
  - 逐字段重建新 projection；缺失映射为 `UNAVAILABLE`，非法映射为 `REJECTED`；
  - 永不把 raw projection 继续传递。
- `astr_embodiment/coordinator.py`
  - 只从 bridge 已 canonicalize 的 result 构造 diagnostic；
  - 把 receipt `active_nodes` 显式投影为 `receipt_active_nodes`；
  - 产生固定三态 calculation 语义，不再把默认 residual 零投影为计算值。
- `main.py`
  - 从 `_conf_schema.json` 读取严格 bool `node_observability_detailed_logging`，默认/非法均为 false；
  - observatory schema 升至 v3；
  - 校验 count/region/revision/size/状态交叉不变量；
  - 详细模式输出单行 JSON；关闭详细模式时只输出固定中文成功/NOOP/失败格式；
  - 失败日志绕过旧 `observatory_enabled=false` 的例行抑制，但仍保持 fixed-code、never-raise 和 request-local at-most-once；
  - 不将 node projection 注入 request/system prompt。
- `_conf_schema.json`
  - 只新增本节规定的中文 bool 配置项，默认 false；不改变其他配置默认值。

## 10. TDD 测试矩阵

实现必须遵循先 RED、再最小 GREEN、最后受影响回归。测试文件限定为现有相关测试位置，不因本规格新建生产子系统。

| 层 | RED 测试 | 必须锁定的行为 |
|---|---|---|
| runtime 聚合 | `node_observability_reports_selected_activated_changed_and_nonzero_counts` | 4096 selected/activated/changed 与 16384 nonzero-after 可同时成立；九 region 求和不变量成立 |
| runtime 饱和 | `node_observability_distinguishes_activated_from_changed_nodes` | 部分节点收到正向 drive 但饱和不变，`changed < activated <= selected` |
| runtime 越界/旁路变化 | `node_observability_rejects_out_of_scope_field_changes` | selected 外变化、region 不一致、数值溢出均失败，不产部分 projection |
| runtime 去重 | `node_observability_is_identical_for_new_deduplicated_and_reopened_results` | 新提交、同进程 dedup、重启 dedup 的 canonical JSON 字节一致 |
| runtime residual | `semantic_node_observability_marks_default_residuals_not_computed` | 五个 receipt 零不会变成 `COMPUTED` 值 |
| PyO3 | `semantic_perception_payload_exposes_only_bounded_node_observability` | 只有固定聚合；无节点数组、token、digest；编码不超过 16,384 bytes |
| bridge | `test_bridge_rebuilds_confirmed_node_observability` | 完整闭合重建、revision/计数/region 交叉校验 |
| bridge 兼容 | `test_bridge_distinguishes_missing_and_malformed_node_observability` | 缺失为 `UNAVAILABLE`，非法为 `REJECTED`，raw sentinel 不泄漏 |
| coordinator | `test_preflight_calculation_never_presents_default_residual_zero_as_result` | residual 是状态化 null，不是五个零 |
| observatory 成功 | `test_spc1_observatory_v3_logs_confirmed_node_counts_and_regions` | INFO、真实 4096 active、九 region、完整状态语义 |
| observatory 失败 | `test_spc1_observatory_v3_failure_has_no_numeric_defaults` | WARNING、`FAILED`、所有计算对象为 null |
| observatory 未执行 | `test_spc1_observatory_v3_noop_is_not_executed` | INFO、`NOT_EXECUTED`，estimate zero 与 node result 明确分离 |
| 配置默认 | `test_node_observability_detailed_logging_defaults_false_and_rejects_truthy_values` | 缺失/字符串/整数/对象不打开详细 JSON，Schema 描述为中文且 default=false |
| 简洁成功 | `test_compact_success_log_contains_exactly_fifteen_ordered_dimensions` | 含“运算已完成｜十五维：”，严格 15 个有序整数值，无大 JSON/节点对象 |
| 模式开关 | `test_detailed_switch_selects_complete_v3_json` | false 为简洁中文，true 为包含 active/changed/nonzero/regions 的完整 v3 JSON |
| 失败强制记录 | `test_failure_is_logged_with_code_and_stage_in_both_modes` | 关闭时简洁中文；开启时完整诊断；`observatory_enabled=false` 也不能压掉失败 |
| 隐私/尺寸 | `test_spc1_observatory_v3_is_bounded_and_content_free` | user/raw/digest/node sentinel 不出现，单行 JSON 大小受限 |

聚焦命令固定为：

```powershell
cargo test --locked -p ae-runtime node_observability
cargo test --locked -p ae-pyo3 semantic_perception_payload
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.\.venv\Scripts\python.exe -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\evidence\ae-rc2-node-observability-spec-20260823\pytest-cache' tests\test_semantic_bridge.py tests\test_runtime_integration.py
```

GREEN 后受影响验收至少包括：

```powershell
cargo fmt --all -- --check
cargo clippy --locked -p ae-runtime -p ae-pyo3 --all-targets -- -D warnings
cargo test --locked -p ae-runtime -p ae-pyo3
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.\.venv\Scripts\python.exe -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\evidence\ae-rc2-node-observability-spec-20260823\pytest-cache' tests\test_semantic_bridge.py tests\test_runtime_integration.py tests\test_release_contracts.py
```

这些命令是未来实现的验收合同，不是本文已执行的测试证据。完整 workspace/release 验收仍由后续独立任务决定。

## 11. 明确不做

本规格不授权、也不设计以下变更：

- 不改变 15 维情感/语义估计器、prompt、confidence 或稀疏零语义；
- 不强迫 15 维全部非零，不新增维度之间的自动填充或归一化；
- 不改变 attention route、regional load、神经动力学、Fixed 运算、饱和规则或 NeuralField 更新；
- 不改变 expression profile、表达注入、ActionContract、Persona 或回复策略；
- 不暴露逐节点 API，不打印或采样 16k 节点值；
- 不改变 `TransitionReceipt` schema、canonical wire、digest、journal/store schema、revision 或 replay identity；
- 不修改 README、CHANGELOG、版本号、release metadata、CI、打包、ZIP、tag、Release；
- 不 push、不建 PR、不发布。

## 12. 完成判据

未来实现只有同时满足以下条件，才能称为节点可观察性完成：

1. 真实 native before/after 产生并校验 `node_observability.v1`，不是 Python 推断。
2. 日志明确写出 `receipt_active_nodes` 与 selected/activated/changed/nonzero 的不同语义。
3. 当前 residual 显示为 `NOT_COMPUTED + null`，日志中不再出现会被误认的默认五零结果。
4. 成功、失败、未执行三态不互相冒充，任何不可用值均为 null 而不是零。
5. 新提交、dedup、重启 dedup 结果一致；旧 native 缺失字段只降级可观察性，不伪造统计。
6. 输出固定九 region、无逐节点内容、无用户原文，compact JSON 不超过 16,384 bytes。
7. `node_observability_detailed_logging` 为中文 Schema bool 且默认 false；关闭时成功行严格包含 15 个有序维度，开启时输出完整 v3 JSON。
8. 失败在两种模式及 `observatory_enabled=false` 下都记录固定失败码与阶段；关闭时简洁中文，开启时完整诊断。
9. 聚焦 RED→GREEN、受影响回归、静态检查均有当前命令证据。
10. 生产、测试和 CI 的实际修改必须由后续获授权实现任务完成；本文提交本身不构成实现、集成或 release 证据。
