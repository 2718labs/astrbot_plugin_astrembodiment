# A01 — AstrBot Host 与 PyO3 FFI

## 目标

让 AstrEmbodiment 原生生活在 AstrBot 生命周期中，同时保持 Python 不拥有任何生产神经状态。

## 边界

Python 负责：

- `Star` 生命周期；
- AstrBot 事件和 provider 对象；
- 语义模型/核验模型调用；
- 装饰链、TTS、图片、工具与消息投递；
- 把最终平台事实转换为 Rust 事件。

Rust 负责：

- Persona/Relation 状态；
- 神经场和连接；
- 本构与 Agent 推演；
- action contract；
- journal、Snapshot、Delta、CAS；
- Observatory projection。

## 粗粒度接口

```python
runtime.apply_event(scope, event_bytes) -> decision_bytes
runtime.settle_delivery(scope, delivery_bytes) -> receipt_bytes
runtime.settle_outcome(scope, outcome_bytes) -> receipt_bytes
runtime.inspect(scope) -> json_bytes
runtime.verify_replay(scope) -> report_bytes
runtime.flush_and_close() -> None
```

每个 hook 最多进行一次主要 FFI 调用。禁止逐神经元 getter/setter。

## 生命周期映射

| AstrBot hook | Rust 事件/操作 |
|---|---|
| `on_message` | `TransportObserved`，只冻结平台事实和重复投递证据 |
| `on_llm_request` | `UserStimulus`，计算 action contract |
| `on_agent_begin` | 冻结本轮 contract 和 claim extraction context |
| `on_llm_response` | `SelfActionCandidate`，尚未提交 |
| `on_decorating_result` | Expression audit 和最终可见动作提取 |
| `after_message_sent` | `DeliveryOutcome`，成功后提交行动所有权 |
| 下一轮消息 | `UserReaction`，绑定上一行动资格迹 |
| verifier 完成 | `CorrectionVerdict` |
| `terminate` | 停止准入、排干 writer、flush、关闭 native runtime |

## 失败约束

- Rust panic 必须在 FFI 边界转化为明确错误；
- Python 不能在 native 失败后自己改状态；
- native wheel 缺失时插件拒绝激活；
- event serialization 必须有 schema/version/digest；
- FFI 返回的 action contract 只对当前 turn token 有效。

## MVP 验收

- 本地 AstrBot 能加载插件；
- request、response、delivery 三个阶段各只跨一次主 FFI；
- Python 进程中不存在可写 neural/residual dict；
- stale turn token 无法结算新一轮；
- native core 缺失时有清晰错误，不会静默降级。
