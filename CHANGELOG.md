# 更新记录

所有重要更改都会记录在此文件中。格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，版本遵循语义化版本。

## [Unreleased]

## [1.0.0] - 2026-08-24

### 正式版合同

- 统一 marketplace metadata、Python project、Rust workspace 和标签目标为 `1.0.0`。
- 发布候选仅能由维护者手动 dispatch 的 `production` 工作流生成；推送 tag 不会发布、不会上传、不会创建 GitHub Release。
- release workflow 只输出经 allowlist 检查的 Windows x64 / Linux x86_64 原生插件 ZIP 及 SHA-256 sidecar，供独立复核。

### 产品边界

- 文档将正式能力表述为“用户话语 → 15 维闭合语义证据 → 原生状态原子提交 → 受限表达投影”。
- 明确升级/重载由持久化原生状态恢复，并说明简洁与调试 Observatory 都不记录内容或获得状态写入权。
- 正式制品仍受 B3 FINAL privacy acceptance、current Windows/Linux 新鲜构建和独立制品验收约束；本条目不是远端发布、上架或人审通过的声明。

## [v1.0.0-alpha.1] - 2026-08-20

### 保全

- 建立公开源码保全快照与对应 Git 标签；不创建 GitHub Release，不提交 AstrBot Marketplace。
- 明确排除本地虚拟环境、构建输出、运行配置、生成清单与未引用的重复 crate 树，避免把本机数据或构建残留误当发布内容。

### 已知未实现

- 用户话语到闭合语义证据、原生状态转移、受控回应策略/投影的完整链路尚未实现。
- 现有 Genesis、SeedCode、Persona、`apply_event` 与响应钩子不能作为“Bot 已具备情绪反应”或生产可用的证据。
- 本标签仅用于可恢复的源码基线；不得安装、上架、发布或替换现有 AstrBot 插件数据。

## [1.0.0-rc1] - 2026-08-20

### 候选记录

- 记录候选期的 README、SeedCode 持久化、跨平台归档和原生 revision 恢复工作；这些记录不替代当前 `1.0.0` 的独立发布门禁。

## [1.0.4] - 2026-08-20

### 历史开发记录

- 修复原生交付提交后的 revision 同步与热重载恢复，并补充相应回归测试。

## [1.0.3] - 2026-08-19

### 历史开发记录

- 记录配置页、辅助 Provider、SeedCode、内容寻址原生加载器和跨平台打包路径的开发演进。

> `1.0.1` 与 `1.0.2` 是错误写入的未发布变更记录，已移除；其内容不构成发布历史。
