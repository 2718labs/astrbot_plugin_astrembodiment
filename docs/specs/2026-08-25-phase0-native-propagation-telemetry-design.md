# Phase 0-A：真实节点边传播与 native telemetry 闭环规格

## 1. 目标与互斥边界

状态：`APPROVED_FOR_IMPLEMENTATION / DOCUMENT_ONLY / NO_RUNTIME_ACTIVATION`。基线 HEAD `7277f4de6d7b512b79569e73f76083a435456bbe`。依据 H3、H4 与 Luna code map（SHA-256 `E5EA37B13A6083F1F22180788FC9E982A16CC8D748432392EFE16959FD0F4660`）。

实现：真实遍历 `SparseGraph.row_offsets/edges` 的确定性传播；PREPARE 阶段真实 energy/capacity/residual/headroom/native gate；同事务 journal/snapshot/graph/telemetry；text-free append-only compensation checkpoint。不得修改版本/发布、Python、对话参数、回复或 expression 注入。

Agent A 独占 `crates/ae-contracts/src/lib.rs`、`crates/ae-neurofield/src/lib.rs`、`graph_development.rs`、`crates/ae-runtime/src/lib.rs`、`semantic.rs`、新建 `semantic_dynamics_v2.rs`/`semantic_telemetry_v1.rs`/`learning_compensation_v1.rs`、`crates/ae-continuum/src/lib.rs`、`crates/ae-store/src/lib.rs`、`crates/ae-pyo3/src/lib.rs` 与一个 focused Rust integration test。A 不改 Python；B 不改 `crates/**`。

## 2. 兼容/迁移

- `TransitionReceipt` v1 codec、journal、`AESEM2` 保持可读；不回写历史，不把旧固定零 residual 当健康。
- 旧 event dedup 返回原 receipt + `UNAVAILABLE_LEGACY`，禁止 compensation。
- 不复用字段不兼容且当前 unsupported 的 `CorrectionClaim/CorrectionVerdictEvent`。
- 空 semantic graph 首次 v2 唯一物化：`develop_graph(manifest_digest,development_seed_digest,GraphFormula::V1)`。非空图只 validate/digest。迁移原子提交 graph before/after；失败无半状态。新 snapshot 为 `AESEM3`。

## 3. FxP6 与真实传播

~~~text
S=1_000_000
mul6(a,b)=(a*b+500_000) div S
smul6(x,g)=sign(x)*((abs(x)*g+500_000) div S)
ratio6(n,d)=(n*S+floor(d/2)) div d
~~~

中间值 signed i128；状态/headroom `[0,S]`，drive/compensation `[-S,S]`；overflow/除零/非法状态 NO_COMMIT。冻结 rate：propagation/neutral/adaptation `125000`，reserve recovery `25000`，energy cost `100000`；连同 route/graph formula 进入新 formula digest。

~~~text
effective[d]=clamp(local[d]+committed_u[d],0,S)
direct[r]=mul6(evidence_mean[r],local_confidence)
source[i]=clamp(potential[i]+excitation[i]-inhibition[i],-S,S)
weighted[j]=sum(edge i->j)(source[i]*edge.weight)
mass[j]=sum(edge i->j)abs(edge.weight)
edge_mean[j]=0 if mass[j]==0 else signed_round(weighted[j]/mass[j])
drive[j]=clamp(direct[region(j)]+smul6(edge_mean[j],125000),-S,S)
~~~

必须 immutable-before Jacobi 并实际遍历 row offsets/edges。八 DOF：

~~~text
recovery=smul6(potential-baseline,neutral_rate)
potential'=clamp(potential+drive-recovery,0,S)
excitation'=clamp(max(drive,0),0,S)
inhibition'=clamp(max(-drive,0),0,S)
prediction_error'=clamp(abs(drive-recovery),0,S)
precision'=local_confidence
adaptation'=clamp(adaptation+smul6(abs(potential'-baseline)-adaptation,125000),0,S)
eligibility'=clamp((eligibility+prediction_error'+1) div 2,0,S)
work=ceil(sum(abs(delta) of six updated activity components)/6)
recovered=mul6(S-reserve,25000); spent=mul6(work,100000)
reserve_unclamped=reserve+recovered-spent
reserve'=clamp(reserve_unclamped,0,S)
~~~

## 4. Telemetry/receipt

~~~text
energy_headroom=min reserve'
energy_residual=mean abs(reserve_unclamped-reserve')
upper_saturated_nodes=count node with any updated signal component==S
node_headroom=S-ratio6(upper_saturated_nodes,NEURON_SLOTS)
edge_headroom=S-ratio6(edges.len,EDGE_CAPACITY)
capacity_headroom=min(node_headroom,edge_headroom)
capacity_residual=normalized positive capacity overrun
renormalization_residual=max normalized clamp loss
continuity/authority residual=0 only after their structural gates
residual_health=S-max(five residuals)
native_gate=min(energy_headroom,capacity_headroom,residual_health)
~~~

结构 gate 失败无 receipt；committed 零是计算结果，不是占位。

`native-telemetry-receipt.v1` 闭合字段：schema/formula、scope/event、base/next revision、`PREPARE`、state/graph before/after、local/compensation/effective digests、energy ledger/headroom、node/edge used/limit/headroom、五 residual、health、gate、canonical telemetry digest。新 closure v2 必须 verified；legacy 明确 unavailable。

## 5. 最低验收

`cargo check --workspace --locked --offline` exit 0；focused fixture 证明 source→target 边传播；telemetry 可复算；AESEM2 可读且不回写。不声称 release acceptance。
