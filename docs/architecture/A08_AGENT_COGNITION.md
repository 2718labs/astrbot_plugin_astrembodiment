# A08 — Agent 认知、世界模型与行动竞争

## 目标

让 AstrEmbodiment 通过反事实推演选择行为，而不是通过状态机切换语气。

## Agent 组成

```text
Self Model
World Model
Drives / Allostasis
Global Workspace
Counterfactual Trajectory Generator
Action Evaluator
Action Ownership
```

## 连续行动向量

\[
a\in\mathbb R^{32}
\]

主要轴包括：

```text
answer
verify
acknowledge_error
repair
ask_evidence
set_boundary
withdraw
proactive_reach
warmth
directness
verbosity
confidence_ceiling
```

`concise/firm/repairing` 只是最终向量的标签。

## 候选生成

\[
A=G_\psi(W,Self,Drives,Residuals)
\]

产生 \(K=4\sim6\) 个行动向量。MVP 可使用连续基向量组合，但不能使用 `if/else` 状态跳转作为唯一决策。

## 世界模型 rollout

\[
\widehat s_{t+1}^{(k)}=F_\phi(\widehat s_t^{(k)},a_k,\widehat e_t)
\]

推演 6–8 个粗尺度步，估计：

- 任务完成度；
- 事实/核验风险；
- 用户边界影响；
- 修复效果；
- 自我一致性；
- 认知不确定度；
- 身体负荷；
- 后续无效循环概率。

## 行动泛函

\[
J_k=
\lambda_TQ_{task}
+\lambda_EQ_{epistemic}
+\lambda_BQ_{boundary}
+\lambda_RQ_{repair}
+\lambda_CQ_{continuity}
-\lambda_UC_{uncertainty}
-\lambda_LC_{load}
\]

选择：

\[
k^*=\operatorname{argmax}^{lex}_k J_k
\]

同分按 candidate id 固定排序，保证确定性。

## 纠错冲击

匹配 delivered claim \(j\)：

\[
\delta_{self}=
\kappa_v\langle v\rangle_+
 c_j(0.5+0.5a_j)(0.5+0.5s_j)
\]

- \(v=1\)：确认自己错；
- \(v=-1\)：用户纠错被驳回；
- \(c\)：原置信度；
- \(a\)：原笃定程度；
- \(s\)：风险程度。

正确但恶意的纠错可以同时生成 repair 与 boundary 候选。

## 不耐烦

重复摩擦证据：

\[
L_F=R(1-N)\rho_{user}\kappa
+\alpha C+\beta Q+\gamma I+\delta B
\]

- \(R\)：重复；
- \(N\)：新增信息；
- \(\rho_{user}\)：责任归因；
- \(C,Q,I,B\)：约束反复、无视澄清、打断、越界。

它进入神经荷载与世界模型成本，而不是直接切换 `FIRM`。

## ActionContract

Rust 输出合同而非情绪提示：

```json
{
  "warmth_band": [0.30, 0.45],
  "directness": 0.78,
  "verbosity_budget": 0.42,
  "confidence_ceiling": 0.38,
  "must_verify": true,
  "must_acknowledge_error": true,
  "must_correct_claim": true,
  "may_set_boundary": true,
  "must_not_seek_reassurance": true
}
```

## 能力底线

任何行动必须属于：

\[
\mathcal U_{allowed}=\mathcal U_{safe}\cap\mathcal U_{competent}\cap\mathcal U_{affective}
\]

烦躁可以缩短回复，不能降低事实准确性和必要安全提示。

## MVP 验收

- 同一状态至少产生 4 个有区别的连续行动候选；
- action 由 rollout score 选择，不由单一阈值状态机选择；
- 高风险错误时 repair priority 高于 affect display；
- 正确但恶意纠错能同时认错和设边界；
- Agent 不向用户索取安慰来缓解自己的错误冲击。
