# A09 — Continuum 持久化与确定性重放

## 目标

继承 AstrContinuum 的核心纪律：append-only authority、committed Snapshot、contiguous Delta、fencing、CAS 和失败时继续使用旧合法状态。

AstrEmbodiment 不依赖 AstrContinuum 插件进程，但在 Rust 中实现同类协议。

## 不保存语义记忆

Journal 禁止保存：

- 原始文本；
- 对话摘要；
- embedding；
- 用户事实画像；
- LLM 自由文本解释。

只保存：

- event kind/source/scope；
- 量化证据；
- causal reference；
- formula digest；
- neural/residual/graph delta；
- action/delivery/outcome digest；
- Transition Receipt。

## 权威读视图

\[
CurrentState=CommittedSnapshot+ContiguousDelta
\]

请求只读取一个冻结高水位，不等待后台重组。

## Transition Receipt

```text
schema_version
scope_token
turn_id
base_revision
next_revision
event_digest
authority_digest
formula_digest
state_digest_before
state_digest_after
graph_digest_after
action_contract_digest
active_node_count
active_edge_count
invariant_residuals
commit_status
```

## Journal 哈希链

\[
J_n=H(J_{n-1}\Vert E_n\Vert S_{n+1}\Vert F_n)
\]

## Snapshot 重组

```text
GraphSnapshot + StructuralDelta[1:H]
→ SnapshotCandidate
→ mechanical verification
→ replay verification
→ CAS active pointer
```

必须满足：

\[
\operatorname{Replay}(S_0,\Delta_{1:H})=S_H^{candidate}
\]

## 单一写者与 fencing

- 每个 Persona/Relation 写作用域同一时刻一个 commit owner；
- worker claim 冻结 base revision 和 target HWM；
- stale owner/epoch 更新零行；
- CAS 失败标记 superseded，不覆盖胜者；
- 候选失败时 active pointer 不移动。

## 1C1G 内存策略

重组采用流式临时文件/SQLite 表，不同时在内存保留两张完整图。热关系缓存缩小不影响权威数据。

## MVP 验收

- crash 后从 Snapshot + Delta 恢复相同 digest；
- 候选校验失败仍可正常读取旧 Snapshot；
- replay 结果跨 1C1G/2C2G 相同；
- Journal 中无原始文本；
- 重复 event id 不产生重复提交；
- stale worker 无法改变 active pointer。
