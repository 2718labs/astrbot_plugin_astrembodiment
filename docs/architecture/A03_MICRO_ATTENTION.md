# A03 — 微型稀疏注意力

## 目标

微型注意力不直接修改情绪。它只决定：**当前证据沿哪些神经路径成为广义荷载。**

## Token 类型

每轮最多 32 个高层 token：

- event evidence；
- 当前 interoceptive/body summary；
- relation residual summary；
- personality projection；
- active claim；
- group/publicness；
- tool/verifier result；
- previous action ownership。

## 四个 head

1. **Salience Head**：显著性、威胁、紧急度、惊讶；
2. **Interoceptive Head**：疲劳、能量、接触/安静/修复需求；
3. **Epistemic Head**：事实冲突、置信、纠错和核验；
4. **Social-Boundary Head**：互惠、拒绝、公开性、边界和修复。

## 稀疏权重

对 head \(h\)：

\[
r_{ij}^{(h)}=
\langle Q_hx_i,K_hx_j\rangle
+b_{ij}^{topo}
+b_{ij}^{personality}
+b_{ij}^{context}
\]

应用拓扑与来源掩码：

\[
M_{ij}=M_{ij}^{topo}M_{ij}^{authority}
\]

使用阈值归一化，不做全连接 softmax：

\[
\alpha_{ij}^{(h)}=
\frac{M_{ij}[r_{ij}^{(h)}-\tau_h]_+}
{\varepsilon+\sum_kM_{ik}[r_{ik}^{(h)}-\tau_h]_+}
\]

广义荷载：

\[
u_i=
\sum_h W_h\sum_j\alpha_{ij}^{(h)}V_hx_j
\]

## 与第一世的根本差异

第一世注意力直接投影 `warmth += ...`。新结构严格分离：

```text
Attention → LoadCandidate
Mechanics/Neurofield → State response
```

注意力层无状态写权限。

## 主动子域

注意力还输出本轮活动节点集合：

\[
\mathcal A_n=\{i\mid s_i>\tau_i\}
\]

普通事件通常激活 2K–4K 节点；高显著事件可扩大，但活动集合由事件和神经状态决定，不由 1C1G/2C2G 配置决定。

## MVP 验收

- 任何 attention 输出不能包含 `warmth_delta` 或 `residual_delta`；
- 同输入和 FormulaProfile 产生确定性相同的稀疏路由；
- `safe` 不作为情感 token；
- authority mask 后，非法路径权重严格为零；
- 32 token、4 head、稀疏边复杂度有明确上限。
