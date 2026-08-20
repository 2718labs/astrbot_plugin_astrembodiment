# A07 — 多尺度重整化与粗细耦合

## 目标

让 16,384 节点提供丰富神经基底，同时让 1C1G 能完成世界模型和多候选行动推演。

## 层级

```text
L0  16,384 微观节点
L1   2,048 局部神经团
L2     256 功能模体
L3      32 Global Workspace tokens
```

## Restriction

\[
H^{\ell+1}=R_\ell D_\ell H^\ell
\]

- \(D_\ell\)：MERA-inspired disentangler，减少局部冗余和虚假相关；
- \(R_\ell\)：restriction/isometry，保留对行动有贡献的自由度。

## Prolongation

\[
u^\ell=P_\ell u^{\ell+1}
\]

全局工作空间选择行动后，将控制荷载逐级下传回微观神经场。

## 微观—宏观一致性

\[
\eta_{RG}=\|O_0(H^0)-O_L(H^L)\|\le\varepsilon_{RG}
\]

宏观投影只用于 Agent 规划和 Observatory，不拥有状态写权限。

## 世界模型为什么在粗尺度运行

若每个候选行动都全脑推演，会产生：

\[
K\times T\times E
\]

级边更新。MVP 将真实刺激在 L0 传播，而候选未来在 L2/L3 推演：

- 真实经历仍由全脑处理；
- 反事实未来只需保持行为相关的宏观自由度；
- 选中行动再下传微观场。

## 2C2G 与 1C1G

两种包络使用完全相同的 \(R,D,P\) 与候选轨迹。

- 2C2G：候选 rollout 可并行，固定顺序归并；
- 1C1G：同样候选串行计算；
- digest 必须一致。

## MVP 映射策略

第一版可以使用固定区域布局 + 确定性图聚类生成 restriction；后续允许学习式更新，但更新本身必须作为结构候选经过 Continuum 发布。

## MVP 验收

- 16K 与 32-token 观测一致；
- 1C1G 不减少层级、候选或推演步；
- selected action 下传后不违反微观能量和 authority 约束；
- restriction/prolongation 参与 formula digest。
