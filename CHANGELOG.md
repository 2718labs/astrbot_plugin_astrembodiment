# 更新记录

所有重要更改都会记录在此文件中。格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，版本遵循语义化版本。

## [Unreleased]

## [1.0.0-rc2] - 2026-08-23

### 新增

- 15 个 SPC1 维度现在都映射到固定的原生 attention load 与神经场区域组合，不再只有部分维度改变提交后的状态。
- 原生提交回执新增闭合的六项 fxp6 表达投影：warmth、sensitivity、guardedness、repair_orientation、engagement 与 epistemic_caution。
- 提交确认后，同一轮 ProviderRequest 会附加受限表达上下文；它只影响风格倾向，不携带用户正文、模型输出、digest、节点或控制字段。
- Observatory 升级到 semantic-injection.v2，持续回显 15 维计算与 native calculation，并额外记录 expression_state 和可信 expression_profile_fxp6。缺失、拒绝与宿主注入失败都会显式可见。

### 工程与发布

- 新增版本契约脚本，校验 AstrBot metadata、Python PEP 440、Rust workspace 和 Cargo.lock 的版本一致性。
- CI 新增 Python 格式/错误检查、Python 回归、Rust fmt/Clippy/回归、版本契约和 Windows/Linux 原生 wheel 冒烟。
- 新增标签驱动的 GitHub Release 工作流：仅在已存在的 `v*` 标签上重新构建 Windows/Linux wheel、组装插件包并创建或更新同名 prerelease/release。
- 新增 Dependabot 对 GitHub Actions、Cargo 和 Python 依赖的周更检查；工作流 action 使用完整 commit SHA 固定版本。

### 兼容性与边界

- 保持既有 native formula digest、状态协议与 revision 连续性不变；RC1 已存在的数据不需要重放或迁移即可继续使用。
- 表达投影不等同于意识、主观感受、真实关系或独立意图；事实、安全、同意、工具调用与平台策略仍独立于这些值。
- 此提交只形成 RC2 待发布候选：没有在本机创建 Git tag、GitHub Release 或 AstrBot Marketplace 上架。

## [v1.0.0-alpha.1] - 2026-08-20

### 保全

- 建立公开源码保全快照与对应 Git 标签；不创建 GitHub Release，不提交 AstrBot Marketplace。
- 明确排除本地虚拟环境、构建输出、运行配置、生成清单与未引用的重复 crate 树，避免把本机数据或构建残留误当发布内容。

### 已知未实现

- 用户话语到闭合语义证据、原生状态转移、受控回应策略/投影的完整链路尚未实现。
- 现有 Genesis、SeedCode、Persona、`apply_event` 与响应钩子不能作为“Bot 已具备情绪反应”或生产可用的证据。
- 本标签仅用于可恢复的源码基线；不得安装、上架、发布或替换现有 AstrBot 插件数据。

## [1.0.0-rc1] - 2026-08-22

### 定版

- 将候选发布版本定为 `1.0.0-rc1`。
- README 改为中文优先并重排为“功能-模块-工作流”结构，补充无 WebUI 使用方式、配置文件路径、模块分层，以及与 `astrbot_plugin_sylanne` 的替代关系说明。
- README 增加总工作流图、每个核心模块的独立工作流图，以及面向后续适配器的双向逻辑 API 头和闭合 JSON 契约说明。
- `ae`、`ae_seed`、`on_llm_request`、`on_llm_response` 和 `after_message_sent` 的 AstrBot 展示描述改为中文。

### 发布契约

- 统一发布版本：AstrBot metadata 与 Rust workspace 使用 `1.0.0-rc1`，Python wheel 使用 PEP 440 的 `1.0.0rc1`。
- 原生 loader 公开 `semantic_revision_v1` 与 `apply_perception_proposal_v1`；打包门禁拒绝缺少任一导出的旧 wheel。
- 远端 GitHub Release 与 AstrBot Marketplace 上架仍由维护者单独执行，本地候选不自动发布。

### 可观测性

- `observatory_enabled=true` 时，成功的语义注入以 INFO 记录完整 15 维 fxp6 证据、confidence、revision 与去重结果；`ZERO_LOAD` 明确标记为未提交。
- NOOP 以 INFO、DEGRADED 以 WARNING 记录，并区分 `stage`、`code` 与 `commit_state`；观测日志不包含用户正文、Provider 输出、token、nonce 或 digest。
- 成功回执额外回显 native 的 `state_changed`、活跃节点/边数和五类 fxp6 残差；失败固定标明计算是未尝试还是未确认。

### 修复

- `ae_seed` 现在明确支持无 WebUI 直接生成 SeedCode，并通过 AstrBot 配置保存接口持久化；插件重载后可再次用命令查看。
- 明确 AstrEmbodiment 是 Sylanne 方向的重制版，不包含 Sylanne 的长期记忆、关系状态、即时聊天、主动消息、TTS 或独立 WebUI 等扩展功能。
- 明确两者共享 LLM/投递钩子，不能在同一 AstrBot 会话中同时启用；迁移不会自动转换 Sylanne 的历史状态。
- 修复原生交付提交后的 revision 未回写到 Python 镜像，导致下一轮请求报 `STALE_CAUSAL_BASE`。
- 修复插件热重载后未从持久化原生状态恢复 revision 和 turn 序号，避免复用旧事件标识或提交过期因果基线。
- 补全 SPC1 辅助模型的精确 15 维 JSON 模板、fxp6 取值范围和维度语义，避免真实 Provider 因无法推断私有闭合协议而返回 `ESTIMATOR_MALFORMED`。

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
