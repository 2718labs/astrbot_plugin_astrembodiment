# ADR-006：注意力只装配荷载

## 状态

Accepted.

## 决策

微型注意力只输出稀疏路由、活动子域和广义荷载；它不得直接输出情绪或 residual 增量。

## 原因

把“关注了什么”和“受到什么长期影响”分离：

```text
attention → load
neuro/mechanics → response
plasticity + authority → permanent change
```

这从结构上避免 `safe` 或普通文本直接累加 warmth。
