# 更新记录

所有重要更改都会记录在此文件中。格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，版本遵循语义化版本。

## [Unreleased]

## [1.1.0] - 2026-09-21

- 将当前 Rust、Python/wheel 与 AstrBot 插件元数据版本统一为 `1.1.0`，保留现有无头人格情绪、语义评价、确定性身体时钟核心及 19 项 Native API。
- 保留 alpha5 引入的安全旧 schema 兼容：只精确识别已知历史结构，并仅在相应升级证据满足保全门禁时事务化兼容；未知、非空或证据不足的结构继续保全原库并拒绝继续。
- 版本去除 alpha 标记仅表示源码身份定版，不代表 AstrBot 实机安装、对话、重启或生产发布验收，也不表示已经重建或验证 `1.1.0` wheel / universal ZIP。`release_verified` 仍为 `false`，既有 alpha5 制品与历史记录保持不变。

## [1.1.0-alpha5] - Source candidate, 2026-09-16

- 候选版本统一为 Rust `1.1.0-alpha5` / Python `1.1.0a5`，保留 19 项 Native API、API/v9 schema 与 IANA 2026c 摘要绑定、同源编译身份和双平台真实导入门禁。
- 修复 alpha4 安装精确历史旧库时缺少 `backup_digest` 的兼容问题：仅识别历史提交 `1774023` / `ee10648` 中的已知旧表结构，并仅在对应 upgrade、备份与 authority 表均为空且不存在 `AE-LSU1` 升级历史时，于事务内兼容到当前结构。普通业务历史完整保留，不要求整个数据库为空；upgrade 表非空、证据不足或结构未知时保全原库并拒绝，不删除、清空或重建数据库。
- 源码状态不代表产物或宿主验收；每个安装包的同源构建、真实导入和数据库兼容结果以对应外部验证回执为准。实际 AstrBot 安装、对话、重启和生产发布仍需单独验收，生产发布授权门禁仍为 `NO_GO`；既有 alpha4 交付记录与产物保持不变。

## [1.1.0-alpha4] - Source candidate, 2026-09-08

- 收敛为人格情绪、语义评价与确定性身体时钟核心；移除主动生成、接触意图、目标/密钥读取与发送执行表面，普通对话的投递结算保留。
- Native 与 Python 包装器固定为 19 个类型化方法；历史 kind-8 DTO/codec 仅用于只读回放和迁移验证，不提供提交路径。
- 版本统一为 Rust `1.1.0-alpha4` / Python `1.1.0a4`；原 Alpha 3 发布记录保留。
- wheel 编译身份绑定干净源码 SHA、API/v9 schema 摘要和 IANA 2026c 内容摘要；打包必须提供匹配 Windows/Linux 实际导入凭据。
- 当前仅源码候选。双平台最终构建、43 项 artifact parity 和最终独立审查未完成，发布门禁为 `NO_GO`。

## [1.1.0-alpha3] - 2026-08-30

### 定位与边界

- 将发布定位收敛为 AstrBot 进程内的无头身体与安全执行器：保留 Genesis、SeedCode、固定神经场、睡眠/唤醒、Persona/关系时区和身体状态，但移除 Pages、Observatory 以及 `mood_card`、`query_inner_events`、`observe_snapshot_v1`、`observe_events_v1` 等旧 live mind 导出。
- 新增四项 content-free 身体观察入口：`observe_body_snapshot_v1`、`observe_execution_receipt_v1`、`observe_budget_summary_v1`、`observe_gate_reasons_v1`。投影绑定 scope、authority、public ref 与 lease，不返回消息正文。
- `integration_availability_v1` 明确返回 `UNAVAILABLE_HOST_ATTESTATION`；当前没有 authenticated ecosystem service、conversation-context/dream 操作、仓库导入器、迁移回执或 Web UI。

### 主动执行与安全

- 主动消息继续显式 opt-in 且默认关闭。睡眠、安静时段、冷却、关系授权、预算、宿主能力或 Provider usage 任一不可证明时 fail closed；当前 headless 控制仅允许 `pause/end`，拒绝 `grant/resume`。
- 保留 Provider usage 的 claim/settlement、持久化 outbox、执行回执、崩溃恢复以及 DPAPI/AES-GCM 加密的目标和候选；未知 usage 按完整预留量结算。
- 修复主动候选在 Provider 已产生 usage 后加密失败却未终结 claim 的问题；现在以 `rejected_terminal` 结算已观测 usage，并在恢复密文后重新执行封闭候选检查，不合规正文不会送往适配器。
- 修复清理 settlement 覆盖原始故障或触发重复 dispatch settlement 的问题；加密边界保留原始异常，dispatch settlement 故障只暴露一次且不会被第二次结算改写。

### 数据保全

- 继续物理保留既有 v7 schema 文件、migration digest 和 retired world/lived-day/dream/ecosystem 表及原始行，但这些域不再拥有活动 authority；正常启动不 drop、copy、rewrite retired rows，也不创建 `body_*` 影子表。
- 对已停止且确认无人持有的数据库，SQLite `.backup` 仅可生成离线归档副本；它不是 live shared store、导入源、迁移路径或 migration receipt。

### 打包与验证边界

- Rust/native/manifest 版本同步为 `1.1.0-alpha3`，Python/wheel 版本同步为 `1.1.0a3`；打包器要求当前 module-level API、四项 body projection 和 `integration_availability_v1`，并拒绝用旧 mood/inner/snapshot/events API 充当发布表面。
- 最终包内容与导入审计必须直接使用 `AE_RELEASE_ARCHIVE` 指向的精确 ZIP；fixture rebuild 只验证打包器，不可替代最终归档验收。fresh 双平台 wheel、最终 ZIP、安装烟测、真实主动发送和 Marketplace/生产授权均不包含在本提交中。

## [1.1.0-alpha2] - 2026-08-29

### 新增

- 加入 AstrBot Pages 只读心智观测台，展示已提交的运行快照、事件时间线、睡眠、情感、意图、因果与资源视图。
- 原生扩展增加 `observe_snapshot_v1` 与 `observe_events_v1` 只读观测入口；页面不展示消息正文或候选回复正文。

### 修复

- 普通对话投递现在可跨自主提交正确结算，避免自主 revision 推进后把真实发送结果误判为过期。

### 打包

- 发布归档现在明确包含 `pages/**` 与 `.astrbot-plugin/i18n/**`，并继续要求 fresh Windows x64 与 Linux x86_64 两个平台 wheel。
- 打包器继续使用内容寻址原生清单，校验 alpha2 wheel 文件名、METADATA、平台 tag 与原生 runtime 版本，拒绝不安全或重复归档路径，并强制包体严格小于 16 MiB。
- 先前包体增量偏小的直接原因是 Pages 与 i18n 尚未列入 `INCLUDE`；其余新功能大多是可高效压缩的源码，因此 ZIP 体积不能用来判断功能是否存在。

### 验证边界

- 本提交只准备 alpha2 版本与打包契约；fresh 双平台原生 wheel、最终 ZIP 和安装烟测仍须从当前提交另行生成并验证。

## [1.1.0-alpha1] - 2026-08-28

### 新增

- 加入以内生驱动为主的自主运行时、持续内心活动、意图形成和可恢复主动消息队列。
- 加入自适应睡眠/唤醒、Persona 独立时区、关系时区以及受授权的紧急唤醒。
- 加入确定性的 `16384 → 2048 → 256 → 32` 多尺度神经工作区。
- 加入 `/ae_mood`、`/ae_mind` 与 `/ae_wake` 展示和控制入口。

### 安全与资源

- 主动聊天默认关闭，默认使用经济模式；外显前必须经过预算、冷却、睡眠和关系授权门禁。
- 持久化 canonical journal、operational checkpoint、authority ledger 与幂等 outbox，支持崩溃恢复。
- Windows 目标绑定使用 DPAPI 与 AES-GCM 保护，缺失保护能力时拒绝主动投递。

### 修复

- 原生自主运行 API 现在完整导出到 Python 包装层，并由打包器验证 17 个自主 API 标记。
- 修复脱离 AstrBot 安装环境运行静态/适配器测试时的 `MessageChain` 导入问题。

### 验证边界

- Windows x64 已完成原生导入、宿主初始化、经济模式零 LLM/零发送与关闭烟测；Linux x86_64 已从同一提交交叉构建 manylinux_2_17 原生 wheel，并完成静态 ELF、版本与 API 表面核验。
- 本 Alpha 尚未执行真实平台主动发送验收，不代表 Marketplace 或生产发布授权。

## [v1.0.0-alpha.1] - 2026-08-20

### 保全

- 建立公开源码保全快照与对应 Git 标签；不创建 GitHub Release，不提交 AstrBot Marketplace。
- 明确排除本地虚拟环境、构建输出、运行配置、生成清单与未引用的重复 crate 树，避免把本机数据或构建残留误当发布内容。

### 已知未实现

- 用户话语到闭合语义证据、原生状态转移、受控回应策略/投影的完整链路尚未实现。
- 现有 Genesis、SeedCode、Persona、`apply_event` 与响应钩子不能作为“Bot 已具备情绪反应”或生产可用的证据。
- 本标签仅用于可恢复的源码基线；不得安装、上架、发布或替换现有 AstrBot 插件数据。

## [1.0.0-rc1] - 2026-08-20

### 定版

- 将候选发布版本定为 `1.0.0-rc1`。
- README 改为中文优先并重排为“功能-模块-工作流”结构，补充无 WebUI 使用方式、配置文件路径、模块分层，以及与 `astrbot_plugin_sylanne` 的替代关系说明。
- README 增加总工作流图、每个核心模块的独立工作流图，以及面向后续适配器的双向逻辑 API 头和闭合 JSON 契约说明。
- `ae`、`ae_seed`、`on_llm_request`、`on_llm_response` 和 `after_message_sent` 的 AstrBot 展示描述改为中文。

### 修复

- `ae_seed` 现在明确支持无 WebUI 直接生成 SeedCode，并通过 AstrBot 配置保存接口持久化；插件重载后可再次用命令查看。
- 明确 AstrEmbodiment 是 Sylanne 方向的重制版，不包含 Sylanne 的长期记忆、关系状态、即时聊天、主动消息、TTS 或独立 WebUI 等扩展功能。
- 明确两者共享 LLM/投递钩子，不能在同一 AstrBot 会话中同时启用；迁移不会自动转换 Sylanne 的历史状态。
- 修复原生交付提交后的 revision 未回写到 Python 镜像，导致下一轮请求报 `STALE_CAUSAL_BASE`。
- 修复插件热重载后未从持久化原生状态恢复 revision 和 turn 序号，避免复用旧事件标识或提交过期因果基线。

### 发布

- 发布包继续携带 fresh Windows x64 与 Linux x86_64 原生扩展，归档不包含 wheel、测试目录或构建缓存。
- 仓库自动化按 2718lab GitHub Repository Template 约定组织；外部机器人只声明权限和回退策略，不会未经管理员安装或自动合并代码。

## [1.0.4] - 2026-08-20

### 修复

- 修复原生交付提交后的 revision 未回写到 Python 镜像，导致下一轮请求报 `STALE_CAUSAL_BASE`。
- 修复插件热重载后未从持久化原生状态恢复 revision 和 turn 序号，避免复用旧事件标识或提交过期因果基线。
- 增加交付 revision 同步和热重载恢复回归测试。

### 验证

- 插件运行时测试、发布契约、静态校验和 fresh Windows/Linux 原生归档冒烟测试通过。

## [1.0.3] - 2026-08-19

### 新增

- 配置页字段和说明改为中文，新增可选辅助 Provider 配置。
- 辅助 Provider 配置非空时固定使用该 Provider；留空时自动使用当前会话的主对话模型。
- 新增 SeedCode 生成、配置页回显和配置持久化，插件重载后可继续查看已生成的值。
- 在 `on_llm_request` 中注入当前身份上下文，使 SeedCode 参与每次 LLM 请求。
- 新增 `ae_seed` 命令，用于生成或重新生成 SeedCode；保留 `ae` 原生状态命令。

### 兼容性

- 发布包继续同时提供 Windows x64 与 Linux x86_64（glibc）CPython abi3 原生扩展。
- 兼容范围保持为 AstrBot `>=4.16,<5`、Python 3.12+；当前声明的适配器仍为 `aiocqhttp`。
- 不新增对 macOS、ARM、musl-only Linux、Python <3.12 或其他适配器的承诺。

### 说明

- SeedCode 是身份指纹，不是 API 密钥、密码或聊天记录备份。
- 辅助 Provider 不会替换会话主模型；其配置无效时会报告错误，不会静默回退。
- 发布归档现在直接携带并加载平台扩展，避免 AstrBot 把随包 wheel 当作在线依赖安装。
- 原生扩展改为内容寻址的 `_bundled/<sha256>/` 路径，避免 AstrBot 热重载时复用
  CPython 同路径扩展缓存。

### 修复

- 修复会话配置被错误地作为异步对象等待，导致人格解析和 Genesis 生成跳过的问题。
- Genesis 无法取得完整原生回执时停止当前请求并报告错误，避免在未生成 SeedCode 的情况下继续调用对话模型。

## [1.0.2] - 2026-08-19

### 修复

- 修复 AstrBot 已创建插件数据目录时，SQLite 将目录误当数据库文件而导致插件加载失败的问题。
- 原生存储现在在插件数据目录内使用固定的 `astrembodiment.sqlite3` 文件，目录语义与 AstrBot `get_data_dir()` 保持一致。

## [1.0.1] - 2026-08-19

### 新增

- Windows x64 MVP 预览版元数据，兼容 AstrBot `>=4.16,<5`。
- 保守的 `aiocqhttp` 适配器声明和原生扩展构建路径。
- `ae` 原生核心状态命令及插件配置项说明。
- AGPL-3.0-or-later 许可证、无密钥 CI 工作流和发布产物忽略规则。

### 变更

- 发布包携带 Windows x64 与 Linux x86_64 原生扩展；`requirements.txt` 不声明运行时
  pip 依赖，避免安装阶段访问 public PyPI。
- 发布脚本从多个平台 wheel 提取扩展和当前源码初始化器，在一个归档中保留各平台加载器。

### 限制

- 这是 Windows x64 / Linux x86_64 MVP 预览版，不是完整的端到端 Agent 发布版。
- 从源码重建发布包仍需要维护者准备匹配目标平台的原生 wheel。
