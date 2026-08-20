# FormulaProfile v1

FormulaProfile 决定 Agent 的数学身份，必须进入 `formula_digest` 并随 Snapshot 持久化。

## 固定项

```text
formula_id                aster-ccn-v1
neuron_slots              16384
node_dof                  8
initial_edge_target       262144
edge_capacity             524288
attention_heads           4
levels                    [16384, 2048, 256, 32]
action_candidates         6
rollout_steps             8
fixed_scale               1000000
```

## 参与 digest 的配置

- region layout；
- operator bank；
- personality constitutive coefficients；
- neural mass/coupling/dissipation；
- attention matrices and thresholds；
- allostatic targets；
- residual yield thresholds；
- residual hardening matrix；
- eligibility decay and learning rates；
- growth/prune hysteresis；
- restriction/prolongation operators；
- world model parameters；
- action objective weights；
- integrator iteration limit/tolerances；
- quantization/rounding rules。

## 不参与 digest 的 RuntimeEnvelope

- worker threads；
- cache size；
- trace retention；
- WebUI refresh；
- maintenance slice；
- rollout parallelism。

## 修改规则

FormulaProfile 任何变化都构成公式升级，不能在旧 Snapshot 上静默生效。需要：

1. 新 formula id/digest；
2. 显式 migration plan；
3. 固定 replay 对比；
4. 迁移 receipt；
5. 回退策略。
