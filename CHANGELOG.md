# 更新记录

所有面向用户的重要变化都会记录在此文件中。格式参考
[Keep a Changelog](https://keepachangelog.com/zh-CN/1.0.0/)，版本遵循语义化版本。

## [Unreleased]

暂无已发布的未归档变更。

## [1.0.0] - 2026-08-24

AstrEmbodiment 首个正式版本。

### 正式发布流程

- 统一 marketplace metadata、Python project、Rust workspace 和标签目标为 `1.0.0`。
- 合并到 `master` 后，维护者可手动 dispatch 正式发布工作流；它会核对当前 `master`、目标 SHA、版本、标签和更新记录是否一致。
- 工作流会重新构建 Windows x64 / Linux x86_64 原生轮子，复现并校验确定性的 allowlist ZIP 与 SHA-256 sidecar，再创建或继续同一目标的草稿 GitHub Release。
- 仅发布步骤拥有仓库写权限：它创建带注释标签、核验远端资产的哈希与大小，发布后要求 GitHub 将 Release 标记为不可变；工作流不直接上传 AstrBot Marketplace。

### 新增

- 提供“用户话语 → 15 维闭合语义证据 → 原生状态原子提交 → 受限表达投影”的完整运行链路。
- 由辅助模型归纳当轮互动信号，由 Rust 原生核心校验并提交状态；主模型仍负责生成最终回复。
- 支持 Genesis 与 SeedCode，持久化同一 Persona 的人格连续性；插件重载和升级后可从原有原生状态恢复。
- 提供简洁 Observatory 与调试 Observatory，展示状态进展、修订信息和聚合诊断，不输出用户正文或 Provider 原始响应。
- 辅助模型响应较慢时支持异步处理，主对话可以继续；只有完成验证的语义结果才会影响后续表达。
- 提供跨平台原生插件包，支持 Windows x64 与 Linux x86_64。

### 改进与修复

- 配置页支持辅助模型 Provider、运行资源档位、诊断模式和原生数据目录配置。
- 修复原生扩展在插件重载、升级或重新安装后可能加载旧版本的问题，并改进版本与 ABI 检查。
- 修复原生状态提交后的 revision 同步、重复事件处理和持久化恢复路径。
- 完善异常和降级状态的可观测信息，使 Provider 超时、语义结果不可用和原生运行时故障能够分别定位。

### 兼容性

- AstrBot `>=4.16,<5`。
- 当前支持 `aiocqhttp` 适配器。
