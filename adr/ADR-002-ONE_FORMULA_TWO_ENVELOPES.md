# ADR-002：一个 FormulaProfile，两个 RuntimeEnvelope

## 状态

Accepted.

## 决策

2C2G 与 1C1G 使用相同节点、连接、候选、推演长度、容差和状态格式。RuntimeEnvelope 只能改变线程、缓存、诊断和维护时机。

## 必须成立

\[
Digest(S_n^{2C2G})=Digest(S_n^{1C1G})
\]

## 禁止

- `lite/pro` 两套大脑；
- 1C1G 减少节点、候选或精度；
- 根据硬件改变人格或随机种子；
- 内存不足时静默切换 Python fallback。
