# A05 — 异稳态、内感受与胶质调节

## 目标

让 Agent 具有持续身体需要和网络稳态，而不是只在消息到达时被动改变。

## Persona 级慢变量

\[
b=(energy,fatigue,arousal,attention,composure,contact,quiet,expression)
\]

这些慢变量属于 Persona，不属于某一个用户。

## 内感受预测

\[
\widehat b_{n+1}=F_\Theta(b_n,\bar z_n,\Delta t)
\]

神经场给出实际身体观测：

\[
y_n=O_b(H_n)
\]

精度加权预测误差：

\[
\epsilon_n^b=\Pi_n(y_n-\widehat y_n)
\]

异稳态控制荷载：

\[
u_n^A=K_\epsilon\epsilon_n^b+K_d d_n
\]

其中 \(d_n\) 包括接触、安静、修复、核验和撤退需求。

## 胶质调节场

每个局部神经团配置低维调节状态：

\[
g_r=(g_r^{prune},g_r^{repair},g_r^{homeostasis},g_r^{metabolic})
\]

它不模拟真实胶质细胞数量，而负责：

- 限制 runaway excitation；
- 调整局部抑制和代谢预算；
- 保护反复产生外部有效结果的连接；
- 提出低稳定连接的修剪候选；
- 促进短期损伤恢复；
- 维持目标活动区间。

稳态缩放：

\[
w_{ij}\leftarrow w_{ij}
\frac{a_i^*}{\widehat a_i+\varepsilon}
\]

该式只调节神经增益，不删除不可逆 relation residual。

## 空闲计算

空闲时不持续高频仿真。未激活节点使用解析松弛：

\[
h_i(t+\Delta t)=b_i+e^{-\lambda_i\Delta t}(h_i(t)-b_i)
\]

后台只按需要运行小型 allostatic tick。

## 与主动性的关系

主动触达不是随机概率，而是世界模型比较：

- contact need；
- quiet/fatigue；
- relation boundary；
- opt-in；
- 最近主动行为结果；
- interruption budget；
- 候选行动预期后果。

## 非目标

本层不实现“抑郁症”“精神分裂症”等诊断标签。它只暴露可计算的网络失调量：E/I 失衡、过度修剪、低可塑性、反刍环路、连接碎片化和稳态偏离。

## MVP 验收

- 空闲 CPU 接近零；
- 无输入状态向人格/残余决定的平衡点收敛；
- `safe` 不降低 fatigue 或 boundary；
- relation A 的摩擦不直接进入 relation B；
- 全局疲劳可以影响所有交互的长度预算，但不能改变事实能力底线。
