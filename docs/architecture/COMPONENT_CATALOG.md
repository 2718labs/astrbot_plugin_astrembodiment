# 架构组件目录

本文件给出全部核心架构的职责、输入、输出、状态写权限和 MVP 退出条件。

| 编号 | 架构 | 核心职责 | 是否可写权威状态 |
|---|---|---|---:|
| A01 | AstrBot Host & FFI | 宿主生命周期、粗粒度 Python/Rust 边界 | 否 |
| A02 | Authority & Causality | 来源权限、scope、turn、causal binding | 否，生成凭据 |
| A03 | Micro-Attention | 稀疏路由和广义荷载装配 | 否 |
| A04 | Neurocontinuum | 16K 节点、E/I、规范算子、动态图传播 | 否，生成 trial |
| A05 | Allostasis & Glia | 身体预测、增益、稳态、修复/修剪调节 | 否，生成调制场 |
| A06 | Constitutive Plasticity | 可逆—不可逆分解、资格迹、残余和连接候选 | 否，生成 candidate |
| A07 | Renormalization | 16K→2K→256→32 多尺度收敛与下传 | 否 |
| A08 | Agent Cognition | Self/World Model、反事实 rollout、行动竞争 | 否，生成合同 |
| A09 | Continuum Persistence | Journal、Snapshot、Delta、CAS、replay | 是，唯一 commit lane |
| A10 | Observatory & Safety | 内容无关观测、不变量、资源和审计 | 否 |

详细文档：

- [`A01_HOST_AND_FFI.md`](A01_HOST_AND_FFI.md)
- [`A02_AUTHORITY_AND_CAUSALITY.md`](A02_AUTHORITY_AND_CAUSALITY.md)
- [`A03_MICRO_ATTENTION.md`](A03_MICRO_ATTENTION.md)
- [`A04_NEUROCONTINUUM.md`](A04_NEUROCONTINUUM.md)
- [`A05_ALLOSTASIS_AND_GLIA.md`](A05_ALLOSTASIS_AND_GLIA.md)
- [`A06_CONSTITUTIVE_PLASTICITY.md`](A06_CONSTITUTIVE_PLASTICITY.md)
- [`A07_RENORMALIZATION.md`](A07_RENORMALIZATION.md)
- [`A08_AGENT_COGNITION.md`](A08_AGENT_COGNITION.md)
- [`A09_CONTINUUM_PERSISTENCE.md`](A09_CONTINUUM_PERSISTENCE.md)
- [`A10_OBSERVATORY_AND_SAFETY.md`](A10_OBSERVATORY_AND_SAFETY.md)
