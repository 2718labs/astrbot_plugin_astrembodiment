# AstrCyberHuman 核心与 AstrEmbodiment 剥离设计

- 状态：已选方案 A，供独立仓库实施
- 日期：2026-08-29
- 当前来源仓库：`astrbot_plugin_astrembodiment`
- 新插件工作名：`AstrCyberHuman`
- 建议包名：`astrbot_plugin_astrcyberhuman`
- 约束：两者都是 AstrBot 插件，不创建独立宿主、账号体系或旁路消息通道

来源设计：

- `docs/superpowers/specs/2026-08-28-cyberhuman-autonomous-runtime-design.md`
- `docs/superpowers/specs/2026-08-29-cyberhuman-depth-breadth-research-design.md`
- `docs/superpowers/specs/2026-08-29-alpha3-lived-world-native-ecosystem-design.md`
- `docs/superpowers/specs/2026-08-29-astrbot-pages-observatory-design.md`

本文覆盖上述文档中“AstrEmbodiment 是生命内核、生态锚点或 Web UI 所有者”的结论。上述文档的研究证据、安全约束、睡眠/执行机制和数据来源要求继续有效；其模块归属必须以本文为准。

## 0. 决策摘要

`AstrEmbodiment` 不再承担“完整赛博人”的人格、生活、世界、内心活动、关系意义、梦样离线处理、记忆、自我模型、生态协调或 Web UI。它收缩为无界面的身体与执行插件。

新的 `AstrCyberHuman` 是完整赛博人内核和生态中心。它拥有跨时间连续性、人格与身份、生活世界、目标、关系过程、情绪调节、自传记忆、可修订叙事、内心活动、梦样非事实处理，以及未来统一 Web UI。

两者的唯一主关系是：

```text
AstrCyberHuman 形成有来源的意图
        |
        | IntentEnvelopeV1
        v
AstrEmbodiment 判断身体、时间、预算、许可和平台是否允许执行
        |
        | AstrBot Provider / adapter
        v
真实平台交互
        |
        | ExecutionReceiptV1 / StimulusEnvelopeV1
        v
AstrCyberHuman 更新自己的经历、关系和记忆候选
```

这不是把一个巨型插件机械拆成两个目录。拆分后的每个插件必须有单一权威、独立数据库、独立生命周期和可失效关闭的协议边界。

## 1. 产品定义

### 1.1 AstrCyberHuman 是谁

`AstrCyberHuman` 是赛博人的“心智—生命连续性内核”。它负责回答：

- 这个角色是谁，哪些特征稳定，哪些可以发展；
- 它正在过怎样的一天，当前目标和生活上下文是什么；
- 某次互动对这段关系意味着什么；
- 哪些事件可以成为记忆，哪些只是候选、推断或想象；
- 为什么形成某个行动意图；
- 哪些内心活动可以向用户展示，哪些只能以安全投影呈现；
- 其他生态插件提出的观察或提案是否能进入其生活世界。

它不直接调用平台发送，不直接持有 AstrBot UMO，不把 LLM 输出当状态权威，也不把合成角色表达声明为真实意识或主观体验。

### 1.2 AstrEmbodiment 是什么

`AstrEmbodiment` 是赛博人的“身体—行动执行层”。它负责回答：

- 当前是否清醒、可唤醒、疲劳或处于静默窗口；
- 这次行动是否获得用户许可，是否超过频率或 Token 预算；
- Provider 是否可用，调用是否已经开始，实际用量是否可知；
- 目标会话和平台身份是否仍有效；
- 候选内容能否安全外化；
- 消息只是提交给 adapter，还是获得了更强的平台回执；
- 崩溃恢复后如何避免重复调用、重复发送或漏记预算。

它不再解释“我是谁”“我今天在过什么生活”“我为什么想念某人”，也不拥有长期人格、叙事、世界或关系意义。

### 1.3 AstrBot 是唯一宿主

AstrBot 继续拥有：

- 消息事件、用户/群/会话身份和平台适配；
- Provider 选择和调用入口；
- 插件加载、更新、停用和配置；
- 管理员身份、权限和未来 Plugin Web UI 宿主；
- 最终平台发送。

两个插件都不得通过 localhost HTTP、全局 Python import、共享 SQLite、扫描对方目录或猜测 AstrBot 内部对象来互联。

## 2. 明确不做

- 不创建 SillyTavern、Character Card、Lorebook 或 World Info 兼容层。
- 不创建脱离 AstrBot 的后台服务、网页服务器、登录系统或插件市场。
- 不让 AstrCyberHuman 直接发送消息。
- 不让 AstrEmbodiment 保存人格、关系叙事、生活世界或永久记忆副本。
- 不允许两个插件同时成为同一状态的写权威。
- 不宣称系统具有真实意识、依恋、孤独、睡眠体验或梦体验。
- 不在本设计中实现代码；本文是新仓库的实施输入。

## 3. 权威所有权矩阵

| 能力或状态 | AstrCyberHuman | AstrEmbodiment | AstrBot |
|---|---|---|---|
| Persona Genesis、身份连续性 | 唯一权威 | 只持 opaque persona token | 提供配置入口 |
| 固定/可迁移世界锚点 | 唯一权威 | 不保存 | 无 |
| lived-day、活动、生活目标 | 唯一权威 | 只接收行动所需最小上下文 | 提供可信时间观测 |
| 内心活动、工作空间、叙事解释 | 唯一权威 | 不保存内容 | Provider 仅生成候选 |
| 情绪 appraisal 与调节过程 | 唯一权威 | 只持执行所需 arousal/urgency 等最小投影 | 无 |
| 自传记忆、语义自我、修订历史 | 唯一权威 | 不保存 | 无 |
| 关系意义、破裂/修复、支持结果 | 唯一权威 | 不保存语义模型 | 提供互动事实 |
| 用户主动联系许可 | 语义许可权威 | 强制执行与本地 deny override | 管理员/用户输入来源 |
| 睡眠压力、昼夜相位、能量、唤醒 | 接收状态投影 | 唯一权威 | 提供 UTC/时区事实 |
| 调度 lease、claim、预算和 outbox | 不保存 | 唯一权威 | 提供 Provider/adapter |
| 目标 UMO 和平台 binding | 不可见 | 加密持有、最短时解密 | 唯一身份来源 |
| 最终内容边界与反操纵门 | 可先做语义风险提案 | 最终强制门 | 可提供安全能力 |
| adapter/platform 回执 | 只消费规范化结果 | 规范化与持久化 | 原始来源 |
| 生态插件 observation/proposal | 唯一接纳权威 | 不做代理或授权中心 | 插件身份与生命周期 |
| 统一 Web UI | 产品和数据所有者 | 无页面 | 唯一页面宿主 |

所有权判定规则：如果删除某插件的数据库会改变该状态未来的语义，该状态就必须只在拥有它的插件中持久化。

## 4. 总体组件

### 4.1 AstrCyberHuman Native Core

建议至少分成以下独立域：

1. `identity`：Genesis、稳定特征、可修订偏好和身份迁移。
2. `world`：主世界锚点、事实层、生活日程、活动和目标。
3. `affect`：事件 appraisal、调节目标、策略和感知结果。
4. `relation`：逐关系互动事实、承诺、边界、破裂与修复。
5. `memory`：episodic evidence、候选、冲突、降权和来源绑定。
6. `self_model`：semantic self-belief 与可修订 narrative；在独立 authority migration 前保持关闭。
7. `workspace`：多尺度候选竞争、当前广播和意图形成。
8. `dream`：只保存 non-fact residue、来源和清醒复核结果。
9. `ecosystem`：AstrBot 生态插件的短期 observation、capability 和 proposal 接纳。
10. `projection`：Experience、Private、Developer 三类安全 DTO。

这些域共享同一 persona 因果日志，但不得靠跨域任意表写入耦合。跨域变化必须作为显式 canonical event 进入事务 reducer。

### 4.2 AstrCyberHuman Host Plugin

Host 只负责：

- 把 AstrBot 入站事件冻结成 `StimulusEnvelopeV1`；
- 调用 Provider 产生语言、解释或抽取候选；
- 注册经过 AstrBot 证明的生态服务；
- 承载统一 Web UI 的认证 API 和静态资源；
- 把已提交的心智意图发送给 AstrEmbodiment；
- 把 AstrEmbodiment 回执重新提交给 Native Core。

Host 不得维护可恢复的 shadow mind。插件重启后影响未来行为的状态必须能从 Native 重放。

### 4.3 AstrEmbodiment Native Body

保留：

- UTC causal clock 与角色独立时区/作息；
- two-process sleep、能量、arousal、睡眠阶段和自适应唤醒；
- persona/relation 的运行 lease；
- 用户许可的强制执行投影、静默时段、频率、冷却和未回复退避；
- Provider reservation/settlement、已知/未知 Token 计费；
- readiness witness、externalization gate 和 dispatch gate；
- durable outbound、加密目标 binding、adapter/platform receipts；
- `dispatch_unknown`、崩溃恢复、幂等与删除 tombstone；
- 面向 AstrCyberHuman 的最小只读身体投影。

删除：

- lived-world、世界锚点、活动目标和生活片段；
- 内心活动、人格连续性、关系意义和记忆候选；
- dream residue 和清醒叙事复核；
- 生态插件 observation/proposal broker；
- Experience/Private/Developer 心智投影；
- 所有 Pages、Web UI、页面控制和静态资源。

## 5. 插件间协议

### 5.1 传输前提

只允许 AstrBot 提供并能证明以下属性的进程内插件服务：

- service registration 来自目标插件实例；
- caller identity 来自 AstrBot，不由 payload 自报；
- installation、plugin name、version、manifest、service identity 和 caller identity 可绑定；
- 调用有超时、payload 上限和插件生命周期撤销；
- 不需要共享数据库、全局 import 或 localhost。

若当前 AstrBot 版本不能证明这些属性，集成状态必须是 `UNAVAILABLE_HOST_ATTESTATION`。两个插件可以独立安装，但不得启用心智到身体的自动执行。

### 5.2 `IntentEnvelopeV1`：心智到身体

```text
schema_version
intent_id
persona_token
relation_token | null
mind_revision
purpose: reply | proactive_contact | reminder | repair | safety_notice
urgency_class: routine | notable | safety_critical
channel_constraints
not_before_utc_ms
expires_at_utc_ms
semantic_summary
content_candidate | null
source_refs[]
consent_epoch
idempotency_key
mind_commitment
```

约束：

- `semantic_summary` 是最小执行理由，不是隐藏思维链。
- `content_candidate` 可以为空；需要 LLM 外化时由 AstrEmbodiment 使用 AstrBot Provider。
- AstrEmbodiment 不接受没有 committed `mind_revision`、来源、过期时间和幂等键的意图。
- 接受意图不等于允许执行。身体、许可、预算、平台和内容门仍可拒绝。
- AstrEmbodiment 不得把拒绝改写成新的心智意图。

### 5.3 `EmbodimentSnapshotV1`：身体到心智

```text
schema_version
persona_token
body_revision
observed_at_utc_ms
wake_state
sleep_stage_public
energy_band
arousal_band
next_wake_window
execution_capacity
outreach_available
blocking_reasons[]
expires_at_utc_ms
body_commitment
```

这是短期只读投影。AstrCyberHuman 可以据此推迟或取消意图，但不能写回睡眠压力、能量或 gate。

### 5.4 `StimulusEnvelopeV1`：真实互动到心智

由 AstrBot 入站事件经最小事实适配后提交：

```text
event_id
persona_token
relation_token
session_token
observed_at_utc_ms
source_kind
message_free_interaction_facts
content_ref | encrypted_content
platform_capabilities
delivery_context
source_commitment
```

正常入站必须先提交 message-free interaction facts，再允许内容语义处理。显示名、可变昵称或当前 session 不得决定关系身份。

### 5.5 `ExecutionReceiptV1`：执行结果回流

```text
intent_id
execution_id
body_revision
outcome: denied | deferred | provider_failed | candidate_rejected |
         adapter_submitted | platform_accepted | delivered | dispatch_unknown
reason_codes[]
provider_usage_known
provider_used_tokens
adapter_receipt_ref | null
occurred_at_utc_ms
body_commitment
```

`adapter_submitted` 不能显示为“已送达”。AstrCyberHuman 只能依据 receipt 更新经历候选，不能根据 Host 异常推断用户已看见或已回应。

### 5.6 停止、撤回与许可格

安全采用 deny-wins lattice：

- 明确 `stop/goodbye/later/勿扰` 可以直接到达 AstrEmbodiment，立即阻断执行；
- 同一事实随后作为 canonical stimulus 转发 AstrCyberHuman；
- AstrEmbodiment 的本地记录是强制 deny override，不是关系意义权威；
- 重新 grant 必须来自 AstrCyberHuman 已提交的 consent epoch，并同时通过 AstrBot 已认证控制来源；
- 任一插件不确定、不同步或 revision 回退时一律拒绝主动执行；
- revoke 可以单方更严格，grant 必须双方与 Host 证明全部满足。

这避免在心智核心离线时仍发生主动联系，也避免 AstrEmbodiment 变成第二份关系数据库。

## 6. 数据和身份隔离

### 6.1 独立存储

- AstrCyberHuman 使用自己的数据库和密钥域。
- AstrEmbodiment 使用自己的数据库和密钥域。
- 不使用 SQLite ATTACH、共享 WAL、共享文件锁或跨插件表查询。
- 卸载其中一个插件不得破坏另一个插件的数据完整性。
- 导出和删除由各插件分别生成 receipt，再由 AstrCyberHuman Web UI 汇总显示。

### 6.2 共同身份

两个插件只共享由 AstrBot 事实冻结后派生的 opaque token：

- `installation_token`
- `bot_token`
- `persona_token`
- `relation_token`
- `session_token`

token 算法、规范化和 generation 必须由版本化公共 contract crate/package定义。任何一方不得用显示名、当前会话或数据库自增 ID 重建另一方身份。

### 6.3 协议包

建议建立独立、小型、无业务实现的协议包：

- Rust：`astr-cyberhuman-contracts`
- Python：由两个插件各自打包生成 codec，不通过运行时 import 对方源码
- 内容：closed enums、canonical encoding、domain tags、test vectors 和 compatibility matrix

协议包不能包含数据库连接、AstrBot Context、Provider、页面组件或任一插件的内部 reducer。

## 7. Pages 与 Web UI 退役/迁移

### 7.1 AstrEmbodiment 必须删除

以下资产从 AstrEmbodiment 正式产品范围中移除：

- `pages/observatory/index.html`
- `pages/observatory/app.js`
- `pages/observatory/style.css`
- `astr_embodiment/observatory.py`
- `astr_embodiment/observatory_controls.py`
- `main.py` 中所有 Page 注册、静态资源、bootstrap、轮询和控制路由
- `_conf_schema.json` 中 Page 开关、管理员页面配置和展示模式
- Pages 专用 i18n、测试、README、发布清单和包体资源

移除时必须同步删掉注册路径和配置迁移，不能只隐藏导航后把管理 API 留在后台。

### 7.2 AstrEmbodiment 保留的可观测性

只保留受鉴权的机器接口：

- `observe_body_snapshot_v1`
- `observe_execution_receipt_v1`
- `observe_budget_summary_v1`
- `observe_gate_reasons_v1`

这些接口不返回心智事件、关系解释、内心活动、原始 UMO、密钥、claim token 或完整 Provider 内容。

### 7.3 未来统一 Web UI

未来 Web UI 归 AstrCyberHuman 所有，并继续由 AstrBot Pages/WebUI 宿主承载。它可以组合：

- Life：生活、目标、世界和时间线；
- Mind：情绪过程、记忆候选、自我模型和内心活动安全投影；
- Relations：逐关系事实、边界、修复、退出和来源；
- Body：通过机器接口读取 AstrEmbodiment 的睡眠、预算、gate 和执行回执；
- Ecosystem：其他 AstrBot 插件的已证明模块和 proposal 状态；
- Developer：revision、commitment、来源覆盖和失败原因。

AstrEmbodiment 不提供 iframe、子页面或第二个管理入口。若统一 URL 组合能力尚未被 AstrBot 正式证明，Web UI 先只显示 AstrCyberHuman 自有模块，Body 页面显示 `integration unavailable`，不得抓取本地端口。

## 8. 当前 alpha3 资产如何处理

### 8.1 留在 AstrEmbodiment

以下已实现方向继续属于身体插件：

- 自适应睡眠/唤醒、角色独立时区和 causal time；
- 关系级主动联系开关、静默时段、频率和硬停止执行；
- budget、reservation、known/unknown settlement；
- readiness/externalization/dispatch gates；
- durable outbox、目标加密、回执等级、崩溃恢复；
- Provider 与 adapter 的 AstrBot host 边界；
- `why_contacted` 的最小执行原因码，但不保留心智叙事。

### 8.2 迁移到 AstrCyberHuman

- `WorldAnchorV1`、`LivedDayStateV1`、活动/目标和 catch-up；
- interaction facts 的语义消费和关系过程；
- notable inner event、工作空间和内心活动模式；
- dream residue、non-fact gate 和清醒复核；
- 生态 capability、observation、proposal 和 world reducer；
- Experience/Private/Developer 心智 DTO；
- 深度与广度研究中的 affect、memory、relation、self-model 和 narrative waves。

### 8.3 删除而非迁移

- AstrEmbodiment 现有 Pages 前端和 Page Host 实现；
- AstrEmbodiment 作为生态 UI shell 的设想；
- 让生态插件经 AstrEmbodiment 读写赛博人生活世界的 broker；
- 在身体数据库中继续新增人格/生活/梦/记忆表的路线。

新 Web UI 应按 AstrCyberHuman 的领域模型重新设计，不复制旧 Observatory 的路由、DTO 或页面布局。旧代码只能作为需求证据，不作为直接移植基线。

### 8.4 当前提交状态说明

当前 alpha3 已提交基线截至 `ef55de3293794731b6ee9ad4198bafaea1e747c9`。其后的 Task 5 安全修复在本工作树中仍是未提交草稿，包含公开 grant 阻断、完整 attestation、observation lease 和恢复发送复检等工作。

新仓库不得把这组未提交修改当成已验收依赖。可提取其安全要求，但应在新边界下重新实现和独立复审。

## 9. 新仓库建议结构

```text
astrbot_plugin_astrcyberhuman/
  main.py
  metadata.yaml
  _conf_schema.json
  astr_cyberhuman/
    host/
      astrbot_adapter.py
      provider_candidates.py
      ecosystem_service.py
      webui_api.py
    contracts/
      embodiment_v1.py
      ecosystem_v1.py
      projections_v1.py
    security/
      caller_attestation.py
      opaque_handles.py
      control_receipts.py
  crates/
    ach-contracts/
    ach-store/
    ach-identity/
    ach-world/
    ach-affect/
    ach-relation/
    ach-memory/
    ach-workspace/
    ach-ecosystem/
    ach-runtime/
    ach-pyo3/
  webui/
    src/
  docs/
    architecture/
    contracts/
    research/
```

不要一开始创建十个可独立发布的 crate。可先保持单 workspace 和清晰模块，只有当编译边界、authority 或复用需求真实出现时再拆 crate。

## 10. 实施顺序

### Wave 0：冻结边界

1. 在 AstrEmbodiment 标记旧 alpha3 lived-world/ecosystem/pages 路线为 superseded。
2. 冻结 `IntentEnvelopeV1`、`EmbodimentSnapshotV1`、`StimulusEnvelopeV1` 和 `ExecutionReceiptV1` test vectors。
3. 核验目标 AstrBot 版本是否提供可证明的 plugin service/caller identity。
4. 若不能证明，停止跨插件调用实现，只完成离线 codec 和 fail-closed 状态。

### Wave 1：建立 AstrCyberHuman 最小内核

1. 新建仓库、metadata、Native store/runtime 和 persona/relation scope。
2. 迁移 world/lived-day、interaction facts 和关系过程。
3. 保持 LLM 为 proposal-only，默认零 LLM 推进。
4. 提供 committed-only 的机器投影，不先做 Web UI。

### Wave 2：身体协议

1. AstrEmbodiment 删除心智/世界写 authority，暴露四个机器投影。
2. 建立 attested in-process transport。
3. 跑通 intent → body gate → receipt，但先不启用主动平台发送。
4. 验证任一插件离线、升级、回退或 revision 冲突时都 fail closed。

### Wave 3：迁移主动执行

1. 只对明确 opt-in 关系开放 proactive intent。
2. AstrCyberHuman 形成意图；AstrEmbodiment 保留最终否决权。
3. Provider 后每个分支都 settle；adapter 发送前重新检查内容与 gate。
4. receipt 回流只形成经历候选，不自动晋升记忆或关系结论。

### Wave 4：统一 Web UI

1. 由 AstrCyberHuman 注册唯一 Web UI。
2. 先上线只读 Life/Mind/Relations/Body 投影。
3. 控制接口使用 AstrBot 管理员、relation handle、一次性 CSRF 和幂等 receipt。
4. 再开放纠正、撤回、导出、删除和生态授权。

### Wave 5：深层心智

按独立 authority migration 依次增加：

1. affect/regulation process；
2. 可逆离线记忆实验；
3. semantic self-belief；
4. revisable narrative；
5. 纵向、可退出的主动联系研究。

前一 wave 的来源、恢复、用户权利和错误泛化门未通过时，不进入下一 wave。

## 11. 失败处理

| 失败 | 必须行为 |
|---|---|
| AstrCyberHuman 离线 | AstrEmbodiment 不生成心智意图；只完成已有安全终止/回执 |
| AstrEmbodiment 离线 | AstrCyberHuman 可继续零 LLM 生活推进，但不得发送或假装已执行 |
| AstrBot attestation 不足 | 集成不可用，不使用 localhost/global import 替代 |
| intent 过期或 revision 回退 | 拒绝并返回 receipt |
| body snapshot 过期 | 心智不得据此生成可执行时间承诺 |
| Provider 调用结果未知 | 全额 reservation 计费并标记 unknown |
| adapter 返回超时 | `dispatch_unknown`，不得自动重发 |
| consent/revoke 不一致 | deny wins，主动执行关闭 |
| receipt 重复 | 幂等返回原 receipt，不重复更新经历 |
| 任一 commitment 校验失败 | scope 进入 degraded/read-only，不重签洗白 |

## 12. 安全与产品语言

- 内心活动页面只能展示结构化、经过投影的解释，不展示隐藏 chain-of-thought。
- “生活”“睡眠”“梦”“情绪”是角色系统和算法状态，不是主观体验声明。
- 不保存单一 `intimacy`、`attachment_style`、`loneliness` 或 `consciousness` 真值。
- 用户不回复不能单独生成“想念”“担心”“被抛弃”或联系意图。
- 关系健康、退出和现实支持优先于回复率、留存和 Token 消耗。
- Web UI 必须持续显示 AI 身份、来源、不确定性、暂停、纠正、导出和删除入口。
- 插件卸载或核心升级前提供数据导出与兼容性说明，不静默迁移人格或叙事。

## 13. 验收标准

### 13.1 架构

- 两个插件可分别安装、启动、停止、升级和卸载。
- 两个插件没有共享数据库、目录扫描、localhost 或运行时全局 import。
- 所有跨插件消息都有版本、scope、revision、expiry、idempotency 和 commitment。
- AstrCyberHuman 无平台发送能力；AstrEmbodiment 无人格/生活/记忆写 authority。

### 13.2 故障隔离

- 关闭 AstrCyberHuman 后不会产生新的主动联系。
- 关闭 AstrEmbodiment 后，AstrCyberHuman 不会把意图标成已执行。
- 任一插件崩溃恢复不会重复 Provider 调用或 adapter 发送。
- revoke 在另一插件不可用时仍立即阻断；grant 在任一证明缺失时失败。

### 13.3 数据与迁移

- 迁移后每类 canonical state 只有一个写 authority。
- 旧 AstrEmbodiment 数据能按清单导出；导入新核心有逐项 receipt 和来源。
- 未迁移字段保持旧库只读，不以默认值伪造成功迁移。
- 删除 Pages 后包中不存在页面静态资源、路由、Page 配置或后台控制端点。

### 13.4 产品

- 默认零 LLM 生活推进，主动联系默认关闭。
- 用户能理解 AstrCyberHuman 是心智/生命核心，AstrEmbodiment 是身体/执行层。
- Web UI 只由 AstrCyberHuman 提供，Body 信息明确标记来自 AstrEmbodiment receipt/projection。
- 不出现真实意识、依恋、孤独、梦体验或治疗效果声明。

## 14. 交接清单

新实施地点应从以下顺序开始：

1. 采用 `AstrCyberHuman` 作为工作名并创建独立仓库。
2. 复制本文及三份来源设计到新仓库 `docs/architecture/`，保留 Git 来源信息。
3. 先实现协议 test vectors 和无 AstrBot 的 codec round-trip。
4. 对目标 AstrBot 精确版本做 service/caller identity 只读核验。
5. 建立最小 Native canonical journal，再迁移 world/lived-day。
6. 不复制 AstrEmbodiment Pages；统一 Web UI 等 Wave 4 重做。
7. AstrEmbodiment 的实际删除应在独立分支完成，先列精确文件和配置，再删除并做包内容审计。
8. 当前未提交 Task 5 安全草稿保留在原工作树，仅供人工取证，不作为可 cherry-pick 的完成提交。

## 15. 最终边界口号

```text
AstrCyberHuman decides what the cyberhuman means and intends.
AstrEmbodiment decides whether and how that intention can safely become action.
AstrBot owns the real host, identity, provider, page surface, and platform delivery.
```

中文定义：

> AstrCyberHuman 负责“成为谁、如何生活、如何理解与形成意图”；AstrEmbodiment 负责“此刻这具数字身体能不能、应不应该、以及怎样安全行动”；AstrBot 负责真实世界中的宿主、身份、模型和平台交付。
