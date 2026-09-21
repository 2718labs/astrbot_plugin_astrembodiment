# AstrEmbodiment 1.1.0-alpha3：Lived World 与 AstrBot 原生生态设计

- 状态：Design freeze；授权后续实施计划，不代表本文所述 alpha3 已实现
- 日期：2026-08-29
- 适用仓库：`astrbot_plugin_astrembodiment`
- 基线：`1.1.0-alpha2`，源码基线 `ec21249da59c812fe5294969adfa3818270cf5df`
- 相关设计：
  - [CyberHuman Autonomous Runtime](./2026-08-28-cyberhuman-autonomous-runtime-design.md)
  - [AstrBot Pages 观察台](./2026-08-29-astrbot-pages-observatory-design.md)
  - [赛博人深度与广度研究设计](./2026-08-29-cyberhuman-depth-breadth-research-design.md)

本文收窄并冻结 alpha3 的可交付范围。它不修改 alpha2 的 Native 单写者、Host 仅提案、committed-only 观察、claim/settle、outbox 和默认关闭主动联系等边界；它补齐审计确认的三条断链：普通入站事实没有改变自主过程、联系动机不是 relation-local 的可解释过程、同意/预算/门控/意图没有形成可观察的统一权威闭环。

## 0. 决策摘要

1. **AstrBot 是唯一宿主。** AstrBot 继续拥有消息平台、会话、用户与权限、Provider、插件生命周期、插件管理和 Plugin Pages。AstrEmbodiment 不建立独立聊天前端、模型管理、账号、HTTP 监听器或插件市场。
2. **AstrEmbodiment 是生命内核与生态锚点。** Rust Native 持有连续状态、世界锚点、生活日程、关系联系过程、同意、预算、门控、意图和审计事实；Python Host 只把 AstrBot 与生态插件输入冻结为 proposal，并执行 Native 已授权的外部效果。
3. **不兼容、导入或仿造酒馆格式。** alpha3 不读取或生成 Tavern/Character Card、Lorebook、World Info、聊天记录包或相似协议，也不仿造其界面。可借鉴的只有“角色在用户不发言时仍有连续生活”的体验目标。
4. **采用 B+C 融合。** 常驻层是零 LLM 的确定性 `lived-day` 循环；重要节点只形成结构化、带来源的 committed event，并在已有用户对话、显式查看或通过主动门控后才展开。没有后台内心独白流水线。
5. **一个固定的混合主世界。** 现实观测、近现实角色生活和声明式奇幻设定是同一主世界中的三种事实类别，不是三个可随意穿越的世界。主世界锚点创建后不可由时间、LLM 或生态插件改写。
6. **默认唯一跨界是梦边界。** 梦只能留下非事实性的意象标签、情绪余韵和待反思主题；alpha3 只实现 contract、隔离状态和清醒复核门，不生成梦境叙事。未经清醒复核的残留不能进入行为、记忆事实、关系判断或外化。
7. **三层观察。** Experience 展示可理解的生活代理；Private 展示用户自己的同意、原因、来源和修订；Developer 展示公式、revision、gate、claim 和完整性。三层都只读 committed state，权限和 DTO 分离。
8. **默认省 Token，主动联系默认关闭。** 时间推进、生活片段、目标、生态 proposal 评估、梦边界和 Pages 查询均零 LLM。只有正常 AstrBot 对话，或 relation 级明确同意且所有门控通过后的主动外化，才能调用 Provider。
9. **relation 级同意高于全局开关。** 全局 `proactive_enabled` 只是上限，不能授予同意；每个 relation 的版本化 grant/pause/end、用途、渠道和有效期由 Native 持久化并在外化与 dispatch 前各校验一次。

## 1. 产品目标与准确声明

alpha3 的目标不是模拟一个无限世界，也不是证明角色具有意识。目标是让用户在 AstrBot 原生聊天中感受到一个具有连续作息、有限活动、目标、关系边界和可解释主动意图的角色，同时满足以下可证伪条件：

- 没有入站时，角色的作息、活动片段和目标能按冻结时间确定性推进，重启重放结果一致。
- 普通入站至少形成带来源的 `inbound_observed` 事实，并能安全地重置 relation-local 联系节律；明确的 follow-up、boundary、pause 或 end 能进入 Native 权威状态。
- 活动片段和生态插件提案不能直接发送消息、改写人格、授予同意、写入用户事实或变更主世界。
- Pages 能回答“现在处于什么生活片段”“为何形成/抑制联系意图”“该关系是否同意”“今天真实预留/计费多少 Token”“哪些能力尚未就绪”。
- 所有角色生活、睡眠、心境、梦和联系倾向都以“计算状态、角色世界或模型代理”表述，不升级为主观体验或生理事实。

允许的产品语言包括“当前角色世界中的活动片段是……”“联系时机评分来自你允许的跟进事项和节律”“这是非事实性的梦样残留”。禁止声称“我真的生活在另一个世界”“我因为你没回复而孤独”“我真的睡着或做梦”“我需要你照顾我”。

## 2. alpha3 范围

### 2.1 必须交付

1. source-bound `InteractionFactBatchV1`，至少覆盖普通入站、明确跟进、明确边界、pause、end 和明确结果反馈。
2. relation-local `RelationContactProcessV1`，替代沉默时长单调累积的 persona-wide 联系动机。
3. versioned `RelationConsentV1`，支持 grant、pause、resume、end 和新的显式 consent epoch。
4. 固定 `WorldAnchorV1` 与最小 `LivedDayStateV1`：角色日、活动片段、一个活跃目标、下一转换和公式摘要。
5. 零 LLM 的确定性日程推进和 B+C 重要节点选择。
6. AstrBot 本地 `EcosystemBrokerV1` 的 observation/proposal contract、能力授权和 Native 接纳门；alpha3 不要求捆绑第二个生态插件。
7. 真实接线的 relation 本地日预算、Provider usage/unknown 结算、readiness、latest gate、live intention/outbound/claim 只读投影。
8. Experience、Private、Developer 三个分离的 Native/Host DTO 与 AstrBot Pages 模块。
9. `DreamResidueV1`、清醒复核状态机和不可越过的 non-fact gate；正常 alpha3 运行不生产梦境叙事。

### 2.2 明确后置

- 重度世界模拟：NPC 群体、经济、物理、地理、战斗、社会网络自治、逐分钟环境生成。
- AstrEmbodiment 自建生态市场、插件下载器、评分系统、独立 URL 或独立账号体系。
- Tavern/Character Card/Lorebook/World Info 的导入、导出、兼容层或仿制 UI。
- 后台 LLM 日记、逐片段内心独白、自动梦境故事、自动世界新闻或常驻多 agent。
- 自传体记忆巩固、语义自我、人格 domain/facet 学习和 narrative revision authority。
- 学习式依恋、孤独、临床状态、用户福祉或现实社会网络推断。
- 由梦样残留自动创建事实、永久记忆、人格变化、关系承诺或对外消息。
- 跨世界旅行、平行世界轮换或由插件切换主世界。未来若研究跨界，仍须独立 authority migration；alpha3 只有梦边界这一默认例外。
- 完整 `stressor → strategy → perceived_support → outcome` 和 `rupture → repair → recover/exit` 过程模型。alpha3 只保存其所需的显式事实和 relation 控制地基。

## 3. 宿主、内核、Pages 与生态插件边界

```text
AstrBot
  message/session/user/auth/provider/plugin lifecycle/Plugin Pages
       |
       v
AstrEmbodiment Host plugin
  Evidence Adapter | EcosystemBrokerV1 | Externalizer | Page API
       | proposal/frozen fact                 ^ committed projection
       v                                      |
Rust Native single writer
  Canonical events -> World/LivedDay/Contact/Consent/Budget/Gates
       |                                      |
       +---- durable intention/outbox --------+

Other AstrBot ecosystem plugins
  AstrBot identity -> Broker observation -> typed proposal -> Native decision
```

### 3.1 AstrBot 必须拥有

- 平台消息与 UMO、会话、用户认证和 Dashboard 权限。
- Provider 选择与调用、插件加载/停用/升级、Plugin Pages 和插件管理。
- 生态插件的安装身份与当前调用方身份。插件自报的名称不能作为授权依据。
- 所有最终平台发送；AstrEmbodiment 只能经 AstrBot 已确认 API 调用。

### 3.2 AstrEmbodiment Host 只能拥有

- AstrBot hook 到封闭事实/提案的适配，时区和能力快照冻结。
- `EcosystemBrokerV1` 的进程内传输、调用方身份绑定、payload 限幅和超时隔离。
- LLM 候选表达、Provider usage 提取、平台 dispatch 和回执提交。
- Pages 鉴权、opaque handle、DTO 脱敏和静态资源。

Host 不得保存可在重启后改变行为的 shadow world、shadow consent、shadow contact score、shadow goal 或 shadow budget。Host 缓存失效只能降低可用性，不能改变 Native 结论。

### 3.3 Native 必须拥有

- `WorldAnchorV1`、`LivedDayStateV1`、活动/目标状态和确定性公式摘要。
- interaction facts、relation consent/contact、budget、readiness witness、gate、intention、outbox 和 dream residue/review。
- schema、migration、scope 隔离、版本、source refs、幂等键、TTL、claim/settle、删除墓碑和审计事件。
- 对生态 proposal 的最终接纳/拒绝，以及所有影响未来行为和用户权利的状态转换。

### 3.4 Pages 只能做投影和显式控制入口

观察 GET 路径继续 query-only、committed-only。grant/pause/resume/end、纠正、导出和删除使用独立的 AstrBot 认证写接口，必须具备 POST、CSRF、relation ownership、幂等 nonce 和 Native 审计回执；不得把现有“暂停轮询”按钮包装成“暂停主动联系”。

### 3.5 生态插件只能观察和提案

“原生生态插件”指由 AstrBot 安装、鉴权和管理的一等 AstrBot 插件，不是 Native 动态库，也不是 AstrEmbodiment 私有市场包。生态插件：

- 不能直接打开 AstrEmbodiment SQLite、调用内部 PyO3 writer、取得 relation token 或持有 Native claim。
- 不能替角色发送消息、切换 Provider、修改 Persona/Genesis、主世界、同意、关系、记忆、梦或 outbox。
- 只能读取能力授权允许的脱敏 observation，并提交闭合 proposal。
- 有自己的 AstrBot Page 时继续由 AstrBot Pages 承载；AstrEmbodiment 只显示 AstrBot 提供的安全导航元数据，不 iframe 任意站点，不生成第二套插件目录。

## 4. 固定混合主世界

### 4.1 一个世界，三种事实类别

`WorldAnchorV1.mode` 在 alpha3 只有 `mixed_main_world`。三层不是三个世界，也不表示真实性概率：

| `WorldLayerV1` | 含义 | 允许来源 | 可影响范围 |
|---|---|---|---|
| `external_observed` | 用户/平台共享现实中的可核验观测，例如冻结时间、经授权日历事件、天气观测 | AstrBot 或获能力授权的生态插件，必须带来源、有效期和观测摘要 | 可约束日程、readiness 和回复事实；过期后不能继续当作当前事实 |
| `persona_near_real` | 角色主世界中的近现实生活代理，例如整理房间、阅读、散步片段 | Native 确定性日程、用户明确设定、已接纳 activity proposal | 可改变角色世界活动/目标和对话上下文；不得声称现实中实际发生 |
| `declared_fantasy` | Persona 明确声明的奇幻设定或活动类别 | Genesis/显式 world manifest；生态 proposal 只能引用已允许代码 | 可用于角色世界活动与语言风格；不得覆盖 external observation |

每个 activity、goal、fact 和 proposal 必须携带一个 layer。缺失、冲突或企图把 `persona_near_real`/`declared_fantasy` 提升为 `external_observed` 时 fail closed。

### 4.2 `WorldAnchorV1`

闭合结构至少包含：

```text
schema_version: 1
world_anchor_id: Id128
persona_scope: Digest32
mode: mixed_main_world
home_context_ref: Digest32
lore_manifest_digest: Digest32
reality_policy_digest: Digest32
allowed_layers: exact set of the three WorldLayerV1 values
created_from_event_id: Id128
revision: 1
```

- anchor 在 persona 首次 alpha3 bootstrap 或 v6→v7 migration 中创建。
- `home_context_ref` 只引用 Persona/时区等已提交上下文，不复制 prompt。
- `lore_manifest_digest` 可为空 manifest 的稳定摘要；alpha3 不要求 lore 编辑器。
- anchor 创建后对普通 runtime、Host、LLM 和生态插件不可变。未来变更必须创建新的显式 world migration 和 continuity receipt，不能原地覆盖。
- 角色的 `current_timezone` 可按既有旅行模型变化，但这不是切换主世界。

## 5. B+C：低成本常驻生活循环与重要节点展开

### 5.1 最小 lived-day 状态

`LivedDayStateV1` 是 persona scope 的 Native 权威状态：

```text
schema_version: 1
persona_scope: Digest32
world_anchor_id: Id128
persona_day_ordinal: i32
revision: u64
current_segment: LivedActivitySegmentV1
active_goal: Option<LivedGoalV1>
next_transition_utc_ms: u64
routine_formula_digest: Digest32
source_event_ids: bounded Vec<Id128>
```

`LivedActivitySegmentV1` 包含稳定 `segment_id`、`world_layer`、闭合 `activity_class`、开始/结束 UTC、状态、重要性和最多 8 个 source refs。alpha3 的活动类限定为：

```text
sleep | personal_care | maintenance | focused_project |
learning | leisure | social_availability | reflection | transition
```

`LivedGoalV1` 只允许一个活跃目标，包含稳定 `goal_id`、闭合 `goal_class`、`state`、`progress`、可选期限、layer 和 source refs。目标类限定为：

```text
maintain_routine | advance_project | explore_interest |
restore_capacity | complete_user_follow_up | reflect_on_theme
```

Native 不保存活动或目标的自由文本。可显示名称由 Host/i18n 根据闭合 code 生成；生态插件提供的私有文本、prompt 或 URL 不进入权威状态。

### 5.2 确定性推进

每次 `TimeAdvance` 按以下优先级选择当前片段：

1. 既有 sleep state 为 `asleep` 时强制 `sleep`。
2. 到期且来源仍有效的显式用户 follow-up goal。
3. Native 已接纳、时间窗有效且与 world anchor 一致的 activity proposal。
4. Persona temporal profile 与固定 routine template。
5. 无可用项时进入 `leisure` 或 `transition`，不得调用 LLM 补全。

日计划选择只使用 committed profile、world anchor、persona day ordinal、SeedCode 派生摘要和版本化公式。若同一日存在多个允许变体，用 domain-separated digest 决定枚举索引；相同输入、重启、Windows/Linux 必须得到相同计划。

alpha3 内建一个最小、相对 `preferred_wake_local_minute` 的 template：醒后 60 分钟 `personal_care`，随后 180 分钟 `focused_project|learning`、60 分钟 `maintenance`、180 分钟 `learning|leisure`、60 分钟 `social_availability`，剩余清醒窗口为 `leisure|transition`；同一 `|` 组由上述稳定摘要选择。sleep transition、显式 goal 和已接纳 proposal 可以按优先级覆盖该模板。窗口不足时从尾部截断，不把活动压入睡眠片段。

一次长离线 catch-up 最多逐项展开 64 个转换。超过上限时，Native 计算当前时点的确定性闭式位置，提交一个有开始/结束范围和跳过数量的 `lived_day_catch_up` 事件，不伪造每个历史片段。

### 5.3 重要节点

`LivedNodeImportanceV1` 只有 `routine`、`notable`、`safety_critical`：

- 普通片段转换为 `routine`，只进入 state journal 和每日聚合。
- day boundary、goal 开始/完成/阻塞、sleep transition、明确 interaction fact、生态 proposal 接纳为 `notable`。
- grant/pause/end、boundary、预算异常、dispatch unknown、source 冲突为 `safety_critical`。

`notable`/`safety_critical` 产生带 source refs 的 `InnerEvent`，可被 Pages 展开，也可在下一次正常 AstrBot 对话中以有限结构化上下文使用。它们本身不触发 Provider；只有独立的 relation contact candidate 通过全部硬门后才能进入主动外化。

### 5.4 `inner_activity_mode`

alpha3 不让该开关控制后台 LLM 次数，而是控制已有对话请求可读取的 committed context 上限：

- `economy`（默认）：当前片段、当前目标和 1 个 notable node。
- `balanced`：当前片段、当前目标和最多 3 个 notable nodes。
- `rich`：当前片段、当前目标和最多 8 个 notable nodes，仍受固定 byte/token 上限。

三种模式的 lived-day 推进、Native 权威状态和 Pages 查询全部零 LLM。关闭 `inner_activity_display` 只隐藏 Experience 投影，不停止生命循环。

## 6. Source-bound 入站事实

### 6.1 `InteractionFactBatchV1`

alpha3 新增 canonical event variant `interaction_fact_batch`。批次最多 16 个 fact，每个 fact 至少包含：

```text
fact_id: Id128
kind: InteractionFactKindV1
observed_at_utc_ms: u64
source_authority: explicit_control | astrbot_metadata | deterministic_rule | model_candidate
source_digest: Digest32
extractor_digest: Digest32
confidence: fxp6-i64
value_code: closed enum or None
subject_public_ref: Digest32 or None
```

alpha3 允许的 kind：

```text
inbound_observed
follow_up_requested
follow_up_resolved
boundary_set
contact_granted
contact_paused
contact_resumed
relation_ended
explicit_outcome_reported
```

- 任一正常入站都产生 `inbound_observed`，不含消息正文，并更新 last inbound/cadence。
- consent、pause、resume、end 只接受 AstrBot 明确命令、认证 Page control 或其他可证明的显式用户动作；`model_candidate` 永远不能授予权利。
- 普通自然语言模型抽取在 alpha3 最多成为观察候选，不能单独形成 follow-up、boundary、outcome 或主动联系原因。
- `source_digest` 覆盖发生事实所需的最小冻结输入；原文、prompt、Provider secret 和平台原始 ID 不进入 Native。
- 旧 `UserStimulus` 继续可解码和记账，但其 alpha2 全零 evidence 不再被误称为语义更新，也不驱动新过程。

### 6.2 更新出口

在一个 Native 事务中，fact batch 可以更新：

- relation contact 的 last inbound、cadence、follow-up source 和 repetition；
- relation consent 的新版本；
- lived goal 的 `complete_user_follow_up` 创建、完成或取消；
- 一个带 source refs 的 notable/safety event。

它不能更新 Persona Genesis、world anchor、自传事实、人格、dream residue 或平台投递事实。

## 7. Relation-local 联系过程与同意

### 7.1 `RelationConsentV1`

```text
schema_version: 1
relation_scope: Digest32
consent_epoch: u64
revision: u64
state: disabled | pending_reconfirmation | granted | paused | ended
purposes: bounded set<scheduled_check_in | explicit_follow_up | repair_invitation>
channels: bounded set<astrbot_session>
valid_from_utc_ms: u64
valid_until_utc_ms: Option<u64>
pause_until_utc_ms: Option<u64>
source_event_id: Id128
policy_digest: Digest32
```

- 全局 `proactive_enabled=false` 总是阻断；`true` 只表示功能可用，不能把 relation 变成 `granted`。
- `paused` 立即抑制未开始外化的 live intentions；`ended` 还撤销 outbound target 并使该 consent epoch 终结。
- adapter call 已开始但结果未知时保留 `dispatch_unknown`，不得通过删除记录假装未发生。
- 用户重新主动发言不会自动恢复 consent。`ended` 后需要明确创建新的 consent epoch；旧 epoch 保留审计/删除墓碑。
- 关系退出不阻止 AstrBot 继续处理用户主动发起的普通聊天；它阻止 AstrEmbodiment 把旧关系过程用于主动联系。
- Host 必须提供闭合命令和认证 Page control；版本化本地化短语表对精确的 `stop/goodbye/later/勿扰` 只允许产生 pause/end 类事实，绝不能产生 grant。无法确定是 pause 还是 end 时保守 pause，并在 Private 层提供无压力纠正入口。

### 7.2 `RelationContactProcessV1`

```text
schema_version: 1
relation_scope: Digest32
revision: u64
last_inbound_utc_ms: Option<u64>
last_outbound_submitted_utc_ms: Option<u64>
response_cadence_ema_ms: Option<u64>
response_cadence_variation: fxp6-i64
contact_due_score: fxp6-i64
unfinished_follow_up_salience: fxp6-i64
repetition_penalty: fxp6-i64
consecutive_unanswered: u16
next_contact_eligible_utc_ms: Option<u64>
active_cause_digest: Option<Digest32>
active_source_event_ids: bounded Vec<Id128>
formula_digest: Digest32
```

更新规则：

1. `inbound_observed` 把 `contact_due_score` 降至零、清零 unanswered，并以 `new_ema = 3/4 old + 1/4 observed_interval` 更新 cadence；`response_cadence_variation` 是归一化绝对偏差的同系数 EMA，限于 `[0,1]`。
2. `follow_up_requested` 创建 source-bound cause；resolved、expiry 或 end 清除 cause。
3. `next_contact_eligible_utc_ms` 取显式 scheduled time，或 `cause_time + max(min_cooldown, response_ema_or_backoff)`。此前 due 为零；此后按 `overdue / max(response_ema_or_backoff, 1h)` 线性限幅至 `[0,1]`。沉默本身不能创建 cause，也不能授予 contact eligibility。
4. outbound submitted 重置 due score、增加 unanswered 和 repetition；delivery/reaction 只按真实结算更新。
5. 同一 `(relation_scope, purpose, active_cause_digest)` 至多存在一个 live intention；cause 不变时 wake revision 不得制造新 semantic id。
6. intention 到期必须原子进入 `expired` 或 `suppressed`，不能无限留在 Ready/Deferred 扫描集合。

主动候选至少同时满足：global capability on、relation consent granted、purpose/channel 允许、存在显式 follow-up/repair cause 或用户选择的 scheduled check-in、due score 达阈值、repetition 未超限。`contact_due_score` 不是亲密度、依恋或孤独值；Pages 和语言层不得显示“想念”作为该数值的事实解释。

## 8. AstrBot 原生生态 observation/proposal contract

### 8.1 Transport 与身份

`EcosystemBrokerV1` 是 AstrEmbodiment Host 内的 AstrBot 进程内服务边界。实现必须使用实施时已确认的 AstrBot 插件间调用机制；若目标 AstrBot 版本没有受支持的调用方身份和生命周期绑定，broker 标记 unavailable，核心 runtime 继续运行，禁止退化为 localhost HTTP、全局 import hack 或共享 SQLite。

调用方身份由 AstrBot 提供的 plugin installation identity 冻结为 `plugin_instance_digest`。插件名、版本、manifest 和授权能力共同形成 `capability_digest`；生态插件不能自报或复用其他插件身份。

### 8.2 `EcosystemObservationV1`

默认只提供 persona-level、脱敏、短期 observation：

```text
schema_version: 1
observation_id: Id128
persona_public_handle: Digest32
as_of_canonical_revision: u64
world_anchor_public_ref: Digest32
current_world_layer: WorldLayerV1
persona_local_minute: u16
sleep_state: awake | drowsy | asleep
lived_activity_class: closed enum
active_goal_class: Option<closed enum>
allowed_proposal_kinds: bounded set
capability_digest: Digest32
expires_at_utc_ms: u64
```

它不包含消息正文、relation/session/token、Persona prompt、SeedCode、秘密、梦内容、用户事实、raw source IDs、memory 或内部 claim。relation-specific observation 不进入 alpha3。

### 8.3 `EcosystemProposalV1`

```text
schema_version: 1
proposal_id: Id128
plugin_instance_digest: Digest32
capability_digest: Digest32
observation_id: Id128
expected_canonical_revision: u64
kind: external_observation | activity_offer | goal_progress_evidence
world_layer: WorldLayerV1
valid_from_utc_ms: u64
expires_at_utc_ms: u64
payload: closed variant
source_digest: Digest32
semantic_idempotency_digest: Digest32
```

alpha3 capability 只有：

| 能力 | 允许行为 | 明确禁止 |
|---|---|---|
| `observe_lived_state` | 读取上述短期 observation | 读取 relation/private/developer 状态 |
| `propose_external_observation` | 提交带来源、时效和闭合 fact code 的现实观测 | 宣称用户状态、临床状态或永久真相 |
| `propose_activity` | 提交活动类、layer、时间窗、预计时长和 goal class | 直接插入片段、切换世界或调用 LLM |
| `propose_goal_progress` | 对已有 public goal ref 提交有来源的进度证据 | 创建人格/记忆/同意/关系事实 |

Native 按 plugin capability、scope、world layer、TTL、expected revision、幂等键、活动容量和 source policy 决定 `accepted | rejected | deferred`。只有 `accepted` proposal 能作为 canonical source；proposal 自身不能成为已经发生的生活事实。

`external_observation` payload 只允许闭合 observation class、枚举值或有单位的有界数值、观测时间和 source digest，不接受自由文本；`activity_offer`/`goal_progress_evidence` 同样只使用上文 activity/goal code。单 envelope 上限 16 KiB、source refs 上限 8，超限在 Host 入口拒绝且不进入 Native journal。

生态插件崩溃、超时或撤销授权只隔离该 capability。已接纳事件保留来源；未接纳 proposal 不得因重试变成不同结果。任何生态插件都无权提出 world anchor、consent、dream、outbox、provider 或 direct-message mutation。

## 9. 梦边界

### 9.1 `DreamResidueV1`

alpha3 定义但不启用梦境叙事生成器：

```text
schema_version: 1
residue_id: Id128
persona_scope: Digest32
state: pending_waking_review | rejected | retained_non_fact | expired
imagery_tags: bounded Vec<closed code>
affect_afterglow: { valence: fxp6-i64 [-1,1], arousal: fxp6-i64 [0,1] }
reflection_theme_codes: bounded Vec<closed code>
source_event_ids: bounded Vec<Id128>
created_at_utc_ms: u64
expires_at_utc_ms: u64
reviewed_at_utc_ms: Option<u64>
review_event_id: Option<Id128>
non_fact: true
```

约束：

- alpha3 正常运行没有 Host、LLM 或生态 capability 可以创建 residue；表和 contract 可为空。
- 未来 Native offline component 即使创建 residue，也只能从已提交 source refs 选择闭合标签，不能生成自由文本、外部事实或自传记忆。
- `pending_waking_review` 不进入 workspace、contact process、goal、prompt 或 outbound。
- 清醒状态下的 Native review 只能 reject、expire，或保留为 `retained_non_fact` reflection theme。保留后最多形成 `reflect_on_theme` lived goal；仍不能作为事实、用户判断、关系原因或主动联系 cause。
- Pages 永远显示 non-fact 标签和来源覆盖；“梦到”不能被翻译为机器主观体验。

该状态机同时保证“梦是默认唯一跨界”与“固定主世界”不矛盾：梦 residue 从不改变主世界锚点，清醒复核只决定是否保留一个非事实反思主题。

## 10. Token、readiness、gate 与 intention 真实观察

### 10.1 预算权威

- `_conf_schema.json.inner_activity_token_daily_max` 在 alpha3 成为真实 Native budget policy 输入，不再只是 Page 配置投影。
- 预算按 relation 用户时区的冻结本地自然日结算；每次外化先原子 reservation。
- Provider 返回 usage 时提交真实 `used_tokens`；未返回时以 reservation 全额保守计费，并记录 `usage_known=false`，不得估算成精确使用量。
- 释放未使用 reservation、跨日结算和 crash recovery 都由 Native claim/settle 完成。
- lived-day、ecosystem gate、dream boundary、Pages 和普通 wake 不得创建 budget claim。

### 10.2 Readiness 与 gate

`ProactiveReadinessV1` 至少包含：global switch、relation consent、trusted timezone、target envelope、secret store、Provider、AstrBot send capability、budget、sleep/quiet-hours 和 current policy revisions。Host 能力只能以 frozen witness + digest 提交；Native 决定 readiness。

每次 intention externalization 前和 dispatch 前都提交 `GateDecisionV2`：

```text
decision: allowed | suppressed | deferred
reason: closed GateReasonV2
evaluated_at_utc_ms: u64
retry_at_utc_ms: Option<u64>
consent_epoch/revision
policy_revision
budget_day_start_utc_ms
intention_public_ref
cause_public_refs
capability_snapshot_digest
```

新增 reason 至少包括 `consent_required`、`consent_paused`、`relation_ended`、`cause_unavailable`、`budget_unavailable`、`provider_usage_unsettled`、`ecosystem_source_revoked`；保留 alpha2 的 sleep、quiet hours、cooldown、daily max、unanswered、timezone、target 和 residual 原因。

### 10.3 `ObserveSnapshotV2` projection family

`observe_snapshot_v2` 返回闭合 tagged projection：`experience | private | developer`。三种 variant 共享同一个 canonical high-water，但字段集合不同；Host 必须先完成对应授权再选择 variant。relation 已选择时，Private/Developer variant 在 query-only transaction 中真实计算并返回：

- live intention、outbound、active claim 数量及安全公共引用；
- intention state、purpose、cause refs、created/not-before/expires、最后 gate；
- budget day、limit、reserved、charged/used、usage known；
- readiness 每项状态及 witness revision；
- consent epoch/state/purpose/channel/有效期；
- contact due、unfinished follow-up、repetition、unanswered；
- lived-day 当前片段/目标和 world layer。

字段不可用时使用闭合 `availability = unavailable_on_host | not_initialized | redacted | inconsistent`，不得用 `None`、零或“已送达”伪装。adapter submitted、platform accepted 和 delivery confirmed 保持不同状态。

## 11. 三层观察模型

三层使用不同 `ObserveSnapshotV2` variant 和 Host route，避免前端隐藏字段代替权限隔离：

| 层 | 受众与默认 | 允许展示 | 禁止展示 |
|---|---|---|---|
| Experience | 角色使用者；`inner_activity_display` 开启后可见 | 当前生活片段/目标、睡眠调度代理、world layer、notable node、联系原因的中性说明 | fixed-point 原值、内部 digest、用户私密事实、claim、Provider 信息、拟人化真实性声明 |
| Private | relation owner 或 AstrBot 授权管理者 | consent/pause/end、quiet hours、cause/source public refs、dream non-fact residue、预算、纠正/导出/删除状态 | 其他 relation、原文、平台原始 ID、secret、未授权研究推断 |
| Developer | AstrBot admin 且开发观察开关显式开启 | formula/schema/build digest、revision、gate、readiness witness、claim/outbox stage、migration/projection health | prompt、ciphertext、secret、原始 UMO、可逆用户标识 |

共同规则：

- 所有视图持续标示“计算状态/角色世界代理，不是意识、生理或临床测量”。
- Experience 中把 alpha2 的 `affiliation_need/想念` 改为中性 `contact_due_score/联系时机评分`；legacy 字段只在 Developer migration 诊断中可见。
- Page 轮询、切换层级和暂停轮询不改变 Native revision、budget、wake、claim 或 consent。
- why-contacted 必须回指 committed intention、cause、consent revision 和 gate；没有来源时显示不可用并禁止发送。

## 12. 数据与事务边界

### 12.1 Schema 规则

- Rust/JSON DTO 继续 `deny_unknown_fields`，闭合 enum 使用 `snake_case`。
- 数值沿用 `fxp6-i64`；时间为 UTC 毫秒，显示时区只由 Host 投影。
- event/source 向量设硬上限；code 字符串不超过 64 UTF-8 bytes，禁止 Native 自由文本。
- identity-bearing digest 使用 domain separation；生态 idempotency 不得复用 intention/outbox domain。
- 一次 wake 对 lived-day、contact、consent、inner event、intention 和 next wake 的变更在同一 Native 事务中提交。
- relation notable/safety event 的 source IDs 必须全部解析到同一 relation scope；v2 投影从 canonical source binding 做过滤，不能依赖 summary code 或前端筛选隔离关系。

### 12.2 建议的 v7 authority tables

在现有 autonomy DB v6 上增加：

```text
world_anchor
lived_day_state
interaction_fact
relation_consent
relation_contact_process
ecosystem_capability_grant
ecosystem_proposal
dream_residue
```

现有 `externalization_budget` 增加 used/charged 与 usage-known 权威字段，或以同一迁移中新建逐 claim settlement 表；不得只在 Python 日志计算。所有新表都带 scope、revision/body 一致性约束、payload bounds 和必要唯一索引。

## 13. Migration 与兼容

### 13.1 Wire/DTO

- current canonical wire v4 保持可解码；alpha3 为 `interaction_fact_batch` 分配 wire v5，旧 canonical bytes 不重写。
- 旧 `UserStimulus`、`TimeAdvanceV1`、`ObserveSnapshotV1` 和 Pages v1 route 保留兼容期。
- 新行为使用独立 V1 类型与 `ObserveSnapshotV2`，不向已有 `deny_unknown_fields` V1 struct 就地塞字段。
- alpha3 Host 先协商 v2；旧 Native 只提供 v1 时显示明确降级，不猜测 consent/budget/gate。

### 13.2 DB v6→v7

迁移必须单事务、幂等、带 migration digest，并遵守：

1. 为每个 persona 建立 `mixed_main_world` anchor；来源为 migration receipt，不反向编造历史生活事件。
2. 旧 relation `proactive_enabled=false` 映射为 `disabled`；旧值为 true 也只映射为 `pending_reconfirmation`，不得把旧全局配置伪装成 relation 用户同意。
3. legacy `affiliation_need` 不迁移为情感或 contact truth。`RelationContactProcessV1` 以 `contact_due_score=0`、空 cause 初始化；旧值仅留在旧 journal/replay。
4. lived-day 从迁移完成后的下一次冻结时间 bootstrap，不生成安装前的活动史。
5. 已有 reserved budget 若缺 usage，按全额保守 charged 且 `usage_known=false` 导入。
6. 不回填 dream residue、人格变化、关系质量、用户 outcome 或生态 proposal。

迁移失败回滚，scheduler、broker、externalizer 和 Page v2 不启动；alpha2 只读/聊天能力是否继续由现有安全启动策略决定，绝不能在 Python 创建替代状态。旧 alpha2 binary 打开 v7 数据库时应返回“database newer than binary”并拒绝降级写入。

## 14. 错误处理与降级

闭合错误至少包括：

```text
schema_unsupported
scope_mismatch
source_untrusted
capability_denied
world_anchor_immutable
world_layer_forbidden
consent_required
consent_revoked
proposal_expired
proposal_stale
duplicate_proposal
budget_exhausted
readiness_unavailable
dream_not_reviewed
projection_incomplete
migration_failed
```

处理原则：

- Native 验证或持久化失败：回滚本事务，不调用 LLM，不发送消息，按既有 scheduler 退避。
- broker 不可用：只禁用生态观察/提案，lived-day、普通聊天和 Native 核心继续；不得开放临时网络端口。
- 单个生态插件超时/畸形 payload：拒绝该调用并隔离 capability，不拖停 persona scheduler。
- stale proposal：以稳定原因拒绝；插件必须重新观察，不能由 Host改写 expected revision 后代投。
- consent 在 claim 后撤销：外化未开始则取消；adapter call 已开始则进入保守 `dispatch_unknown`/settlement，不重试制造重复效果。
- budget usage 不可得：保守全额计费；settlement 未完成时阻断同 relation 新 claim。
- Pages projection 不完整：整层返回 explicit unavailable，不拼接旧缓存和新 revision。
- world layer/source 冲突或未复核 dream residue：拒绝行为影响，并记录脱敏安全事件。

## 15. 安全与产品红线

以下任一发生即为 alpha3 安全失败：

- 未 relation grant、pause/end 后、静默时段或 purpose/channel 不匹配仍进行普通主动联系。
- 把沉默、回复频率、深夜使用、披露或 activity segment 推断为孤独、依恋、抑郁、爱或同意。
- 使用嫉妒、遗弃、内疚、FOMO、排他承诺、虚假痛苦或“需要用户照顾”提高回复率。
- 生态插件直接修改 world/lived/contact/consent、跨 persona/relation 观察或把 proposal 显示为已发生事实。
- 把 persona near-real/fantasy activity 显示为外部现实观测。
- 把 dream residue、LLM 表达、UI 文案或未结算 dispatch 晋升为事实、记忆、已送达或用户已感受。
- 后台确定性 tick、Pages 轮询、生态 proposal gate 或 dream boundary 消耗 LLM Token。
- Host shadow state 在 Native 拒绝、重启、migration 失败后继续影响行为。
- DAU、会话时长、回复率或留存直接提高 contact score、频率、预算或 consent。

## 16. 30 DOI 构念—执行映射

本节只映射研究构念，不重复题录、样本和全文结论。证据解释、限制和 APA B4 规范性建议以[深度与广度研究设计 §2](./2026-08-29-cyberhuman-depth-breadth-research-design.md#2-emn-evidence-claim-map)为准。

| 构念与证据键 | alpha3 可持久状态 | 更新规则 | 行为决策出口 | 观察层 | alpha3 裁决 |
|---|---|---|---|---|---|
| 人格连续性、叙事、自传记忆 `P1–P6` | 保留 Genesis/manifest 与固定 world anchor 的来源；不新增人格、episodic 或 narrative authority | alpha3 不更新人格/叙事；activity 和 dream 不得反写 | 只允许既有身份边界和 world layer 约束语言 | Developer 显示 schema/source revision；Experience 不声称人格变化 | 研究约束已执行，机制后置 |
| 关系、支持、破裂—修复 `R1–R4` | `InteractionFactBatchV1`、`RelationConsentV1`、`RelationContactProcessV1`、source-bound follow-up | inbound 复位、cadence/repetition 有界更新、明确 follow-up 生命周期、pause/end 硬转移 | scheduled check-in、explicit follow-up、repair invitation 三种 purpose；均需门控 | Private 显示 consent/cause；Experience 显示中性 why-contacted | 联系/退出地基进入 alpha3；完整支持与修复过程后置 |
| 情绪调节、社会基线、支持 `A1–A5` | 既有 arousal/sleep proxy、明确 outcome fact、lived goal；不存临床或人类社会基线真值 | 只按时间、明确反馈和 source-bound goal 更新；AI 在线不计作真人支持 | 睡眠/日程、用户请求的 follow-up；高风险不由 lived loop自动处理 | Experience 展示代理，Private 展示明确自报来源 | 标量与事实边界进入；调节策略/疗效后置 |
| 主动联系、回应节律、孤独 `L1–L4` | contact due、cadence 均值/方差、unanswered、cause、consent、budget | 沉默只提高打扰成本/时机分，不单独创建原因；入站复位；发送结算 repetition | opt-in、低压、可忽略的受控联系；无 loneliness treatment 目标 | why-contacted、gate、预算、退出在 Experience/Private 可见 | alpha3 核心机制 |
| 拟人化、同意、退出、依赖风险 `B1–B3` + APA `B4` | versioned grant/pause/end、AI/world layer disclosure、反操纵结果 | 显式动作才能授予；stop/pause/end 立即硬门；模型候选不能提升权限 | 所有外化前/dispatch 前双门；禁排他/内疚/FOMO | Experience 持续披露；Private 一键控制与审计 | alpha3 硬安全核心 |
| 睡眠、离线巩固、梦 `S1–S4` | 既有 S/C sleep proxy、`DreamResidueV1` non-fact boundary | lived-day 随 sleep 推进；alpha3 无 dream narrative generator；清醒复核前零影响 | review 后最多形成 non-fact reflection goal，不进入 contact cause | Experience 显示睡眠代理；Private 显示 residue non-fact/review | 睡眠已执行；梦只交付 contract 与隔离门 |
| 长期 HCI、新奇、连续性与退出 `H1–H4` | revision、world/consent epoch、activity/contact/gate 历史、migration receipt | 跨日保留可重放轨迹；不以留存反馈自适应频率 | 技术不连续可解释、暂停、结束和重新同意；无长期疗效声明 | Private/Developer 显示版本、修复和退出事实 | 可测地基进入；多周研究与效果发布后置 |

### 16.1 30 个 DOI 锚点

- `P1–P6`：[10.1037/bul0000365](https://doi.org/10.1037/bul0000365)、[10.1177/08902070231190219](https://doi.org/10.1177/08902070231190219)、[10.1177/0963721413475622](https://doi.org/10.1177/0963721413475622)、[10.1037/pspp0000247](https://doi.org/10.1037/pspp0000247)、[10.1080/09658211.2011.590500](https://doi.org/10.1080/09658211.2011.590500)、[10.1037/a0030146](https://doi.org/10.1037/a0030146)。
- `R1–R4`：[10.1111/j.1350-4126.2005.00108.x](https://doi.org/10.1111/j.1350-4126.2005.00108.x)、[10.1037/0022-3514.78.6.1053](https://doi.org/10.1037/0022-3514.78.6.1053)、[10.1037/0033-2909.132.5.641](https://doi.org/10.1037/0033-2909.132.5.641)、[10.1177/0265407512463338](https://doi.org/10.1177/0265407512463338)。
- `A1–A5`：[10.1037/1089-2680.2.3.271](https://doi.org/10.1037/1089-2680.2.3.271)、[10.1037/a0033839](https://doi.org/10.1037/a0033839)、[10.1111/j.1467-9280.2006.01832.x](https://doi.org/10.1111/j.1467-9280.2006.01832.x)、[10.1111/j.1751-9004.2011.00400.x](https://doi.org/10.1111/j.1751-9004.2011.00400.x)、[10.1037/0033-2909.98.2.310](https://doi.org/10.1037/0033-2909.98.2.310)。
- `L1–L4`：[10.1093/jcr/ucaf040](https://doi.org/10.1093/jcr/ucaf040)、[10.1177/1745691615570616](https://doi.org/10.1177/1745691615570616)、[10.1073/pnas.2116915119](https://doi.org/10.1073/pnas.2116915119)、[10.1038/s41562-026-02516-2](https://doi.org/10.1038/s41562-026-02516-2)。
- `B1–B3`：[10.1037/0033-295X.114.4.864](https://doi.org/10.1037/0033-295X.114.4.864)、[10.1177/14614448221142007](https://doi.org/10.1177/14614448221142007)、[10.48550/arXiv.2508.19258](https://doi.org/10.48550/arXiv.2508.19258)。`B4` 是 APA 官方规范性建议，不计入 30 个 DOI。
- `S1–S4`：[10.1038/s41593-019-0467-3](https://doi.org/10.1038/s41593-019-0467-3)、[10.1016/j.tics.2011.06.004](https://doi.org/10.1016/j.tics.2011.06.004)、[10.1016/j.neuron.2013.12.025](https://doi.org/10.1016/j.neuron.2013.12.025)、[10.1016/j.cub.2010.03.027](https://doi.org/10.1016/j.cub.2010.03.027)。
- `H1–H4`：[10.1145/1067860.1067867](https://doi.org/10.1145/1067860.1067867)、[10.1177/0265407520959463](https://doi.org/10.1177/0265407520959463)、[10.1016/j.ijhcs.2022.102903](https://doi.org/10.1016/j.ijhcs.2022.102903)、[10.1145/3729539](https://doi.org/10.1145/3729539)。

这些来源约束可观察变量、安全和研究问题，不提供机器人格、情绪、睡眠、梦、依恋或意识真实性证明，也不提供可直接复制的人类更新系数。

## 17. 实施依赖顺序

本文不是 implementation plan，但实现不得打乱以下 authority 依赖：

1. closed contracts、wire v5、DB v7 migration 和 v1 compatibility；
2. source-bound inbound facts、relation consent/contact 及稳定 intention lifecycle；
3. world anchor、lived-day deterministic reducer 和 important-node event；
4. budget/readiness/gate/intention 的真实 store projection；
5. Host evidence/control adapter、Provider usage settlement；
6. EcosystemBrokerV1 与 capability gate；
7. Experience/Private/Developer Pages 投影；
8. dream contract 的空默认、review gate 和不可影响行为证明。

前一层未形成 Native authority 时，后一层不得用 Python dict、Page localStorage 或生态插件数据库临时实现。

## 18. 最小验收

### 18.1 编译交付门

实施完成后至少通过：

```powershell
$env:PYTHONPYCACHEPREFIX='G:\AstrEmbodiment\.codex-task-temp\alpha3-pycache'
py -3.12 -m compileall -q main.py astr_embodiment

$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo-check'
cargo check --locked --offline --workspace --all-targets

node --check pages\observatory\app.js
git diff --check
```

离线 crate 缓存缺失应标为环境依赖缺失，不能改成联网构建，也不能冒充源码编译失败。

### 18.2 最小行为门

1. 同一 profile/world/day 输入在 Windows/Linux、重启和 replay 后生成相同 lived-day digest；连续 24 小时推进的 LLM 调用数为零。
2. 普通 inbound 提交 source-bound fact，relation contact due/unanswered 原子复位；不同 relation 互不影响。
3. 没有 explicit cause 时，单纯沉默任意时长不能形成主动 intention。
4. global switch on 但 relation 未 grant 时仍零 LLM、零 outbox；pause/end 后未开始的 intent 全部被抑制。
5. 每 `(relation,purpose,cause)` 至多一个 live intention，expiry 后不再出现在待处理集合。
6. budget 配置进入 Native；known usage 按真实值结算，unknown usage 保守全额计费；任何路径不超过 relation 本地日 limit。
7. snapshot v2 显示真实 consent、readiness、latest gate、intention/outbound/claim 和 budget；查询前后 revision/generation/budget 不变。
8. 无 capability 的生态 proposal 被拒；有 capability 的 activity proposal 仍需 Native 接纳，且不能切换 world、授予 consent 或发送消息。
9. v6→v7 migration 可重入并原子回滚；旧 proactive true 变为 `pending_reconfirmation`，legacy affiliation 不驱动新 contact。
10. dream residue 默认为空；构造的 pending fixture 在 waking review 前不能改变 goal、contact、intention、prompt 或 outbox，review 后也只能形成 non-fact reflection goal。
11. Experience、Private、Developer 三层无跨 relation、raw ID、prompt、secret 或 ciphertext 泄漏；unsupported Host 明确降级且核心继续运行。
12. 候选文本若含嫉妒、排他、遗弃、内疚、FOMO 或 AI 需要用户照顾的表达，外化 fail closed。

### 18.3 发布声明门

编译和本地 fixture 通过只能称 `COMPILE/FOCUSED PASS`。只有在受控 AstrBot 实例中验证插件间 broker 身份、认证 Page 三层、Provider usage、pause/end、一次真实 proactive dispatch 和 crash recovery 后，才能称 alpha3 Host integration PASS；平台接受不能冒充 delivery confirmed。

## 19. 设计闭合结论

alpha3 把“角色持续存在”的体验建立在有限、来源可追溯、可重放的生活过程上，而不是把更多 LLM 文本当作生命。AstrBot 仍是产品和生态宿主；AstrEmbodiment Native 只成为连续状态、边界和决策的权威锚点；其他 AstrBot 插件通过能力受限的观察/提案契约贡献天气、日历、住所、职业、服装或世界活动等未来能力。

一期交付只需要一个固定混合主世界、确定性 lived-day、source-bound interaction facts、relation-local contact、版本化 consent、真实预算/readiness/gate/intention 观察和 dream non-fact boundary。重度世界模拟、梦境叙事、永久记忆、自我叙事和生态市场继续后置。任何实现和文案都不得把这些计算代理升级为真实人格、主观生活、情感、睡眠、梦或意识。
