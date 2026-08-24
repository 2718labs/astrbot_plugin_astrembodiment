# AstrEmbodiment

让你的 Bot 不只记住经历，更能延续「Ta是谁」。

AstrEmbodiment 是 AstrBot 的 Rust 原生人格连续性运行时。它不保存长期聊天文本，而是把可验证的互动证据交给原生核心，在后续对话中以受限、可审计的方式体现由经历形成的倾向。

> **用户话语 → 15 维闭合语义证据 → 原生状态原子提交 → 受限表达投影**

本仓库的生产版本合同是 `1.0.0`。这不等于已经在 GitHub Release、AstrBot Marketplace 或任意主机上完成发布；实际发包仍必须通过仓库的 B3 FINAL 隐私验收、双平台新鲜构建和独立制品复核。

<p align="center">
  <img src="logo.png" alt="AstrEmbodiment" width="260" />
</p>

<p align="center">
  <img src="https://img.shields.io/badge/版本-1.0.0-0f766e?style=flat-square" alt="版本 1.0.0">
  <img src="https://img.shields.io/badge/AstrBot-%3E%3D4.16%2C%3C5-f08c46?style=flat-square" alt="AstrBot >=4.16,<5">
  <img src="https://img.shields.io/badge/平台-Windows%20x64%20%7C%20Linux%20x86__64-475569?style=flat-square" alt="Windows x64 and Linux x86_64">
  <img src="https://img.shields.io/badge/许可证-AGPL--3.0--or--later-5b403a?style=flat-square" alt="AGPL-3.0-or-later">
</p>

## 它解决什么

普通“情绪插件”往往是“文本分类 → 标签 → prompt 语气”。AstrEmbodiment 把状态改变限制在一个可复核的原生闭环中：

1. 宿主把当前回合封装为闭合输入；原始用户文本不会写入原生状态库。
2. 语义层只给出固定 15 个维度的已校验证据，不能携带任意自由文本来修改状态。
3. Rust 核心验证因果基线和容量后原子提交状态，并返回当前 revision 的受限表达投影。
4. AstrBot 仅把该投影作为受限上下文；真实消息投递后才会结算相应行动事实。

这让角色可以更稳定地表现出谨慎、边界、修复取向或表达收束，同时保持事实、安全策略、工具策略和 AstrBot 宿主规则的优先级。

## 已承诺的运行边界

- 15 个语义维度均通过固定路由进入原生计算；Python 不在旁路猜测或改写提交结果。
- 生产状态由 Rust 单写者持久化。插件重载或插件升级后继续从持久化原生状态恢复，而不是维护一份会漂移的 Python 镜像。
- `Windows x64 与 Linux x86_64` 的 CPython 3.12 abi3 原生扩展是正式制品的目标支持范围；一个 ZIP 只可包含经过 allowlist 检查的运行时文件和相应 SHA-256。
- Observatory 只读且不记录用户正文、Provider 输出、token、SeedCode、状态 digest 或原始神经节点。

## Observatory：简洁模式与调试模式

默认的**简洁模式**只报告安全、可操作的结果：运行状态、是否提交、revision 是否前进以及稳定错误码。它适合日常运维。

显式启用的**调试模式**用于维护者排查：会提供无内容的计算状态、被拒绝原因类别和观测字段。两种模式都不暴露用户原文、模型输出、身份 token、SeedCode 或原生内部图数据；调试模式也没有写入权限。

## 共享 API 头

宿主与原生层共享固定的逻辑请求头。它绑定协议、作用域、因果基线和本轮来源，但不把用户正文或任意字段塞进状态通道：

```json
{
  "schema": "astr-embodiment.request-header.v1",
  "scope": "opaque scope token",
  "causal_base_revision": 42,
  "event_id": "opaque event token",
  "source": "astrbot"
}
```

请求载荷只承载闭合契约允许的证据；响应也只返回白名单中的状态、错误码与表达投影。任何字段缺失、过期或未经验证时，原生层拒绝提交，宿主不得用 Python 旁路补写。

## 安装与升级

只有带有 release receipt、allowlisted ZIP 和 SHA-256 sidecar 的正式制品才可安装。不要把本地测试 ZIP、历史候选包或单个平台 wheel 当成正式版。

升级前请保留 AstrBot 分配的插件数据目录。兼容升级应由原生存储回放和 revision 验证决定：成功时继续既有身份状态；失败时明确报错，不静默重生、清空或改写旧状态。

常用命令（以 AstrBot 实际命令前缀为准）：

```text
<命令前缀>ae
<命令前缀>ae_seed
```

- `ae`：查看无内容运行状态与 Observatory 摘要。
- `ae_seed`：在 Genesis 成功后显示已持久化的 SeedCode；SeedCode 不是 API 密钥、密码或聊天记录备份。

## 限制与不做的事

- 它不保存长期聊天文本、摘要、embedding 或用户事实画像，也不是 RAG、知识库或聊天记录备份。
- 它不替代 AstrBot 的主对话模型、平台适配器、权限系统、工具策略或安全策略。
- 它不承诺 macOS、ARM、musl-only Linux、Python <3.12 或其他 AstrBot 适配器。
- 它不等同于意识、主观感受或真实关系；不应被用于医学、心理诊断或人格治疗声明。
- 它不提供长期记忆、主动消息、TTS、独立 WebUI、社交媒体发布或对 Sylanne 状态的自动迁移。

## 发布流程

CI 在 Windows 与 Linux 上执行格式、lint、测试和原生打包矩阵。`release.yml` 仅能由维护者手动 dispatch，且只接受 `production` 通道；推送标签不会触发发布。该流程只产出待复核的 allowlisted ZIP 与 SHA-256，远端 tag、Release、Marketplace 上架和包上传均需要另行的显式维护者动作。

更多历史记录见 [CHANGELOG.md](CHANGELOG.md)，产品文案边界见 [docs/product/PRODUCT_COPY.md](docs/product/PRODUCT_COPY.md)。

## 许可证

AstrEmbodiment 使用 [GNU AGPL-3.0-or-later](LICENSE) 发布。
