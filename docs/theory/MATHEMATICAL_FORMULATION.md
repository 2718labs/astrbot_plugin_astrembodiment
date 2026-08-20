# ASTER-CCN Formula v1 — 数学规范

## 0. 定位

ASTER-CCN（Affective State Transition with Embodied Residuals under Continuum-Constitutive Neurodynamics）是 AstrEmbodiment 1.0.0 的唯一权威计算公式。

它不是人脑的生物学复刻，也不是精神疾病模型。它是一套满足以下工程要求的具身 Agent 数学结构：

- 万级稀疏神经自由度；
- 稳定人格作为本构参数；
- 可恢复神经/身体状态；
- 不可逆关系与结构残余；
- 外部权威约束的三因子学习；
- 多尺度 Agent 世界模型；
- 可审计、可重放、跨资源包络确定性。

## 1. 符号

| 符号 | 含义 |
|---|---|
| \(N=16384\) | 固定神经节点槽位 |
| \(d_h=8\) | 每节点自由度 |
| \(H_n\in\mathbb Q^{N\times d_h}\) | 第 \(n\) 轮神经场 |
| \(B_n\in\mathbb Q^8\) | Persona 级异稳态身体变量 |
| \(G_n=(V,E_n)\) | 动态稀疏神经图 |
| \(Z_n^r\in\mathbb R_+^{12}\) | Relation \(r\) 的不可逆 residual |
| \(\Theta\) | 核心人格/本构参数 |
| \(\Gamma_n\) | 胶质/稳态调制场 |
| \(C_n\) | 行动所有权与 claim ledger |
| \(Q_n\) | Continuum revision/HWM/digest 坐标 |
| \(E_n\) | CanonicalEvent |
| \(P_\omega\) | 来源 \(\omega\) 的写权限投影 |
| \(u_n\) | 微型注意力装配的广义荷载 |
| \(a_k\) | 第 \(k\) 个连续行动候选 |

完整状态：

\[
\mathcal S_n=(\Theta,H_n,B_n,G_n,\{Z_n^r\},\Gamma_n,C_n,Q_n)
\]

系统在扩展状态空间中是 Markov 的；对外由于 \(Z,G,C\) 保留历史后果而呈现路径依赖。

## 2. 定点数与确定性

生产内核使用定点标量：

\[
x_{fixed}=\operatorname{round}(x\cdot 10^6)
\]

乘法使用宽中间量并按固定舍入规则量化。禁止 `NaN`、`Inf`、未定义溢出和非确定随机源。

所有候选排序使用：

```text
score descending → candidate_id ascending
```

因此线程调度不影响结果。

## 3. 事件与权威

CanonicalEvent：

\[
E_n=(\omega_n,\phi_n,\kappa_n,scope_n,causal_n,t_n,revision_n)
\]

- \(\omega\)：来源；
- \(\phi\)：量化证据；
- \(\kappa\)：逐维置信；
- `causal`：turn/action/delivery/verdict 绑定。

来源权限约束：

\[
(I-P_{\omega_n})\Delta z_n=0
\]

核心不变量：

\[
P_{SELF\_CRITIQUE}=0
\]

\[
P_{SELF\_ACTION,bond}=P_{SELF\_ACTION,repair}=0
\]

\[
P_{PLATFORM\_OBSERVED,acceptance}=0
\]

## 4. 人格本构参数

\[
\Theta=(
\theta_w,
\theta_p,
\theta_s,
\theta_i,
\theta_c,
\theta_{ep},
\theta_{eo},
\theta_b,
\theta_f,
\theta_a,
\theta_x,
\theta_q)
\]

分别表示基础温度、耐心、敏感性、易烦、镇定、认知自尊、纠错开放、边界、宽容、依恋、表达和好奇。

有效人格：

\[
\Theta_{eff}=\Theta+G_\Theta(\bar Z)
\]

其中：

\[
\bar z=\frac{z}{1+z}
\]

核心人格固定；后天 residual 改变能量景观和阈值。

## 5. 微型注意力与广义荷载

高层 token \(x_j\) 经四个 head：

\[
r_{ij}^{(h)}=\langle Q_hx_i,K_hx_j\rangle+b_{ij}^{topo}+b_{ij}^{\Theta}+b_{ij}^{context}
\]

\[
M_{ij}^{(h)}=M_{ij}^{topo}M_{ij}^{authority}
\]

\[
\alpha_{ij}^{(h)}=
\frac{M_{ij}^{(h)}[r_{ij}^{(h)}-\tau_h]_+}
{\varepsilon+\sum_kM_{ik}^{(h)}[r_{ik}^{(h)}-\tau_h]_+}
\]

\[
u_i=\sum_hW_h\sum_j\alpha_{ij}^{(h)}V_hx_j
\]

Attention 只输出 \(u\) 与活动子域 \(\mathcal A_n\)，无状态写权限。

## 6. 神经图与共享算子

每条边：

\[
e_{ij}=(\tau_{ij},w_{ij},\xi_{ij},s_{ij},d_{ij},health_{ij})
\]

共享规范算子：

\[
U_{ij}=w_{ij}U_{\tau_{ij}}
\]

局部坐标变换 \(g_i\)：

\[
h_i'=\rho_i(g_i)h_i
\]

\[
U_{ij}'=\rho_i(g_i)U_{ij}\rho_j(g_j)^{-1}
\]

宏观行动观测必须规范不变。

环路内部冲突：

\[
\Omega_C=\prod_{(i,j)\in C}U_{ij},\qquad
\kappa_C=\|I-\Omega_C\|
\]

## 7. 神经能量与增量积分

定义能量：

\[
\mathcal H(H,B,Z;\Theta)=
\frac12(H-b_H)^TK_H(H-b_H)
+\frac12(B-b_B)^TK_B(B-b_B)
+\Phi_{coupling}(H,B,Z;\Theta)
\]

采用离散梯度 Port-Hamiltonian：

\[
M\frac{H_{n+1}-H_n}{\Delta t}
=(J_n-R_n)\bar\nabla\mathcal H(H_n,H_{n+1})+B_u u_n
\]

\[
J_n^T=-J_n,\qquad R_n\succeq0
\]

能量增量：

\[
\mathcal H_{n+1}-\mathcal H_n=
-\Delta t\,\bar\nabla\mathcal H^TR_n\bar\nabla\mathcal H
+\Delta t\,y_n^Tu_n
\]

无输入时：

\[
\mathcal H_{n+1}\le\mathcal H_n
\]

### MVP 数值步骤

1. 对未激活节点执行解析松弛；
2. 对活动子域装配稀疏 \(J,R,M\)；
3. 运行固定上限的离散梯度/不动点迭代；
4. residual 超容差则拒绝候选，不采用未收敛状态；
5. 量化后再生成 state digest。

## 8. 异稳态与内感受

\[
\widehat B_{n+1}=F_\Theta(B_n,\bar Z_n,\Delta t)
\]

\[
y_n^B=O_B(H_n)
\]

\[
\epsilon_n^B=\Pi_n(y_n^B-\widehat y_n^B)
\]

\[
u_n^A=K_\epsilon\epsilon_n^B+K_dd_n
\]

总荷载：

\[
u_n^{total}=u_n+u_n^A
\]

## 9. 胶质/稳态调节

局部活动均值 \(\widehat a_r\) 需要接近目标 \(a_r^*\)：

\[
\Gamma_{r,n+1}=\Gamma_{r,n}
+\eta_h(a_r^*-\widehat a_r)
-\eta_f fatigue_r
+\eta_o validated\_outcome_r
\]

稳态缩放：

\[
w_{ij}\leftarrow w_{ij}\frac{a_i^*}{\widehat a_i+\varepsilon}
\]

该过程不能改变 Relation residual。

## 10. 可逆—不可逆分解

神经/身体 trial：

\[
X^{trial}=\mathcal T_{neural}(H_n,B_n,u_n^{total};\Theta,Z_n)
\]

Residual 驱动力：

\[
f=G_z\phi_n+C_zO(X^{trial})-Y_0(\Theta)-H_z\bar Z_n-G_m\mu_n
\]

返回映射：

\[
\Delta Z_n=
\arg\min_{d\ge0,(I-P_{\omega_n})d=0}
\left[
\frac12d^TDd-f^Td
\right]
\]

\[
D\succ0
\]

更新：

\[
Z_{n+1}=Z_n+\Delta Z_n
\]

对角 MVP 版本：

\[
\Delta Z_n=P_{\omega_n}[D^{-1}f]_+
\]

## 11. 可观测情绪是投影，不是可写状态

\[
q=O_q(H,B,Z;\Theta)
\]

例如温暖：

\[
q_{warm}=sat(
\theta_w
+\alpha_b\bar z_{bond}
+\alpha_r\bar z_{repair}
-\alpha_s\bar z_{scar}(1-\rho\bar z_{repair})
-\alpha_f\bar z_{friction}
+v_{affective})
\]

耐心：

\[
q_{patience}=sat(
\theta_p
-\beta_f\bar z_{friction}
-\beta_b\bar z_{boundary}
-\beta_i q_{irritation}
-\beta_l q_{fatigue}
+\beta_r\bar z_{repair})
\]

不存在 `warmth += delta`。

## 12. 资格迹与外部第三因子

行动时：

\[
\xi_{ij,n+1}=e^{-\Delta t/\tau_\xi}\xi_{ij,n}+\psi(h_i,h_j,a_n)
\]

不立即强化。

合法外部 outcome：

\[
\Delta w_{ij}=\eta\xi_{ij}\sum_kA_{\omega k}m_k
\]

必须满足：

\[
\Delta w_{ij}\neq0
\Rightarrow
\xi_{ij}>0\land causal\_match=1\land A_{\omega k}\neq0
\]

## 13. 结构生长、修剪与节点招募

\[
s_{ij}^{n+1}=clip(s_{ij}+\alpha c_{ij}+\beta o_{ij}-\gamma idle_{ij}-\delta conflict_{ij})
\]

迟滞：

\[
s<s_{prune}\Rightarrow prune\ candidate
\]

\[
s>s_{grow}\land capacity\ available\Rightarrow grow/reinforce\ candidate
\]

固定节点槽位：

\[
N=16384
\]

节点健康：

\[
0\le d_i\le1,\qquad \widetilde h_i=(1-d_i)g_ih_i
\]

节点优先隔离/恢复；储备槽位招募必须通过结构候选提交。

## 14. 多尺度重整化

\[
H^{\ell+1}=R_\ell D_\ell H^\ell
\]

\[
u^\ell=P_\ell u^{\ell+1}
\]

层级：

\[
16384\rightarrow2048\rightarrow256\rightarrow32
\]

一致性：

\[
\eta_{RG}=\|O_0(H^0)-O_3(H^3)\|\le\varepsilon_{RG}
\]

## 15. Agent 世界模型与行动

工作空间：

\[
W_n=H_n^{(3)}
\]

候选：

\[
A_n=G_\psi(W_n,Self_n,Drives_n,Z_n)
\]

世界模型：

\[
\widehat s_{t+1}^{(k)}=F_\phi(\widehat s_t^{(k)},a_k,\widehat e_t)
\]

作用量/价值：

\[
J_k=\sum_{t=0}^{T}
[
\lambda_TQ_{task}
+\lambda_EQ_{epistemic}
+\lambda_BQ_{boundary}
+\lambda_RQ_{repair}
+\lambda_CQ_{continuity}
-\lambda_UC_{uncertainty}
-\lambda_LC_{load}
]
\]

\[
k^*=argmax_k^{lex}J_k
\]

输出连续 `ActionContract`。

## 16. 纠错与可错性

用户主张不是 verdict。核验值：

\[
v\in[-1,1],\qquad \kappa_v\in[0,1]
\]

确认自己错误冲击：

\[
\delta_{self}=\kappa_v\langle v\rangle_+
 c_j(0.5+0.5a_j)(0.5+0.5s_j)
\]

用户错误冲击：

\[
\delta_{other}=\kappa_v\langle-v\rangle_+
\]

礼貌正确纠错：fallibility/fair-correction 有权写，friction 无权写。

恶意正确纠错：以上成立，同时 hostility 可写 humiliation/boundary。

## 17. 重复与不耐烦

\[
L_F=R(1-N)\rho_{user}\kappa
+\alpha C+\beta Q+\gamma I+\delta B
\]

若上一轮答案错误或工具失败：

\[
\rho_{user}\approx0
\]

因此 Agent 不得把自己的失败转化为对用户的摩擦学习。

## 18. Continuum

\[
S_n=Snapshot_b+\sum_{i=b+1}^{n}Delta_i
\]

Journal 哈希：

\[
J_n=H(J_{n-1}\Vert E_n\Vert S_{n+1}\Vert FormulaDigest)
\]

候选 Snapshot：

\[
Replay(S_b,\Delta_{b+1:H})=S_H^{candidate}
\]

只有 CAS 成功后成为 active。

## 19. 提交残差

\[
\eta_{authority}=\|(I-P_\omega)\Delta Z\|
\]

\[
\eta_{continuity}=\|S_{replay}-S_{candidate}\|
\]

\[
\eta_{energy}=\max(0,\Delta\mathcal H-y^Tu\Delta t)
\]

\[
\eta_{RG}=\|O_0(H^0)-O_3(H^3)\|
\]

\[
\eta_{capacity}=\max(0,|E|-E_{max})
\]

提交条件：

```text
authority == 0
continuity == 0
capacity == 0
energy <= tolerance
RG <= tolerance
revision still current
```

## 20. 一轮伪代码

```text
input: committed state S_n, canonical event E_n

1. validate schema/scope/revision/causal authority
2. propagate inactive nodes analytically by elapsed time
3. assemble tokens and sparse generalized load u_n
4. solve active neurofield trial H_trial
5. update allostatic/glial candidates
6. restrict H_trial through 2048 → 256 → 32 levels
7. generate K continuous action trajectories
8. roll out world model and choose action contract
9. compute residual/plasticity candidates under P_omega
10. calculate invariant residuals
11. if any hard residual fails: reject candidate, retain S_n
12. otherwise create TransitionReceipt
13. CAS commit Delta / state revision
14. return action contract + receipt projection
```

对于 `SelfActionCandidate`，步骤 9 的关系 residual 权限为零；只有 delivery/outcome 后续事件才能结算相应学习。
