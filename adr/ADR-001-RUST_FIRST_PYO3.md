# ADR-001：Rust-first + PyO3

## 状态

Accepted for 1.0.0.

## 决策

AstrBot 生命周期保持 Python `Star`，全部生产状态与核心计算由 Rust runtime 持有，通过粗粒度 PyO3 接口交互。

## 原因

- AstrBot 插件入口和 hooks 属于 Python；
- 16K 稀疏神经场、定点数、CAS、SQLite 和唯一 writer 更适合 Rust；
- 强类型事件可以从结构上阻止 `safe → warmth`、self-score 自我奖励等非法回路；
- native wheel 比独立 sidecar 更适合 1C1G。

## 后果

- 必须发布多平台 wheel；
- native core 缺失时拒绝启动；
- Python fallback 禁止形成第二颗脑；
- FFI envelope 必须有界、版本化、可校验。
