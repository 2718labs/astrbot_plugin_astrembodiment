# AstrEmbodiment 开工实施计划

## 总原则

内部按 Gate 实现，对外只发布完整 1.0.0。每个 Gate 必须产生可运行代码、测试和收据，不以设计文档替代实现。

## G0 — 仓库与契约地基

目标：编译空 runtime，冻结所有 closed contracts。

交付：

- Cargo workspace；
- PyO3/Maturin native module；
- `CanonicalEvent`、`ActionContract`、`TransitionReceipt`；
- fixed-point math；
- FormulaProfile/RuntimeEnvelope 分离；
- authority matrix 配置；
- AstrBot 插件加载与 native health command。

退出：

- `cargo check --workspace`；
- `maturin develop`；
- AstrBot 加载；
- schema roundtrip；
- Python 无状态写接口。

## G1 — Continuum 与唯一 writer

交付：

- SQLite migration；
- Journal/Snapshot/Delta；
- revision/CAS/fencing；
- Transition Receipt；
- crash recovery；
- replay verifier。

退出：

- 重复事件幂等；
- stale writer 零写；
- Snapshot candidate 失败保持旧指针；
- Journal content-free。

## G2 — 16K 神经基底

交付：

- SoA fixed-point field；
- 9 区域布局；
- CSR/overlay 动态图；
- OperatorBank；
- 微型注意力 4 heads；
- event-driven active subdomain；
- Port-Hamiltonian increment integrator。

退出：

- 16K 节点/边容量；
- 无输入能量不增；
- 100K random transitions；
- 1C1G 内存初测。

## G3 — 异稳态、本构与结构塑性

交付：

- Persona body/allostasis；
- glial regulator；
- relation residual bank；
- return mapping；
- eligibility traces；
- third-factor learning；
- structural candidate queue；
- growth/prune hysteresis。

退出：

- self-action 无关系学习；
- residual 不可逆；
- repair 不删 scar；
- 图容量和结构 replay 通过。

## G4 — 多尺度 Agent

交付：

- 16K→2K→256→32 restriction/prolongation；
- Global Workspace；
- Self Model；
- coarse World Model；
- 6 candidate generator；
- 8-step rollout；
- action objective；
- ActionContract；
- Expression Auditor。

退出：

- 行动不是阈值状态机；
- 1C1G/2C2G digest 一致；
- 微观—宏观 residual 通过。

## G5 — 行动责任、纠错与不耐烦

交付：

- claim extractor；
- delivery ownership；
- correction verifier adapter；
- responsibility attribution；
- friction/new-information/constraint instability evidence；
- S1–S6 scenario suite。

退出：

- 正确、错误、恶意纠错分离；
- 不耐烦有后效但不损害能力；
- stale outcome 无法结算。

## G6 — 主动性、群聊和 Observatory

交付：

- allostatic proactive rollout；
- opt-in/cooldown/budget；
- group/publicness field；
- content-free Observatory；
- reset-affect；
- 资源自检。

退出：

- 主动投递真实闭环；
- 群聊公开性只影响冲击，不影响真值；
- WebUI 零写。

## G7 — 1.0.0 Gauntlet

交付：

- 2C2G 24 h；
- 1C1G 无 swap 24 h；
- 跨包络 replay；
- AstrBot 真机 hooks；
- wheel matrix；
- plugin ZIP；
- SBOM、license、release notes。

退出：全部 `VERIFICATION_GAUNTLET.md` 通过。

## 首批 Issue 建议

1. `contracts: canonical event and authority projection`
2. `math: deterministic fixed-point scalar and digest rules`
3. `store: journal/snapshot/delta schema v1`
4. `runtime: single commit lane with CAS revision`
5. `ffi: pyo3 health and apply_event closed envelope`
6. `astrbot: thin Star lifecycle and native failure mode`
7. `tests: self-action zero-authority invariant`
8. `bench: 1C1G constrained container harness`

## 首个垂直切片

不要先做完整神经场。第一条可运行链应是：

```text
AstrBot on_llm_request
→ CanonicalEvent
→ authority validation
→ no-op deterministic runtime transition
→ ActionContract
→ TransitionReceipt
→ SQLite commit
→ inspect/replay
```

该切片通过后，再把 no-op transition 替换为 16K 神经积分。这样不会让复杂数学建立在不可靠的宿主和持久化之上。
