# ASTER-CCN 不变量与初步证明

本文给出 MVP 必须通过的形式性质。它们是软件公式的性质，不是对真实人类情感的科学声明。

## P1：无自我关系强化

若：

\[
P_{SELF\_ACTION,bond}=P_{SELF\_ACTION,repair}=0
\]

返回映射约束：

\[
(I-P_{SELF\_ACTION})d=0
\]

则：

\[
\Delta z_{bond}=\Delta z_{repair}=0
\]

因此无论 SelfAction 的文本多温柔，都不能自行提升关系 residual。

## P2：不可逆 residual

返回映射可行域：

\[
d\ge0
\]

更新：

\[
Z_{n+1}=Z_n+d
\]

因此逐分量：

\[
Z_{n+1}\ge Z_n
\]

## P3：修复不删除伤痕

scar 与 repair 是不同坐标，且：

\[
\Delta z_{scar}\ge0,\quad \Delta z_{repair}\ge0
\]

修复仅通过观测映射减弱 scar 的现时影响，不能使 \(z_{scar}\) 下降。

## P4：局部塑性解唯一

若 \(D\succ0\)，目标函数：

\[
\frac12d^TDd-f^Td
\]

严格凸；可行域 \(d\ge0,(I-P_\omega)d=0\) 为闭凸集，因此存在唯一最优解。

## P5：无输入能量不增

离散梯度积分满足：

\[
\Delta\mathcal H=-\Delta t\,\bar\nabla\mathcal H^TR\bar\nabla\mathcal H
\]

当 \(R\succeq0\) 且 \(u=0\)：

\[
\Delta\mathcal H\le0
\]

这排除无输入的正反馈永动回路。

## P6：硬件包络不改变行为

FormulaProfile 冻结，RuntimeEnvelope 不进入：

- 神经算子；
- 候选集合；
- 推演长度；
- 排序；
- 量化规则；
- state digest。

并行归并使用固定顺序。因此：

\[
S_n^{2C2G}=S_n^{1C1G}
\]

在所有整数运算与外部证据序列相同时成立。

## P7：未核验纠错不写 fallibility

只有 `VERIFIER_RESULT::ConfirmedSelfError` 的权限投影在 fallibility 维度为 1。`CorrectionClaim` 对该维度为 0，因此：

\[
\Delta z_{fallibility}^{CorrectionClaim}=0
\]

## P8：外部反馈因果隔离

只有满足：

\[
causal\_match=1\land eligibility>0\land not\ settled
\]

的 outcome 进入塑性公式。因此迟到、重复、跨 scope outcome 不改变权重。

## P9：状态有界与容量有界

神经定点变量通过稳定的饱和映射/能量守卫限制；边数必须满足：

\[
|E|\le E_{max}=524288
\]

候选突破容量时 \(\eta_{capacity}>0\)，提交被拒绝。

## P10：宏观可观测无写权限

warmth、patience 等是：

\[
q=O(H,B,Z;\Theta)
\]

它们不出现在权威状态 setter 接口中，因此 Observatory/LLM 不能反向直接写入。
