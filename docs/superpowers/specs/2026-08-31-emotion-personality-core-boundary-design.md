# AstrEmbodiment 情感与人格核心边界设计

**日期：** 2026-08-31  
**状态：** 已确认的产品边界，供实施计划冻结  
**目标版本：** alpha4 源码增量；发布版本以合并后的实施计划为准

## 1. 决策摘要

AstrEmbodiment 回到最初的定位：它是 AstrBot 内的**情感与人格身体核心**，不是一个主动聊天机器人，也不是一个缩小版“赛博人生活模拟器”。

AstrEmbodiment 必须拥有并持续演化：

- 15 维情感证据、九区路由、16K×8 神经场和确定性情感动力学；
- 人格 Genesis、人格边界、人格长期稳定性和可验证的人格投影；
- 关系局部的情感状态，但不拥有平台收件人身份；
- 睡眠、昼夜节律、时间衰减、梦境残留和内稳态；
- 内生的社交需要、接触倾向和联系意图；
- 对入站消息和显式授权刺激做事件驱动的 LLM 语义评估，将上下文、含义和关系评价封闭为可验证的 15 维证据；
- 对宿主和其他插件提供最小、只读、可验证的投影。

AstrEmbodiment 不得拥有或执行：

- 主动消息调度、发送时间选择或重试循环；
- 收件人发现、枚举、排序或选择；
- 因内生联系意图而触发的 Provider/LLM 调用、主动文本提示词编排、生成或润色；
- 主动表达的 Provider 选择、用量估算或 Token 预算；
- outbox、平台投递、delivery retry 或平台适配器；
- 后台 tick、睡眠、时间推进或意图形成引起的网络访问。

AstrEmbodiment 保留 AstrBot 授权路径内的语义评估 Provider 和相应 Token 配置/遥测。LLM 是感知与评价器，不是情感矩阵：它只把授权事件转换为封闭证据；矩阵传播、衰减、睡眠/时间推进和人格调制仍是确定性的本地计算。

可选的独立主动对话插件可以读取一个经授权、一次性、最小化的 `AffectContactIntentProjectionV1`，再独立完成 consent、调度、收件人选择、Provider、主动表达预算、生成、投递和结算。没有该插件时，联系意图只在 AstrEmbodiment 内部形成、衰减和被观察，不会自行产生消息、后台 Provider 调用、主动表达 Token 消耗或网络活动。

本设计取代“把主动联系做完整后再拆分”的路线。已经完成的主动设置 Tasks 1–3 及其提交历史必须保留，作为兼容与可追溯证据；它们不再构成当前包的活动运行时能力。

## 2. 产品语义边界

### 2.1 情感、人格与“赛博人的生活”不是同一层

| 层 | AstrEmbodiment 是否拥有 | 精确定义 |
| --- | --- | --- |
| 情感动力学 | 是 | 证据如何改变情感场、关系情感和身体状态；确定性、可重放、可审计。 |
| 人格 | 是 | 相对稳定的价值、表达边界、依恋/社交参数、冲突约束及随时间受限变化。 |
| 内生接触意图 | 是 | 身体内部出现“想联系/想靠近/想保持距离”的倾向；它是状态，不是行动授权。 |
| 语义感知与评价 | 是 | 对入站消息或显式授权刺激做事件驱动的 LLM 语义判断，并把结果验证为封闭 15D 证据。 |
| 主动对话表达 | 否 | 如何把联系倾向写成一句话、使用哪个模型以及花多少主动表达 Token。 |
| 主动联系执行 | 否 | 联系谁、何时联系、是否重试、走哪个平台。 |
| 赛博人的生活叙事 | 否 | 日程、世界事件、职业、剧情、奇幻设定、长期生活编排和视觉小说式叙事。 |

因此，“她想联系某人”可以是 AstrEmbodiment 的真实内生状态；“她决定今晚 21:00 给用户 A 发一段文字”必须由外部主动对话插件决定。AstrEmbodiment 不生产虚构生活事实来解释情绪，也不把梦境之外的生活叙事写回人格或矩阵。

### 2.2 内生意图不是发送权限

任何联系意图都必须满足以下不变量：

1. `intent != consent`：意图不能代替用户对主动消息或情感投影的同意。
2. `intent != recipient`：意图不能携带平台用户 ID、会话地址或推荐收件人列表。
3. `intent != schedule`：意图不能包含发送时刻、下次尝试时刻或重试策略。
4. `intent != content`：意图不能携带成品文本、提示词或模型上下文。
5. `intent != authority`：意图不能直接调用 Provider、平台、outbox、claim 或 settlement。
6. `projection != mutation`：读取投影不能改变神经矩阵、关系情感、人格或睡眠状态。
7. `appraisal != propagation`：LLM 只能提出语义证据，不能直接写节点、决定情绪或替代 Native 动力学。

这些不变量必须同时体现在类型、导出 API、包体扫描和运行时验收中，而不是依赖调用者自觉。

## 3. 当前实现与精确退场范围

### 3.1 已完成工作必须保留历史

主动设置简化 Tasks 1–3 已形成提交链：

- `3c60c06`、`39f61c9`、`952798a`：纯配置解析及收敛修复；
- `26fa25d`、`e290033`、`565cd2f`：摘要、迁移和回滚；
- `cf3cd7a`、`1e67b36`、`b05d7b5`：Native 频率合同、证据封闭和自定义兼容。

不得 reset、rebase 掉或改写这些提交，也不得删除用户已保存的旧配置。它们证明先前路线的行为和迁移边界，并为独立主动对话插件提供可选择移植的纯逻辑来源。当前实现不继续执行原计划 Task 4、Task 5，也不把已经存在的频率合同接到发送链。

### 3.2 Host 侧必须停止的活动入口

实施时按当前符号精确处理：

| 当前入口 | 处理 | 兼容结果 |
| --- | --- | --- |
| `main.py` 的 `AutonomousSupervisor` 导入、`self._supervisor`、`initialize()` 构造/启动及 `terminate()` 停止 | 移除主动 supervisor；以只推进睡眠、时间衰减、矩阵和内生意图的本地 `EmbodimentClock` 替代 | 不再扫描 relation、pending intention 或 outbox |
| `main.py::_on_autonomous_externalization` | 删除 | 没有因内生意图而发生的 Provider、生成或投递回调 |
| `main.py::_prepare_proactive_settings`、`_persist_proactive_migration`、`PROACTIVE_INPUT_KEYS` 和 `_autonomy_binding` 中的发送策略字段 | 从活动启动路径移除 | 旧配置原样留在 AstrBot 配置存储，不再影响核心运行时 |
| `main.py` 的 `execute_proactive_intention`、`recover_outbound_once` 和 `astr_embodiment/proactive.py` | 从生产包和导入图移除 | 不保留休眠发送代码 |
| `astr_embodiment/autonomy.py::AutonomousSupervisor` | 删除或拆为纯本地 `EmbodimentClock`；不得保留 `on_externalization`、relation 枚举、recovery 或 pending work | 只允许本地确定性 tick |
| `bridge.py` 的 `gate_and_claim_externalization_v2`、`settle_externalization_v2`、`gate_and_claim_dispatch_v2`、`pending_autonomy_work` 及 legacy 等价封装 | 从公开 Python bridge 删除 | 新增只读意图投影 API；旧 DB 仍可迁移/审计 |
| `_conf_schema.json` 的主动总开关、频率、用户免打扰、意图 TTL、未回复退避、主动表达 Token 预算 | 从可见配置页移除并标记 legacy ignored | 不清空、不覆盖用户值；语义评估 Provider/预算配置不在此删除范围 |
| `ae_contact_pause`、`ae_contact_end` | 停止注册 | 外部插件拥有发送 consent；旧 relation fact 只读保留 |
| `ae_wake` | 仅保留“身体紧急唤醒”语义 | 唤醒只能推进内部状态，不能形成发送许可或启动外部化 |

`main.py::on_llm_request` 可以继续把已提交的人格/情感边界被动注入 AstrBot 当前请求。入站消息或显式授权刺激还可以进入事件驱动的语义评估器；这条路径必须使用 AstrBot 已授权的 Provider 能力，接受独立的语义评估预算/遥测约束，并只产出候选 `EvidenceVector`，不得生成主动文本。

### 3.3 LLM 保留为语义感知与评价器

LLM 对语义判断是核心能力，不能随主动聊天一起删除。AstrEmbodiment 必须保留事件驱动的 semantic appraisal/estimator，用于处理：

- 用户入站消息；
- AstrBot 已验证并显式授权的外部刺激；
- 为当前事件解析必要的上下文、语义、指向和关系评价。

评估器的唯一可提交输出是关闭、版本化且严格验证的 15 维 `EvidenceVector`，并附带 `confidence`、`model_digest`、`source_digest` 和输入/合同 revision。原始文本只能经过 AstrBot 授权的语义 LLM 路径；持久化层保存封闭证据和摘要，不保存 chain-of-thought、隐藏推理或为评估构造的原始私密上下文。persona/relation scope 必须从授权事件继承，禁止跨关系拼接。

候选输出若字段缺失、越界、非有限、schema 不匹配、来源不可信或摘要不闭合，则 fail closed：不提交任何语义情感 mutation，并返回可观察的稳定 reason code。只有当前实现已经具有、且有冻结合同和验证证据的确定性 fallback 才可使用；否则行为必须是 no-op，不能用默认情绪猜测。

`main.py::_llm_generate` 或等价通用调用设施，以及 inbound apply 所需的事件语义 estimator，不得被笼统删除；实施时只移除 supervisor/outbound proactive 对它们的调用。Provider 选择必须受 AstrBot 当前授权和语义评估用途约束，不能复用为主动表达 Provider。语义评估所需的 Provider、Token 上限和遥测设置继续保留，并在现有 schema 能力范围内明确命名；本设计不要求动态隐藏 UI，也不得虚构宿主不支持的条件可见性。

语义评估预算与主动表达预算必须分域。语义预算耗尽、Provider 不可用或调用失败时，本次事件不产生语义 mutation，记录封闭失败原因；不得因此调用备用主动模型、延迟到后台重试或在 clock tick 补算。`inner_activity_token_daily_max` 等主动/内生活动旧预算不自动成为语义预算。

Persona Genesis 是否使用已有 Provider 编译路径，按其现有 authority 合同单独保留或收敛，不得因剥离主动聊天而误删；无论采用本地或模型辅助方案，最终都必须经过现有 `validate_proposal` 和 Rust Genesis authority，且不得由内生联系意图触发。

### 3.4 Native 公开表面退场

以下能力可以暂时保留为历史数据迁移实现，但不能继续从 `crates/ae-pyo3/src/lib.rs` 或 Host bridge 导出：

- `bootstrap_autonomy` 中的主动 relation policy 写入；
- `recover_autonomy`；
- `pending_autonomy_work`；
- `gate_and_claim_externalization[_v2]` / `settle_externalization[_v2]`；
- `gate_and_claim_dispatch[_v2]` / dispatch settlement；
- outbox materialization、Provider usage settlement 和 proactive budget authority。

Rust 内部不允许形成一条仍可由泛化 `alpha3_call` 字符串触达的旁路。公开操作枚举必须删除这些名称并默认拒绝未知/旧操作。包体不得注册后台主动任务，也不得链接平台发送或网络客户端。

历史表和 codec 的读取能力只服务迁移、完整性检查和回滚取证；它们不得被周期扫描。完成一次兼容迁移后，生产代码应删除无用途的活动 gate、claim、dispatch 状态机，避免把整套主动发送复杂度作为“以后也许会用”的休眠代码长期留在核心。

## 4. 目标核心架构

### 4.1 数据流

```text
 AstrBot inbound text / explicit authorized semantic stimulus
                         |
                         v
       authorized event-driven LLM semantic appraisal
                         |
                         v
 closed validated 15D EvidenceVector + confidence/model/source digests
                         |
                         +----------------------+
                                                |
 local clock tick / explicit bodily intervention|
                         |                      |
                         v                      v
                  authority check + canonical evidence mapping
                                      |
                                      v
                15D evidence -> 9-region routing -> 16Kx8 emotion matrix
                         |
              +----------+-----------+
              |                      |
              v                      v
      personality constraints   sleep/time/allostasis
              |                      |
              +----------+-----------+
                         v
          relationship affect + endogenous intent
                         |
             internal committed state only
                         |
          least-authority projection request
                         |
                         v
      optional external plugin (consent/schedule/own LLM/send)
```

LLM appraisal 只产生候选证据；Native 的矩阵传播、衰减、睡眠/时间推进和人格调制均为本地确定性路径，不调用 LLM。矩阵、人格、睡眠与关系情感仍在同一 Native 单写入者/原子提交边界内。投影层只读取最后一次已提交快照；投影失败不能回写默认情绪或推进时间。

### 4.2 本地时间推进器

`EmbodimentClock` 不是主动消息调度器。它只能：

- 按 Persona 自己的时区和睡眠策略提交确定性时间推进；
- 触发情感衰减、内稳态恢复、梦境残留和内生意图的自然形成/衰减；
- 使用 persona scope，不枚举 relation scope 来寻找可联系对象；
- 不读取 Provider、Token、平台能力、会话地址或发送配置；
- 不调用语义评估器；只有新到达且已授权的事件可以发起 appraisal；
- 不创建 asyncio 发送任务，不调用任何网络 API。

关系情感只在已有、已授权的 relation scope 因真实互动事实而更新。时钟可以对既有关系情感做统一的确定性衰减，但不能比较关系后选出“最值得联系的人”。

## 5. 通用意图投影合同

### 5.1 请求：`AffectContactIntentProjectionRequestV1`

请求由外部插件为**一个已经选定的 opaque relation scope** 发起：

```text
schema_version: 1
caller_plugin_instance_digest: Digest32
capability_grant_id: Id128
capability_grant_revision: u64
persona_scope: Digest32
relation_scope: Digest32
purpose: "contact_consideration"
observation_nonce: Id128
observed_at_utc_ms: u64
max_age_ms: u32   # 1..300000
consent_epoch: u64
consent_revision: u64
```

合同要求：

- capability 必须精确授权 caller、persona、relation、purpose 和一次 observation nonce；
- caller 先选择 relation，核心不提供 relation 列表、搜索、排名或“下一个联系人”；
- `observed_at_utc_ms` 必须来自 Host 冻结时间证明，不能由核心读取壁钟后猜测；
- consent 仅授权读取该 relation 的粗粒度情感投影，不代表同意收到主动消息；
- 请求不含平台 ID、用户名、会话地址、Provider、预算或消息草稿。

### 5.2 响应：`AffectContactIntentProjectionV1`

```text
schema_version: 1
projection_id: Digest32
persona_scope: Digest32
relation_scope: Digest32
observation_nonce: Id128
issued_at_utc_ms: u64
expires_at_utc_ms: u64
matrix_revision: u64
personality_revision: u64
relationship_affect_revision: u64
sleep_revision: u64
state_commitment: Digest32
evidence_commitment: Digest32
intent_state: "absent" | "present" | "inhibited" | "unavailable"
approach_fxp6: i64
care_fxp6: i64
unresolved_tension_fxp6: i64
novelty_seeking_fxp6: i64
withdrawal_fxp6: i64
urgency_fxp6: i64
confidence_fxp6: i64
inhibition_reasons: closed set
send_authority: false
recipient_authority: false
schedule_authority: false
generation_authority: false
delivery_authority: false
projection_digest: Digest32
```

所有 `fxp6` 值使用冻结的有界整数域；响应不含原始 15D 证据、节点向量、源文本、生活叙事、平台身份或可直接发送的文本。`inhibition_reasons` 是关闭枚举，例如 `asleep`、`relationship_boundary`、`insufficient_confidence`、`state_stale`、`consent_unavailable`、`matrix_unavailable`；不得输出自由文本诊断。

`intent_state=present` 只陈述内在倾向存在。外部插件必须重新证明自己的消息 consent、频率、免打扰、预算、Provider、目标和平台能力；任何字段都不能作为绕过依据。

### 5.3 投影不能修改矩阵

生成响应的事务必须是只读快照：

- 不推进矩阵 revision；
- 不改变情绪、人格、关系情感、睡眠或内生意图；
- 不把“被查看”当作互动事实；
- 不消耗或降低 urgency；
- 不创建 outbox、claim、delivery 或 Token ledger。

为防重放可以在独立的 capability/audit 域写入“nonce 已观察”收据，但该收据与情感状态分库/分表、分 revision，不能参与矩阵摘要。若无法原子证明 nonce 未使用，则请求 fail closed，不返回有效投影。

### 5.4 证据、时效与重放

投影必须绑定：request canonical digest、capability revision、consent epoch/revision、四个状态 revision、冻结时间输入、state/evidence commitment 和 projection schema version。`projection_digest` 使用域分离 `ae.affect-contact-intent-projection.v1`。

以下任一条件返回 `unavailable` 或错误，不返回旧的 `present`：

- projection revision 未来、倒退或 commitment 链不闭合；
- 矩阵/人格/关系/睡眠任一状态未提交或损坏；
- consent/capability 缺失、过期、撤销或 scope 不匹配；
- nonce 已观察、时间倒退、超过 `max_age_ms` 或响应已过期；
- renorm、迁移或状态一致性检查失败。

外部插件只能消费一次 projection。重试必须提交新 nonce 并获取新投影；不得缓存 `present` 跨越 consent revision、sleep revision、matrix revision 或 expiry。AstrEmbodiment 不接收“发送成功”作为奖励信号；只有经过现有 CanonicalEvent authority 验证的真实互动/投递事实才可能在后续版本进入矩阵，而且投递事实不能伪造成用户情感证据。

## 6. 隐私、同意与最小权限

### 6.1 两种同意必须分开

1. **投影读取同意**：允许指定插件读取一个 relation 的粗粒度情感/意图投影。
2. **主动消息同意**：允许外部插件在自己的策略和平台上尝试联系。

前者由 AstrEmbodiment 的 capability grant 验证；后者完全由外部插件拥有。旧 `proactive_enabled`、旧 contact grant 或旧发送记录都不能自动升级成任一种新同意。

### 6.2 不可枚举与不可反推

- API 不提供 relation count、列表、排序、最高 urgency 或批量投影。
- relation token 是不可逆 opaque digest；核心不保存或返回平台原始 ID。
- 默认 Experience 投影只包含 persona 级粗粒度情绪；关系投影需要单 scope capability。
- Developer 观察可以看到类型化计数/原因码，但不能看到原始证据、节点数组、平台身份或私密文本。
- 日志只记录 projection digest、reason code、schema/revision 和成功/失败，不记录 fxp6 明细。

### 6.3 撤销

撤销投影 capability 立即阻止新读取。已发出的 projection 最长有效期 5 分钟，但外部插件必须在执行前再次检查自己的 consent；AstrEmbodiment 不声称能撤回已经交给外部插件的数据。高隐私模式可把 `max_age_ms` 强制为更短或完全禁用 relation projection，而不停止核心情感计算。

## 7. 配置、状态迁移与回滚

### 7.1 配置迁移

新核心模式引入隐藏、只增的 `core_boundary_revision=1`。迁移只写该 revision 和必要的 capability 默认关闭标记；不得删除、覆盖或规范化以下历史值：

- `proactive_enabled`、`proactive_frequency`、`proactive_settings_revision`；
- daily max、cooldown、quiet hours、TTL、未回复退避和 emergency threshold；
- `inner_activity_token_daily_max`；
- 旧的主动表达专用 Provider 字段（若 provenance 证明其仅服务主动发送）。

这些主动字段从 UI 和运行时摘要中移除，状态为 `legacy_preserved_ignored`。配置摘要必须明确排除它们，避免旧值继续改变核心行为。不得把旧主动开关映射成 projection capability；新 capability 默认关闭且需要显式授权。

语义评估当前实际使用的 `assistant_provider_id`、`model_settings.assistant_provider_id` 或等价 Provider/Token/遥测字段不得按名称误删：先通过调用图和 provenance 区分 semantic-appraisal 与 proactive-expression 用途。语义字段保留在设置页和活动摘要中，并以宿主现有 schema 能力静态展示；不要求动态条件 UI。若同一旧字段混合两种用途，迁移必须拆分为明确的 semantic-appraisal 字段，同时保留原值供回滚，不得让语义判断失去可配置 Provider。

Tasks 1–3 的 `astr_embodiment/proactive_settings.py` 和 Native frequency evaluator 不进入活动导入图。纯频率 resolver、严格整数校验和安全迁移逻辑适合以后按来源映射移到独立主动对话插件，但不得为了“复用”让 AstrEmbodiment 保持 dormant supervisor、Provider gate 或 dispatch 状态机。

### 7.2 历史运行状态

升级事务必须保留历史表及每一行，但关闭所有可执行工作：

- `Ready`/`Deferred`/`Externalizing` intention 进入终态 `Suppressed(core_boundary_upgrade)`；
- 尚未开始 adapter call 的 outbox 进入 `Terminal(core_boundary_upgrade)`；
- 已标记 adapter call started 但没有可证明结果的记录保持 `DispatchUnknown`，不自动重试；
- claim 失效，Provider/Token reservation 按旧合同保守结算或冻结，不能释放后再次使用；
- relation contact、consent、budget、readiness 保留为历史只读事实，不自动转换成新 capability。

迁移必须单事务、幂等、带 source/target digest；失败则核心以 `externalization_disabled=true` 启动，情感矩阵仍可用，但投影 API fail closed。不得因迁移失败恢复 supervisor。

### 7.3 回滚

代码回滚不等于重新启用主动发送。回到旧版本前必须：

1. 备份并哈希数据库和配置；
2. 明确接受旧版本无法理解 `core_boundary_revision` 的风险；
3. 保持 proactive 总开关关闭；
4. 人工审计 `DispatchUnknown`，绝不重试；
5. 重新建立独立消息 consent、目标、预算和 Provider 配置后才允许旧实现形成**新的** intention。

已被 `core_boundary_upgrade` 抑制的历史 intention 永不复活。人格、矩阵和关系情感 revision 不因回滚降级或重算；若旧二进制不能读取新矩阵 schema，回滚为只读恢复，不运行。

## 8. 对既有矩阵设计与计划的修订

### 8.1 保留不变

`2026-08-30-emotion-matrix-forward-port-design.md` 的 15D→九区→16K×8、原公式、图生成/重放、AESEM2/3 兼容、单写入者、sleep/time decay、renorm、关系隔离、隐私投影和零能力丢失要求保持有效。

矩阵前移计划 Tasks 1–9 继续按 provenance/parity 门禁实施。当前已经完成的 provenance 冻结和修复提交继续保留。

### 8.2 取消和替换

- 取消旧主动设置计划 Task 4“把 effective frequency 和正预算接入 Native gate”。
- 取消旧主动设置计划 Task 5“在 recovery/Provider 前准备 relation policy/budget”。
- 旧计划 Tasks 6–7 仍由矩阵合并发布任务取代，但发布文案不得再宣称主动发送。
- 删除矩阵计划中的 `Interleave P4`、旧 Task 10“让 proactive gate 消费 verified affect authority”和 `Interleave P5`。
- 将矩阵 Task 10 改为“发布 least-authority verified affect/contact-intent projection”，实现本设计第 5 节合同。
- 将矩阵 Task 11 改为“Host 只读语义/人格/投影 API 与历史主动状态封存”，不得保留 async outbox compatibility 的活动调度。
- 将矩阵 Tasks 12–13 的版本、README、包体验收改为证明**无主动发送表面**，而不是证明发送 gate 完整。

新的实施顺序是：矩阵 Tasks 1–9 → 新 intent projection Task 10 → Host/core-boundary Task 11 → 版本/双平台包 Task 12 → bounded acceptance Task 13。

## 9. API 与包体验收

### 9.1 API 正向合同

允许的公开能力至少包括：

- Genesis 的确定性本地建立、读取和验证；
- CanonicalEvent 的 authority-checked 情感更新；
- 授权入站消息/刺激的事件驱动语义评估及封闭 `EvidenceVector` 验证；
- 本地 sleep/time advancement；
- persona 级粗粒度 Experience/personality projection；
- capability-scoped `project_affect_contact_intent_v1`；
- flush/close、完整性检查和历史迁移状态查询。

投影 API 只能返回第 5 节的封闭类型，不能返回内部 `DurableIntentionV1`、outbound、claim 或 Provider policy。

### 9.2 API 负向合同

Python bridge、PyO3 module、Rust public façade、plugin command 和配置 schema 均不得出现可调用的：

- proactive generate/send/submit/retry/recover/pending；
- externalization/dispatch gate-and-claim/settle；
- 因 intent、clock、sleep 或后台工作触发的 Provider selection/generation/usage；
- token reservation/budget for proactive expression；
- relation enumeration/ranking/recipient selection；
- platform send adapter or network client。

通用字符串 dispatch 必须采用 allowlist；旧操作名返回稳定 `UNSUPPORTED_CORE_BOUNDARY`，而不是转发到内部历史函数。

### 9.3 静态与包体门禁

源码和最终 Windows/Linux archive 必须证明：

1. `astr_embodiment/proactive.py`、主动 supervisor 和发送 adapter 不在 archive manifest；
2. `_conf_schema.json` 不显示任何主动开关、频率、免打扰或主动表达 Token 预算；语义评估 Provider/预算/遥测设置仍可见且命名清楚；
3. PyO3 导出表无 externalization、dispatch、pending autonomy、recover autonomy 或主动表达 Provider/Token API；
4. import/call graph 证明 AstrBot `llm_generate`、`get_provider_by_id` 只可从授权的入站语义评价或独立 Genesis 路径触达，不能从 intent、clock、sleep、supervisor 或 projection 触达；`send_message` 不在核心调用图；
5. wheel/native manifest 在 Windows/Linux 同构，且两端都包含完整矩阵/人格/投影能力；
6. 旧配置和历史 DB fixture 可打开、迁移、关闭并重开，字节级历史记录未删除；
7. provenance ledger 无 `UNMAPPED`、`UNKNOWN` 或空证据。

### 9.4 运行时主动链零外部副作用验收

使用记录型 Host fake，在以下没有新授权语义事件的场景统计 `provider_calls=0`、`send_calls=0`、`network_calls=0`、`token_reservations=0`：

- 全新安装运行至少一个 awake→drowsy→asleep→awake 周期；
- 旧配置中 `proactive_enabled=true` 且 Provider ID 非空；
- 数据库含 Ready、Deferred、Externalizing、DispatchPending 和 DispatchUnknown 历史记录；
- 矩阵形成高 urgency 内生联系意图；
- 没有外部插件；
- 外部插件 capability 无效、撤销、过期、nonce 重放或 scope 错配。

有效 capability 场景只允许返回一个已选择 relation 的投影；仍必须保持上述四项外部副作用为零。矩阵 revision 在投影前后相同，只有独立 audit nonce receipt 可变化。

另设语义评估正向场景：一条授权入站消息最多触发合同允许的 appraisal Provider 调用与语义 Token 记账，输出合法 15D 证据后才允许 Native 提交。畸形、不可信或超预算输出必须是可观察 no-op。该正向调用不放宽 idle、sleep、time advance、intent-only 和 projection-only 的零调用要求。

### 9.5 手工 AstrBot 验收

1. 安装新包并打开设置页：只看到情感、人格、身体/睡眠、语义评估 Provider/预算/遥测和开发观测相关设置。
2. 使用含旧主动配置的真实配置副本升级：主动频率、免打扰和主动表达预算仍在存储文件中，但 UI 不显示、运行时不读取；语义评估实际使用的 Provider 设置继续有效。
3. 不安装外部插件，观察数小时：睡眠、时间衰减和内生意图演化正常，绝无主动消息。
4. 普通用户发消息时，AstrEmbodiment 可经授权语义 LLM 路径做一次事件评价，并将严格验证的 15D 证据送入矩阵；不生成主动文本、不发送消息。
5. 检查语义评估设置与遥测：Provider/预算仅服务 appraisal，耗尽或失败时当次事件无语义 mutation，clock 不补算。
6. 安装测试消费者并显式授予单 relation 投影权限：只看到粗粒度合同，无法枚举其他关系。
7. 撤销权限并重放旧 nonce：读取被拒绝；人格和矩阵状态不变。

手工验收不要求发送一条主动消息，因为主动消息已经不属于本插件的能力。

## 10. 失败语义

| 失败 | 核心行为 | 禁止行为 |
| --- | --- | --- |
| 矩阵/人格/关系/睡眠状态损坏 | 投影 `unavailable`；保留最后已提交状态；报告稳定 reason | 不回填默认情绪，不生成消息 |
| capability/consent 不可证明 | 拒绝 relation 投影 | 不降级为 persona 投影，不泄露 relation 是否存在 |
| nonce 重放或时间证据异常 | 拒绝并记录无敏感信息的 audit reason | 不返回缓存的 `present` |
| 历史主动状态迁移失败 | 强制 externalization disabled；矩阵可继续或只读恢复 | 不启动旧 supervisor，不扫描 pending work |
| 确定性 Genesis 输入无效 | 明确 `GENESIS_UNAVAILABLE` | 不临时调用未授权 Provider 补全，不创建虚构人格 |
| 本地 clock 失败 | 停止 tick，等待下一次有界恢复/人工诊断 | 不触发外部插件，不用壁钟猜补多个周期 |
| 语义评估输出畸形、不可信或预算耗尽 | 当次语义 mutation no-op；记录稳定 reason 与摘要 | 不猜默认情绪，不后台重试，不暴露 chain-of-thought |

## 11. 方案比较与取舍

### A. 核心内保留完整主动链但默认关闭

拒绝。优点是以后打开方便；缺点是主动表达 Provider、预算、outbox、重试和平台能力仍污染核心，包体与调用图无法证明“不会主动发送”，休眠代码也会持续产生迁移与安全成本。此拒绝不涉及入站语义 appraisal。

### B. 核心产生意图，外部插件拥有全部外部化（采用）

优点是保留“她真的会想起用户”的内生性，又让用户在不安装外部插件时得到纯情感/人格核心；权限、Token 和平台风险清晰分离。代价是两个插件之间需要版本化的投影合同，外部插件必须重新实现自己的 consent、调度和预算。

### C. 核心只保留情绪，连联系意图也交给外部插件

拒绝。边界最简单，但会把“想联系”退化为外部调度规则，破坏内生情感驱动的初心，也使不同消费者各自猜测人格/情绪含义。

### 关键取舍

- 核心仍需要一个本地时间推进器，但它只推进身体，不是消息 scheduler。
- 关系情感保留在核心，但 API 只能查询 caller 已选 relation，不能让核心选择收件人。
- 旧主动设置代码保留 Git 历史和迁移证据，不保留活动 runtime complexity。
- 外部插件得到的是动机投影而非成品文本，因此可以服务现实伴侣、奇幻 Persona 或其他生态组件，而不把叙事世界塞进 AstrEmbodiment。

## 12. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| 外部插件把 `present` 当发送许可 | 类型中五个 authority 字段恒为 false；文档、capability 和负向测试共同封闭 |
| relation projection 被用于关系枚举 | 单 scope request、无列表 API、无存在性差异错误、短 TTL、一次 nonce |
| 旧 pending outbox 在降级后重复发送 | 升级时终态化/冻结；回滚默认 proactive off；DispatchUnknown 永不自动重试 |
| 剥离主动 Provider 时误删语义评估 | 以调用图和 provenance 按用途拆分；保留授权 appraisal Provider、预算、遥测和事件 estimator |
| LLM 输出越界或注入隐藏推理 | 只接受关闭 15D schema 与置信度/模型/来源摘要；畸形输出 fail closed 且不持久化 chain-of-thought |
| 删除 supervisor 误伤睡眠/时间衰减 | 先提取纯 `EmbodimentClock`，以零 externalization 负向合同验收 |
| 历史表长期拖累核心 | 只保留迁移/取证 reader；兼容窗口后按独立设计归档，不保留可执行状态机 |
| 外部插件版本漂移 | projection schema/version、domain digest、capability revision 和 fail-closed negotiation |
| 内生意图变成 OOC 生活故事 | 投影只含有界动机轴和 inhibition reason；禁止生活事实、自由文本和世界叙事写回 |

## 13. 发布 NO-GO 条件

以下任一成立即不得发布或打包：

- 情感矩阵 provenance/parity 有旧能力未映射或证据为空；
- 主动设置 Task 4/5 或旧矩阵 proactive Task 10 仍被计划为活动工作；
- 包体仍导出主动表达 Provider、主动生成、发送、externalization、dispatch、pending/recovery 或主动 Token 预算入口；
- 旧配置为 true 时能启动 supervisor、主动 Provider 或 send path；
- 投影可枚举 relation、包含平台身份/原始证据/自由文本，或能修改矩阵；
- intent 被用作 consent、recipient、schedule 或 send authority；
- 历史 pending work 会在升级/重启/回滚时自动重试；
- 入站/授权刺激的语义评估器、15D 输出验证或其 Provider/预算/遥测配置被主动链剥离误删；
- 畸形或不可信 estimator 输出仍能修改矩阵，或持久化 raw hidden reasoning/chain-of-thought；
- 没有外部插件且没有新授权语义事件时，由 intent、idle、sleep 或 time advance 发生任何 Provider、Token、网络或平台发送活动；
- intent/projection 单独存在即可触发文本生成或 Provider 调用；
- Windows/Linux archive 的核心边界或 Native 导出不一致。

满足本设计只证明 AstrEmbodiment 是可发布的情感与人格核心，不证明任何外部主动对话插件已经实现或安全。
