# AstrEmbodiment 混合赛博人自主运行时设计（v0）

- 状态：Approved for one coordinated implementation（用户于 2026-08-28 批准一次性实施）
- 日期：2026-08-28
- 适用仓库：`astrbot_plugin_astrembodiment`
- 目标版本：CyberHuman Autonomous Runtime v0

本文件冻结跨 wave 方向与停止线，并授权生成一份全 v0 实施计划后进行一次协调实施。M1–M4 保留为内部依赖、commit、回滚与验收边界；后一个 wave 只有在前一个 wave 的契约、编译和定向验证通过后才能开始，但不再要求用户逐 wave 重新批准。协调器在全部 wave 集成并完成最终验收后一次性交付；任一前序门禁失败都必须停止下游写入并报告真实阻塞，不能以 Host shadow state 或后续 wave 绕过。

## 1. 背景与产品定义

AstrEmbodiment 要实现的不是带定时器的聊天机器人，而是一个具有连续生命过程的“赛博人”。即使没有用户输入，角色也应当拥有持续演化的身体—情感状态、独立时间感、睡眠与清醒、待巩固记忆候选、内生动机和行动倾向；永久记忆能力须在未来独立 authority migration 获批后接入。当某个意图经过意识竞争、关系边界和打扰预算后，系统才能主动联系用户。

本设计采用混合架构：类脑核心决定“为什么想、想什么、是否表达”，LLM 只在内容已赢得 Global Workspace 且确有语言化或复杂推理需要时参与“如何思考、如何说”。后台生命循环不能退化为周期性调用 LLM。

## 2. 目标

v0 必须建立一条可以独立于入站消息运行、可持久化、可恢复的最小生命闭环：

1. 没有用户消息时，内部状态仍能随真实经过时间推进。
2. 运行时根据当前状态自适应预测下一次唤醒，而不是固定高频轮询。
3. 角色拥有独立于用户和 AstrBot 宿主的本地时间、昼夜节律与睡眠状态。
4. 睡眠由 Two-process sleep model 驱动，允许熬夜、补觉、自然醒和紧急唤醒。
5. 内生候选经 16K→2K→256→32 多尺度聚合后竞争进入 Global Workspace。
6. 意识内容可以被内部消解、形成可重建的持久操作意图、形成待巩固记忆候选，或在门控通过后转化为真实主动消息；永久记忆写入不属于 v0，只能在未来独立 authority migration 获批后启用。
7. 默认节省模式下，日常后台演化不消耗 LLM token。
8. 重启后从持久状态继续；内部重复唤醒不得创建第二个 outbound。`claim_dispatch` 事务一旦提交，任何崩溃或结果丢失都必须进入 `dispatch_unknown`，非幂等平台不得自动重发。
9. 用户能够查看由真实结构化事件生成的心境与内心活动记录。

## 3. 非目标

v0 明确不包含以下内容：

- 不训练端到端语言模型，也不要求 16K 节点自行涌现自然语言。
- 不实现持续语言化的“意识流”，不在每个后台 tick 调用 LLM。
- 不模拟完整人脑生理学；睡眠模型只保留对行为连续性有用且可验证的变量。
- 不让 L1/L2/L3 成为第二套可独立写入的权威人格状态。
- 不把 AstrBot 全局时区改为角色时区，也不假设 AstrBot 知道每位用户的所在地。
- 不把“已交给 AstrBot 适配器”声明为平台已送达或用户已读。
- 不在 v0 实现学习式重整化、梦境文本生成、完整世界模型、多角色群体生活或跨设备分布式一致性。
- 不因实现自主运行时而改变现有 Persona Genesis、authority、privacy 或外部效果的既有安全边界。

## 4. Current Baseline 与权限边界

本规格是目标设计，不是对当前实现能力的描述。当前基线必须按以下事实理解：

- `ae-runtime` 当前只接通 **G0 deterministic no-op lane**；`TimeAdvance` 虽是闭合 canonical event，但提交结果不改变神经状态。
- `ae-renorm` 当前只有 `[16_384, 2_048, 256, 32]` 层级常量和 32 个全零 token 的 `empty_workspace()`；restriction、prolongation、候选竞争和 residual 守门尚未实现。
- AstrBot 插件当前没有常驻 scheduler、sleep/wake loop 或真正的 proactive dispatch；`proactive_enabled` 配置项本身不证明主动行为已接线。
- 当前 Python Host 使用墙上 UTC epoch 生成时间观测；它没有 Persona/Relation 时区模型，也不能把宿主全局时区视为用户时区事实。

### 4.1 `TimeAdvance` 的可逆派生状态边界

v0 可以扩展 `TimeAdvance`，但其 authority 仅允许推进由“旧状态 + 冻结时间输入 + 确定性参数”可重算的派生状态，包括 Process S/C、睡眠状态、circadian phase、workspace 派生投影和下一唤醒点。这些值可以持久化为快照/缓存，但删除后必须能从权威事件与参数重建。

`TimeAdvance` 不得仅凭时间流逝写入永久记忆事实、改变关系承诺/亲密度、修改人格或 Persona Genesis、授予主动联系权限、确认平台送达，亦不得把 LLM 生成内容升格为自传记忆。记忆巩固在 v0 仅能生成“待巩固引用/候选”；永久记忆、关系和人格写入必须由各自的 canonical event、source authority、schema migration 与独立批准接线后才能启用。若该 authority migration 尚未完成，相应写入必须 fail closed，不得藏在 `TimeAdvance` delta 中。

### 4.2 交付门槛

实现必须以当前 G0/no-op 为 RED 基线逐层替换；文档、配置、空 workspace、后台任务注册或 Host 侧 shadow state 都不构成 Native 权威能力。各内部 wave 只能声明自身闭合的范围；只有全部 wave 集成、编译、定向验证与手工 AstrBot smoke 通过后，协调器才能一次性声明“赛博人 v0 已完成”。

## 5. 总体架构

```text
                       UTC Causal Clock
                               |
                 Autonomous Runtime Supervisor
                  /            |              \
      Persona Temporal   Relation Policy   Durable Scheduler
            |                  |                 |
            +---------- Sleep/Wake Dynamics ----+
                               |
                       Neural Body (L0)
                               |
                 Multiscale Restriction Pipeline
                   L0 -> L1 -> L2 -> L3
                               |
                       Global Workspace
                   /       |        |        \
             resolve   consolidate  defer   externalize
                                              |
                                    Language/Action Cortex
                                              |
                                   AstrBot Host Adapter
```

### 5.1 Autonomous Runtime Supervisor

常驻、可停止、可恢复的生命周期所有者。它负责读取持久状态、推进经过时间、选择运行强度、提交原生演化、安排下一次唤醒，并在终止时停止接受新工作和刷写状态。它不能依赖“收到聊天事件”才启动。

监督器同一时间对一个 persona 只允许一个有效 lease。并发唤醒可被合并，但不能并行修改同一权威状态。

### 5.2 UTC Causal Clock

所有因果事件、截止时间、lease、幂等键和重放边界均使用 UTC Unix epoch 毫秒。墙上时钟只用于生成观测；状态推进采用已持久化的 `last_advanced_at_utc_ms` 到当前可信 UTC 的有界差值。

### 5.3 Persona Temporal Profile

Persona 级、与具体用户无关的时间表型：

- `home_timezone`：角色长期归属地，使用 IANA TZID，例如 `America/Los_Angeles`；创建后仅由明确的人设变更修改。
- `current_timezone`：角色当前所在地，使用 IANA TZID；旅行时可以立即改变。
- `circadian_phase_minutes`：当前生物钟相位，以角色主观昼夜周期表示，不随 `current_timezone` 瞬间跳变。
- `chronotype`：晨型、中间型或夜型，决定相位锚点和允许的自然波动。
- `preferred_sleep_local`、`preferred_wake_local`：角色归属作息的软锚点，不是强制定时开关。
- `sleep_flex_minutes`：情绪、互动和疲劳可使作息浮动的范围。
- `entrainment_rate_minutes_per_day`：旅行后生物钟向所在地光暗周期靠拢的最大日速率。

### 5.4 Relation Temporal Policy

每段 persona—user 关系独立保存：

- 用户 IANA 时区及其来源：明确设置、平台提供或宿主全局时区 fallback。
- 用户安静时段和紧急突破授权。
- 主动联系总开关、每日次数上限、冷却时长和连续未回复退避。
- 最近入站、最近主动提交、最近确认回复的 UTC 时间。
- 关系亲近度、未完成话题和打扰成本所需的最小结构化信号。

AstrBot 全局时区只能作为首次初始化的 fallback。fallback 必须带来源标记，后续用户明确设置应覆盖它；不得把 fallback 悄然写成永久事实。

#### 5.4.1 冻结 relation key

relation identity 不从显示名、当前 session 或可变昵称推断。Host 必须将每个字段做 UTF-8/NFC 规范化，以 `u32be(length) || bytes` 串接成 canonical relation key，再交给既有 `ae.relation-token.v1` 域散列为 16-byte opaque token：

```text
private: [v1, platform_id, bot_id, persona_id, "private", user_id]
group:   [v1, platform_id, bot_id, persona_id, "group", group_id]
```

`private` 表示 persona 与该平台用户的私聊关系；`group` 表示 persona 与该群会话整体的关系，不能继承某个群成员的私聊授权。缺少任一 identity 字段、字段来自未经验证的缓存、kind 与 UMO 不一致、token 重算不一致时 fail closed。迁移 platform/bot/persona identity 不允许复用旧 relation token，必须显式迁移关系记录。

#### 5.4.2 冻结 outbound target envelope

主动发送目标必须在最近一次经验证的 inbound binding 或显式管理绑定时被冻结，禁止发送时用“最近会话”猜测。`OutboundTargetEnvelopeV1` 包含：

- `target_kind`: 闭合枚举 `private | group`；
- `umo_ciphertext`、`umo_nonce`、`key_id`: AstrBot 原始 UMO 使用 OS credential store 提供的 AES-256-GCM key 加密后的 opaque bytes；AAD 绑定下列所有 token 与 schema version，明文只在 Host 发送调用的最短生命周期内出现；
- `umo_digest`（同一 key 派生的 HMAC-SHA256）、`platform_token`、`bot_token`、`persona_token`、`relation_token`、`session_token`；
- `bound_at_utc_ms`、`binding_generation`、`schema_version`；
- `binding_digest`: 上述 canonical 字段的摘要。

私聊 envelope 必须绑定 private relation 与该用户的 verified UMO；群聊 envelope 必须绑定 group relation 与该群的 verified UMO。`session_token` 只证明目标来源会话，不决定 relation identity；session 轮换需产生新 binding generation。密钥/key_id 不可用、UMO 解密或 HMAC 验证失败、平台/bot 不可用、Persona/Relation/Session 任一 token 不匹配、kind 不匹配、binding 被撤销或过期时，Native 将 outbound 终结为 target-invalid，Host 不得降级到其他会话、私聊或群聊。

### 5.5 Sleep/Wake Dynamics

负责睡眠压力、昼夜相位、清醒程度、睡眠阶段和紧急唤醒阈值，不负责生成语言。

### 5.6 Multiscale Attention 与 Global Workspace

现有神经连续体是身体—情感权威底层。多尺度层提取行为相关模式并进行候选竞争，Global Workspace 只持有当前意识候选与广播结果，不拥有永久人格状态。

### 5.7 Language/Action Cortex

只有已获准外化的结构化意图才能进入此组件。它可调用 LLM 做一次受预算约束的语言化或复杂推理，并将候选结果交给最终内容边界检查。LLM 无权绕过自主性、睡眠、关系、预算、隐私或 authority 门控。

### 5.8 AstrBot Host Adapter

负责把已批准的 outbound envelope 转换为 AstrBot 可接受的发送调用。适配器返回成功仅表示“已提交适配器”；只有具体平台提供且通过验证的回执才能升级为“平台接受”或“已送达”。

## 6. 三层时间模型

系统必须同时持有三种时间视角，但只有 UTC 是因果真源：

```text
UTC instant（事件顺序、持久化、调度、幂等）
├─ Persona local time（睡眠、困倦、自我时间表达）
└─ Relation user local time（安静时段、问候语义、打扰成本）
```

规则如下：

1. 数据库存储 UTC 毫秒与 IANA TZID，不持久化无时区本地时间戳。
2. Persona 本地投影使用 `current_timezone`；`home_timezone` 保留身份与作息锚点。
3. 用户本地投影使用 relation 记录的用户时区。
4. DST offset 由 Host 在事件发生时使用 IANA 时区数据库解析，并把冻结后的 offset/local phase 随原生输入记录。每个冻结时间输入还必须携带 `tzdb_fingerprint = SHA256(provider || tzdb_version || canonical_transition_slice)`；fingerprint 不同的重放不得悄然覆盖原结果。
5. “我这里很晚了”使用 Persona 本地时间；“你该休息了”使用用户本地时间且不得在时区来源仅为低置信 fallback 时武断表达。
6. 系统墙上时钟回退时，`effective_now_utc_ms = max(observed_now_utc_ms, last_committed_utc_ms)`；不允许负时间推进。
7. 超大向前跳变按离线恢复规则处理，不逐 tick 补算。
8. 每日主动次数与 token 预算按 **Relation 用户时区的本地自然日** 结算；边界由 Host 冻结为 `[day_start_utc_ms, next_day_start_utc_ms)` 并携带 tzdb fingerprint。DST 重叠取第一次到达的 00:00，DST 缺口取该日第一个有效 instant，Native 不自行查询系统时区。

### 6.1 Host→Native 冻结时间输入

每次 advance 的 Host 输入必须是闭合结构：`observed_now_utc_ms`、Persona/Relation TZID、对应 UTC offset 秒、Persona local minute/day ordinal、Relation local minute/day ordinal、预算日 UTC 边界、下一条相关 timezone transition UTC、tzdb fingerprint 和输入 schema version。Native 只消费这些冻结值并校验时间单调性；它不得调用操作系统 localtime，也不得从 Python 当前时区推测用户位置。

## 7. 旅行与渐进倒时差

旅行事件包含新 `current_timezone`、到达 UTC 时间和可选的光照/作息线索。事件提交后：

- 地理所在地和当地墙上时间立即切换到新时区。
- 生物钟相位保持连续，不立即跳到新当地时间。
- 每次时间推进根据当地光暗周期、实际睡眠和 `entrainment_rate_minutes_per_day` 对相位做有界调整。
- 相位调整必须使用最短合理方向并保留 accumulated phase error，避免跨日界线时突变。
- 睡眠不足提高 Process S，可使角色到达后补觉；高唤醒与互动可以暂时延迟入睡，但不能清除睡眠债。
- 返回 home timezone 也走同样的渐进同步，不提供隐式“瞬间复位”。

v0 使用确定性相位误差与有界校正。令 `e = wrap_to_720(target_local_phase_min - circadian_phase_min)`，范围为 `[-720, 720)` 分钟；一次推进 `dt_days` 的校正为 `delta = clamp(e, -90 * dt_days, 90 * dt_days)` 分钟，并令 `circadian_phase = mod(circadian_phase + elapsed_minutes + delta, 1440)`。同一旧状态和冻结输入必须产生逐位一致的结果。90 分钟/天是 v0 冻结参数，不声称为医学诊断模型；修改它需要参数 schema version 递增。

## 8. Two-process sleep model

睡眠采用确定性的双过程模型。所有计算使用 Native fixed-point，状态量范围为 `[0, 1]`：

- **Process S（homeostatic sleep pressure）**：清醒时 `S' = 1 - (1-S) * exp(-dt/18h)`；睡眠时 `S' = S * exp(-dt/4h)`。v0 不增加由 LLM 推测的隐式倍率。
- **Process C（circadian wake drive）**：`C = cos(2π * circadian_phase_min / 1440)`，其中 phase 0 是 Persona 的 preferred wake anchor，`C=1` 表示最大促醒。
- **Arousal A**：已提交结构化刺激的有界促醒量 `[0, 0.20]`；每小时乘 `exp(-dt/2h)` 衰减，LLM 无权写入。
- **Sleep score Q**：`Q = clamp(S - 0.25*C - A, 0, 1)`。

状态转换使用滞回阈值，避免在边界附近反复入睡/醒来：

```text
AWAKE -> DROWSY:   Q >= 0.62 持续 10 分钟
DROWSY -> ASLEEP:  Q >= 0.72 持续 15 分钟
DROWSY -> AWAKE:   Q < 0.55
ASLEEP -> AWAKE:   Q <= 0.38，或已授权刺激 urgency >= 0.90
```

v0 的睡眠状态闭合为 `AWAKE | DROWSY | ASLEEP`，不再引入未定义的深浅睡分期。`DROWSY` 降低普通外化优先级；`ASLEEP` 关闭普通外化与 LLM，只允许确定性的 Process S/C 推进、派生记忆索引候选和紧急刺激检查。

熬夜是动力学结果：已提交的强互动、兴奋、担忧或明确目标只能通过 `A <= 0.20` 暂时延后入睡，且累积的 Process S 不会被伪造为零。紧急刺激可以唤醒角色，但唤醒后仍需独立通过用户打扰门控。

### 8.1 v0 冻结参数

| 参数 | 值 | 单位/语义 |
|---|---:|---|
| `tau_s_awake` | 18 | 小时 |
| `tau_s_sleep` | 4 | 小时 |
| `tau_arousal` | 2 | 小时 |
| `arousal_cap` | 0.20 | 归一化值 |
| `drowsy_enter_q` | 0.62 | 归一化阈值，持续 10 分钟 |
| `sleep_enter_q` | 0.72 | 归一化阈值，持续 15 分钟 |
| `drowsy_exit_q` | 0.55 | 归一化阈值 |
| `wake_q` | 0.38 | 归一化阈值 |
| `emergency_wake_urgency` | 0.90 | 归一化阈值 |
| `entrainment_rate` | 90 | 分钟/天 |

阈值持续时间通过累计满足时长计算，不依赖 scheduler 恰好在边界触发。参数以 versioned Native 常量进入 formula digest；Host 配置不能静默改写。

`exp` 与 `cos` 禁止调用平台浮点数学库。v0 使用随 Native 发布的 Q32.32 `exp_neg_v1` 和 1440-entry `cos_minute_v1` 查表/线性插值，表内容 SHA256 进入 formula digest；`dt` 先取整到毫秒，所有乘除采用 round-to-nearest-ties-even。这样跨重启和受支持平台的结果不依赖 libm。

## 9. 自适应唤醒

每次运行结束都输出 `next_wake_at_utc_ms` 和 `wake_reason_class`。下一次唤醒取以下候选的最早安全时间：

- Process S/C 的下一个状态转移边界；
- 未完成意图的重新评估截止时间；
- 派生记忆索引候选或其衰减的下一个显著变化点；
- 用户安静时段结束或主动联系冷却结束；
- 计划事件的到期时间；
- 平静期最大心跳间隔。

运行强度分四级：

1. `micro`：更新时间、Process S/C、睡眠状态和下次唤醒；纯本地计算。
2. `associative`：检索相关记忆引用、传播未完成目标与关系信号；纯本地计算。
3. `ignition`：执行多尺度候选竞争并形成结构化意识事件；默认不调用 LLM。
4. `externalization`：门控通过后才允许语言化、复杂推理或外部行动。

唤醒间隔必须有最小值，防止热循环；同一 persona 的重复调度项通过 generation 和 lease 合并。系统休眠、宕机或插件卸载期间不补跑每个遗漏 tick，而是在恢复时执行一次有界积分。

### 9.1 v0 唤醒参数与选择式

`next_wake = max(now + min_wake_delay, min(valid_candidates))`。若候选计算失败，使用 fail-safe interval；不得立即自旋。

| 参数 | 值 | 使用条件 |
|---|---:|---|
| `min_wake_delay` | 60 秒 | 所有状态 |
| `active_recheck` | 5 分钟 | 有未终结意图或高显著性候选 |
| `asleep_recheck` | 60 分钟 | `ASLEEP` 且无紧急事件 |
| `calm_heartbeat_max` | 360 分钟 | 清醒平静期 |
| `fail_safe_interval` | 30 分钟 | 候选时间无效/溢出 |
| `max_offline_integration` | 168 小时 | 单次恢复积分上限（7 天） |

离线超过 168 小时时，Process S/A 使用 168 小时有界闭式推进，过期意图按当前 UTC 直接终结，并记录一个 `offline_gap`；更早的逐 tick 内心事件不补造。

## 10. 16K→2K→256→32 多尺度边界

层级沿用既定 renormalization 结构：

```text
L0  16,384  微观神经节点（唯一权威动态状态）
L1   2,048  局部神经团（可重建派生状态）
L2     256  功能模体（候选/行动相关宏观状态）
L3      32  Global Workspace tokens（意识竞争工作集）
```

边界要求：

- 真实刺激和最终状态变更在 L0 提交。
- `restriction` 逐级聚合 L0→L1→L2→L3，保留来源引用、显著性、情感价度、目标相关性和置信度。
- L1/L2/L3 可由 L0 与确定性映射重建，不得成为独立可写的人格副本。
- 世界模型和有限候选推演优先运行在 L2/L3；胜出行动通过 `prolongation` 逐级产生对 L0 的受控荷载，而不是直接覆盖 L0。
- 每次跨尺度周期记录 consistency residual；超过阈值时禁止外部行动，只允许降级为内部结构化事件并请求后续重新计算。
- restriction/prolongation 的版本和 digest 随事件记录，使重放可以验证使用了同一映射。
- v0 可以使用固定、确定性的映射；学习式更新不在本范围内。

### 10.1 M2 authority 与瞬态写边界

M2 的 intention 和 prolongation 都是由已批准 canonical authority event 加上固定 formula digest 可重建的**临时操作态**：

- intention 虽为崩溃恢复而耐久化，但不属于 Persona Genesis、关系事实或自传记忆；它必须由 `SelfActionCandidate`（或单独批准的等价 authority）事件重建。
- prolongation 只产生绑定 winning event/revision 的瞬态 L0 activation/control-load delta；它不得修改长期连接权重、人格参数、关系状态、永久记忆或 Genesis seed。
- 瞬态 activation 在事件完成、取消或 TTL 到期后按冻结公式衰减/清除；重启必须从 canonical event、旧 L0 revision 与 mapping digest 得到同一结果。
- 将 intention 内容、activation 结果或 consolidation candidate 提升为永久记忆、关系或人格写入，必须经过独立 authority migration gate；未获批准时 Native 必须拒绝该 commit，而非让 Host 旁路保存。

## 11. Global Workspace 与意图生命周期

L3 最多容纳 32 个 workspace token。候选由内稳态偏差、情绪显著性、记忆共振、未完成目标、关系需要、外部事件和安全约束共同产生。

候选竞争结果必须是结构化的：

- `candidate_id` 与来源事件引用；
- 关注对象与动机类别；
- salience、urgency、confidence、expected_value；
- 关系目标和建议行动类别；
- 支持证据引用及反对信号；
- 竞争结果：`resolved_internal`、`consolidate`、`deferred_intention`、`externalization_candidate` 或 `suppressed`。

只有胜出的 `externalization_candidate` 才能创建持久意图。意图具有 `created_at`、`not_before`、`expires_at`、衰减函数、最大尝试次数和稳定的 idempotency key。持久意图必须通过现有 `SelfActionCandidate` authority 或另行批准的等价 canonical authority 创建，不能作为 `TimeAdvance` 的隐式副作用。未胜出候选可记录为聚合统计，但不得伪装成角色明确“想过”的语言内容。

### 11.1 闭合意图状态机

`IntentionStateV1` 是拒绝未知值的闭合枚举：

```text
formed | deferred | ready | externalizing | dispatch_pending | adapter_call_started |
submitted | dispatch_unknown | retry_wait |
completed | suppressed | expired | cancelled | target_invalid | failed_terminal
```

允许转移如下；未列出的转移一律拒绝：

| From | To | 条件 |
|---|---|---|
| `formed` | `deferred`, `ready`, `suppressed`, `expired`, `cancelled` | 首次门控 |
| `deferred` | `ready`, `suppressed`, `expired`, `cancelled` | 到 `not_before` 后重评 |
| `ready` | `externalizing`, `suppressed`, `expired`, `cancelled` | 预算 claim 成功 |
| `externalizing` | `dispatch_pending`, `retry_wait`, `suppressed`, `expired`, `failed_terminal` | LLM settle |
| `retry_wait` | `ready`, `expired`, `cancelled`, `failed_terminal` | 仅一次允许的生成重试 |
| `dispatch_pending` | `adapter_call_started`, `target_invalid`, `failed_terminal` | Native `claim_dispatch` 事务 |
| `adapter_call_started` | `submitted`, `dispatch_unknown`, `target_invalid`, `failed_terminal` | dispatch settle 或崩溃恢复 |
| `submitted` | `completed`, `dispatch_unknown` | 有证据的后续回执或状态丢失 |
| `dispatch_unknown` | `completed`, `failed_terminal` | 人工/平台对账；不自动再发 |

`completed`、`suppressed`、`expired`、`cancelled`、`target_invalid`、`failed_terminal` 为终态。`dispatch_unknown` 不是成功也不是可自动重试态；它保留对账入口，但阻止同一 semantic intention 创建新 outbound。

## 12. Token 与 LLM 策略

默认模式为 `economy`：

- `micro`、`associative` 和普通 `ignition` 不调用 LLM。
- 内心活动默认只存结构化事件，不生成文字日记。
- 只有主动发送门控全部通过后，才允许启动一次 externalization attempt；每个 attempt 只调用一次 LLM，受 12.1 的总次数与重试契约约束。
- LLM 失败、超时、预算不足或结果被内容边界拒绝时，不强行发送模板闲聊；意图按策略延迟、降级或过期。
- 后台 token 预算耗尽时退回纯结构化演化，生命时钟和睡眠模型继续运行。

### 12.1 LLM 调用、重试与预算契约

- 每个 `externalization attempt` 最多调用 LLM **1 次**；同一 intention 最多 **2 个** externalization attempts，因此终生最多 2 次 LLM 调用。
- attempt 1 仅在所有非语言门控通过且 Native 原子 claim 预算后开始。只有 provider 超时、连接失败或明确可重试的 5xx 可进入一次 `retry_wait`；退避固定 30 分钟且不得越过 `expires_at`。
- 内容安全拒绝、隐私拒绝、目标无效、预算不足、空输出或不可解析输出不触发第二次生成，直接进入对应终态。
- LLM 成功后，以 `SHA256(intention_id || attempt_no || prompt_contract_digest || normalized_candidate)` 固定 candidate digest；后续 dispatch 重试不得再次调用 LLM 或改写候选。
- 每次 claim 先从 Relation 本地日预算预留 `max_tokens_per_attempt=512`，settle 后记实际 provider usage 并释放未用额度。claim 失败时调用数必须为零；Host 崩溃导致 usage 未知时按 512 全额记账，防止重启绕过预算。
- v0 默认每 relation 每本地自然日后台外化预算 1024 tokens，最多覆盖两个 attempts；用户正常入站对话预算不计入此自主预算。

后续可以提供显式启用的 `balanced` 和 `rich` 展示模式，但它们不属于 v0 默认路径，也不能改变类脑核心的权威决策。

## 13. 结构化内心活动与展示

系统记录可审计的 `InnerEvent`，事件类型至少包括：

- `homeostasis_changed`
- `sleep_transitioned`
- `memory_resurfaced`
- `workspace_ignited`
- `intention_formed`
- `action_arbitrated`
- `memory_consolidation_candidate`
- `travel_phase_adjusted`

事件只保存展示所需的最小字段、来源引用和数值变化，不保存未经批准的 provider prompt、原始私密神经向量或第三方聊天内容副本。

默认界面可用本地模板生成两类视图：

- 心境卡片：当前清醒/困倦状态、粗粒度情绪、联系倾向、主要来源类别和当前选择。
- 内心活动时间线：按 UTC 排序并投影到用户选择的显示时区，展示状态变化、记忆引用、意图形成和行动仲裁。

所有展示必须能追溯到已提交的 `InnerEvent`。LLM 不得事后编造角色“刚才想过什么”；若未来启用语言化心迹，文本必须标明为结构化事件的摘要，并受单独开关和 token 预算控制。

## 14. 主动发送门控

主动消息必须按顺序通过以下门控，任一失败即不发送：

1. 存在未过期且仍具内生动机的持久意图。
2. 意图赢得当前 Global Workspace 竞争，跨尺度 residual 在允许范围内。
3. Persona 处于清醒状态，或外部紧急度已越过角色紧急唤醒阈值。
4. relation 的 `proactive_enabled` 明确开启；缺失配置按关闭处理。
5. 用户安静时段允许发送，或意图达到用户授权的紧急突破等级。
6. 每日次数、最小冷却、连续未回复退避和全局速率限制均允许。
7. 当前 LLM token 预算允许一次生成；生成只使用允许进入语言层的最小上下文。
8. 候选文本通过隐私、内容安全、重复、长度和目标路由检查。
9. 为 outbound envelope 取得持久化发送 lease，并完成 dispatch intent 写入。

“紧急”必须来自受约束的事件类别和显著性证据，不能由 LLM 自行宣称。紧急突破安静时段不突破用户总关闭、隐私、authority 或平台能力边界。

连续未回复采用指数退避并设置上限；任何新入站消息都只按关系策略更新退避，不自动等价为“欢迎持续主动联系”。

### 14.1 v0 Relation 门控参数

| 参数 | 默认值 | 语义 |
|---|---:|---|
| `proactive_enabled` | `false` | 必须显式 opt-in |
| `proactive_daily_max` | 2 | 每 Relation 用户本地自然日成功进入 `adapter_submitted` 的最大条数 |
| `min_proactive_cooldown` | 6 小时 | 两次普通主动提交之间 |
| `intention_ttl` | 24 小时 | 超时转 `expired` |
| `unanswered_backoff_base` | 6 小时 | 第 n 次连续未回复后为 `min(72h, 6h * 2^(n-1))` |
| `unanswered_hard_stop` | 3 次 | 达到后停止普通主动消息，直至新入站或明确授权恢复 |
| `emergency_threshold` | 0.90 | 仅可突破已授权的 quiet hours，不突破总关闭、daily max 或 hard stop |

计数归属以 outbox 第一次进入 `adapter_submitted` 时所在的冻结预算日为准；`dispatch_unknown` 保守计入当日次数，后续对账不得跨日重复扣减。

## 15. 持久化模型与 Native ownership

生产环境中，Rust `ae-store` 管理的 SQLite 是下列自主状态的**唯一权威存储**。Python Host 不得用内存字典、JSON sidecar、AstrBot config 或第二个 SQLite 保存可恢复的 shadow authority；它只能持有可丢弃缓存和由 Native claim 返回的短生命周期工作项。

v0 至少持久化以下逻辑记录：

- `persona_temporal_profile`
- `relation_temporal_policy`
- `autonomous_runtime_state`
- `inner_event`
- `durable_intention`
- `wake_schedule`
- `outbound_attempt`

`autonomous_runtime_state` 包含 L0 状态引用/版本、多尺度映射 digest、sleep state、Process S、circadian phase、last advanced UTC、next wake UTC 和 monotonic generation。

### 15.1 Schema 与 migration ownership

- `ae-store` crate 独占建表、索引、约束、schema version 和 migration 顺序；`ae-pyo3`/Host 只能调用版本化 FFI，不能发任意 SQL。
- 每个 migration 是 Native 编译产物中的单向、事务化步骤，记录 `from_version`、`to_version`、migration digest 和完成 UTC；数据库版本高于当前二进制或缺少连续迁移时拒绝打开。
- identity-bearing canonical bytes、relation key、intention enum、outbox enum 和 frozen-input schema 的变化必须提升 schema version；不允许依靠 serde 默认值静默解释旧记录。
- migration 只能转换已有 authority。把永久记忆、关系或人格写权限交给 `TimeAdvance` 属于 authority migration，必须另有已批准的 canonical contract，不能作为普通表结构迁移夹带。
- migration 失败须回滚并保持原数据库可重开；启动期间 migration 未完成前 scheduler 与 dispatch 均不得运行。

持久化采用单事务提交：一次自主周期的状态推进、InnerEvent、意图变化和下一唤醒时间必须原子提交。外部发送不能与数据库事务形成假原子性，因此采用 transactional outbox：

1. 事务内写入带稳定 `outbound_id` 的 `dispatch_pending`。
2. 提交后由适配器执行发送。
3. 记录 `adapter_submitted`、`platform_accepted`、`delivery_confirmed`、`dispatch_unknown` 或具体失败态中实际可证明的最高状态。
4. 只有 `claim_dispatch` 尚未成功时，capability/target preflight 的可重试失败才可重新评估同一 `dispatch_pending`。一旦 claim 成功，任何崩溃、超时或结果丢失都进入 `dispatch_unknown`；v0 不在 claim 后自动重发，非幂等平台尤其禁止重发。

## 16. 重启、claim/settle 与幂等

初始化恢复顺序固定为：

1. 验证 schema 版本、persona identity、映射 digest 和持久化完整性。
2. 获取 persona runtime lease。
3. 读取最后已提交状态和未终结 outbox。
4. 计算 `elapsed = clamp(now - last_advanced, 0, max_offline_integration)`。
5. 对快变量采用闭式衰减或稳定积分，对慢变量执行一次有界推进；不重放每个遗漏 tick。
6. 处理已到期意图和计划事件。
7. 先协调未终结 outbound；`dispatch_unknown` 阻止同语义新发送并等待平台/人工对账，再创建其他主动发送候选。
8. 提交新的状态与下一唤醒点。

幂等键由 persona、relation、intention、semantic revision 和 action class 构成，不包含重试次数。相同语义意图在 cooling window 内不得生成新的 outbound id。调度 generation、runtime lease 和数据库唯一约束用于降低重复概率，但本设计**不承诺跨非幂等外部平台 exactly-once**；在“调用可能已发生但结果不可知”时，宁可进入 `dispatch_unknown` 并停止自动发送，也不冒险重复打扰用户。

### 16.1 FFI claim/settle 状态机

所有跨 Python Host 的非纯计算工作都采用 Native 原子 claim/settle；Host 不能直接改状态：

```text
scheduled --claim_wake--> wake_claimed --settle_wake--> scheduled | failed
ready --claim_externalization--> externalizing
externalizing --settle_externalization--> dispatch_pending | retry_wait | terminal
dispatch_pending --claim_dispatch/native-tx--> adapter_call_started
adapter_call_started --settle_dispatch--> submitted | dispatch_unknown | terminal
```

- `claim_wake(frozen_time_input, expected_generation)` 在 `ae-store` 单事务内校验 lease/generation，保存待提交的确定性 advance proposal 并返回 `wake_claim_token`；权威 revision 尚不推进。重复 claim 返回原 claim 或冲突，不重复生成 proposal。
- `settle_wake(claim_token, outcome)` 原子提交 InnerEvent、意图变化与下一唤醒；token 过期但未产生外部效果可回收。
- `claim_externalization(intention_id, attempt_no, budget_day, expected_revision)` 原子预留 token 预算并返回最小脱敏 prompt contract。`settle_externalization` 必须提交 provider outcome、实际/未知 usage、candidate digest 与候选密文，且只接受一次。
- capability snapshot、目标 binding、密钥可用性和适配器能力 preflight 必须在 `claim_dispatch` 前完成；仅这些 preflight 的显式可重试失败允许保留 `dispatch_pending` 并稍后重试。
- `claim_dispatch(outbound_id, expected_target_digest)` 必须在同一个 `ae-store` 事务中再次校验 preflight digest/revision/lease、把 outbox 从 `dispatch_pending` 原子改为 `adapter_call_started`，提交事务后才向 Host 返回冻结 target envelope 与 `dispatch_claim_token`。Host 没有写 `adapter_call_started` 的 FFI 或 SQL 权限。
- `settle_dispatch` 的闭合 outcome 为 `adapter_rejected_terminal | adapter_submitted | platform_accepted | delivery_confirmed | dispatch_unknown`。只要 `claim_dispatch` 的 Native 事务已经提交，即使返回值在到达 Host 前丢失，也按“外部调用可能发生”处理；即使 Host 在真正调用 AstrBot 前崩溃，恢复也必须把遗留 `adapter_call_started` 归类为 `dispatch_unknown`，非幂等平台不得自动 claim 第二次。安全优先于可能漏发。
- claim token 绑定 persona、relation、record id、revision、lease deadline 与 caller incarnation；错误 incarnation、过期 revision 或重复 settle 一律拒绝。

### 16.2 Host→Native 非时间冻结输入

Host 必须冻结并传入：AstrBot capability snapshot、proactive 配置来源与 revision、relation/target binding digest、provider identifier、token budget day、平台幂等能力和内容边界版本。Native 把输入 digest 写入 decision/outbox；Host 状态变化后必须重新 claim，不能在旧决策上替换目标或权限。

## 17. 错误处理与降级

- 时区无效：拒绝该配置更新；运行时保持最后一个有效 TZID。首次启动无有效 Persona 时区则禁用自主外化，但允许 UTC 下的低成本状态维持。
- 用户时区未知：标为低置信 fallback，使用更保守的打扰策略；无法可靠判定安静时段时不主动发送。
- 时钟回退：推进量置零并记录异常，不反向演化。
- 超长离线：执行有界恢复并记录 gap；过期意图直接过期，不集中补发。
- 多尺度 residual 超限或映射 digest 不匹配：禁止外化，保留内部诊断事件。
- Native 演化失败：回滚本周期事务，按指数退避重新调度；不得调用 LLM 补偿。
- LLM 失败或返回不合格文本：不发送；按意图剩余寿命决定一次延迟重试或终止。
- AstrBot dispatch 失败：只有成功 `claim_dispatch` 之前的 capability/target preflight 显式可重试失败可以重试。claim 成功后的任何崩溃、超时、适配器异常或结果丢失都进入 `dispatch_unknown`；v0 不自动重发，非幂等平台不得重发。
- 持久化损坏、identity/authority 不匹配：fail closed，停止主动动作并保留只读诊断入口。
- 调度器失效：入站事件可触发一次恢复检查，但不能成为自主性的唯一运行路径。

## 18. 安全与隐私

- 自主发送默认关闭，必须由 relation 明确 opt-in。
- 用户可随时关闭主动联系；关闭后取消尚未提交适配器的 outbound，并保留最小审计记录。
- 内部神经向量、私密记忆正文和未公开关系推断不得进入 AstrBot history、日志或 LLM prompt。
- LLM 仅接收为当前已批准意图构造的最小、脱敏上下文。
- 心境展示与主动消息是独立开关；关闭展示不停止类脑演化，关闭主动消息也不清除内部生活。
- 日志不得记录消息全文、访问令牌、平台凭据或可反推出原始神经状态的数据。
- 紧急突破只绕过明确允许绕过的安静时段，不绕过总关闭、速率、隐私、内容安全和 authority。
- 所有外部效果保留 persona/relation scope，禁止跨用户泄漏或把一个用户的安静时段用于另一个用户。
- 数据保留策略应支持删除可读心迹与关系数据，同时保持满足一致性所需的最小墓碑，防止删除后旧 outbox 复活。

## 19. v0 实现范围

v0 只需打通以下纵向切片：

1. 一个 persona 的持久 Autonomous Runtime 生命周期。
2. UTC 时钟、Persona/Relation 时区投影和宿主时区 fallback 来源标记。
3. Two-process sleep、独立作息、熬夜、自然醒、紧急唤醒和旅行后的渐进倒时差。
4. 自适应下一唤醒计算与离线有界恢复。
5. 固定确定性的 16K→2K→256→32 restriction、L3 竞争和受控 prolongation 边界。
6. 一个可持久、可衰减、可过期的内生待表达意图。
7. 默认 economy 模式与一次受控 LLM 语言化。
8. 一条通过 AstrBot Host Adapter 的真实 proactive dispatch 路径及 outbox 状态。
9. 心境卡片和内心活动时间线所需的结构化事件查询。

v0 不要求一次实现全部情绪种类和世界模型。首个内生动机可聚焦“关系联结需要 + 未完成话题”，但其形成必须来自持续状态与记忆引用，不能由固定时刻随机生成问候。

## 20. 验收标准

### 20.1 时间与睡眠

- Persona 为 `America/Los_Angeles`、用户为 `Asia/Shanghai` 时，角色睡眠判断与用户安静时段分别按各自时区计算。
- DST 切换不改变 UTC 因果顺序，不造成重复或漏掉已持久化唤醒。
- 改变系统时区不改变既有 UTC 事件和角色生物钟相位。
- Persona 旅行到 `Asia/Shanghai` 后，current timezone 立即变化，circadian phase 在多日内有界靠拢而非瞬移。
- 高互动可导致有限熬夜，随后 Process S 保留睡眠债并允许补觉。

### 20.2 自主性与多尺度

- 在没有任何 inbound message 的观察窗口内，至少产生一次可验证的内部状态推进和 InnerEvent。
- 平静期、活跃期、睡眠期能够产生不同的下一唤醒间隔。
- L0 是唯一权威状态；L1/L2/L3 可重建且不能直接提交人格变化。
- 一个内生候选能够经过 L0→L3 进入 Global Workspace，并形成或拒绝持久意图。
- residual 超限时不发生外部发送。

### 20.3 成本与展示

- 默认 economy 模式下，micro、associative 和未外化的 ignition 周期 LLM 调用数为零。
- 只有全部发送门控通过后才发生候选消息生成；每 attempt 最多一次、每 intention 最多两次，且第二次只用于规定的 transient provider failure。
- 心境卡片和时间线中的每一项都能追溯到已提交 InnerEvent；不存在事后生成的虚构意识流。

### 20.4 可靠性与边界

- 插件重启后从最后提交状态继续，不逐 tick 补算长时间离线窗口。
- 相同意图的重复唤醒或进程崩溃恢复不会产生第二个 outbound id。
- `claim_dispatch` 事务提交后的任何崩溃或结果丢失都进入 `dispatch_unknown`；非幂等平台恢复后不自动重发，即使无法证明 adapter call 已真正开始。
- `proactive_enabled=false`、用户时区不可靠、Bot 深睡且非紧急、安静时段未授权突破、预算耗尽、冷却中或连续未回复退避时均不发送。
- AstrBot 调用成功最多记录 `adapter_submitted`，除非存在可验证的平台回执。
- terminate 后不再启动新周期；已开始事务要么提交完整周期，要么完整回滚。

## 21. 手工 AstrBot smoke

该 smoke 是 v0 的真实宿主验证，不以单元测试或静态检查替代。执行前使用专用测试 persona、专用测试会话和可辨识的测试消息，避免打扰真实用户。

1. 在 AstrBot 中安装当前构建，配置 Persona：`home_timezone=America/Los_Angeles`、`current_timezone=America/Los_Angeles`；配置测试用户：`timezone=Asia/Shanghai`、`proactive_enabled=true`，并设置一个短、可观察的冷却窗口。
2. 启动插件并记录 runtime state 的 UTC、Persona local time、user local time、sleep state、Process S/C、generation 和 next wake；三种时间投影应与当时 tzdata 一致。
3. 不向 Bot 发送任何消息，等待自适应唤醒；确认 state revision 和结构化 InnerEvent 增加，且 LLM 计数保持为零。
4. 通过受控配置/夹具建立“关系联结需要 + 未完成话题”信号，使候选进入 Global Workspace；在一个门控失败条件下确认仅生成 deferred/suppressed 事件，无 AstrBot outbound。
5. 解除该单一门控并等待下一次评估；确认该 attempt 只调用一次 LLM，outbox 创建一个稳定 outbound id，AstrBot 测试会话收到一条主动消息，状态最高只声明实际可证明的适配器/平台阶段。
6. 分别在 `claim_dispatch` 前、claim 成功后但 adapter call 前、adapter call 后/回执前执行插件重启：仅第一种可在重新完成 capability/target preflight 后继续同一 `dispatch_pending`；后两种都必须进入 `dispatch_unknown`，且不得在非幂等测试适配器上自动重发。
7. 把 Persona 置于 `ASLEEP`，确认普通意图不发送；注入允许类别的高紧急刺激，确认角色先产生 wake transition，再独立执行用户打扰门控。
8. 将 `current_timezone` 改为 `Asia/Shanghai`，确认当地时间立即变化而 circadian phase 仅按日速率推进；恢复为洛杉矶时同样不瞬间复位。
9. 关闭 `proactive_enabled`，确认后续内部状态仍继续演化，但无新 outbound；关闭内心活动展示，确认只影响查询/呈现，不停止生命时钟。
10. 卸载或停止插件，确认 terminate 完成、lease 释放且没有后台任务继续产生写入。

smoke 证据至少保留：插件/仓库版本、persona/relation scope、UTC 时间线、状态 revision、generation、InnerEvent 类型、LLM 调用计数、outbound id、AstrBot 提交结果和可用的平台回执。消息正文与私密内部状态应脱敏。

## 22. 一次协调实施的内部 waves 与完成定义

用户已明确批准一次性实施。实施计划必须把下列边界编译成一个有向无环任务图；各 wave 是内部依赖、commit、回滚和验收边界，不是新的用户审批点。后一个 wave 不得用 Host mock 冒充前一个 Native 契约完成，同一路径的写入必须串行，不相交的路径可由 Fast Lane 在同一 ready wave 并行执行。

### Wave 0 — Authority Migration（内部硬先决任务）

范围：先把当前 `TimeAdvance { elapsed_ms }` 与空 `delta_bytes` 的 G0 契约迁移为带 schema version、冻结时间输入 digest 和闭合 derived-state delta 的 canonical contract；同时冻结 autonomous state、InnerEvent、Intention、outbox、claim/settle 的所有权。`TimeAdvance` 仍保持零 residual / 零永久记忆 / 零关系承诺写权限；本 wave 迁移的是“可逆自主派生状态可由 Native 单事务提交”的 authority，不是给时间流逝授予人格、关系或自传记忆 authority。

完成证据：旧数据库可经连续、事务化 migration 打开；旧 `TimeAdvance` 要么被显式版本迁移，要么以稳定错误拒绝，绝不靠 serde 默认值解释；authority matrix 继续证明 `time_advance.allow=[]`；Native 拥有派生状态与 claim/settle，Host 不存在第二份可恢复 authority。Wave 0 未通过时 M1–M4 均不得开始。

### M1 — Clock + Scheduler + Sleep

范围：`ae-store` autonomous schema、Host→Native FrozenTimeInput、UTC 单调推进、Process S/C 与旅行相位方程、wake claim/settle、自适应 next wake、离线 168 小时 cap、terminate/lease。M1 仅允许 `TimeAdvance` 写可逆派生状态，不产生永久记忆或主动发送。

完成证据：G0 no-op 基线被明确替换为可重放的时间派生 delta；相同旧状态和冻结输入 digest 得到相同状态/next wake；重启不补造 tick；默认 LLM 调用为零。

### M2 — Renorm + Workspace + Intention

依赖已验收的 M1 Native 时间状态与 store schema。范围：固定 16K→2K→256→32 restriction/prolongation、formula digest、consistency residual、L3 候选竞争、闭合 IntentionStateV1、externalization claim/settle、每 intention LLM 次数与预算账本。intention 与 prolongation 必须保持可从权威事件重建的临时操作态；prolongation 只改瞬态 L0 activation。永久记忆只输出 consolidation candidate，等待未来独立的永久记忆 authority migration。

完成证据：L1/L2/L3 可从 L0 重建；residual 失败 fail closed；无入站时能形成并持久化一个非随机内生意图；每 attempt/意图调用上限由 Native 约束而非 Host 约定。

### M3 — Relation + Outbox + Proactive Smoke

依赖已验收的 M1、M2。范围：canonical private/group relation key、加密 OutboundTargetEnvelopeV1、Relation 本地日预算、九级门控、transactional outbox、dispatch claim/settle、`dispatch_unknown`、AstrBot adapter capability 与真实 proactive smoke。

完成证据：目标失效不降级路由；private/group 授权不串用；claim 前 preflight 失败可安全重试；claim 成功后无论实际 call 是否开始，只要结果未知都进入 `dispatch_unknown` 且非幂等不自动重发；真实 AstrBot 会话收到一次主动消息，但报告只声明可证明的 delivery stage。

### M4 — Display Query

依赖已验收的 M1–M3 及其稳定 InnerEvent schema。范围：Native 只读分页查询、心境卡片模板、按选定时区投影的内心活动时间线、展示开关、保留/删除与 tombstone 行为。查询不得触发生命推进、LLM、意图或发送。

完成证据：每个展示项可回指 committed InnerEvent；关闭展示不停止 runtime；删除后旧 outbox 不复活；未知 enum/schema fail closed 而非编造文字。

固定 DAG 为 `Wave 0 → M1 → M2 → M3 → M4 → final integration acceptance`。每个边界均产生独立 candidate commit 与可回滚点；改变前序 wire/schema 必须先回到对应 wave 修订并重跑所有下游验收。协调器不在中途向用户交付半成品，除非出现无法在授权范围内消除的阻塞。

v0 完成必须同时满足：代码可编译、持久状态可恢复、默认后台零 LLM、无入站时确有状态演化、睡眠与双时区行为符合本规格、门控失败不发送、手工 AstrBot smoke 完成一次真实主动消息，并在受控场景中验证 claim 提交后未知会暂停而非自动重发。该验证不构成对所有外部平台 exactly-once 的承诺。仅有类型、配置项、空 workspace、定时器注册、静态检查或 focused test 均不能单独宣称 v0 完成。
