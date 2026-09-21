# AstrEmbodiment AstrBot Pages 观察台设计（1.1.0-alpha2）

- 状态：Implemented in `1.1.0-alpha2`; code review approved，真实已鉴权 AstrBot Page 验收仍待执行
- 日期：2026-08-29
- 适用仓库：`astrbot_plugin_astrembodiment`
- 目标版本：`1.1.0-alpha2`
- 宿主能力：AstrBot Plugin Pages（AstrBot `>=4.24.2`）

## 1. 决策摘要

AstrEmbodiment 在 alpha2 中提供一个嵌入 AstrBot 管理面板的开发态观察台。它使用 AstrBot 内建 Plugin Pages，而不启动独立 HTTP 服务、独立登录或第二套静态资源服务器。

观察台是证据投影，不是运行时控制面。它仅展示已提交的时间线、睡眠、情绪/唤醒、意图/抑制、因果链以及 Token 相关开关与用量。它不允许唤醒角色、开启主动消息、修改配置、重建投影、生成语言、调用 LLM 或发送消息。

alpha2 采用两个闭合原生读模型：

- `observe_snapshot_v1`：读取一个 persona scope 的当前已提交状态、完整性和成本摘要。
- `observe_events_v1`：按稳定 cursor 读取 committed inner-event，并同时返回 canonical high-water mark。alpha2 v0 不为凑齐界面而拼接尚无安全闭合投影的 operational payload。

初版通过 AstrBot Page bridge 使用有界 GET 轮询。AstrBot 已提供带鉴权的 SSE bridge，但 alpha2 不引入第二份事件真源；SSE 只作为后续“有新 cursor”的失效通知优化。

截至提交 `ccef619`，本设计的 Native、Host 和 Page 前端均已实现。Native 最终修复提交 `de5e45a` 已通过独立规格与质量复审；Host/Frontend 最终复审结论为 APPROVED。Playwright mock bridge 已在桌面与 390 px 视口通过，但它不是 AstrBot 鉴权链证据；真实已鉴权 AstrBot Dashboard 验收仍为 pending。Windows wheel 构建与运行验证为 PASS；Linux wheel 构建与静态检查为 PASS，Linux 原生运行 smoke 仍为 pending。

## 2. 宿主契约与版本边界

AstrBot Plugin Pages 的已确认宿主契约如下：

1. 每个 Page 位于 `pages/<page_name>/index.html`，并从插件详情页打开。
2. Page 在限制 iframe 中运行，不能读取 Dashboard cookie、LocalStorage 或父 DOM。
3. Page 通过 `window.AstrBotPluginPage` bridge 调用插件 API。
4. 插件使用 `context.register_web_api()` 注册后端路由，并使用 `astrbot.api.web` 请求/响应抽象。
5. Dashboard 将 Page 请求转发到 `/api/v1/plugins/extensions/<plugin_name>/...`。
6. Page 使用相对资源路径；AstrBot 重写资源 URL 并添加短期 `asset_token`。
7. JSON GET/POST、文件响应和 `stream_response()`/`bridge.subscribeSSE()` 由宿主提供；alpha2 只使用 GET。

Plugin Pages 自 AstrBot 4.24.2 开始可用。AstrEmbodiment 当前的广泛运行声明是 `>=4.16,<5`。alpha2 不得因页面能力让旧宿主整个插件加载失败，因此冻结以下兼容策略：

- `main.py` 仅在 `context.register_web_api` 存在且可调用时延迟导入 `astrbot.api.web` 并注册路由。
- AstrBot `<4.24.2` 继续运行聊天、自主循环和原生核心，仅 Pages 不可用；主机日志只记录一次明确降级。
- README 的功能矩阵明确标注“观察台需 AstrBot >=4.24.2”。
- 不在模块顶层无条件导入 `astrbot.api.web`。

AstrBot v4.26.7 暴露的 Page asset token 与 Dashboard JWT 属于不同 token 类型和权限域。alpha2 不接收 Page 在 query/body/header 中传入的任一 token，不把 `asset_token` 当 API 身份，也不把 JWT 当静态资源凭证；两类凭证只由 AstrBot bridge/资源服务消费。为缩小类型混淆后的影响面，alpha2 后端严格 GET-only，不注册任何 POST、上传、下载、配置保存、wake、recover、rebuild 或 settlement 路由。

## 3. alpha2 目标

alpha2 交付以下闭环：

1. 从 AstrBot 插件详情页打开一个 `observatory` Page。
2. 复用 Dashboard 已认证会话、主题和语言上下文。
3. 服务端从已注册 scope 中签发短期 opaque `scope_handle`，不向浏览器交付原始 bot/persona/relation/session token。
4. 展示当前生命状态与有界时间线，所有语义来自闭合枚举和已提交投影。
5. 能够区分 committed inner evidence、canonical watermark 和 mutable projection；operational 联合证据缺少安全闭合投影时明确显示 `unavailable`，不把 claim 或 adapter submission 显示成 delivery confirmed。
6. 页面查询不修改 canonical revision、operational ordinal、generation、next wake、claim、budget 或 outbound state。
7. 打开和刷新观察台的 LLM 调用数为零。

## 4. alpha2 非目标和停止线

alpha2 不包含：

- 从 Page 更改 `proactive_enabled`、`inner_activity_mode`、Token 上限或任何其他配置。
- 从 Page 执行 `/ae_wake`、重建投影、恢复 orphaned claim、重发 outbound 或调用 provider。
- 全库导出、消息正文、候选文本、prompt、原始 UMO 或 SeedCode 展示。
- 新的 persona actor、CommitArbiter 或可重基 settlement 实现。本设计只让观察契约与该方向兼容。
- 把前端缓存、SSE 队列或 Python 对象变成第二份真源。
- 全历史大型力导图。因果页只显示选中事件一至两跳局部图。
- 直接从 Page 或 Python handler 查 SQLite 表。

任何缺少 scope ownership、投影验证失败、原生 schema 不识别或 Dashboard 用户未认证的情况都必须 fail closed。

## 5. 总体架构

### 5.0 前置因果修复：普通 DeliveryOutcome 的一次性重建

Pages 接入前先独立修复现有 Host revision 镜像可能落后于 Native canonical revision 的 `STALE_CAUSAL_BASE` 路径。该修复只适用于**尚未提交的普通 `DeliveryOutcome`**：Host 收到明确的 `STALE_CAUSAL_BASE` 后，从 Native 读取当前 committed revision，使用原冻结 delivery evidence 重建事件，并且最多重试一次。重试前必须确认原 event digest 没有提交证据；第二次冲突直接失败，不能循环。

该修复不适用于 user stimulus、wake、externalization 或 proactive dispatch settlement。proactive settlement 继续使用 `claim_token/outbound_id` 的 ID-addressed gate/settle 路径，不得包装成普通 `DeliveryOutcome` 来取得一次性重试。此修复使用独立测试和独立提交，避免 Pages 观察能力掩盖既有因果缺口。

```text
AstrBot Dashboard / Plugin Detail
              |
     restricted Plugin Page iframe
     pages/observatory/index.html
              |
     window.AstrBotPluginPage bridge
       |                              |
 GET observatory/bootstrap      GET observatory/events
       |                              |
       +------ authenticated extension route ------+
                              |
              ObservatoryPageApi (Host boundary)
                 | auth / range / handle checks
                              |
             ObservatoryProjectionService
                 | scope ownership / redaction
                              |
                     NativeBridge (read only)
               | observe_snapshot_v1
               | observe_events_v1
                              |
       canonical journal + operational authority + projections
```

### 5.1 与 persona actor / CommitArbiter 的关系

目标运行时方向是 `PersonaScope Actor -> CommitArbiter -> SQLite`。persona actor 拥有一个 persona 的逻辑时序和 HotBrain；CommitArbiter 是唯一物理写入者。claim/outbound/intention 的 settlement 通过 ID-addressed ownership index 找到 actor，而不依赖当前 Python scope。

观察台在这个模型中是 actor 与 CommitArbiter 之后的读投影：

- 它不接收可变命令，不分配 actor sequence。
- 它不持有有效 claim token，不执行 settlement。
- `observe_snapshot_v1` 显示已提交 canonical revision 和 operational ordinal，而不使用 Python 缓存版本。
- `observe_events_v1` 的每行标明来源，不把 projection 当成 authority。
- 未来 actor 落地时，只需在同一契约填充 `actor_epoch`/`actor_sequence`；Page URL 和模块不需改名。

### 5.2 Host 边界

`ObservatoryPageApi` 只做五件事：

1. 要求 `request.username` 非空，且 `request.plugin_name` 等于本插件名。
2. 限定请求方法为 GET，校验 cursor、limit 和 range；Native 只接收 UTC/committed-only 查询，本地化仅在脱敏响应之后进行。
3. 把当前 Dashboard 用户与已注册 scope 绑定为短期 `scope_handle`。
4. 调用原生闭合读 API，进行最后一次 Host 脱敏与数字限幅。
5. 使用 `astrbot.api.web.json_response/error_response` 返回纯业务 JSON。

Page 不能传入原始 scope JSON。`scope_handle` 不是持久 identity，它至少绑定 Dashboard username、plugin name、persona scope、可选 relation scope、签发时间和过期时间。插件重载后旧 handle 失效。

## 6. 原生读模型

### 6.1 `observe_snapshot_v1`

请求：

```text
ObserveSnapshotRequestV1 {
  schema_version: 1,
  persona_scope: Digest32,
  relation_scope: Option<Digest32>,
  mode: committed_only
}
```

响应至少包含：

```text
ObserveSnapshotV1 {
  schema_version: 1,
  generated_at_utc_ms: u64,
  persona_scope: Digest32,
  relation_scope: Option<Digest32>,
  canonical_revision: u64,
  operational_ordinal: Option<u64>,
  actor_epoch: Option<u64>,
  actor_sequence: Option<u64>,
  projection: { head_consistent },
  runtime: AutonomousRuntimeStateV1,
  mood: Option<MoodCardV1>,
  pending: { intentions, outbounds, active_claims },
  budget: { day_start_utc_ms, reserved_tokens, used_tokens },
  latest_gate: Option<GateDecisionV1>
}
```

约束：

- `canonical_revision` 从 Store 读取，不从 `main.py::_revisions` 或 HotBrain 镜像读取。
- `operational_ordinal` 从已验证 operational authority head 读取。
- 当 actor 尚未实现时，`actor_epoch` 和 `actor_sequence` 为 `None`，不伪造 `0`。
- `latest_gate` 只有在存在已提交 gate evidence 时返回；不为了 UI 重新运行 gate，也不将“未记录”显示为许可。
- `used_tokens` 只统计已结算 provider usage；不把 reservation 伪称为实际消耗。
- 调用前后 canonical revision、operational ordinal、generation、claim count 和 budget 必须相同。
- `mode` 是闭合枚举且只接受 `committed_only`；Native 不接收 display timezone，不在权威层做本地化。
- `projection.head_consistent` 只声明当前 head 的 canonical delta 与 runtime/snapshot projection 一致，不宣称从 genesis 完整重放验证。
- alpha2 暂无安全闭合 operational/pending/budget 细分投影时，对应 `Option` 字段返回 `None`，不得用零值伪装“没有”。

### 6.2 `observe_events_v1`

请求：

```text
ObserveEventsRequestV1 {
  schema_version: 1,
  persona_scope: Digest32,
  relation_scope: Option<Digest32>,
  through_revision: Option<u64>,
  after: Option<ObserveCursorV1>,
  limit: u16,                 // 1..=64
  mode: committed_only
}
```

响应：

```text
ObserveEventsV1 {
  schema_version: 1,
  items: Vec<ObserveEventV1>,
  next: Option<ObserveCursorV1>,
  high_water: ObserveHighWaterV1
}

ObserveEventV1 {
  public_event_ref: String,
  source: inner,
  kind: closed enum,
  committed_at_utc_ms: u64,
  relation_scope: Option<Digest32>,
  authority_status: committed,
  causal: { turn_id, action_id, delivery_id, claim_id, parent_event_ref },
  summary_code: String,
  value_before: Option<Fixed>,
  value_after: Option<Fixed>
}
```

联合顺序冻结为：

```text
(journal_revision, event_id)
```

cursor 绑定 persona scope，并用 `journal_revision` 作为权威先后次序；`event_id=None` 表示该 revision 已完整消费，也覆盖零 inner-event 的 canonical TimeAdvance。分页运行用 `through_revision` 固定水位，单次至多验证 65 个有界 revision 并返回至多 64 项；不依赖可能回拨的 UTC 时间、SQLite rowid 或 Python 字典顺序。`high_water` 返回 `through_revision`；只有 operational head 有安全只读投影时才携带 operational ordinal，否则该字段为 `None`，UI 明示 `unavailable`。

Store v6 维护逐 revision 的 inner-event manifest，包括零事件 revision。打开、迁移、重建、提交和观察路径均双向核验 canonical delta、manifest 与 projection；任一缺行、多行或不一致均 fail closed，不允许观察接口静默返回不完整历史。

alpha2 v0 不将 canonical journal 原始 bytes 或 operational delta 快照投影进联合时间线。若后续闭合投影通过独立契约评审，使用新 schema version 增加来源；不得在 v1 中 mock canonical/operational 事件行。

## 7. 共享 URL 生态模块壳

观察台只声明一个 AstrBot Page：`pages/observatory/`。页内模块共用一个 URL shell，不建立多个彼此割裂的 Page：

```text
#/overview
#/timeline?range=24h
#/sleep?range=7d
#/affect?range=24h
#/intentions?state=pending
#/causality?focus=<public_event_ref>
#/resources
```

模块壳负责：

- 从 `bridge.ready()` 接收 `pluginName/pageName/locale/isDark/i18n`。
- 只解析白名单 hash route 和白名单 query key。使用 hash routing，不使用 history routing。
- 在页内共享 scope selection、time range、cursor、loading/error 和 focus event。
- URL 中只允许 `public_event_ref` 和无敏感筛选条件。`scope_handle`、原始 digest、username 和 provider identifier 不写入 URL。
- 模块之间使用同一份已验证 snapshot/events store，不各自直接调 API。
- 页面卸载时停止 timer；未来如开启 SSE，同时取消 subscription。

这个 shell 是 AstrEmbodiment 未来 Pages 模块的共享 URL 生态边界。alpha2 只有 observatory，不提前抽出跨插件 npm 包，不使用远程 CDN。

## 8. 信息架构

### 8.1 全局顶栏

始终显示：

- `DEV / READ ONLY`；
- AstrEmbodiment build/version 与 Native health；
- Dashboard 已认证用户（仅用于当前页面，不进入导出）；
- Bot -> Persona -> 可选 Relation selector；
- UTC、Persona current timezone、Relation user timezone；
- projection verify 状态与 snapshot 新鲜度。

### 8.2 Overview

概览卡片包括：

- 生命：sleep state、generation/state revision、last advanced、next wake、wake intensity；
- 情绪：mood、arousal、affiliation need、social energy、unfinished-topic salience、workspace residual；
- 意图：pending intention/outbound/claim 数量；
- 主动表达：最近已提交 gate evidence 或 `unavailable`；
- 成本：配置开关状态、reservation、actual usage 和上限。

### 8.3 Timeline

时间线使用六条泳道：

1. user/time input；inbound、offline gap、clock rollback、authorized emergency。
2. sleep/wake；sleep transition、scheduled wake、emergency bypass、next-wake change。
3. affect/workspace；homeostasis、workspace ignition/suppression/residual rejection。
4. intention；formed、deferred、suppressed、action arbitrated。
5. external action；externalization、outbox、dispatch、unknown/recovery。
6. authority；canonical revision watermark、可用时的 operational ordinal、projection status；不可安全投影的 operational timeline 标为 `unavailable`。

每行必须有 source 和 authority status。只有 `delivery_confirmed` 可显示为已送达；`adapter_submitted` 不得升格。

### 8.4 Sleep

显示 24h/7d 阶梯轨道：Persona 本地作息窗口、Process S/C、circadian phase、sleep state、next wake、wake reason、travel phase adjustment 和 emergency evidence。图表不对 fixed-point 样本做伪平滑。

### 8.5 Affect

以已提交样本显示 arousal、affiliation need、social energy、unfinished salience 和 workspace residual。Mood 文案仅来自 `MoodCardV1`，必须携带 `event_id/as_of_utc_ms`；不生成自由“内心独白”。

### 8.6 Intentions

显示状态流转和已提交 suppression reason。闭合 suppression 包括 `proactive_disabled`、`target_unavailable`、`intention_unavailable`、`residual_rejected`、`persona_asleep`、`quiet_hours`、`daily_limit`、`cooldown`、`unanswered_hard_stop`、`timezone_unreliable`。不展示 `prompt_contract`、candidate ciphertext 或 target 明文。

### 8.7 Causality

alpha2 v0 只用 committed inner event 与 canonical watermark 绘制可证局部链。canonical/operational 逐事件节点、turn/action/delivery/claim/parent 边或 previous->head operational 链缺少安全投影时显示 `unavailable`，不以 mock 节点补齐。projection invalid 时只显示失败，不从 Page 触发 rebuild。

### 8.8 Resources

只读显示：

- `autonomous_runtime_enabled`；
- `inner_activity_mode`；
- `inner_activity_display`；
- `proactive_enabled`；
- `inner_activity_token_daily_max`；
- `proactive_daily_max`；
- Provider 是否配置（匿名）；
- 当日 reserved/used Token。

页面明确提示“请在 AstrBot 插件配置中修改”，但不提供 POST 或绕过 `_conf_schema.json` 的开关。

## 9. 安全与隐私边界

### 9.1 鉴权与请求

- 复用 AstrBot Dashboard 认证链，但 handler 仍需求 `request.username` 非空。
- 当前 AstrBot Page 契约未向插件暴露可验证的细粒度 role/ACL，所以本设计只声称“已认证 Dashboard 用户”，不伪称 RBAC。
- 每个 handler 校验 plugin name、scope handle、limit、cursor 和时区。
- `observatory_pages_enabled` 默认 `false`；关闭时路由返回 404/403，不返回部分数据。

### 9.2 禁止序列化的数据

下列值不得进入 Page JSON、SSE、浏览器 URL、console 或诊断导出：

- SeedCode 与 genesis 私密材料；
- 用户/机器人消息正文；
- `prompt_contract`、模型候选文本与 `candidate_ciphertext`；
- outbound target 明文、UMO、DPAPI/AES key material；
- provider secret 与完整 provider identifier；
- `caller_incarnation`；
- 原始 bot/persona/relation/session token。

可视 ID 默认显示脱敏 `public_event_ref`。完整内部 digest 只能在开发者明确展开且经 Host 再次脱敏后显示，alpha2 不提供该展开。

### 9.3 读路径不变式

对任一观察请求，请求前后必须满足：

```text
canonical_revision unchanged
operational_ordinal unchanged
runtime generation unchanged
next_wake_at_utc_ms unchanged
claim set unchanged
outbound lifecycle unchanged
budget reservation/usage unchanged
LLM calls = 0
send_message calls = 0
```

不允许用“为了展示最新结果”为由运行 gate、recover 或 rebuild。

## 10. 轮询、SSE 和资源限制

alpha2 默认：

- bootstrap 一次；
- snapshot 每 5 秒最多一次；
- events 按 cursor 增量获取，Native 与 Host 都严格执行 `1 <= limit <= 64`，避免把跨多来源聚合伪装成一个原子快照；
- 页面 hidden 或用户点击 pause 时停止轮询；
- 错误采用 1s/2s/5s/10s/30s 有界退避，不热循环；
- 展示窗口最多保留 2,000 个脱敏事件，更早数据需新 cursor 查询。

如后续开启 SSE：

- 必须使用 `bridge.subscribeSSE()`，不能直接使用 `EventSource`；
- SSE 只推送 `cursor/high_water/changed_kinds`，完整数据仍经 GET；
- 每个 username/plugin 最多一条连接，15–30 秒 heartbeat；
- unload 时取消 subscription，断线后用 cursor GET 恢复。

## 11. 包体与发布边界

`scripts/package_plugin.py` 已显式包含：

```text
pages/observatory/index.html
pages/observatory/app.js
pages/observatory/style.css
```

页面使用原生 ES module/CSS，不添加 Node runtime、远程 CDN 或大型 source map。归档后必须继续满足 AstrBot 市场 16 MiB 上限，并保留 Windows/Linux 原生双平台内容。

alpha2 对外声明严格限定为：

- Pages 是开发态观察能力，默认关闭；
- 支持 Pages 的宿主为 AstrBot >=4.24.2；
- mock bridge 的桌面/390 px 验收不能替代真实 AstrBot Dashboard 鉴权和路由验证；该项完成前整体结论为 PARTIAL。
- Windows wheel 已通过构建与运行验证；Linux wheel 已通过构建与静态检查，Linux 运行 smoke 完成前不得声称双平台运行 PASS。

## 12. 验收准则

alpha2 Pages 只在以下条件全部成立时 PASS；当前因第 1、2、3 项的真实宿主组合验证和 Linux 原生运行 smoke 尚未完成，发布验收为 PARTIAL：

1. AstrBot >=4.24.2 能在插件详情页发现并打开 `observatory`。
2. 未登录 Dashboard 时静态资源/API/未来 SSE 均不可用。
3. AstrBot <4.24.2 不会因 Pages 导入失败而阻止插件核心加载。
4. `observe_snapshot_v1` 和 `observe_events_v1` 只返回目标 scope 的闭合已提交证据。
5. 查询前后所有读路径不变式成立。
6. Timeline 显示 committed inner evidence 与 canonical watermark；operational/canonical 逐事件证据不可用时明确显示 `unavailable`，意图页只显示已提交闭合 suppression reason，资源页只读显示 Token 开关与 reservation/usage。
7. 页面响应和浏览器 URL 不含禁止序列化数据。
8. 打开、轮询、切换模块、暂停和关闭 Page 产生零 LLM 调用、零 `send_message`、零 claim。
9. Windows + Linux x86_64 通用 alpha2 ZIP 包含 Page 资源与双平台 Native，且不超过 16 MiB。

## 13. 实施门禁

本文档记录 alpha2 已实现边界，不授权推送、发布、安装到生产 AstrBot、打开主动发送或修改 persona actor 权威边界。实现顺序已遵循“原生读契约 -> Host API -> Page 资源与包体”；mock bridge 截图只证明前端契约和响应式布局，不得用来声称真实 AstrBot 鉴权验收完成。
