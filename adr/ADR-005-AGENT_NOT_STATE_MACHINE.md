# ADR-005：Agent，不以状态机作为决策核心

## 状态

Accepted.

## 决策

行为由 Global Workspace、Self/World Model、多候选连续行动向量和反事实 rollout 产生。`concise/firm/withdrawn` 只允许作为可观测标签。

## 禁止

```python
if patience < 0.3:
    mode = "FIRM"
```

## 允许

- 离散生命周期只用于事务与 hook 阶段；
- 延迟/迟滞可以用于结构修剪和资源治理；
- 最终行动由连续目标函数与硬约束选出。
