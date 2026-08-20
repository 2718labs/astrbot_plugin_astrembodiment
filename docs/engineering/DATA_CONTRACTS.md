# 数据契约与 Rust 类型

## 1. 原则

- 不使用 `Vec<String>` flags 驱动计算；
- 来源、因果、scope、event 和 outcome 使用强类型枚举；
- LLM 返回的是 `SemanticEstimate`，不是状态增量；
- Python 只传 closed envelope；
- 所有 envelope 含 `schema_version` 和 digest；
- 未知字段/未知枚举默认拒绝，不静默映射到 safe。

## 2. CanonicalEvent

```rust
pub enum CanonicalEvent {
    UserStimulus(UserStimulus),
    UserReaction(UserReaction),
    CorrectionClaim(CorrectionClaim),
    CorrectionVerdict(CorrectionVerdict),
    SelfActionCandidate(SelfActionCandidate),
    DeliveryOutcome(DeliveryOutcome),
    TimeAdvance(TimeAdvance),
    AdminAction(AdminAction),
}
```

## 3. 来源

```rust
pub enum SourceAuthority {
    UserObserved,
    ExplicitFeedback,
    PlatformObserved,
    VerifierResult,
    SelfAction,
    SelfCritique,
    TimeAdvance,
    AdminAction,
}
```

## 4. Scope 与因果

```rust
pub struct ScopeRef {
    pub bot_token: [u8; 16],
    pub persona_token: [u8; 16],
    pub relation_token: Option<[u8; 16]>,
    pub session_token: [u8; 16],
}

pub struct CausalRef {
    pub turn_id: [u8; 16],
    pub action_id: Option<[u8; 16]>,
    pub delivery_id: Option<[u8; 16]>,
    pub claim_id: Option<[u8; 16]>,
    pub base_revision: u64,
}
```

原始平台 ID 在 Python/AstrBot 边界转成不可逆 token；Rust store 不保存原始 sender 文本标识。

## 5. 语义证据

```rust
pub struct SemanticEstimate {
    pub schema_version: u16,
    pub dimensions: EvidenceVector,
    pub confidence: EvidenceConfidence,
    pub estimator_digest: [u8; 32],
}

pub struct EvidenceVector {
    pub positive: Fixed,
    pub affiliation: Fixed,
    pub harm: Fixed,
    pub boundary: Fixed,
    pub repair: Fixed,
    pub repetition: Fixed,
    pub new_information: Fixed,
    pub constraint_instability: Fixed,
    pub epistemic_conflict: Fixed,
    pub self_responsibility: Fixed,
    pub other_responsibility: Fixed,
    pub hostility: Fixed,
    pub publicness: Fixed,
    pub engagement: Fixed,
    pub rejection: Fixed,
}
```

不允许：

```text
warmth_delta
bond_delta
personality_delta
```

## 6. ActionContract

```rust
pub struct ActionContract {
    pub action_id: [u8; 16],
    pub turn_id: [u8; 16],
    pub continuous: ActionVector,
    pub must_verify: bool,
    pub must_acknowledge_error: bool,
    pub must_correct_claim: bool,
    pub may_set_boundary: bool,
    pub may_withdraw: bool,
    pub must_not_seek_reassurance: bool,
    pub confidence_ceiling: Fixed,
    pub verbosity_budget: Fixed,
    pub directness: Fixed,
    pub warmth_min: Fixed,
    pub warmth_max: Fixed,
    pub expires_at_ms: u64,
}
```

## 7. 行动候选与评分

```rust
pub struct ActionCandidate {
    pub id: u16,
    pub vector: ActionVector,
    pub score: ActionScore,
    pub rollout_digest: [u8; 32],
}

pub struct ActionScore {
    pub task: Fixed,
    pub epistemic: Fixed,
    pub boundary: Fixed,
    pub repair: Fixed,
    pub continuity: Fixed,
    pub uncertainty_cost: Fixed,
    pub load_cost: Fixed,
    pub total: Fixed,
}
```

## 8. Action Ownership 与 Claim

```rust
pub struct DeliveredAction {
    pub action_id: [u8; 16],
    pub delivery_id: [u8; 16],
    pub delivered_at_ms: u64,
    pub visible_action_digest: [u8; 32],
    pub claims: Vec<ClaimCommitment>,
}

pub struct ClaimCommitment {
    pub claim_id: [u8; 16],
    pub confidence: Fixed,
    pub assertiveness: Fixed,
    pub stakes: Fixed,
    pub audience_publicness: Fixed,
    pub expires_at_ms: u64,
}
```

未投递草稿不能生成 `DeliveredAction`。

## 9. CorrectionVerdict

```rust
pub enum VerdictKind {
    ConfirmedSelfError,
    RejectedChallenge,
    SharedAmbiguity,
    HostFailure,
    Unresolved,
}

pub struct CorrectionVerdict {
    pub verdict: VerdictKind,
    pub claim_id: [u8; 16],
    pub confidence: Fixed,
    pub contradiction: Fixed,
    pub hostility: Fixed,
    pub evidence_digest: [u8; 32],
}
```

## 10. TransitionReceipt

```rust
pub struct TransitionReceipt {
    pub schema_version: u16,
    pub formula_digest: [u8; 32],
    pub scope_digest: [u8; 32],
    pub event_digest: [u8; 32],
    pub authority_digest: [u8; 32],
    pub base_revision: u64,
    pub next_revision: u64,
    pub state_before: [u8; 32],
    pub state_after: [u8; 32],
    pub graph_after: [u8; 32],
    pub action_contract: Option<[u8; 32]>,
    pub active_nodes: u32,
    pub active_edges: u32,
    pub residuals: InvariantResiduals,
    pub status: CommitStatus,
}
```

## 11. Authority Matrix v1

| 来源 | bond | friction | boundary | scar | repair | fallibility | fair correction | humiliation | 权重 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| UserObserved | 条件 | 条件 | 条件 | 条件 | 0 | 0 | 0 | 条件 | 条件 |
| ExplicitFeedback | 条件 | 条件 | 条件 | 条件 | 条件 | 0 | 0 | 条件 | 条件 |
| PlatformObserved | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| VerifierResult | 0 | 0 | 条件 | 0 | 条件 | 条件 | 条件 | 条件 | 条件 |
| SelfAction | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | eligibility only |
| SelfCritique | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 |
| TimeAdvance | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | no irreversible write |
| AdminAction | 显式 | 显式 | 显式 | 显式 | 显式 | 显式 | 显式 | 显式 | reset/migration only |

“条件”表示还必须满足 confidence、causal binding、责任归因和屈服阈值。

## 12. Wire Envelope

Python/Rust 边界推荐使用 MessagePack 或 canonical JSON 的 closed envelope：

```json
{
  "schema": "astr-embodiment.event.v1",
  "scope": "opaque-token",
  "event_kind": "user_stimulus",
  "event_id": "...",
  "base_revision": 42,
  "payload": {},
  "digest": "..."
}
```

生产实现应限制最大字节数和最大集合长度。
