# AstrEmbodiment 主动联系设置简化设计

日期：2026-08-30

状态：方向已批准，本文只定义设计，不修改生产代码

设计修订：`1`；本规格尚未实施，`auto` 直接纳入首次迁移，不另造 revision `2`

基线：`aed5a8db86fc0c4daff63f4ca394293ca6ec784b`（`release: prepare 1.1.0-alpha3`）

## 1. 背景与当前事实

当前 `_conf_schema.json` 把以下实现参数直接暴露给普通用户：

- `proactive_daily_max`；
- `min_proactive_cooldown_minutes`；
- `intention_ttl_minutes`；
- `unanswered_backoff_base_minutes`；
- `unanswered_hard_stop`。

这五项混合了“用户希望被联系得多频繁”和“系统怎样保证意图不过期、未回复后怎样退避”两类不同责任。普通用户实际需要控制的是是否允许联系、打扰强度、免打扰时段和花费上限，不应理解 TTL、指数退避或硬停止阈值。

当前代码与发布状态还有四项必须保留的事实：

1. `main.py::_autonomy_binding()` 直接读取旧的 daily max、cooldown、TTL、backoff 和 hard-stop 键，并把它们写入 `RelationTemporalPolicyV1`。
2. `_config_value()` 已有兼容旧配置别名的先例，但当前没有一个统一的主动联系设置解析器。
3. `inner_activity_token_daily_max` 的显示名称仍是“内心活动每日 Token 上限”；Alpha 3 已移除 live mind/inner-event 表面，因此该名称与产品职责不符。
4. 当前 release HEAD 只声明源码和发布契约准备完成。按 README 与 CHANGELOG，此提交本身不包含 fresh 双平台 wheel、最终 ZIP、离线宿主烟测或真实平台主动发送验收。本设计不改变该发布结论。

## 2. 目标

本设计把主动联系相关的基础配置收敛为四个用户概念：

1. **允许机器人主动联系**；
2. **主动联系频率**：自动、克制、适中、自定义；可由 Host 证明是当前 schema 首次创建的全新安装默认自动，无法取得来源证明时保持克制并提示用户可手动选择自动；
3. **用户免打扰时段**；
4. **主动表达每日 Token 预算**。

同时必须满足：

- “频率”只生成约束上限，不创建定时任务、不形成联系原因，也不保证达到某个发送次数；
- 自动档位只能在克制 `2/360` 与适中 `4/180` 的闭区间内收放，不能突破每日 4 条或把最短间隔降到 180 分钟以下；
- 旧配置继续可读，旧的非默认数值不会被预设静默覆盖；
- 过期、未回复退避和停止策略从基础界面移出，由内部策略管理；
- 配置解析失败时收紧或关闭主动联系，不能放宽限制；
- 不虚构 AstrBot 配置 schema 不具备的条件显示能力。

## 3. 非目标

本设计不做以下工作：

- 不增加 Pages、Observatory、独立 WebUI 或新的设置页面；
- 不改变 relation 级 consent、cause、睡眠、时区、目标加密、Provider、预算、宿主能力和 dispatch claim/settlement 门禁；
- 不把频率档位解释成固定时间表或“每天必发 N 条”；
- 不恢复 `grant/resume`；当前 headless 外部控制仍只允许 `pause/end`；
- 不删除旧配置键，不清理旧配置文件，也不改写 retired 数据表；
- 不以配置页保存成功代替真实 AstrBot 主动消息验收；
- 不在本设计提交中修改生产代码、构建原生模块或生成发布包。

## 4. 方案比较与选择

### 4.1 方案 A：只改中文说明，继续展示五个数字

优点是实现最小，运行时无需迁移。缺点是没有解决用户必须理解内部参数的问题，也无法形成“克制/适中”的清晰产品语义。

### 4.2 方案 B：新增档位下拉，并声称选择“自定义”后动态显示数字框

体验理想，但不能采用。AstrBot 官方插件配置契约公开了静态 `invisible`、`object/items`、`options/labels` 等能力，没有公开依赖其他字段值的条件显示契约。插件支持范围是 `>=4.16,<5`，不能依赖某个 Dashboard 版本可能存在的未公开行为。

### 4.3 方案 C：四个顶层概念，静态分组承载自定义项

采用此方案。`主动联系频率` 使用一个 `object` 分组，组内显示档位和两个自定义数字；两个数字始终可见，但说明明确写出“仅选择自定义时生效”。`用户免打扰时段` 同样使用一个分组承载开始、结束及是否允许已授权紧急事件绕过。旧字段保留在 schema 中并设为静态隐藏。

这是当前 AstrBot schema 下的诚实退化：没有动态显隐，但基础配置只呈现四个顶层概念，也不需要重新引入 Pages。

AstrBot schema 能力依据：[官方插件配置文档](https://github.com/AstrBotDevs/AstrBot/wiki/zh-dev-star-guides-plugin-config)。该文档还说明 schema 更新会递归补齐缺失默认值并移除 schema 中已不存在的配置项，因此兼容旧键必须继续留在 schema 中。

## 5. 用户界面契约

### 5.1 顶层概念

主动联系相关配置在管理面板中只保留以下四个顶层概念：

| 顶层键 | 显示名称 | 形态 | 默认值 |
| --- | --- | --- | --- |
| `proactive_enabled` | 允许机器人主动联系 | `bool` | `false` |
| `proactive_frequency` | 主动联系频率 | `object` | 自动；自定义值预填 `2/360` |
| `user_quiet_hours` | 用户免打扰时段 | `object` | `00:30–08:30`，保留当前紧急绕过默认值 |
| `inner_activity_token_daily_max` | 主动表达每日 Token 预算 | `int` | `1024` |

`proactive_frequency.mode` 的 schema 默认值是 `auto`，供 Host 明确的首次创建流程使用。schema 补默认后的普通 config dict 不携带“全新安装”证明；来源未知的 `2/360` 实例必须按 8.2 迁移成 `restrained`，不能仅凭这个默认值启用 auto。

这里的“四个”只指主动联系设置表面。资源包络、原生数据目录、Persona 作息、用户时区、Provider 和 SeedCode 等其他插件设置不属于本设计，不因本设计隐藏或重排。

### 5.2 `proactive_frequency` 分组

该分组包含：

| 子键 | 显示名称 | 类型与默认值 | 规则 |
| --- | --- | --- | --- |
| `mode` | 频率档位 | `string`，默认 `auto` | 选项为 `auto/restrained/moderate/custom`，标签为“自动/克制/适中/自定义” |
| `custom_daily_max` | 自定义每日上限 | `int`，默认 `2` | 仅在 `mode=custom` 时生效 |
| `custom_cooldown_minutes` | 自定义最短间隔（分钟） | `int`，默认 `360` | 仅在 `mode=custom` 时生效 |

分组 hint 必须同时说明：

- 频率只是上限，不会定时触发联系；
- 是否联系仍由有效 cause、relation consent 和全部原生门禁决定；
- 自动档位会参考睡眠、关系活跃度、回复/未回复趋势和当日 Token 预算，但始终受 `2/360–4/180` 包络约束；
- AstrBot 当前没有本设计可依赖的条件显示能力，因此两个自定义输入会一直显示，只有“自定义”档位读取它们。

### 5.3 `user_quiet_hours` 分组

该分组包含：

| 子键 | 显示名称 | 类型与默认值 | 规则 |
| --- | --- | --- | --- |
| `start` | 开始时间 | `string`，`00:30` | 按 `user_timezone` 解释 |
| `end` | 结束时间 | `string`，`08:30` | 支持跨午夜；起止相同表示不启用该时段 |
| `allow_authorized_emergency_bypass` | 允许已授权紧急事件绕过 | `bool`，`true` | 只保留现有已授权紧急绕过语义，不绕过总关闭、daily max、hard stop 或预算 |

`emergency_threshold` 仍是内部安全参数，不在基础表面展示。旧的 `quiet_hours_emergency_bypass` 作为隐藏兼容键保留。

### 5.4 Token 预算改名

本轮只改显示语义，不改存储键：

- 存储键继续使用 `inner_activity_token_daily_max`；
- `description` 改为“主动表达每日 Token 预算”；
- hint 明确它限制关系级主动表达的 Provider 预留/结算，未知 usage 按完整预留量计费；
- README 将该键标注为历史存储名，避免用户误以为 Alpha 3 仍提供内心活动读取。

保留存储键可以避免同一预算同时出现新旧两个可编辑值，也避免 AstrBot schema 更新删除旧值。

## 6. 频率档位的精确语义

### 6.1 档位映射

| 档位 | `proactive_daily_max` 有效值 | `min_proactive_cooldown_minutes` 有效值 | 设计理由 |
| --- | ---: | ---: | --- |
| 自动 `auto` | `2..4` | `180..360` | 按 6.2 的确定性规则在克制、平衡、适中三档中选择；仅 Host 可证明的当前 schema 首次创建实例默认采用 |
| 克制 `restrained` | `2` | `360` | 与 Alpha 3 当前默认完全相同，升级不改变默认行为 |
| 适中 `moderate` | `4` | `180` | 在仍有每日总量和三小时最短间隔的前提下提供更宽松上限 |
| 自定义 `custom` | `custom_daily_max` | `custom_cooldown_minutes` | 使用用户给出的两个合法整数 |

daily max 和 cooldown 必须同时满足。举例：适中档位并不表示每三小时发送一次，也不表示每天发送四条；它只表示在其他门禁全部通过且确有有效原因时，一天最多四条、两次成功计入主动提交之间至少三小时。

### 6.2 自动档位

`auto` 是一个动态**上限策略**，不是调度器、联系原因或 consent。它不能创建 cause、推进 contact due、唤醒睡眠角色、调用 Provider 或发送消息；它只在一次本来就要执行的原生 gate/claim 中，为 daily max 和 cooldown 选择更克制或更宽松的上限。

#### 6.2.1 冻结输入

每次 gate/claim 都从同一个冻结且可摘要的权威快照读取：

- Persona 当前 sleep state；
- 除 daily max/cooldown 以外的现有安全门结果，包括全局开关、relation consent、有效 cause、目标、可信时区、quiet hours、Provider/宿主能力、usage settlement 和预算 authority；
- relation contact process 中的 `last_inbound_utc_ms`、`last_outbound_submitted_utc_ms`、`response_cadence_ema_ms` 和 `consecutive_unanswered`；
- 当前用户本地预算日的 `daily_submitted`、剩余 Token，以及下一次 externalization claim 的完整预留量。

任一必需状态缺失、摘要不匹配或来源不可信时，`auto` 直接抑制本次主动联系，不能根据默认值猜测。`sleep_state` 不是 `Awake` 时同样抑制；已授权紧急事件必须先通过既有 wake 路径形成新的 Awake 冻结快照，再重新 gate，`auto` 本身不绕过睡眠。

auto 档位选择由 Native 单写者在原子 gate/claim 内拥有。Host 只提交配置合同和已绑定摘要的冻结证据，不能自行选择平衡或适中档；Host 上的预览结果没有发送 authority。这样 daily/cooldown、预算、unanswered 和 cause 使用同一事务快照，避免配置读取后状态变化造成 TOCTOU。

#### 6.2.2 活跃与预算派生

关系活跃窗口按以下规则唯一计算：

```text
activity_window =
  response_cadence_ema 存在时：clamp(2 × response_cadence_ema, 24 小时, 14 天)
  response_cadence_ema 缺失时：7 天

relation_active =
  last_inbound 存在
  且 effective_now - last_inbound <= activity_window
```

时间相减必须使用无回退的冻结时间；出现未来时间或回退证据时抑制，不把它当成“刚刚活跃”。

预算对本日频率的支持上限按以下规则计算：

```text
remaining_claims = floor(remaining_tokens / next_claim_reservation_tokens)
token_supported_daily_ceiling = min(4, daily_submitted + remaining_claims)
```

`next_claim_reservation_tokens` 必须大于零且与后续真实 claim 完全相同。即使 auto 选出的 effective daily max 最低为 `2`，Token 预算门仍可把当天实际剩余次数压到 `1` 或 `0`；频率包络不能放宽预算。

#### 6.2.3 三档选择

在前置安全门可证明、Persona awake 且预算 authority 完整后，按以下顺序选择，先匹配即停止：

1. `consecutive_unanswered >= unanswered_hard_stop`：由现有 hard-stop 门直接抑制，不产生 auto 档位。
2. `consecutive_unanswered > 0`：选择**克制** `2/360`，并继续叠加现有 `base × 2^(n-1)` 未回复退避；退避比 360 分钟更长时以更长者为准。
3. relation 不活跃，或 `token_supported_daily_ceiling <= 2`：选择**克制** `2/360`。
4. relation 活跃、`consecutive_unanswered == 0`、最近一次 inbound 严格晚于最近一次主动 outbound，且 `token_supported_daily_ceiling >= 4`：选择**适中** `4/180`。
5. relation 活跃、`consecutive_unanswered == 0` 且 `token_supported_daily_ceiling >= 3`：选择内部**平衡**档 `3/240`。
6. 其余可证明状态：选择**克制** `2/360`。

没有历史主动 outbound 时不能满足第 4 条，只能在证据充分时进入平衡档。内部平衡档不是新的 UI 选项，只是 auto 在两个公开预设之间的确定值。

auto 每次 gate/claim 重新计算，不把动态档位写回插件配置。入站可以把 unanswered 清零并在下一次真实 gate 时恢复到平衡或适中；没有入站时不得因时间经过自行变宽松。无论状态如何，auto 必须同时满足：

```text
2 <= effective_daily_max <= 4
180 <= effective_cooldown_minutes <= 360
```

因此 auto 永远不会突破每日 4 条，也不会把最短间隔降到 180 分钟以下。

### 6.3 自定义值域

为了兼容旧配置，解析器接受当前原生类型可以表示的非负整数，而不擅自把旧值钳制为新的产品推荐范围：

- `daily_max` 必须是 `0..65535` 的整数；`0` 表示主动联系始终被 daily-limit 门禁阻断；
- `cooldown_minutes` 必须是非负整数，乘以 `60_000` 后不得超过无符号 64 位整数；`0` 表示不额外施加 cooldown，但其他门禁仍然生效；
- 布尔值不得当作整数接受；字符串数字不得隐式转换。

新 UI 的 hint 应推荐普通用户使用预设。自定义值无效时，不回退到可能更宽松的隐藏旧值，而是令本次运行的主动联系有效开关为 `false` 并记录明确警告；配置文件原值保持不变，供用户修正。

Token 预算必须是原生预算类型可表示的非负整数。`0` 表示不允许主动表达；非零值若小于当前单次 claim 的完整预留量，也必须在调用 Provider 前被预算门禁阻断，不能先生成再补记账。

## 7. 隐藏的内部策略

以下旧键继续存在于 `_conf_schema.json`，但设置 `invisible: true`：

- `proactive_daily_max`；
- `min_proactive_cooldown_minutes`；
- `quiet_hours_start`；
- `quiet_hours_end`；
- `quiet_hours_emergency_bypass`；
- `intention_ttl_minutes`；
- `unanswered_backoff_base_minutes`；
- `unanswered_hard_stop`；
- `emergency_threshold`。

前三组键承担升级兼容，不再是新 UI 的权威输入。TTL、未回复退避和硬停止继续由内部策略使用：

- 新安装默认意图 TTL 为 24 小时；到期或 cause 失效时作废，不因 TTL 尚未结束而跳过 cause 复核；
- 未回复退避默认以 6 小时为基数，按现有连续未回复次数做指数退避；
- 连续 3 次未回复后停止普通主动联系，直至真实入站复位；
- auto 遇到第一次连续未回复即降到克制档，随后只会被更长退避或 hard stop 进一步收紧；没有真实入站时不会自行回升；
- 旧安装中合法的非默认 TTL/backoff/hard-stop 值继续作为隐藏兼容覆盖值读取，本轮不静默改写。

这里的“内部自适应”指系统根据意图有效性、连续未回复状态和真实入站复位自动决定可联系时间，而不是把这些机制继续作为普通用户的频率偏好。hard stop 是固定安全边界，不应被包装成频率档位。

## 8. 迁移与优先级

### 8.1 版本标记

新增隐藏整数键 `proactive_settings_revision`，默认 `0`。解析器完成一次兼容迁移并成功通过 AstrBotConfig 保存后写为 `1`。

本设计尚未进入生产实现，`auto` 属于 revision `1` 的初始合同，不把同一份未实施设计虚增为 revision `2`。

AstrBot 会在 schema 更新时自动补齐新字段，因此插件收到的合并后 dict 不能证明某个键原先是否存在，也不能仅凭新 `mode` 的默认值判断新装还是升级。legacy `proactive_daily_max` 和 `min_proactive_cooldown_minutes` 在新 schema 中继续保留历史默认 `2/360`，不引入数值哨兵。

新装来源只能来自 AstrBot Host 在默认合并前提供的、可摘要的配置来源证据，例如“该插件配置实体由当前 schema 首次创建”。证据必须绑定插件 ID、配置实体、schema digest 和本次加载；普通 config dict、字段值、文件时间、Native 数据库是否存在或插件自行探测路径都不是来源 authority。

当前受支持的 AstrBot 插件配置公开合同没有承诺把这类首次创建证据传给插件。因此设计定义两种诚实路径：

- Host 能证明配置由当前 schema 首次创建，且合并后的新频率分组与当前 schema 默认表面一致：全新安装采用 `auto`；
- Host 不能证明：按已有配置处理。合并后的 `2/360` 保持 `restrained`；其他合法 pair 保持 `custom`。界面 hint/日志说明“未取得新装来源证明，已保持克制；可手动选择自动”。

因此，如果实现只修改插件而当前 Host 仍不提供首次创建证据，可交付的安全默认就是 `restrained`；不得把 schema 合并出的 `mode=auto` 当作新装证明。用户之后明确保存 `auto`，才按 revision `1` 的新分组运行。

不得为识别新装而直接读取、改写或比较 AstrBot 的宿主配置文件，不得使用文件创建时间、目录是否存在、SeedCode 或数据库状态猜测来源。

### 8.2 第一次迁移

当 revision 等于 `0` 时，按下列顺序处理。revision 缺失时使用 schema 默认 `0`；非整数或负数直接 fail closed，不进入迁移：

1. 读取 Host 提供的 pre-merge config-origin evidence；缺失或验签、绑定、时效校验失败时把来源记为 `unknown`，不尝试文件旁路。Host 若明确证明配置实体早于当前 schema，则记为 `existing`。
2. 读取旧 `proactive_daily_max` 和 `min_proactive_cooldown_minutes`，但不修改它们；旧键缺失时按旧运行时真实 fallback `2/360` 解析。
3. 来源被证明为 `fresh-current-schema`，且新频率分组完整合法、`mode=auto`、自定义预填和 legacy pair 都是当前 schema 默认 `2/360` 时，把新 `mode` 保持为 `auto`。任一值不一致都视为来源/值矛盾并 fail closed，不能降级猜测。
4. 其余来源中，legacy pair 合法且正好是 `2/360` 时，把新 `mode` 设为 `restrained`，并把自定义输入预填为 `2/360`。新 schema 的 mode 默认 auto 不得改变既有或来源未知实例的行为。
5. 其余来源中，legacy pair 合法且不是 `2/360` 时，把新 `mode` 设为 `custom`，把两个旧值原样复制到自定义输入。包括 `daily_max=0` 或 `cooldown=0`，不擅自改成预设。
6. legacy pair 无效时，本次主动联系 fail closed，revision 保持 `0`；即使新 mode 的合并后默认是 auto，也不得启用。第 3 步的来源/值矛盾采用同一结果。
7. 把合法的旧 quiet-hours 值复制到 `user_quiet_hours`；旧值无效时保留旧键原文，并让新分组使用现有安全默认，同时记录警告。
8. 保持 `inner_activity_token_daily_max` 原值不变，只更新 schema 的显示文案。
9. 通过 AstrBotConfig 的保存接口一次性保存新分组和 revision；不得直接写插件目录或另造配置文件。
10. 保存失败时回滚本次内存写入，revision 保持 `0`。已有或来源未知实例继续使用合法 legacy 等价值；被证明为 fresh 的实例对本次运行 fail closed，并在下次加载重试。保存失败本身不能扩大频率或启用主动联系。

迁移日志必须说明来源判定为 `fresh-current-schema/existing/unknown`、验证结果为 `valid/origin-value-mismatch/invalid-legacy`、采用了 `auto/restrained/custom/fail-closed` 中哪一种，以及迁移是否保存成功，但不得输出目标、消息正文、密钥或其他关系隐私。

### 8.3 运行时权威优先级

解析优先级固定为：

1. `proactive_settings_revision == 1` 且新分组完整合法时，新分组权威；`mode=auto` 时再按 6.2 的冻结运行时证据计算当次有效档位；
2. revision 等于 `0` 或新分组尚未成功持久化时，按 8.2 的 Host-proven fresh、legacy default、legacy custom、invalid 四类规则生成有效值；
3. 新分组在 revision 已为 `1` 后被手工改坏时，主动联系对本次运行 fail closed，不复活可能更宽松的隐藏旧值；
4. revision 大于 `1` 时视为当前实现不认识的未来合同并 fail closed，不能按 revision `1` 猜测解析；
5. 新旧输入都无法形成合法策略时，主动联系有效开关为 `false`。

用户保存 auto、克制、适中或自定义档位时，不反向改写隐藏的 old daily/cooldown 键。这样旧值仍可审计，且不会出现“选择新档位后历史配置被无提示覆盖”。revision 等于 `1` 时，隐藏旧值只用于审计，不与新档位竞争；新分组缺失或损坏时按失败矩阵关闭主动联系，不用旧值灾难性放宽。

### 8.4 配置摘要与 revision

当前 `main.py` 在构造函数中直接用原始 `_config_values` 计算 source digest 和 revision。实现本设计时，必须先完成纯解析/迁移决定，再以**有效且规范化后的主动联系设置**计算 digest；不能先生成旧 digest，随后把另一组频率传给 Native。

若持久化必须等待 `initialize()` 的异步保存接口，则在启动 AutonomousSupervisor 前重新生成 digest/revision。任何 bootstrap、claim 或 Provider 调用都不得发生在解析结果和 digest 不一致的窗口内。

配置 source digest 对 auto 只绑定稳定合同：`mode=auto`、包络 `2/360–4/180`、内部平衡档 `3/240` 和 `auto_policy_version=1`，不把不断变化的当次档位写回配置或配置 digest。每次 gate/claim 的 decision/input digest 另行绑定 6.2 的冻结 sleep、contact、budget、安全门证据及最终 effective daily/cooldown。这样同一配置可安全自适应，而任何状态替换都会使旧 decision 失效。

## 9. 运行时读取兼容

实现分成两个单一职责边界：

1. `resolve_proactive_settings(config) -> ResolvedProactivePolicy` 只解析和规范化稳定配置、迁移来源与 fail-closed 状态，不读取运行时 sleep/contact/budget；
2. `evaluate_auto_frequency(resolved_policy, frozen_evidence) -> EffectiveFrequency` 只在 `mode=auto` 的一次 gate/claim 内消费 6.2 的冻结证据。非 auto 模式直接从 resolved policy 得到固定 pair，不调用 auto evaluator。

第二个边界的生产 authority 属于 Native 单写者；Python 可以提供同公式的只读预览或序列化 wrapper，但其结果不能授权发送。`_autonomy_binding()` 和 gate/claim 调用方只消费这两个类型化结果，不再分别读取新旧键或自行重算 auto。

`ResolvedProactivePolicy` 至少包含：

- effective enabled；
- configured frequency mode；
- auto policy version 和固定包络；非 auto 模式为空；
- 固定模式的 daily max/cooldown，auto 模式为空；
- quiet-hours start/end minute；
- authorized emergency bypass；
- intention TTL；
- unanswered backoff base；
- unanswered hard stop；
- proactive expression daily token budget；
- source kind（fresh auto、new preset、new custom、legacy migration、fail-closed）；
- 规范化配置摘要输入。

`EffectiveFrequency` 至少包含 effective band、daily max、cooldown milliseconds、冻结 evidence digest 和选择 reason code。非 auto 的固定 pair 仍用同一结果类型，但 evidence digest 标为配置固定值，确保下游只有一个字段来源。

兼容读取规则：

- 旧版平铺配置、旧版默认合并配置和新版 object 配置都必须得到确定结果；
- 缺值只使用本文规定的默认值，不从其他无关字段猜测；
- 解析器不得修改传入对象；迁移持久化由独立步骤完成；
- `_autonomy_binding()` 只把 effective 值转换成现有 `RelationTemporalPolicyV1` 字段；Native 字段名和单位不因 UI 改名而改变；
- 旧 `inner_activity_token_daily_max` 必须继续作为预算存储键读取。

source digest 的主动联系部分必须来自上述有效结果，而不是同时散列两套互相冲突的 daily/cooldown 值。revision 等于 `1` 时，非权威的隐藏 daily/cooldown 副本不进入有效策略摘要；TTL/backoff/hard-stop 等仍实际生效的隐藏兼容值继续进入摘要。解析失败时摘要必须绑定 `fail-closed` 状态及失败类别，不能散列成克制档位后假装配置有效。

代码核验发现当前 `main.py` 没有读取 `inner_activity_token_daily_max`。因此实现阶段不能只改中文标签：必须把有效 Token 预算绑定到现有 relation budget policy 写入路径；若该绑定或 Native budget authority 不可用，结果必须是 `budget_unavailable`/等价抑制，不得忽略预算继续生成。该接线完成前只能宣称“UI 文案已改”，不能宣称预算配置已生效。

## 10. 失败与回退

| 失败 | 有效行为 | 持久化行为 |
| --- | --- | --- |
| 未知 frequency mode | 本次主动联系关闭 | 不改原值，记录警告 |
| custom 数字类型、范围或乘法溢出无效 | 本次主动联系关闭 | 不钳制、不回写 |
| revision `0`，Host 提供有效 `fresh-current-schema` 证据，且当前 schema 默认表面完整合法 | 采用 auto；当次 effective pair 仍须由 6.2 的冻结证据决定 | 成功后 revision=`1`；旧兼容键保持 `2/360`，不反向改写 |
| revision `0`，Host 未提供可验证新装来源，legacy 为 `2/360` | 保持 restrained 等价行为，并提示可手动选择 auto | 成功后 revision=`1`；旧值不改 |
| revision `0`，Host 未提供可验证新装来源，legacy 为其他合法 pair | 保持 custom 等价行为 | 成功后 revision=`1`；旧值不改 |
| revision `0`，legacy pair 无效，或 `fresh-current-schema` 证据与当前 schema 默认表面矛盾 | 本次主动联系关闭 | revision 保持 `0`，不猜测来源、不修补原值 |
| revision 为负数或非整数 | 本次主动联系关闭 | 不迁移、不改写 |
| 新分组缺失/损坏且 revision `1` | 本次主动联系关闭 | 不用隐藏旧值放宽策略 |
| revision `>1` | 当前实现不认识未来合同，主动联系关闭 | 不降级、不改写 |
| auto 的冻结 sleep/contact/budget/安全门证据缺失或摘要无效 | 本次主动联系关闭，不使用默认档位 | 不改配置，记录无敏感信息的 reason code |
| auto 计算结果不是 `2/360`、`3/240`、`4/180` 之一 | 本次主动联系关闭，视为策略实现错误 | 不改配置，记录错误 |
| auto 遇到 sleep 或其他现有安全门抑制 | 沿用该门的抑制结果，不形成 cause、不调用 Provider | 不改配置 |
| quiet-hours 新值无效 | 本次主动联系关闭，而不是跳过免打扰 | 不改原值 |
| 迁移保存接口失败 | existing/unknown 使用合法 legacy 等价值；Host-proven fresh 和来源矛盾配置关闭 | 回滚持久化变更，revision 保持 `0`，下次重试 |
| Token 预算无效或无法绑定 Native | 抑制主动表达和 Provider 调用 | 不改原值 |
| AstrBot Dashboard 不支持条件显示 | 自定义数字静态显示，仅 custom 模式读取 | 无额外回退页面 |

所有回退都必须满足一个原则：配置不确定不能带来更多消息、更短冷却或绕过预算。

## 11. 文档与发布表面

实现时同步更新：

- `_conf_schema.json`：新分组、显示名、hint、静态隐藏兼容键；
- `README.md`：用四个用户概念替换五个内部数字的基础说明，保留 raw JSON 兼容键说明；
- `CHANGELOG.md` 的 Unreleased：只记录设置表面与兼容解析变化；
- 配置示例：默认使用 `proactive_frequency` 和 `user_quiet_hours`，并说明历史存储名；
- 若打包器或 release contract 校验 schema 键，则同步最小契约断言。

不能把这一文档提交或后续 schema 变更写成 Alpha 3 发布包已经重建、安装通过或真实主动发送已经验收。

## 12. 最少验收

实现阶段只要求能证明本设计关键语义的最小验收，不扩张为大规模测试工程。

### 12.1 静态与编译

1. `_conf_schema.json` 可被标准 JSON 解析；四个顶层主动联系概念存在，旧兼容键仍存在且为静态隐藏。
2. Python 受影响模块可编译；若未改 Rust 类型，不要求无关 Cargo 全量回归。
3. README、schema 和运行时预设表中的 auto 包络 `2/360–4/180`、平衡档 `3/240` 以及两个公开预设一致。

### 12.2 一个表驱动解析测试

用一个表驱动测试覆盖以下最小矩阵：

| 输入 | 预期 |
| --- | --- |
| revision `0`，Host 提供绑定本次加载、插件 ID、配置实体与 schema digest 的首次创建证据，当前 schema 默认表面合法 | 迁移为 auto，自定义预填 `2/360` |
| revision `0`，无来源证明，legacy `2/360` | 迁移为克制，有效值仍为 `2/360`，旧键不变，并产生兼容提示 |
| revision `0`，伪造、过期或 digest 不匹配的来源证据，legacy `2/360` | 忽略无效证据，按来源未知迁移为克制，不启用 auto |
| revision `0`，无来源证明，legacy `7/90` | 迁移为自定义，有效值精确为 `7/90`，旧键不变 |
| revision `0`，legacy `-1/360` | legacy pair 无效，effective enabled 为 `false`，revision 不推进；`-1` 不具有哨兵语义 |
| revision `1`，auto，Persona asleep | 由 sleep gate 抑制，不生成档位、不调用 Provider |
| revision `1`，auto，active+已回复最近 outbound+token ceiling `4` | 适中 `4/180` |
| revision `1`，auto，active+无未回复+token ceiling `3` | 平衡 `3/240` |
| revision `1`，auto，`consecutive_unanswered=1` | 克制 `2/360`，并叠加 6 小时基数退避 |
| revision `1`，auto，达到 hard stop | 直接抑制；时间经过不会自动恢复 |
| revision `1`，auto，冻结证据缺失 | effective enabled 为 `false`，不猜测为克制 |
| revision `1`，moderate，隐藏 legacy `7/90` | 新档位权威，有效值 `4/180` |
| revision `1`，custom `5/120` | 有效值精确为 `5/120` |
| revision `1`，custom 无效 | effective enabled 为 `false`，不回退到隐藏 legacy |
| revision `2` | 当前实现不认识未来合同，effective enabled 为 `false`，不降级解析 |
| 迁移保存失败 | revision 仍为 `0`；existing/unknown 使用合法 legacy 等价值，Host-proven fresh 关闭；内存和持久配置无半迁移 |
| 历史 Token 预算 `2048` | 显示语义为主动表达预算，Native policy 接收 `2048` |

### 12.3 一次手工 AstrBot 配置页检查

在受支持的 AstrBot 实例中检查：

1. 主动联系相关顶层只出现四个概念；
2. 频率组显示自动/克制/适中/自定义，schema 首次创建默认自动；自定义数字始终显示，并明确标注仅 custom 生效；
3. 保存并重载后档位和自定义值保持；
4. 升级一份 legacy `7/90` 配置后显示为自定义且数值不变；
5. 升级一份 legacy `2/360` 配置后显示为克制；Host 能证明当前 schema 首次创建时显示为自动，不能证明时显示为克制并提示可手动选择自动；
6. 选择自动或克制不会把隐藏 legacy `7/90` 回写成 auto 包络或 `2/360`；
7. auto 在未回复后只会收紧，并且任何观测状态都不能执行每日上限大于 `4` 或最短间隔小于 `180` 分钟的有效 pair；
8. 开启全局开关仍不能绕过 relation consent、cause、睡眠、免打扰、预算、Provider 或宿主能力门禁。

这一手工检查只验收配置体验与解析兼容，不构成真实平台 proactive delivery PASS。

## 13. 完成标准

后续实现只有同时满足以下条件，才可称“主动联系设置简化完成”：

- 四个基础概念按本文显示；
- auto、preset 和 custom 到旧运行时字段的映射唯一且与文档一致；
- Host 可证明的当前 schema 首次创建实例默认 auto；缺少新装来源证明时安全保持 restrained，legacy `2/360` 等价升级为 restrained，其他合法 legacy pair 等价升级为 custom，旧值未被静默覆盖；
- auto 的每次决定绑定冻结证据，始终落在 `2/360–4/180` 包络内，未回复只能收紧直至 hard stop；
- schema 能力限制被如实呈现，没有伪造条件显示；
- Token 预算不只是改名，而是能被运行时读取并绑定，或在不可用时 fail closed；
- 无效配置、保存失败和缺失 budget authority 都不会增加打扰；
- 最少静态/编译、表驱动解析测试和一次配置页检查完成；
- 发布结论仍与实际构建、安装和主动发送证据严格分开。
