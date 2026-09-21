# AstrEmbodiment 情感计算矩阵选择性前移设计

**状态：** 已获用户方向确认；本文仅定义设计与验收边界，不实现生产代码。  
**目标版本：** 当前 `codex/astrembodiment-headless-body-alpha3` 发布线的下一增量。  
**源基线：** `710829ae5d3bef82ce818754354272517cb28056`。  
**目标设计基线：** 写作开始时为 `b05d7b58eb21b91c2d931d8b7efcbcace4175381`。  
**共同祖先：** `8c4a606e63888351b3d8854e9006b89aa6623f07`。

## 1. 决策摘要

AstrEmbodiment 必须自带可工作的情感计算矩阵。安装者不需要安装任何“赛博人”高层插件，也能得到确定性的情感证据计算、持久化状态演化和受限表达投影。可选的赛博人插件只能消费该矩阵的公开投影、提交受约束刺激，不能拥有或替代矩阵。

本次采用**审计后的选择性前移**：以源基线中已实现的 15 维语义证据、冻结九区路由、16,384 节点八自由度神经场、确定性稀疏图和 Phase-0 定点动力学为能力基线，逐项适配当前 alpha3 的权威、事件、持久化、睡眠和主动联系边界。禁止盲合并整个历史，也禁止抛弃成熟实现后从零重写。

当前 alpha3 **不包含完整情感矩阵运行链**。它有 `NEURON_SLOTS = 16_384`、`NODE_DOF = 8`、九区布局、初始场和空图摘要，但 `ae-runtime::apply_event` 明确是 G0 no-op，提交的 `state_after == state_before`。已发布的 `1.1.0-alpha3` 包由这条分支生成，因此同样不具备源基线的 `semantic.rs`、`semantic_dynamics_v2.rs`、确定性图开发和语义持久化闭环。不能把“有 16,384 节点骨架”表述成“已具备情感计算”。

## 2. 产品边界

### 2.1 AstrEmbodiment 拥有

- 15 维 `EvidenceVector` 的闭合校验和证据承诺；
- 15 维证据到九个区域的冻结路由；
- 16,384 节点、每节点八个状态自由度的有界定点场；
- 由出生材料确定性生成的稀疏图；
- 神经场传播、回归基线、适应、精度、预测误差、资格迹和代谢储备演化；
- 每人格独立的矩阵状态、游标、快照、图、收据、迁移和可观察摘要；
- 睡眠/时间推进导致的无新语义证据衰减与恢复；
- 向现有行动、睡眠和主动联系系统提供只读、可验证、最小化的情感投影。

### 2.2 可选赛博人插件拥有

- 更长时间尺度的人生叙事、内心活动、世界生活和高级认知编排；
- 以受约束接口读取 AstrEmbodiment 的情感投影；
- 以普通、可审计的刺激或时间事件影响矩阵。

它不得注册第二个矩阵写入者，不得直接修改节点，不得绕过证据、关系、同意、睡眠或主动联系闸门。未安装该插件时，矩阵仍完整运行。

### 2.3 明确不做

- 不把矩阵改造成 Transformer、LLM 或赛博人认知层；
- 不在矩阵步中调用 LLM；
- 不因主动联系关闭而停止情感演化；
- 不以 Token 预算控制矩阵计算；
- 不在本次前移中任意改变节点数、区域容量、15 维语义或原始 Phase-0 公式。

## 3. 现状与源事实

### 3.1 当前 alpha3

`crates/ae-neurofield/src/lib.rs` 已保留：

- `NEURON_SLOTS = 16_384`；
- `NODE_DOF = 8`；
- `EDGE_CAPACITY = 524_288`；
- 九区 `REGION_LAYOUT`；
- `NeuralField` 的 potential、excitation、inhibition、adaptation、precision、prediction_error、eligibility、metabolic_reserve；
- 由 GenesisManifest 生成初始场的 `initial_state_from_manifest`；
- 状态和图摘要。

但当前图初始化为空，未包含 `graph_development.rs`、`graph_replay.rs`、`structural_delta.rs`；`apply_event` 只接受 `UserStimulus`/`DeliveryOutcome`，生成 no-op action contract，并把同一个摘要写入 `state_before` 与 `state_after`。

### 3.2 经审计的源能力

源基线 `710829a` 的能力链为：

1. `ae-contracts`：`EvidenceVector` 的 15 个固定槽；`PHASE0_SEMANTIC_ROUTE_RULES_V1` 冻结为 15 条 primary/secondary 路由；primary 系数 1.0、secondary 系数 0.5；路由与动力学都进入公式摘要。
2. `ae-attention`：把全部 15 维装配成九区负载，不把缺失维度静默当作可用证据。
3. `ae-neurofield`：九区容量总和 16,384；每节点八自由度；`develop_graph(..., GraphFormula::V1)` 从 manifest digest 与 development seed 生成 262,144 条规范边；同输入跨进程、跨资源封装得到相同字节与摘要。
4. `ae-runtime::semantic_dynamics_v2`：采用不可变 before-state 的 Jacobi 语义做稀疏传播；全程 FXP6 定点和显式舍入；更新八自由度并输出能量、容量和重归一残差。
5. `ae-runtime::semantic`：校验闭合证据，准备图和场的下一状态，产生语义收据、节点可观察投影、表达投影与 AESEM3 快照。
6. `ae-store`/runtime：独立语义游标、原子提交、去重、陈旧因果拒绝、AESEM2 回放与有界迁移、公式升级证明、重开恢复、异步语义 outbox 和失败分类。

这是一条情感证据计算链，不是 16,384 节点容器本身，也不是赛博人高层认知。

### 3.3 不可变源基线清单

实现开始前必须把下列基线复制成目标仓库内的机器可读 provenance manifest；源工作树和其 Git 历史只读，禁止 reset、delete、overwrite 或在其上提交：

| 源文件（相对源基线） | SHA-256 |
|---|---|
| `crates/ae-contracts/src/lib.rs` | `4b42175c17cdf4a4e5e982bea60f4f6c63c30323c0af7969e99fc82d90f32b70` |
| `crates/ae-neurofield/src/lib.rs` | `c0ab2ed01917564d95865065b5b51f6f86a3e9a1eaed4a4fa5cd793f41587a16` |
| `crates/ae-neurofield/src/graph_development.rs` | `d014151e501e55938200b2773e94a5a2f290ad09cda5c615fff5a6f9969ed00e` |
| `crates/ae-neurofield/src/graph_replay.rs` | `a3b7355d6f35ba04ea42c0959f1b96793026104ae504244cd20a9dc96f040ddf` |
| `crates/ae-neurofield/src/structural_delta.rs` | `4121208c9d6ecf9fd207616a6e95c0c65a4f73d9d24867131195ae92b40a5891` |
| `crates/ae-runtime/src/semantic.rs` | `63ea4b3ac75fdd5703fd2a8cd753a6d677b7eb6d8250b26f3b81e03372864649` |
| `crates/ae-runtime/src/semantic_dynamics_v2.rs` | `9162735da896c82bc07fbebdcc446354bbc12138efebd05e2d8cc89566f9f91d` |
| `crates/ae-runtime/src/semantic_telemetry_v1.rs` | `737a1ee8d8341586f46a552ea5de0e36ca41d50381e332e9092b2562c3616a7f` |
| `crates/ae-runtime/src/lib.rs` | `930bfe25565ae221cd05a8b0d81d4a8a5318e3dbe7ec855e3dc38b290e64a0ad` |
| `model/regions-v1.toml` | `6a2af87fe65e01b17a512b9796bee011e6ea319b1362467d426fb685e887383a` |

provenance manifest 的每条记录必须包含源 commit、源路径、源文件哈希、源符号/fixture、目标路径、目标符号、处置方式（exact/adapted/superseded）和验证证据。`superseded` 必须引用显式批准的新契约和等价/更强验收，不能代表删除。

相关能力不是单一提交产生的。选择性前移审计至少必须覆盖以下源提交及其后续修复，而不能只取最早 feature commit：

| 源提交 | 能力意义 |
|---|---|
| `a036391` | 确定性 Genesis 稀疏图开发 |
| `dac330e`、`15a2da5`、`d6cfbe5` | 结构增量校验、图历史回放和 replay-rule 摘要 |
| `d8bfc7c` | 语义 native lane 的主体前移 |
| `867eec6`、`8209a2b` | Phase-0 传播遥测和公式真实性绑定 |
| `1774023`、`3984ffb` | legacy semantic state/有限域原子迁移 |
| `cc725f2` | invalid neural state 的闭合分类 |
| `9d373c4` | node observability 与 typed v2 contract 绑定 |
| `ca8d719` | durable async semantic outbox |
| `710829a` | 本设计冻结的源树最终头，包含 store typed error 修复 |

`git merge-base` 已确认两线从 `8c4a606` 分开。目标线在此后加入 alpha3 lived-world、sleep、relation-local contact/proactive、headless-body 和简化主动设置；源线在此后加入 semantic/R7/continuity/rebirth 等正式 1.0.0 能力。因此适配点必须逐项审计，不能把任一分支当成另一分支的线性祖先。

## 4. 目标架构

### 4.1 数据流

```text
受认证 CanonicalEvent / PerceptionProposal
        │  authority + scope + causal + evidence commitment
        ▼
15D EvidenceVector ──冻结 route digest──► 九区局部负载
        │                                  │
        │ manifest digest + development seed
        ▼                                  ▼
确定性 262,144 边稀疏图 ─────────────► 16,384 × 8 FXP6 Jacobi 动力学
                                           │
                    state/graph/formula/evidence/identity commitments
                                           ▼
           原子写入语义快照、收据、游标、遥测与主事件关联
                                           │
                   ┌───────────────────────┼──────────────────────┐
                   ▼                       ▼                      ▼
              表达投影                睡眠/时间衰减          主动联系只读信号
```

### 4.2 单写入者与原子边界

矩阵采用**人格级独立语义命名空间和游标**，不复用 relation-local 联系游标，也不把 alpha3 自治 revision 当作语义 revision。所有修改只有 runtime/store 单写入者能提交。

当前 `UserStimulus` 已包含 15 维 `SemanticEstimate`，因此适配器不需要 LLM。流程为：

1. 先验证 event authority、persona/relation scope、causal base、事件去重键、15 维范围、estimator confidence 和 estimator digest；
2. 对当前已提交场/图做纯函数 prepare；
3. 预先计算全部状态、图、公式、证据、身份和收据摘要；
4. 在一个 SQLite immediate transaction 中提交主事件关联与语义 sidecar；
5. 只有提交成功才替换 hot field/graph/semantic revision。

任何一步失败都不得留下只提交主事件或只提交矩阵的半状态。`DeliveryOutcome` 仍保留当前无情感语义输入的交付结算行为；不得伪造 EvidenceVector。其他 CanonicalEvent 只有在存在明确冻结映射时才能加入矩阵，不做“见事件就猜情绪”。

### 4.3 原公式先原样前移，再扩展时间衰减

第一阶段必须让源 `FullVectorRouteNeutralRelaxationV1` 公式字节、路由摘要、图摘要和黄金向量在目标分支保持一致。禁止为了接入 alpha3 顺手调参。

睡眠和无输入时间衰减使用独立、版本化的 `MatrixTimeAdvanceV1` 公式承诺：输入只能是已提交场、Genesis baseline、冻结时间跨度、睡眠状态和公式版本。它执行有界 neutral relaxation、adaptation 松弛与 reserve 恢复，不注入语义证据，不产生主动消息，不改变图。awake/drowsy/asleep 使用冻结的不同时间常数，但同一输入必须跨平台字节一致。该扩展不得修改或伪装成原 Phase-0 semantic formula；旧 formula 仍可回放。

## 5. 与 alpha3 子系统的结合

### 5.1 权威与事件

- 继续以当前 `CanonicalEvent`、`SourceAuthority`、scope 和 causal base 为入口；矩阵不能扩权。
- `UserStimulus.evidence` 的 estimator digest 必须和宿主提交的证据字节绑定；来源不可信、维度缺失、越界、nonce/identity 不匹配时拒绝。
- 重复事件返回原收据；同 event ID 不同 digest 为 identity conflict；陈旧 base 返回 typed stale，不自动重算后提交。
- matrix state、graph、route、formula、EvidenceVector、estimator、manifest、incarnation、persona scope 都进入持久化承诺或其可验证关联。

### 5.2 持久化、迁移与向后兼容

- 新写入使用源 AESEM3 能力的目标版 schema，并保留 canonical hot-state/AESEM2 解码与 fixture，直到显式 supersession。
- 已安装 alpha3 仅有 no-op 历史时，不改写旧 journal，不把 `state_before == state_after` 冒充语义计算。首次矩阵提交从已认证 Genesis baseline 建立独立 semantic revision 0，再提交 revision 1。
- 如果数据库已有源实现的 semantic namespace，则按源公式摘要、图摘要、快照形状和逐 revision 收据恢复；不能验证就拒绝启动写入。
- AESEM2 的有限域归一、备份、回放、公式升级证明、并发陈旧检测全部进入能力平价账本；迁移必须 copy/verify/commit，失败保留原字节和游标。
- wire 字段、状态自由度、15 维顺序、九区顺序、legacy fixtures 和错误分类保持兼容；任何删除必须另立设计并由用户批准。

### 5.3 renorm

矩阵内部的 `renormalization_residual` 是定点饱和/舍入守恒证据；alpha3 的 `ae-renorm` 金字塔是下游工作区摘要。二者不可混名或互相覆盖。

只有已提交矩阵状态可进入 `ae-renorm::restrict`。残差超阈时：保留最后已提交矩阵，写入受限诊断，禁止用不可信摘要形成主动意图。不得因 renorm 失败回滚或损坏矩阵 journal。

### 5.4 睡眠

`TimeAdvanceV1` 先由现有冻结时区/睡眠权威产生可提交的状态提案，再准备 `MatrixTimeAdvanceV1`。二者在同一 wake 结算事务中关联提交，或都不提交。睡眠只改变衰减速率和恢复速率，不改变人格身份、路由或图。

紧急唤醒可以改变后续时间步的 sleep state，但不能跳过矩阵因果、制造语义证据或重写之前的场。

### 5.5 主动联系

主动联系从矩阵读取小型、已提交的 affect projection，例如区域均值、变化方向、置信度、revision 和 state digest；它不能读取原始用户文本，也不能直接读写 16,384 节点。

矩阵信号只能影响候选的内生强度/紧迫度，不能绕过：用户开关、Auto/固定频率、每日上限、冷却、未回复硬停止、关系 consent、安静时间、睡眠、Token 预算、出站目标绑定和 claim/settle。矩阵计算自身不消耗 LLM Token；关闭主动联系或 Token 预算耗尽时，矩阵仍可在本地演化，但不会触发 LLM 或发消息。

### 5.6 关系隔离

情感场属于 persona，可以保留跨会话的整体情感连续性；这不等于允许关系数据泄漏。具体约束为：

- 原始证据、event identity、consent、未回复、目标、预算和原因引用严格 relation-local；
- 公共 affect projection 不携带 relation token、文本、用户名、原因内容或可反查事件的标识；
- 一个关系的 event 不能去重、确认、结算、清除或覆盖另一个关系的事件；
- 下游面向关系的主动候选只能绑定当前关系已授权的公开投影和 cause ref；
- 验收中的“无跨关系串扰”指上述权威/隐私隔离，不否认 persona 整体心境连续性。

## 6. 失败语义

所有错误均 fail closed：

- 无可信证据：不变更矩阵，不把零向量当作已观察到的中性；
- field/graph shape、范围或排序非法：`INVALID_NEURAL_STATE` typed subcode，不修补后继续；
- 摘要/公式/身份不匹配：拒绝 hydrate/commit；
- 算术溢出、定点舍入前提失败：丢弃 prepared state，保留 committed state；
- SQLite、快照、遥测或 paired-commit 失败：事务回滚，hot state 不前移；
- semantic lane 不可用：主动联系不得宣称拥有情感依据，可降级为当前受限的非矩阵策略，但必须显式 `matrix_unavailable`，禁止静默 no-op 冒充成功；
- 旧状态无法迁移：只读保留旧状态，发布/启动写入 NO-GO，不删除数据库或创建平行假历史。

日志和公共错误只给闭合 code、revision 和摘要，不泄露原始证据或私有节点。

## 7. 能力零丢失约束

实现仓库必须维护 capability parity ledger，至少逐项覆盖：

- 15 个 EvidenceVector 槽、顺序、范围与 canonical codec；
- 15 条冻结路由、primary/secondary 系数和 route digest；
- 九区名称、start/count/reserve；
- 16,384 节点、八自由度、524,288 最大边、V1 的 262,144 规范边；
- graph develop/replay/delta、排序和摘要；
- FXP6 运算、Jacobi before-state、neutral/adaptation/energy 公式和遥测；
- semantic receipt v2、native telemetry、node observability、expression projection；
- independent semantic cursor、atomic commit、dedupe、stale/identity conflict；
- AESEM2/AESEM3 codec、迁移、恢复、公式升级、备份和 fixture；
- async semantic outbox/crypto 状态（若当前宿主尚不消费，仍保留兼容表面和测试，不得删除）；
- Windows/Linux native export、loader manifest、wheel/ZIP 和回归向量。

每项必须标成 `EXACT_PORT`、`ADAPTED_WITH_EQUIVALENCE` 或 `EXPLICITLY_SUPERSEDED`，并附测试/工件证据。存在 `UNMAPPED`、`UNKNOWN`、未验证或静默降级项时，发布一律 **NO-GO**。

## 8. 实施切片

### Slice 0：冻结证据和差异账本

- 记录源 commit、目标 commit、merge-base、源文件哈希、符号和 fixtures；
- 建 provenance manifest 与 parity ledger；
- 只读保留源工作树；确认目标工作树无未归属修改。

**门：** 所有源能力可追溯；任何遗漏即停。

### Slice 1：纯契约与纯计算

- 选择性前移 route/formula commitments、图开发/回放/增量和 semantic dynamics；
- 保持源黄金向量；不接 store、不接宿主。

**门：** 同输入得到同 graph bytes/state bytes/digests；15 维全部可达；八自由度确有受约束变化。

### Slice 2：语义状态、codec 与迁移

- 前移 AESEM3、legacy AESEM2 读取/归一/备份/公式升级；
- 建立独立 persona semantic namespace/cursor；
- 保留 wire、fixtures、typed errors。

**门：** 新库、alpha3 no-op 老库、源 semantic 老库都能按规则恢复；损坏/不可信状态零写入。

### Slice 3：当前事件与原子 authority 适配

- 把 `UserStimulus.evidence` 映射到原语义 proposal；
- 以 paired atomic commit 关联主事件与 semantic sidecar；
- 保留 DeliveryOutcome 和 alpha3 既有 authority 行为。

**门：** RED→GREEN 证明非零证据改变 state；重复、陈旧、identity conflict、事务 fault 均不产生双写或半写。

### Slice 4：睡眠、衰减、renorm 与主动联系

- 增加独立 `MatrixTimeAdvanceV1`；
- 发布最小 affect projection；
- 只读接入 renorm、sleep 和 proactive candidate，所有现有闸门仍优先。

**门：** 无新证据时确定性趋近 baseline/恢复 reserve；睡眠速率可验证；关闭主动联系时零 Token、零消息但矩阵继续本地演化。

### Slice 5：宿主、观测与包

- 绑定 Python/native ABI、contract info、loader manifest 和闭合诊断；
- 构建 Windows 与 Linux wheel，装入 universal ZIP；
- 扫描实际包内二进制与 manifest，而非只检查源码。

**门：** 双平台包均包含同一 semantic route/formula/domain/export；fresh install、upgrade、restart 后行为和摘要一致。

## 9. 验收矩阵

发布至少需要以下有边界的证据；不以“编译通过”替代行为验收：

1. **语义状态改变：** 给定认证的非零 15 维证据，`state_after != state_before`，changed nodes、区域负载和收据互相一致。
2. **中性不是伪证据：** 明确有效的全零 proposal 可执行 neutral relaxation；缺失/不可信 evidence 则拒绝，二者可区分。
3. **衰减：** 冻结时间推进使偏离 baseline 的状态单调有界回归、reserve 有界恢复；不创建语义 evidence。
4. **复现性：** 同 manifest、seed、初态、事件序列和冻结时间在两次进程、Windows/Linux 上得到相同 graph/state/receipt digests。
5. **稀疏传播：** 已有源→目标 golden case 保持 Jacobi 结果；图边数、CSR 排序和容量受限。
6. **身份与公式：** 修改 route、formula、manifest、incarnation、estimator 或 state bytes 任一项均被摘要/验证门捕获。
7. **原子性：** 在 journal、snapshot、graph、telemetry、paired commit 每个 fault point 注入失败，数据库和 hot state 都停在旧 revision。
8. **恢复迁移：** 新装、alpha3 no-op 数据、AESEM2、AESEM3 重开；非法历史拒绝；合法历史不丢字节、不跳 revision。
9. **关系隔离：** A 关系的事件不能命中 B 的 dedupe/settle/cause；公共 affect projection 不含 A 的私有标识；允许的 persona mood carryover 必须是匿名聚合。
10. **主动联系：** 矩阵强度不能突破 consent、quiet hours、sleep、频率、未回复硬停止、Token 和 target binding；关闭时不调用 LLM。
11. **能力平价：** ledger 无 `UNMAPPED`/`UNKNOWN`；所有原公式、路由、状态维度、语义路径、fixtures 和 exports 都有目标证据。
12. **实际包：** 对 Windows `.pyd` 与 Linux `.so` 做导出/域字符串/contract-info 验证，必须含预期 semantic APIs、route/formula domains 与 ABI；ZIP manifest 哈希必须指向对应二进制。仅源码通过不算包通过。

广泛测试不是目标；每个验收测试必须对应上述风险或源 fixture。优先复用源黄金向量、迁移 fixture 和故障注入，不堆叠同义测试。

## 10. 性能与 Token 边界

- 矩阵步为纯本地 Rust 定点计算，严禁为了计算情感调用 LLM；Token 成本为零。
- 图只在首次可信创建或显式结构变更时生成；普通事件复用已提交 CSR 图。
- 传播保持 O(nodes + edges)，目标基线为 16,384 节点与 262,144 边；不复制无界历史。
- prepare 阶段可分配 next field，但提交前不修改 hot field；后续优化不得改变规范字节或舍入。
- 观测只输出有界聚合，不序列化全部节点到日志/Python/提示词。
- 建立 release 性能基线：事件步、时间衰减步、hydrate、峰值内存。超过源基线的回归必须解释并批准，不能通过关闭矩阵隐藏。

## 11. 恢复与回滚

- 源工作树/commit/hash manifest 是不可变恢复锚；任何前移错误可逐符号回到源事实。
- 目标实现以切片提交，不改写历史；回滚使用新增的兼容开关停止**新矩阵写入**，不是删除数据库或回退 Git 工作树。
- 开关关闭时旧 semantic namespace 保持只读可验证；现有主事件、睡眠、主动联系设置和会话数据继续存在。
- 每次 schema 升级先备份原 snapshot bytes 和 cursor witness，再在事务内写新版本；验证失败恢复旧游标。
- 已发布包保留上一包 hash、native manifest 和安装收据。升级失败可重装上一包读取旧主状态；若上一包不认识新 semantic schema，则只是不写矩阵，绝不能覆盖或清空它。
- 禁止用 `git reset --hard`、目录覆盖、数据库重建或删除源工作树作为回滚方案。

## 12. 方案比较

### A. 审计后的选择性前移（采用）

优点：保留成熟公式、迁移和 fixture；能适配 alpha3 新 authority/proactive 架构；每项能力可追溯。代价：需要 provenance/parity 账本和逐切片适配，工作量高于 cherry-pick。

### B. 盲 merge/cherry-pick 整段源历史（拒绝）

优点：表面上快。缺点：共同祖先之后两边都大幅演化，源分支还包含 R7、continuity、rebirth、CI 和正式 1.0.0 的大量无关权威；会覆盖 alpha3 的 headless-body、自治、关系 consent 和主动联系设计，冲突解决也无法证明能力未丢。

### C. 依据概念从零重写（拒绝）

优点：目标代码可能更整齐。缺点：极易遗漏冻结路由、定点舍入、公式承诺、AESEM2/3 迁移、故障语义和跨平台黄金向量；用户要求“不丢任何原能力”，重写无法以合理成本证明平价。

## 13. 发布 NO-GO 条件

满足任一项即不得发包或宣称矩阵完成：

- 当前 no-op 仍是用户语义的最终状态转移；
- 包只有 16,384 节点骨架，没有真实 semantic dynamics；
- 任一原公式、路由、状态维度、语义路径、fixture、迁移或 package parity 项未映射/未验证；
- 源工作树或历史被修改、删除或覆盖；
- semantic 与主事件可能半提交；
- 不可信 evidence 会改变场，或失败会损坏已提交场；
- 关系私有原因进入公共情感投影；
- 矩阵可绕过主动联系闸门或在关闭时消耗 LLM Token；
- Windows/Linux 实际包未分别证明包含正确 native domains/exports；
- 只凭静态检查、focused test 或源码文件存在就声称双平台发布通过。

## 14. 实施前仍需产出的工程文档

下一步实现计划必须把每个切片拆成明确文件、源符号、目标符号、验证命令和提交边界，并首先创建：

1. source provenance manifest；
2. capability parity ledger；
3. schema/wire compatibility table；
4. paired atomic commit 的 store transaction contract；
5. Windows/Linux package acceptance checklist。

这些是实现入口，不是发布后补写的说明。
