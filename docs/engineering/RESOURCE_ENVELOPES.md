# 2C2G 参考包络与 1C1G 兼容包络

## 1. 核心原则

两种包络共享同一 FormulaProfile、同一 16K 大脑、同一动态连接图、同一候选行动、同一推演长度、同一容差和同一状态格式。

\[
Digest(S_n^{2C2G})=Digest(S_n^{1C1G})
\]

硬件只改变并行度、缓存和延迟。

## 2. Canonical Brain

```text
neuron slots           16,384
node DOF               8
initial active edges   ~262,144
edge capacity          524,288
attention heads        4
levels                 16,384 → 2,048 → 256 → 32
action candidates      6
rollout steps          8
```

## 3. RuntimeEnvelope

| 运行项 | 2C2G 参考 | 1C1G 兼容 |
|---|---:|---:|
| worker threads | 2 | 1 |
| commit writer | 1 | 1 |
| rollout execution | 并行、固定归并 | 串行、固定顺序 |
| SQLite cache | 32 MiB | 8 MiB |
| hot relations | 256 | 32–64 |
| trace retention | 1,024 | 128 |
| maintenance | 第二核心后台 | turn 后小切片/空闲期 |
| WebUI refresh | 1–2 s | 5–10 s |
| Snapshot rebuild | 后台流式 | 空闲流式 |
| math/precision | 完整 | 完整 |

## 4. 线程职责

### 2C2G

```text
Core 0: event integration, workspace, action choice, sole commit
Core 1: rollouts, structural candidates, replay, snapshot rebuild, observatory
```

Core 1 只能基于冻结 revision 生成候选，不能直接写脑。

### 1C1G

所有任务按固定顺序串行；后台任务分割成有界切片。用户前台事件优先，维护可以延后但不能丢失。

## 5. 设计内存预算

这些是目标，不是已实测结果。

| 组成 | 1C1G | 2C2G |
|---|---:|---:|
| 节点与工作缓冲 | 4 MiB | 4 MiB |
| 稀疏图 + overlay | 16 MiB | 16 MiB |
| OperatorBank | 1 MiB | 1 MiB |
| 多尺度映射 | 6 MiB | 6 MiB |
| Workspace/World Model | 6 MiB | 6 MiB |
| rollout/integrator workspace | 16 MiB | 32 MiB |
| hot relation cache | 2 MiB | 8 MiB |
| SQLite cache | 8 MiB | 32 MiB |
| Observatory | 4 MiB | 16 MiB |
| allocator/FFI/余量 | 24 MiB | 40 MiB |
| Rust core 目标 | ≤96 MiB | ≤160 MiB |

完整 AstrBot 进程在 1C1G 的目标 RSS ≤850 MiB，并保留至少 150 MiB 安全余量。该目标必须在真实容器实测。

## 6. 延迟目标

| 指标 | 2C2G | 1C1G |
|---|---:|---:|
| 常规本地事件 p95 | ≤40 ms | ≤120 ms |
| 高显著全脑事件 p95 | ≤150 ms | ≤350 ms |
| 空闲 CPU | ≤1% | ≤1% |

不包含远程 LLM 延迟。

## 7. 资源不足时让渡顺序

1. 降低 Observatory 刷新；
2. 丢弃旧非权威 trace；
3. 缩小热关系缓存；
4. 延后 Snapshot 重组；
5. 延后结构修剪/生长候选；
6. 将 rollout 串行；
7. 延迟非关键统计。

禁止让渡：

- 节点数；
- 连接权威状态；
- 人格；
- residual；
- 候选数量；
- rollout 步数；
- 积分容差；
- 纠错 verdict；
- 真实用户反馈；
- replay 一致性。

仍不足时明确拒绝启动。

## 8. 支持边界

- 1C1G 需要远程 LLM；
- 不承诺同机本地大模型；
- 不承诺同时运行大量高内存插件；
- native wheel、Python 宿主和 AstrBot 基础进程都必须纳入 RSS 实测；
- 无 swap 运行是 1C1G 发布门之一。
