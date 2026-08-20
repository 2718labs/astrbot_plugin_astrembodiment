# ADR-007：Continuum 单一写者

## 状态

Accepted.

## 决策

所有子模块只生成候选。只有 Rust `CommitLane` 能在 revision、authority、energy、capacity、RG 和 replay 验证通过后提交。

## 继承思想

- append-only authoritative journal；
- committed Snapshot + contiguous Delta；
- fencing；
- compare-and-swap active pointer；
- candidate 失败时继续使用旧合法状态。
