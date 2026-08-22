# RC2 节点计算可观察性设计规格

日期：2026-08-23
任务：`AE-RC2-NODE-OBSERVABILITY-SPEC-20260823`
基线：`80cec9c829ad4606bc4cd77f88b5c4adc44b730d`
状态：方向已批准，本文仅定义设计，不授权实现或发布

## 1. 结论

唯一推荐是：**把 semantic transition 升为 full-vector receipt v2：每个非空 user semantic turn 的 15 个维度必须全部取得明确数值、全部进入 native route 计算；数值 `0` 是可用的中性基线/回落输入，不是“缺失”或“不执行”。节点仍按真实有效 delta 稀疏选择，并用独立 `node_observability.v1` projection 输出聚合；生产 observatory 日志继续使用 v3。没有用户语义输入的 `EMPTY_REQUEST` 不构成 semantic turn。**

具体边界如下：

- `TransitionReceipt.active_nodes` 保留节点层含义：它表示本次 full-vector dynamics 产生非零有效 delta 后进入写入循环的节点索引数量，不表示 15 维是否完整。维度完整性由 v2 receipt 的独立计数证明。
- semantic receipt v2 必须证明 `evaluated_dimension_count=15`、`injected_dimension_count=15`、`unavailable_dimension_count=0`。任何维度不可用都使整向量原子失败，绝不把不可用转换为 `0`。
- 新投影 schema 固定为 `astr-embodiment.node-observability.v1`。新 native 对成功的新提交和去重命中都必须给出同一份可重建结果；旧 v1 receipt 只能标记为 `LEGACY_UNATTESTED`，不得追认 15/15。
- Python 对“字段缺失”和“字段存在但非法”分别记录 `UNAVAILABLE` 与 `REJECTED`，绝不补零。
- observatory 日志 v3 明确输出 `selected_node_count`、`activated_node_count`、`changed_node_count`、变化后非零节点计数、九个 region 的 `potential`/`excitation` 聚合与 delta，以及 residual 的计算状态。
- 新增配置 `node_observability_detailed_logging`，类型为 bool、默认 `false`。关闭时成功只输出包含严格 15 维值的简洁中文单行；开启时才输出完整 observatory v3 JSON。失败在任一模式下都必须记录固定失败码与阶段。
- 当前 RC2 路径没有计算五项 invariant residual，因此日志必须输出 `residuals.state="NOT_COMPUTED"`、`formula=null`、`values_fxp6=null`。receipt 中现有五个默认零不得继续被解释或展示为实际 residual 计算结果。
- 现有 attention 的 15 项 route 表继续作为拓扑输入，但 load 公式升级为 `full-vector-route-neutral-relaxation-v1`：非零值提供 evidence drive，零值提供确定性的中性回落；不能再依赖“literal 0 相加等于无效果”的旧算法。
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

当前审计样例中，`positive`、`affiliation`、`engagement` 三个非零值经固定路由命中 region 1、7、8；旧算法因其余 12 个 literal zero 没有数值作用而得到 `active_nodes=4096`。这个计数只能解释旧节点写集，不能证明旧算法完成了 15/15 dynamics。新合同保留三个非零值和十二个零的真实性，但把十二个零作为十二个**可用的中性回落输入**完整注入；不能为了让日志“看起来完整”而填成非零，也不能把零跳过。

### 2.2 维度完整与节点稀疏是两个正交轴

每轮 semantic input 固定有 15 个命名 slot。维度轴的成功条件是 15 个 slot 全部取得 `AVAILABLE` 数值并全部被 native assembler 消费；节点轴则只选择最终有效 delta 非零的 region/node。由此允许：

- `evaluated_dimension_count=15` 且 `injected_dimension_count=15`；
- 同时 `selected_node_count` 可以是 `0..16384` 的任意真实值；
- 全零中性向量在 field 已处于 neutral baseline 时可以得到 `selected_node_count=0`、`changed_node_count=0`，但仍是“15/15 已计算、15/15 已注入”，不是 `ZERO_LOAD` 未执行；
- 只要一个维度不可用，就没有成功注入；不得用该维度的 literal `0` 继续计算。

### 2.3 16k、状态变化与非零值不是同一概念

`NEURON_SLOTS=16_384` 是九个固定 region 的总容量。神经场在 Genesis 后并非简单的全零数组，且历史 transition 可以使本次未选中的节点继续保持非零。因此：

- `selected_node_count` 只描述本次候选写集；
- `changed_node_count` 只描述本次 before/after 的真实变化；
- `signal_nonzero_after_count` 描述 transition 后全场 `potential != 0 || excitation != 0` 的节点数，可能包含大量本次未选中节点；
- 三者不得互相替代。

### 2.4 residual 当前没有被计算

`InvariantResiduals` 的五个字段是 `authority`、`continuity`、`energy`、`renormalization`、`capacity`。当前 semantic commit 直接写入 `InvariantResiduals::default()`，即五个固定零；这只是路径尚未计算/输出 residual 的事实，不证明 invariant 已以零残差通过，也不证明节点值为零。

## 3. Full-vector dynamics 与兼容策略

### 3.1 输入状态：零与不可用必须分离

Python estimator boundary 升级为 `astr-embodiment.semantic-estimate.v2`。15 个固定 slot 各自只有两种形态；下方只展示两种字段形态，不是可提交的完整 payload：

```json
{
  "positive": {"state": "AVAILABLE", "value_fxp6": 0},
  "affiliation": {"state": "UNAVAILABLE", "value_fxp6": null}
}
```

- `AVAILABLE` 必须携带 `0..1000000` 的整数；其中 `0` 是中性基线/回落输入。
- `UNAVAILABLE` 必须携带 `null`，不得携带数值。
- 15 个名称必须全部出现；未知、缺失、重复或额外字段均失败。
- 兼容旧 v1 estimator 的 exact 15-key integer map 时，15 项都解释为 `AVAILABLE`；旧值 `0` 从本版本起解释为中性输入，不再解释为缺失。
- provider/estimator 整体失败时，diagnostic 固定记录 `evaluated=0`、`unavailable=15`，不得构造全零向量。

维度计数不变量固定为：

```text
evaluated_dimension_count + unavailable_dimension_count = 15
nonzero_evidence_dimension_count + neutral_baseline_dimension_count = evaluated_dimension_count
injected_dimension_count in {0, 15}
```

只有 `evaluated=15 && unavailable=0` 才可进入 native；native 注入是原子的，成功必须 `injected=15`，失败必须 `injected=0`，永不出现 1..14 的部分注入。

### 3.2 现有 route 上的最小完整向量算法

保留 `ae-attention` 现有 15 项 `ROUTES`、primary/secondary region 和既有 route coefficient；替换“只累计非零 literal”的 load 公式。固定新公式名为 `full-vector-route-neutral-relaxation-v1`，并纳入 formula digest。

对每个可用维度 `d` 的 `x_d`：

```text
evidence_d = x_d
neutral_d  = FXP6_SCALE - x_d
```

对每个 region `r`，按既有 route coefficient 计算覆盖权重归一化均值：

```text
evidence_mean_r = weighted_mean(evidence_d routed to r)
neutral_mean_r  = weighted_mean(neutral_d routed to r)
```

两个均值都由该 region 的完整路由维度集合产生，不能过滤 `x_d=0` 的项。然后对 potential 与 excitation 分别执行一次固定点更新：

```text
drive_r        = fxp_mul(evidence_mean_r, estimator_confidence)
neutral_rate_r = fxp_mul(neutral_mean_r, Fixed::from_raw(125000))
recovery_r     = fxp_mul(current_r - genesis_neutral_baseline_r, neutral_rate_r)
candidate_r = current_r + drive_r - recovery_r
```

`fxp_mul` 表示每次乘法后立即按 `FXP6_SCALE=1000000` 重缩放并向零截断；所有减法、乘法和最终 add 按现有 checked/saturating 规则及上方固定顺序执行。`125000` 表示每轮最大八分之一中性回落率，是 v1 formula 的常量，不可由运行配置改变。potential/excitation 使用各自在 Genesis snapshot 中的 region neutral baseline。该方案具有四个必要性质：

1. `x=0` 不产生 evidence drive，但会把偏离 baseline 的状态向 baseline 回落；因此零真实参与 dynamics。
2. field 已在 baseline 时，全零向量允许 delta 为零；维度仍是 15/15 injected。
3. 非零值提供向上的 evidence drive，同时其 complement 仍参与有限回落，避免旧算法只增不回。
4. 某维度不可用时既不能 drive，也不能 recovery；整向量在 Python/native 边界前原子失败，避免把缺失误当中性导致人格错误回落。

region 只在最终 per-node effective delta 非零时进入 selected set，所以节点仍可稀疏。若所有 region 的 effective delta 都为零，receipt 允许 `active_nodes=0`、`state_before==state_after`，但 semantic revision 仍为这次已完成计算递增一次，以留下 15/15 注入的 durable receipt。该“已计算但状态未变”只允许 v2 full-vector semantic receipt；v1 仍保持旧约束。

### 3.3 Receipt v2 是必要的 canonical 证明

这次不是纯日志增强，而是 semantic dynamics 合同变化。新 semantic commit 必须使用独立 version/domain 的 `TransitionReceiptV2`；不得在 v1 domain 下追加字节或重新解释历史 receipt。v2 在现有 identity、revision、state、graph、active node/edge 字段之外，canonical 编码 `semantic_vector`：

```json
{
  "schema": "astr-embodiment.semantic-vector-receipt.v2",
  "formula": "full-vector-route-neutral-relaxation-v1",
  "dimension_slot_count": 15,
  "evaluated_dimension_count": 15,
  "injected_dimension_count": 15,
  "nonzero_evidence_dimension_count": 3,
  "neutral_baseline_dimension_count": 12,
  "unavailable_dimension_count": 0,
  "state_changed": true
}
```

成功 receipt 必须满足：

- `dimension_slot_count=evaluated_dimension_count=injected_dimension_count=15`；
- `unavailable_dimension_count=0`；
- `nonzero_evidence_dimension_count + neutral_baseline_dimension_count = 15`；
- `state_changed == (state_before != state_after)`；
- `active_nodes` 继续等于 node projection 的 `selected_node_count`，不等于 injected dimension count。

任何维度不可用时不生成 committed transition receipt；DEGRADED diagnostic 使用同一计数语义记录例如 `evaluated=13, injected=0, nonzero=3, neutral=10, unavailable=2`，并使用固定码 `SEMANTIC_VECTOR_UNAVAILABLE`。这份失败 summary 不是 receipt，不能推动 revision。

### 3.4 历史兼容、去重与重启

- 所有 v1 receipt 继续按原 domain/codec 只读解码和 replay；非 semantic G0 lane 继续使用 v1。
- 新 runtime 不得产生新的 v1 semantic receipt。
- 命中历史 v1 semantic event 时返回 `full_vector_state="LEGACY_UNATTESTED"`；可以展示历史 `active_nodes`，但不得追认 evaluated/injected 15/15，也不得用新公式重算后冒充当年结果。
- v2 新提交、同进程 dedup、重启后 dedup 必须返回 byte-identical v2 receipt 与 node projection。
- v2 dedup 读取 exact base/next snapshot；`base_revision=0` 通过 Genesis hydration 重建 neutral baseline。任一 snapshot、digest、formula 或 revision 不匹配均 fail closed，不用 latest/zero/估算 field 替代。
- observatory v3 接受 v1 legacy 与 v2 full-vector 两种明确状态；只有 v2 合法 receipt 可以显示 `FULL_VECTOR_CONFIRMED`。

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

- `selected_node_count`：所在 region 的 `drive_r != 0 || recovery_r != 0`，因而进入候选写集的唯一节点数；必须等于 v2 receipt `active_nodes`。15 个维度可以全部注入而 selected 为零，因为维度注入发生在 region accumulator，节点选择发生在有效动力项计算之后。
- `activated_node_count`：selected 节点中，合成后的 `drive_r - recovery_r` 严格非零的节点数。evidence drive 与 neutral recovery 恰好抵消时可以 selected 但未 activated。
- `changed_node_count`：全场节点中，经过 checked/saturating update 后 `potential_before != potential_after` 或 `excitation_before != excitation_after` 的唯一节点数。当前 transition 只准修改 selected set，因此必须满足 `changed_node_count <= activated_node_count <= selected_node_count`；发现 selected set 外变化时 projection 生成失败。
- `potential_nonzero_after_count`：全场 after state 中 `potential != 0` 的节点数。
- `excitation_nonzero_after_count`：全场 after state 中 `excitation != 0` 的节点数。
- `signal_nonzero_after_count`：全场 after state 中 `potential != 0 || excitation != 0` 的节点并集数。

所有计数均为 JSON integer，范围固定在 `0..16384`。数值可以出现 `selected > activated > changed`；饱和、exact cancellation 和已处于 neutral baseline 都必须按实际 before/after 报告，不得用 injected dimension count 推算节点计数。

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

只有 `EMPTY_REQUEST` 保留为未执行 NOOP，固定写为：

```text
AstrEmbodiment：未执行运算｜原因=EMPTY_REQUEST｜十五维：不可用
```

新 full-vector 路径不得产生 `ZERO_LOAD` NOOP。15 项全零是合法中性向量，必须输出“运算已完成”并提交 v2 receipt；只有历史 v1 记录可以继续被读取为 legacy `ZERO_LOAD`，不得作为新行为复现。NOOP 是否输出继续服从上一节的兼容矩阵。

## 6. 日志语义

### 6.1 固定三态

observatory v3 的 `calculation_state` 只允许：

| 值 | 含义 | 数值字段规则 |
|---|---|---|
| `SUCCEEDED` | validated v2 receipt 证明 15/15 evaluated、15/15 injected，并完成新提交或命中同一提交；允许状态未变 | full-vector receipt 与 node projection 必须为 `CONFIRMED` |
| `FAILED` | 端到端计算管线未产生可验证的成功结果 | 所有节点统计和 region 聚合必须为 `null`；它不等于“算出零” |
| `NOT_EXECUTED` | 输入为空，或在取得完整 15 维前 estimator/provider 不可用，native apply 未开始 | 所有节点统计和 region 聚合必须为 `null`；dimension failure summary 可记录 unavailable count |

`FAILED` 不自动证明 native 未写入：在 `stage=NATIVE_APPLY|RECEIPT|INTERNAL` 时继续用现有 `commit_state="UNKNOWN"` 表达持久化不确定性。失败原因只允许现有固定 code，不记录异常文本。

### 6.2 node projection 状态

`node_observability_state` 固定为：

- `CONFIRMED`：projection 存在并通过完整闭合校验。
- `UNAVAILABLE`：成功 result 来自兼容旧 native，字段缺失；同时 `full_vector_state` 必须为 `LEGACY_UNATTESTED`。
- `REJECTED`：字段存在但非法，原始内容已丢弃。
- `NOT_APPLICABLE`：`calculation_state` 为 `FAILED` 或 `NOT_EXECUTED`。

只有 `CONFIRMED` 可以伴随非空 `node_observability`。其他三态必须为 `null`，绝不使用全零对象占位。

### 6.3 详细模式的 v3 日志结构与级别

只有 `node_observability_detailed_logging=true` 时才构造并输出单行 compact JSON，前缀仍为：

```text
AstrEmbodiment SPC1 observatory:
```

现有 status、code、stage、commit/value/revision、15 维数值、confidence、expression 字段继续保留。计算相关字段固定为：

- `schema="astr-embodiment.observatory.semantic-injection.v3"`；
- `calculation_state="SUCCEEDED"`；
- `full_vector_state="FULL_VECTOR_CONFIRMED"`；
- `semantic_vector` 必须是 v2 receipt 中六个维度计数、formula 和 `state_changed` 的闭合副本；
- `native_calculation={"state_changed":true,"receipt_active_nodes":4096,"active_edges":0}`，其中 `state_changed` 也允许为 false；
- `node_observability_state="CONFIRMED"`；
- `node_observability` 必须是第 4 节定义的完整对象，不允许空对象、字段子集或字符串占位。

- `receipt_active_nodes` 是现有 receipt 值的显式改名投影，避免继续让日志读者把 `active_nodes` 误认为 changed 或 nonzero。
- `semantic_vector` 必须显示 evaluated/injected/nonzero-evidence/neutral-baseline/unavailable 五项计数；缺任一项时不得显示 `FULL_VECTOR_CONFIRMED`。
- v3 不再输出当前 `residuals_fxp6` 五个默认零；residual 只出现在带状态的 `node_observability.residuals` 中。
- `SUCCESS` 且 projection `CONFIRMED`：INFO。
- `NOOP`：INFO。
- `DEGRADED`：WARNING。
- commit 成功但 projection 为 `UNAVAILABLE` 或 `REJECTED`：WARNING；主 status 仍保持 `SUCCESS`，不得伪造 commit 失败。
- 详细模式的失败记录必须包含固定 `code`、`stage`、`commit_state` 与 calculation/node 状态；节点结果不可用时保持 null，不得用简洁错误行取代完整诊断。

## 7. Full-vector 验收样例

### 7.1 完整输入：3 个非零证据 + 12 个中性项

使用 exact 15-key 输入，其中 `positive=350000`、`affiliation=250000`、`engagement=600000`，其余 12 项为可用零。成功结果的关键字段必须为：

```json
{
  "status": "SUCCESS",
  "code": "SEMANTIC_COMMITTED",
  "calculation_state": "SUCCEEDED",
  "full_vector_state": "FULL_VECTOR_CONFIRMED",
  "semantic_vector": {
    "schema": "astr-embodiment.semantic-vector-receipt.v2",
    "formula": "full-vector-route-neutral-relaxation-v1",
    "dimension_slot_count": 15,
    "evaluated_dimension_count": 15,
    "injected_dimension_count": 15,
    "nonzero_evidence_dimension_count": 3,
    "neutral_baseline_dimension_count": 12,
    "unavailable_dimension_count": 0,
    "state_changed": true
  },
  "node_observability_state": "CONFIRMED"
}
```

所有 12 个零必须出现在 neutral aggregation 中。若测试 field 恰在这些 route 的 Genesis baseline，则它们的 recovery delta 可以为零；这不改变 `injected_dimension_count=15`。节点计数只按最终 dynamics 报告，不能硬编码为 15 维对应的全场节点数。

### 7.2 全零中性：已注入但允许状态不变

对 15 个 `AVAILABLE/0` slot 和恰处 Genesis neutral baseline 的 field，必须提交：

```json
{
  "evaluated_dimension_count": 15,
  "injected_dimension_count": 15,
  "nonzero_evidence_dimension_count": 0,
  "neutral_baseline_dimension_count": 15,
  "unavailable_dimension_count": 0,
  "state_changed": false,
  "selected_node_count": 0,
  "activated_node_count": 0,
  "changed_node_count": 0
}
```

receipt 的 `next_revision=base_revision+1`，`state_before==state_after`，status 仍为 committed；日志是“运算已完成”，不是 NOOP、ZERO_LOAD 或失败。若 field 偏离 baseline，同一个全零向量必须产生 neutral recovery、`state_changed=true` 和真实非零 changed count。

### 7.3 数据不可用：原子失败，不用零代替

若 13 项可用、2 项不可用，且可用项中 3 项非零、10 项为零，DEGRADED diagnostic 的关键字段必须为：

```json
{
  "status": "DEGRADED",
  "code": "SEMANTIC_VECTOR_UNAVAILABLE",
  "stage": "ESTIMATOR",
  "commit_state": "NOT_ATTEMPTED",
  "calculation_state": "NOT_EXECUTED",
  "dimension_summary": {
    "evaluated_dimension_count": 13,
    "injected_dimension_count": 0,
    "nonzero_evidence_dimension_count": 3,
    "neutral_baseline_dimension_count": 10,
    "unavailable_dimension_count": 2
  },
  "node_observability": null
}
```

不得生成 transition receipt、不得调用 native apply、不得推进 semantic revision。简洁模式仍必须输出 `运算失败｜失败码=SEMANTIC_VECTOR_UNAVAILABLE｜阶段=ESTIMATOR`；详细模式输出完整 diagnostic，但不可输出不可用项的伪值。

### 7.4 连续人格漂移与回落

用同一 persona、连续 revision 的确定性 fixture 验收：

1. 第 1 轮给某 route 非零 evidence，目标 region 的 potential/excitation 偏离 Genesis baseline，v2 receipt 为 15/15。
2. 第 2 轮该维度取可用零，其余维度仍完整提供；相应 region 按 `125000/1000000` 最大回落率向 baseline 移动，绝对偏差严格减小但不得跨越 baseline。
3. 第 3 轮重复全零中性向量，绝对偏差再次单调不增；每轮都产生独立 v2 receipt、revision 递增、evaluated/injected 均为 15。
4. 将同一维度改为 `UNAVAILABLE` 时，整轮失败且状态、revision 保持第 3 轮结果；不得发生一次错误的中性回落。
5. 重启后重放三份 committed receipt，state digest、dimension counts、node projection 与原执行逐字节一致。

这组验收证明人格状态既能受完整输入连续推进，也能在证据回到零时渐进回落；它不允许把“未提供数据”伪装成“人格应回落到中性”。

### 7.5 历史 v1 与新 v2 不混淆

历史 v1 成功 result 必须显示 `full_vector_state="LEGACY_UNATTESTED"`、`node_observability_state="UNAVAILABLE"`，日志为 WARNING。它可以展示历史 `receipt_active_nodes`，但 evaluated/injected/nonzero/neutral/unavailable 计数全部为 null；不得从历史 15 个数值反推并回填 v2 receipt。

## 8. 性能、尺寸与隐私边界

### 8.1 固定性能预算

- projection helper 对 before/after field 做一次顺序扫描，时间复杂度固定为 `O(NEURON_SLOTS)`，当前精确为 16,384 节点。
- full-vector assembler 必须恰好遍历 15 个 dimension slot，并按固定 route 表一次聚合；复杂度为 `O(15 + NEURON_SLOTS)`，不得因零值跳过维度，也不得因完整维度产生 15 份节点数组。
- 不得对每个 region 重扫全场；region 由节点索引在单次扫描中定位。
- 额外输出只允许九个固定 region 对象；不得分配或返回节点 ID、节点值或节点 delta 数组。
- canonical compact JSON 编码后必须 `<= 16_384 bytes`；超过上限视为 projection `REJECTED`，不得截断后输出。
- `regions.len()` 必须为 9，`field_node_capacity` 必须为 16,384；不允许配置扩大日志基数。
- 统计只用 deterministic integer/fixed-point 运算，不引入浮点、随机采样或近似直方图。
- 详细配置关闭时，logger 不得序列化完整 projection；只从已闭合 estimate 构造固定 15 维中文行，以避免无意义的大 JSON 开销。
- 任一维度 `UNAVAILABLE` 时必须在节点扫描、snapshot mutation 和 native commit 之前停止；失败路径的成本上限为固定 15-slot 校验，不允许用部分向量先算后回滚。

### 8.2 明确禁止的内容

native projection、Python diagnostic 和最终日志都不得包含：

- 16,384 个节点的逐节点值、索引、排序或采样；
- 任一 unavailable 维度的猜测值、默认零、上轮值或插补值；
- user 原文、Provider 原始输出、prompt/history、系统提示、工具输入；
- bot/persona/session/relation/event/turn token；
- SeedCode、incarnation、nonce、formula/scope/event/state/graph digest；
- exception 文本、任意上游 dict、未闭合 extension 对象；
- 可由多次日志拼接恢复用户原文的 token、n-gram、embedding 或向量。

只允许固定 schema 名、formula 名、revision、固定 region 名、整数计数和 fixed-point 聚合。该数据是运行时状态的有损、固定基数统计，不得进入 LLM prompt 或表达策略。

## 9. Python/native 接口边界与最小改动路径

### 9.1 Native owner

Rust 是节点状态、selected set、before/after 差值和 region 聚合的唯一 owner。Python 不得读取 snapshot 后自行计算，也不得从 15 维 estimate 推断节点数。

最小 native 改动集中于现有 semantic lane，不新增并行计算系统：

- `crates/ae-contracts/src/lib.rs`
  - 新增独立 domain/codec 的 `TransitionReceiptV2` 与闭合 `SemanticVectorReceiptV2`；
  - 保留 v1 byte-for-byte 解码和非 semantic G0 语义，按 schema/domain 分派，禁止同 domain 变长。
- `crates/ae-attention/src/r7.rs`
  - 保留 15 项 `ROUTES` 和 primary/secondary coefficient；
  - assembler 必须读取 exact 15 个可用值，计算 evidence/neutral 两通道与 region 归一化聚合，禁止过滤零值。
- `crates/ae-runtime/src/lib.rs`
  - 定义私有/公开只读的 `NodeObservabilityProjectionV1` 闭合类型与纯计算 helper；
  - 在 `PerceptionProposalDecisionV1` 增加 v2 receipt 与 projection；
  - 用 `full-vector-route-neutral-relaxation-v1` 更新 potential/excitation，并允许合法 v2 commit 的 state digest 不变；
  - 新提交用真实 before/after 与 full-vector load 计算，dedup 用 exact base/next snapshot 重建相同结果；
  - journal/store 表结构不变，但新 semantic row 必须保存 v2 canonical receipt bytes，replay 同时验证版本与 formula。
- `crates/ae-pyo3/src/lib.rs`
  - 仅在 semantic perception 成功 payload 中投影闭合 v2 receipt 和 node JSON；
  - 不增加新 Python callable，不暴露 NeuralField getter。

### 9.2 Python validator、coordinator 与配置

- `astr_embodiment/semantic_estimator.py`
  - 定义 exact 15-slot `AVAILABLE/value` 与 `UNAVAILABLE/null` schema；
  - v1 exact integer map 兼容转换为 15 个 AVAILABLE；全零向量合法，不再产生 ZERO_LOAD。
- `astr_embodiment/bridge.py`
  - 分开校验历史 v1 与 full-vector v2 receipt，不对 v1 追认新计数；
  - 逐字段重建 v2 receipt 和 node projection；缺失映射为 `UNAVAILABLE`，非法映射为 `REJECTED`；
  - 永不把 raw projection 继续传递。
- `astr_embodiment/coordinator.py`
  - native apply 前验证 `evaluated=15, unavailable=0`；否则固定 `SEMANTIC_VECTOR_UNAVAILABLE`、`injected=0`；
  - 只从 bridge 已 canonicalize 的 v2 result 构造 diagnostic；
  - 把 receipt `active_nodes` 显式投影为 `receipt_active_nodes`；
  - 产生 fixed three-state calculation 与 full-vector state，不再把默认 residual 零投影为计算值。
- `main.py`
  - 从 `_conf_schema.json` 读取严格 bool `node_observability_detailed_logging`，默认/非法均为 false；
  - observatory schema 升至 v3；
  - 校验 dimension count、receipt version/formula、node count/region/revision/size/状态交叉不变量；
  - 详细模式输出单行 JSON；关闭详细模式时只输出固定中文成功/NOOP/失败格式；
  - 失败日志绕过旧 `observatory_enabled=false` 的例行抑制，但仍保持 fixed-code、never-raise 和 request-local at-most-once；
  - 不将 node projection 注入 request/system prompt。
- `_conf_schema.json`
  - 只新增本节规定的中文 bool 配置项，默认 false；不改变其他配置默认值。

## 10. TDD 测试矩阵

实现必须遵循先 RED、再最小 GREEN、最后受影响回归。测试文件限定为现有相关测试位置，不因本规格新建生产子系统。

| 层 | RED 测试 | 必须锁定的行为 |
|---|---|---|
| estimator v2 | `test_estimate_v2_distinguishes_available_zero_from_unavailable` | `AVAILABLE/0` 是 neutral；`UNAVAILABLE/null` 独立；其他组合拒绝 |
| estimator 兼容 | `test_legacy_exact_vector_maps_all_fifteen_slots_to_available` | v1 exact 15-int map 兼容为 evaluated=15，零不变成 unavailable |
| attention 完整性 | `full_vector_assembler_consumes_all_fifteen_slots_including_zero` | exact 15 项各消费一次；zero 进入 neutral accumulator；少一项立即失败 |
| attention 公式 | `neutral_channel_relaxes_deviation_without_literal_zero_skip` | 全零对偏离 baseline 的 field 产生八分之一受限回落，对 baseline field 产生零 delta |
| receipt v2 | `semantic_vector_receipt_v2_attests_atomic_fifteen_of_fifteen` | 成功只允许 15/15/0 unavailable；nonzero+neutral=15；active_nodes 与维度数独立 |
| receipt 兼容 | `transition_receipt_v1_and_v2_have_separate_domains_and_codecs` | v1 byte-for-byte replay；v2 独立 domain；禁止跨版本解码或历史追认 |
| runtime 完整输入 | `full_vector_commit_injects_three_evidence_and_twelve_neutral_slots` | evaluated=15、injected=15、nonzero=3、neutral=12，并输出真实 node 统计 |
| runtime 全零 | `all_zero_neutral_vector_commits_even_when_state_is_unchanged` | 15/15 receipt、revision+1、state_changed=false、active/changed 可为 0，不产 ZERO_LOAD |
| runtime unavailable | `unavailable_dimension_fails_before_native_apply_without_revision_advance` | evaluated+unavailable=15、injected=0，无 receipt/snapshot mutation |
| runtime 连续漂移 | `repeated_neutral_turns_monotonically_return_toward_genesis_baseline` | 每轮 15/15；偏差单调不增、不越过 baseline；unavailable 轮完全不动 |
| runtime 聚合 | `node_observability_reports_selected_activated_changed_and_nonzero_counts` | dimension 15/15 与稀疏 selected/activated/changed 正交；九 region 求和不变量成立 |
| runtime 饱和 | `node_observability_distinguishes_activated_from_changed_nodes` | 部分节点收到正向 drive 但饱和不变，`changed < activated <= selected` |
| runtime 越界/旁路变化 | `node_observability_rejects_out_of_scope_field_changes` | selected 外变化、region 不一致、数值溢出均失败，不产部分 projection |
| runtime 去重 | `full_vector_receipt_and_observability_are_identical_after_reopen` | 新提交、同进程 dedup、重启 dedup 的 v2 receipt/projection 字节一致 |
| runtime residual | `semantic_node_observability_marks_default_residuals_not_computed` | 五个 receipt 零不会变成 `COMPUTED` 值 |
| PyO3 | `semantic_perception_payload_exposes_bounded_v2_receipt_and_observability` | v2 counts + fixed region 聚合；无节点数组、token、digest；编码不超过 16,384 bytes |
| bridge | `test_bridge_rebuilds_full_vector_receipt_and_node_observability` | 完整闭合重建、formula/revision/dimension/node/region 交叉校验 |
| bridge 兼容 | `test_bridge_marks_v1_semantic_receipt_legacy_unattested` | v1 可读但不能出现 15/15；malformed v2 为 REJECTED，raw sentinel 不泄漏 |
| coordinator | `test_preflight_fails_closed_when_any_dimension_is_unavailable` | 不调用 native，injected=0，固定 code/stage，零与 unavailable 不混淆 |
| observatory 成功 | `test_spc1_observatory_v3_logs_full_vector_and_sparse_node_counts` | INFO、15/15 receipt、真实 sparse nodes、九 region、residual NOT_COMPUTED |
| observatory 全零 | `test_spc1_all_zero_vector_logs_completed_not_noop` | “运算已完成”、15 个零、full-vector confirmed，不出现 ZERO_LOAD |
| observatory 失败 | `test_spc1_unavailable_vector_logs_counts_without_numeric_defaults` | WARNING、code/stage、unavailable count；不可用项没有伪零 |
| 配置默认 | `test_node_observability_detailed_logging_defaults_false_and_rejects_truthy_values` | 缺失/字符串/整数/对象不打开详细 JSON，Schema 描述为中文且 default=false |
| 简洁成功 | `test_compact_success_log_contains_exactly_fifteen_ordered_dimensions` | 含“运算已完成｜十五维：”，严格 15 个有序整数值，无大 JSON/节点对象 |
| 模式开关 | `test_detailed_switch_selects_complete_v3_json` | false 为简洁中文，true 为包含 active/changed/nonzero/regions 的完整 v3 JSON |
| 失败强制记录 | `test_failure_is_logged_with_code_and_stage_in_both_modes` | 关闭时简洁中文；开启时完整诊断；`observatory_enabled=false` 也不能压掉失败 |
| 隐私/尺寸 | `test_spc1_observatory_v3_is_bounded_and_content_free` | user/raw/digest/node sentinel 不出现，单行 JSON 大小受限 |

聚焦命令固定为：

```powershell
cargo test --locked -p ae-runtime node_observability
cargo test --locked -p ae-runtime full_vector
cargo test --locked -p ae-contracts transition_receipt
cargo test --locked -p ae-attention full_vector
cargo test --locked -p ae-pyo3 semantic_perception_payload
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.\.venv\Scripts\python.exe -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\evidence\ae-rc2-full-vector-dynamics-spec-revision-20260823\pytest-cache' tests\test_semantic_estimator.py tests\test_semantic_bridge.py tests\test_runtime_integration.py
```

GREEN 后受影响验收至少包括：

```powershell
cargo fmt --all -- --check
cargo clippy --locked -p ae-contracts -p ae-attention -p ae-runtime -p ae-pyo3 --all-targets -- -D warnings
cargo test --locked -p ae-contracts -p ae-attention -p ae-runtime -p ae-pyo3
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
.\.venv\Scripts\python.exe -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\ae-rc1-takeover-20260821\evidence\ae-rc2-full-vector-dynamics-spec-revision-20260823\pytest-cache' tests\test_semantic_estimator.py tests\test_semantic_bridge.py tests\test_runtime_integration.py tests\test_release_contracts.py
```

这些命令是未来实现的验收合同，不是本文已执行的测试证据。完整 workspace/release 验收仍由后续独立任务决定。

## 11. 明确不做

本规格不授权、也不设计以下变更：

- 不改变 15 个维度的名称、顺序、`0..1000000` 数值范围或跨维度独立性；本次只把零明确为 neutral，并新增 unavailable 状态。
- 不训练、更换或增加 estimator 模型/Provider；只升级闭合输出合同和 prompt 对 zero/unavailable 的定义。
- 不强迫 15 维全部非零，不自动填充 unavailable，不从其他维度推断缺失值。
- 不改变现有 attention route 拓扑、primary/secondary region、region layout 或 16,384 节点容量；只把 load 更新为本规格的 full-vector evidence/neutral 公式。
- 不引入新的 NeuralField 分量；full-vector dynamics 仍只更新 potential 与 excitation，其他分量变化即失败。
- 不改变 expression profile、表达注入、ActionContract、Persona 或回复策略；
- 不暴露逐节点 API，不打印或采样 16k 节点值；
- 不改写历史 v1 receipt、digest、revision 或 replay identity；v2 使用独立 domain/codec，只作用于新 semantic commit。
- 不修改 README、CHANGELOG、版本号、release metadata、CI、打包、ZIP、tag、Release；
- 不 push、不建 PR、不发布。

## 12. 完成判据

未来实现只有同时满足以下条件，才能称为 full-vector dynamics 与节点可观察性完成：

1. 每个成功 semantic v2 receipt 都证明 evaluated=15、injected=15、unavailable=0；nonzero+neutral=15。
2. `AVAILABLE/0` 确实进入 neutral recovery，`UNAVAILABLE/null` 使整轮在 native 前原子失败且 injected=0。
3. 全零 neutral vector 可以 committed 且 state unchanged；ZERO_LOAD 不再是新路径的 NOOP。
4. 真实 native before/after 产生并校验 `node_observability.v1`，不是 Python 推断；日志区分 dimension 15/15 与 sparse selected/activated/changed/nonzero。
5. 连续 evidence/neutral turn 产生受限、可重放的人格状态推进与回落；unavailable turn 不改变状态或 revision。
6. 当前 residual 显示为 `NOT_COMPUTED + null`，日志中不再出现会被误认的默认五零结果。
7. 新提交、dedup、重启 dedup 的 v2 receipt/projection 一致；历史 v1 只标 `LEGACY_UNATTESTED`，不伪造统计。
8. 输出固定九 region、无逐节点内容、无用户原文，compact JSON 不超过 16,384 bytes。
9. `node_observability_detailed_logging` 为中文 Schema bool 且默认 false；关闭时成功行严格包含 15 个有序维度，开启时输出完整 v3 JSON。
10. 失败在两种模式及 `observatory_enabled=false` 下都记录固定失败码与阶段；关闭时简洁中文，开启时完整诊断。
11. 聚焦 RED→GREEN、受影响回归、静态检查均有当前命令证据。
12. 生产、测试和 CI 的实际修改必须由后续获授权实现任务完成；本文提交本身不构成实现、集成或 release 证据。
