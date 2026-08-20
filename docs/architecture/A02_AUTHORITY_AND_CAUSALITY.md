# A02 — 权威与因果绑定

## 目标

解决一个核心问题：**谁有资格改变她的哪一部分？**

## 来源类型

```text
USER_OBSERVED
EXPLICIT_FEEDBACK
PLATFORM_OBSERVED
VERIFIER_RESULT
SELF_ACTION
SELF_CRITIQUE
TIME_ADVANCE
ADMIN_ACTION
```

## 权限投影

对每个来源 \(\omega\) 定义对角投影矩阵 \(P_\omega\)：

\[
\Delta z = P_\omega \Delta z
\]

必须满足：

\[
P_{\mathrm{SELF\_ACTION},\,bond}=0
\]

\[
P_{\mathrm{SELF\_ACTION},\,repair}=0
\]

\[
P_{\mathrm{SELF\_CRITIQUE}}=0
\]

\[
P_{\mathrm{PLATFORM\_OBSERVED},\,acceptance}=0
\]

真实投递只证明“她做了”，不能证明“用户接受了”。

## 因果引用

每个外部结果必须携带：

```text
scope_token
turn_id
action_id
delivery_id
base_revision
outcome_kind
observed_at
```

只有当：

\[
\operatorname{match}(outcome, eligibility)=1
\]

且未超过 TTL、未结算、scope/revision 一致时，才能触发长期学习。

## 纠错权限

- `CorrectionClaim`：只能提升 epistemic conflict/verification need；
- `CorrectionVerdict::ConfirmedSelfError`：可写 fallibility 和 fair correction；
- `CorrectionVerdict::RejectedChallenge`：fallibility 写权限为零；
- hostility 来自用户表达证据，可独立写 humiliation/boundary；
- 正确与恶意可以同时成立，因此权限按维度而非按单标签控制。

## DevKit 思想落点

```text
CAPTURE → CLAIM → BIND → COMPUTE → VERIFY → ACCEPT → COMMIT
```

工作包、LLM JSON、消息文本和 caller-supplied ID 都不是 authority。只有宿主冻结事实、Rust 生成的 capability token 与验证终态共同形成写权限。

## MVP 验收

- `SELF_ACTION` 连续 100 次，所有关系 residual 增量严格为零；
- 相同 outcome 不能结算两次；
- 迟到 outcome 不能结算下一 turn；
- 错误纠错主张不能增加 fallibility；
- delivery success 不能增加 bond；
- authority residual 始终为零：

\[
\eta_{authority}=\|(I-P_\omega)\Delta z\|=0
\]
