# A06 — 本构塑性、资格迹与结构演化

## 目标

用计算力学的可逆—不可逆分解表达“当前情绪会恢复，但经历会留下后果”。

## 不可逆 residual

每个 Relation 保存：

```text
bond
reciprocity
friction_repetition
friction_instability
boundary_violation
scar
repair
fallibility
fair_correction
humiliation
false_accusation
outreach_rejection
```

\[
z_k\ge0,\qquad z_{k,n+1}\ge z_{k,n}
\]

不硬 clamp 到 1；对行为使用饱和映射：

\[
\bar z_k=\frac{z_k}{1+z_k}
\]

## 人格作为本构参数

\[
\Theta_{eff}=\Theta+G(\bar z)
\]

核心人格不被每轮输出改写；残余改变其后天呈现。

## 塑性驱动力

\[
f=G_za+C_zO(H)-Y_0(\Theta)-H_z\bar z-G_m\mu
\]

其中：

- \(a\)：评价/证据向量；
- \(O(H)\)：神经场宏观观测；
- \(Y_0\)：人格决定的初始屈服阈值；
- \(H_z\bar z\)：硬化；
- \(\mu\)：近期活动导致的元可塑性阈值移动。

## 返回映射

\[
\Delta z=
\arg\min_{d\ge0,(I-P_\omega)d=0}
\left[
\frac12d^TDd-f^Td
\right]
\]

\(D\succ0\) 时局部解唯一。

对角 MVP 版本：

\[
\Delta z=P_\omega[D^{-1}f]_+
\]

## 修复不删除伤痕

\[
\Delta z_{scar}\ge0,\qquad \Delta z_{repair}\ge0
\]

当前 scar 影响可被 repair 缓和：

\[
effect_{scar}=\bar z_{scar}(1-\rho\bar z_{repair}),\quad 0\le\rho<1
\]

但不等于从未发生。

## 突触资格迹

行动时只登记局部资格：

\[
\xi_{ij,n+1}=e^{-\Delta t/\tau_\xi}\xi_{ij,n}+\psi(h_i,h_j,a_n)
\]

此时不产生长期关系奖励。

外部结果到达：

\[
\Delta w_{ij}=\eta\,\xi_{ij}\sum_k A_{\omega k}m_k
\]

无匹配 outcome、无 authority 或 TTL 过期时：

\[
\Delta w_{ij}=0
\]

## 连接生长与修剪

结构稳定度：

\[
s_{ij}^{n+1}=\operatorname{clip}
(s_{ij}+\alpha c_{ij}+\beta o_{ij}-\gamma idle_{ij}-\delta conflict_{ij})
\]

- \(c\)：共同激活；
- \(o\)：外部验证的有效结果；
- `idle`：长期无使用；
- `conflict`：持续产生错误或不稳定输出。

使用迟滞阈值：

\[
s<s_{prune}\Rightarrow prune\ candidate
\]

\[
s>s_{grow}\Rightarrow reinforce/grow\ candidate
\]

结构候选由后台生成，唯一 writer 在 revision 仍有效时提交。

## 节点健康

节点固定，但可暂时沉默、疲劳、隔离或从储备招募。永久节点删除不是正常学习机制。

## MVP 验收

- self output 只产生 eligibility，不提交关系 residual；
- repair 永不减少 scar；
- 高置信确认错误的 fallibility 增量大于低置信错误；
- 礼貌纠错不增加 friction；
- 重复但有新增信息的 friction 驱动力显著更低；
- 图增长受容量和修剪约束。
