# Alpha3 Lived World and Native Ecosystem Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship AstrEmbodiment `1.1.0-alpha3` as an AstrBot-native, zero-LLM-by-default lived-world runtime with source-bound interaction facts, relation-local consent/contact, a fixed mixed world, deterministic lived days, real budget/readiness/gate projections, a capability-gated ecosystem proposal boundary, and three authority-separated Pages views.

**Architecture:** Rust Native remains the only recoverable writer. New closed alpha3 contracts live beside the existing v1 contracts; canonical wire v5 and autonomy DB v7 add authority without rewriting v4 bytes or changing existing v1 request/response structs. Python freezes AstrBot facts, performs authenticated controls and the one allowed Provider/dispatch sequence, while Pages and ecosystem integrations consume only Native committed projections or submit bounded proposals.

**Tech Stack:** Rust 2021, serde closed schemas, `ae-fixed` fxp6 values, rusqlite SQLite single-writer transactions, PyO3 0.29, Python 3.12, AstrBot Plugin Pages and `Context.send_message`, vanilla ES modules/CSS, Cargo offline checks, pytest release contracts.

---

## Execution contract

- Repository/worktree: `G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0-worktree`.
- Frozen design: `docs/superpowers/specs/2026-08-29-alpha3-lived-world-native-ecosystem-design.md` at design commit `11d022e8e3c7a77dab86d315c2ecd20992f459ce`.
- Baseline product version: Rust/metadata `1.1.0-alpha2`, Python wheel `1.1.0a2`, canonical wire v4, autonomy DB v6.
- Write order is authority order: Task 1 through Task 7 must land sequentially. A later worker starts from the accepted predecessor commit; no worker substitutes Python dictionaries, browser storage, or an ecosystem database for an unfinished Native authority.
- Keep the old `UserStimulus`, `TimeAdvanceV1`, `ObserveSnapshotV1`, Pages v1 routes, and existing PyO3 symbols callable. New behavior uses independent alpha3 V1 types, `ObserveSnapshotV2`, and one closed `alpha3_call` FFI envelope.
- The v6→v7 migration maps old `proactive_enabled=true` to `pending_reconfirmation`, never to a grant. A false value maps to `disabled`. Legacy `affiliation_need` remains replayable history only and never seeds `contact_due_score` or a cause.
- Default behavior is zero Provider calls: lived-day advance, contact scoring, dream review, ecosystem admission, readiness/gate evaluation, Page queries, and ordinary background wakes are deterministic. A Provider call is reachable only after relation grant, an explicit cause, both Native gates, a budget reservation, and Host capability checks.
- No task starts a real AstrBot server. Local compile/focused-contract evidence earns `COMPILE/FOCUSED PASS`; it does not earn alpha3 Host integration PASS or delivery confirmation.
- Keep build/cache roots below `G:\AstrEmbodiment\.codex-task-temp\alpha3-*`. Do not use network fallback when `--offline` reports a missing crate cache.
- Keep regression work bounded: one focused contract target per authority layer, then one final release-contract run. Do not expand this plan into a broad combinatorial suite.

### Fast Lane coordination

The coordinator owns integration and acceptance. Dispatch only an attested Fast Lane route with exact model/effort, lease, predecessor SHA, worktree, write scope, and bounded context; missing routing evidence is `NO_SAFE_WORK`. Because Tasks 1–5 touch the Native transaction spine, execute them serially even where new leaf files are disjoint. A worker returns its candidate commit, changed paths, exact command results, and blocker classification; it does not self-accept or merge unrelated work.

```text
ALPHA3-01 contracts + wire v5 + DB v7
  -> ALPHA3-02 interaction facts + consent + contact
    -> ALPHA3-03 world anchor + lived day + dream boundary
      -> ALPHA3-04 budget + readiness + gate + projection v2
        -> ALPHA3-05 AstrBot adapters + broker + controls
          -> ALPHA3-06 three Pages layers + i18n + docs
            -> ALPHA3-07 version + package + compile/release gates
```

## File responsibility map

| Path | Alpha3 responsibility |
|---|---|
| `crates/ae-contracts/src/alpha3.rs` | All new closed alpha3 vocabulary; existing v1 structs remain byte/JSON compatible. |
| `crates/ae-contracts/src/lib.rs` | Re-export alpha3 contracts and add only the canonical `interaction_fact_batch` wire-v5 variant. |
| `crates/ae-store/src/alpha3/schema.rs` | DB v7 DDL, bounds, idempotent migration, conservative legacy mapping. |
| `crates/ae-store/src/alpha3/interaction.rs` | Atomic interaction/consent/contact and stable contact-intention persistence. |
| `crates/ae-store/src/alpha3/lived_world.rs` | World anchor, lived-day and dream review persistence. |
| `crates/ae-store/src/alpha3/projection.rs` | Budget/readiness/gate settlement and committed-only v2 projection queries. |
| `crates/ae-store/src/alpha3/ecosystem.rs` | Capability grants and proposal admission/deduplication. |
| `crates/ae-agent/src/contact.rs` | Pure relation-local contact formula and explicit-cause intention construction. |
| `crates/ae-autonomy/src/lived_world.rs` | Pure cross-platform deterministic routine, catch-up, importance, and dream non-fact reducer. |
| `crates/ae-runtime/src/alpha3/` | Orchestrate Native authority transactions; no Host policy or SQL duplication. |
| `crates/ae-pyo3/src/lib.rs` | Existing v1 functions plus one strict `alpha3_call(request_json)` dispatcher. |
| `python/astrembodiment_core/__init__.py` | Export `alpha3_call` while retaining every alpha2 native export. |
| `astr_embodiment/interaction.py` | Freeze normal inbound and exact explicit controls into source-bound batches without message text. |
| `astr_embodiment/ecosystem.py` | Process-local broker adapter that is unavailable without Host-attested caller identity. |
| `astr_embodiment/observatory_controls.py` | Authenticated POST, relation binding, one-use CSRF nonce and idempotency nonce validation. |
| `astr_embodiment/proactive.py` | Provider usage extraction, manipulation-language rejection, v2 settle and dispatch recheck. |
| `astr_embodiment/observatory.py` | Authorize a view before selecting its Native DTO; never filter a developer DTO in the browser. |
| `main.py` | AstrBot lifecycle, first-turn fact barrier, explicit chat commands, Page/broker registration. |
| `pages/observatory/*` | One shared AstrBot Page shell with Experience, Private and Developer layers. |
| `.astrbot-plugin/i18n/*.json` | Neutral, non-anthropomorphic labels and disclosure text for all three layers. |

## Cross-task invariants

1. Every persisted enum is closed and serialized as `snake_case`; every request/record uses `deny_unknown_fields`.
2. Native code strings are at most 64 UTF-8 bytes, event/source vectors are bounded, and no message body, prompt, platform raw ID, Provider secret, candidate text, URL, or dream prose enters the new authority tables.
3. Identity digests use distinct domains for world anchors, interaction sources, contact causes, intentions, ecosystem capability/idempotency, Page public refs, and migration receipts.
4. `external_observed` requires an attested AstrBot/ecosystem source and TTL. Neither `persona_near_real` nor `declared_fantasy` can be promoted to it.
5. One wake commits legacy runtime state, lived-day, contact, consent effects, notable events, intentions and next wake in one Native transaction. Query paths enable SQLite query-only mode and leave revisions, generations, claims and budgets unchanged.
6. `adapter_submitted`, `platform_accepted`, `delivery_confirmed`, and `dispatch_unknown` remain distinct. `Context.send_message(...) == True` proves only adapter submission on the currently verified AstrBot contract.

### Task 1 (ALPHA3-01): Freeze alpha3 contracts, wire v5, and autonomy DB v7

**Files:**
- Create: `crates/ae-contracts/src/alpha3.rs`
- Modify: `crates/ae-contracts/src/lib.rs` (`CanonicalEvent`, `wire::WIRE_SCHEMA_VERSION`, kind/enum codecs)
- Create: `crates/ae-contracts/tests/alpha3_contracts.rs`
- Create: `crates/ae-store/src/alpha3/mod.rs`
- Create: `crates/ae-store/src/alpha3/schema.rs`
- Modify: `crates/ae-store/src/lib.rs` (declare `alpha3` module)
- Modify: `crates/ae-store/src/autonomy.rs` (`AUTONOMY_DB_VERSION`, digest verification, `migrate_autonomy`)
- Modify: `crates/ae-store/tests/autonomy_migration.rs`

- [ ] **Step 1: Add one contract test and one migration fixture**

Create `alpha3_contracts.rs` with one test that round-trips a two-fact `CanonicalEvent::InteractionFactBatch`, rejects a seventeenth fact and an unknown enum, and decodes an existing v4 `TimeAdvanceV1` byte fixture unchanged. Extend `autonomy_migration.rs` with one v6 fixture containing two relation policies (`proactive_enabled` true/false), non-zero legacy affiliation, and a reserved budget.

```rust
#[test]
fn wire_v5_is_closed_and_v4_remains_decodable() {
    let encoded = wire::encode_event(&CanonicalEvent::InteractionFactBatch(batch(2)));
    assert_eq!(&encoded[..2], &5_u16.to_le_bytes());
    assert_eq!(wire::decode_event(&encoded).unwrap(), CanonicalEvent::InteractionFactBatch(batch(2)));
    assert!(matches!(wire::encode_event_checked(&CanonicalEvent::InteractionFactBatch(batch(17))), Err(WireError::TooManyInteractionFacts)));
    assert!(matches!(wire::decode_event(V4_TIME_ADVANCE_FIXTURE).unwrap(), CanonicalEvent::TimeAdvance(_)));
}
```

Name the migration test `alpha3_v6_to_v7`. Its assertion is exact: true → `pending_reconfirmation`, false → `disabled`, both contact scores/cause lists → zero/empty, each persona has one immutable `mixed_main_world` anchor, old reserved tokens become `charged_tokens` with `usage_known=0`, and a second `Store::open` changes no row or digest. In the same test, a malformed v6 policy fixture must fail and retain schema version 6 with no v7 table, proving transaction rollback.

- [ ] **Step 2: Run only the new RED targets**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-01-red'
cargo test --locked --offline -p ae-contracts --test alpha3_contracts
cargo test --locked --offline -p ae-store --test autonomy_migration alpha3_v6_to_v7 -- --exact
```

Expected: the contract target fails because `alpha3`, wire kind 10 and checked bounds do not exist; the migration target fails because the current DB version is 6.

- [ ] **Step 3: Add the complete closed type vocabulary in `alpha3.rs`**

Use `Fixed`, `Digest`, `Id128` and `ScopeRef`; do not put free text in Native state. Define these exact enum families:

```rust
pub const ALPHA3_SCHEMA_VERSION: u16 = 1;
pub const MAX_INTERACTION_FACTS: usize = 16;
pub const MAX_ALPHA3_SOURCE_REFS: usize = 8;

pub enum WorldModeV1 { MixedMainWorld }
pub enum WorldLayerV1 { ExternalObserved, PersonaNearReal, DeclaredFantasy }
pub enum LivedActivityClassV1 { Sleep, PersonalCare, Maintenance, FocusedProject, Learning, Leisure, SocialAvailability, Reflection, Transition }
pub enum LivedSegmentStateV1 { Scheduled, Active, Completed, Superseded }
pub enum LivedGoalClassV1 { MaintainRoutine, AdvanceProject, ExploreInterest, RestoreCapacity, CompleteUserFollowUp, ReflectOnTheme }
pub enum LivedGoalStateV1 { Active, Completed, Blocked, Cancelled }
pub enum LivedGoalOriginV1 { Routine, ExplicitFollowUp, AcceptedEcosystem, DreamNonFactReview }
pub enum LivedNodeImportanceV1 { Routine, Notable, SafetyCritical }
pub enum InteractionFactKindV1 { InboundObserved, FollowUpRequested, FollowUpResolved, BoundarySet, ContactGranted, ContactPaused, ContactResumed, RelationEnded, ExplicitOutcomeReported }
pub enum InteractionSourceAuthorityV1 { ExplicitControl, AstrbotMetadata, DeterministicRule, ModelCandidate }
pub enum RelationConsentStateV1 { Disabled, PendingReconfirmation, Granted, Paused, Ended }
pub enum ContactPurposeV1 { ScheduledCheckIn, ExplicitFollowUp, RepairInvitation }
pub enum ContactChannelV1 { AstrbotSession }
pub enum DreamResidueStateV1 { PendingWakingReview, Rejected, RetainedNonFact, Expired }
pub enum EcosystemCapabilityV1 { ObserveLivedState, ProposeExternalObservation, ProposeActivity, ProposeGoalProgress }
pub enum EcosystemProposalKindV1 { ExternalObservation, ActivityOffer, GoalProgressEvidence }
pub enum EcosystemDecisionV1 { Accepted, Rejected, Deferred }
pub enum GateDecisionKindV2 { Allowed, Suppressed, Deferred }
pub enum ProjectionUnavailableV1 { UnavailableOnHost, NotInitialized, Redacted, Inconsistent }
```

`GateReasonV2` contains `all_requirements_satisfied`, `proactive_disabled`, `consent_required`, `consent_paused`, `relation_ended`, `cause_unavailable`, `budget_unavailable`, `provider_usage_unsettled`, `ecosystem_source_revoked`, `persona_asleep`, `quiet_hours`, `cooldown`, `daily_limit`, `unanswered_hard_stop`, `timezone_unreliable`, `target_unavailable`, `provider_unavailable`, `send_capability_unavailable`, and `residual_rejected`.

Define the following records with the design fields and these clarifications:

```rust
pub struct InteractionFactBatchV1 {
    pub schema_version: u16,
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub causal: CausalRef,
    pub facts: Vec<InteractionFactV1>, // 1..=16
}

pub struct InteractionFactV1 {
    pub fact_id: Id128,
    pub kind: InteractionFactKindV1,
    pub observed_at_utc_ms: u64,
    pub source_authority: InteractionSourceAuthorityV1,
    pub source_digest: Digest,
    pub extractor_digest: Digest,
    pub confidence: Fixed,
    pub value_code: Option<InteractionValueCodeV1>,
    pub subject_public_ref: Option<Digest>,
    pub consent_terms: Option<ConsentTermsV1>,
    pub scheduled_at_utc_ms: Option<u64>,
    pub expires_at_utc_ms: Option<u64>,
}

pub struct ConsentTermsV1 {
    pub purposes: Vec<ContactPurposeV1>,
    pub channels: Vec<ContactChannelV1>,
    pub valid_until_utc_ms: Option<u64>,
    pub pause_until_utc_ms: Option<u64>,
}
```

`InteractionValueCodeV1` is closed to `follow_up`, `follow_up_resolved`, `no_contact_boundary`, `grant`, `pause`, `resume`, `end`, `outcome_positive`, `outcome_neutral`, and `outcome_negative`. Validate field combinations by `kind`: only a grant carries non-empty consent terms, only follow-up may carry schedule/expiry, and `model_candidate` can only be `inbound_observed` with no value/control terms.

Also define the exact design records below. Every `Vec` shown is validated against its stated bound before hashing or persistence:

```rust
pub struct WorldAnchorV1 {
    pub schema_version: u16,
    pub world_anchor_id: Id128,
    pub persona_scope: Digest,
    pub mode: WorldModeV1,
    pub home_context_ref: Digest,
    pub lore_manifest_digest: Digest,
    pub reality_policy_digest: Digest,
    pub allowed_layers: Vec<WorldLayerV1>, // exactly the three unique values
    pub created_from_event_id: Id128,
    pub revision: u64, // exactly 1 in alpha3
}

pub struct LivedActivitySegmentV1 {
    pub segment_id: Id128,
    pub world_layer: WorldLayerV1,
    pub activity_class: LivedActivityClassV1,
    pub starts_at_utc_ms: u64,
    pub ends_at_utc_ms: u64,
    pub state: LivedSegmentStateV1,
    pub importance: LivedNodeImportanceV1,
    pub source_event_ids: Vec<Id128>, // <=8
}

pub struct LivedGoalV1 {
    pub goal_id: Id128,
    pub goal_class: LivedGoalClassV1,
    pub state: LivedGoalStateV1,
    pub progress: Fixed, // [0,1]
    pub due_at_utc_ms: Option<u64>,
    pub world_layer: WorldLayerV1,
    pub origin: LivedGoalOriginV1,
    pub source_event_ids: Vec<Id128>, // <=8
}

pub struct LivedDayStateV1 {
    pub schema_version: u16,
    pub persona_scope: Digest,
    pub world_anchor_id: Id128,
    pub persona_day_ordinal: i32,
    pub revision: u64,
    pub current_segment: LivedActivitySegmentV1,
    pub active_goal: Option<LivedGoalV1>,
    pub next_transition_utc_ms: u64,
    pub routine_formula_digest: Digest,
    pub source_event_ids: Vec<Id128>, // <=8
}

pub struct RelationConsentV1 {
    pub schema_version: u16,
    pub relation_scope: Digest,
    pub consent_epoch: u64,
    pub revision: u64,
    pub state: RelationConsentStateV1,
    pub purposes: Vec<ContactPurposeV1>,
    pub channels: Vec<ContactChannelV1>,
    pub valid_from_utc_ms: u64,
    pub valid_until_utc_ms: Option<u64>,
    pub pause_until_utc_ms: Option<u64>,
    pub source_event_id: Id128,
    pub policy_digest: Digest,
}

pub struct RelationContactProcessV1 {
    pub schema_version: u16,
    pub relation_scope: Digest,
    pub revision: u64,
    pub last_inbound_utc_ms: Option<u64>,
    pub last_outbound_submitted_utc_ms: Option<u64>,
    pub response_cadence_ema_ms: Option<u64>,
    pub response_cadence_variation: Fixed,
    pub contact_due_score: Fixed,
    pub unfinished_follow_up_salience: Fixed,
    pub repetition_penalty: Fixed,
    pub consecutive_unanswered: u16,
    pub next_contact_eligible_utc_ms: Option<u64>,
    pub active_cause_digest: Option<Digest>,
    pub active_source_event_ids: Vec<Id128>, // <=8
    pub formula_digest: Digest,
}

pub struct DreamResidueV1 {
    pub schema_version: u16,
    pub residue_id: Id128,
    pub persona_scope: Digest,
    pub state: DreamResidueStateV1,
    pub imagery_tags: Vec<DreamImageryCodeV1>,
    pub affect_afterglow: DreamAffectAfterglowV1,
    pub reflection_theme_codes: Vec<ReflectionThemeCodeV1>,
    pub source_event_ids: Vec<Id128>, // <=8
    pub created_at_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub reviewed_at_utc_ms: Option<u64>,
    pub review_event_id: Option<Id128>,
    pub non_fact: bool, // validator requires true
}
```

`DreamImageryCodeV1` and `ReflectionThemeCodeV1` contain a small closed alpha3 vocabulary, and `DreamReviewActionV1` is `reject | retain_non_fact | expire`. `DreamAffectAfterglowV1` has `valence: Fixed` in `[-1,1]` and `arousal: Fixed` in `[0,1]`. No dream type has a text field.

Define ecosystem payloads as tagged closed variants rather than maps:

```rust
pub struct EcosystemCapabilityGrantV1 {
    pub schema_version: u16,
    pub plugin_instance_digest: Digest,
    pub plugin_name_digest: Digest,
    pub plugin_version_digest: Digest,
    pub manifest_digest: Digest,
    pub capability_digest: Digest,
    pub capabilities: Vec<EcosystemCapabilityV1>,
    pub state: CapabilityGrantStateV1, // active | revoked | expired
    pub valid_until_utc_ms: Option<u64>,
    pub revision: u64,
    pub source_event_id: Id128,
}

pub struct EcosystemObservationV1 {
    pub schema_version: u16,
    pub observation_id: Id128,
    pub persona_public_handle: Digest,
    pub as_of_canonical_revision: u64,
    pub world_anchor_public_ref: Digest,
    pub current_world_layer: WorldLayerV1,
    pub persona_local_minute: u16,
    pub sleep_state: SleepStateV1,
    pub lived_activity_class: LivedActivityClassV1,
    pub active_goal_class: Option<LivedGoalClassV1>,
    pub allowed_proposal_kinds: Vec<EcosystemProposalKindV1>,
    pub capability_digest: Digest,
    pub expires_at_utc_ms: u64,
}

pub struct EcosystemProposalV1 {
    pub schema_version: u16,
    pub proposal_id: Id128,
    pub plugin_instance_digest: Digest,
    pub capability_digest: Digest,
    pub observation_id: Id128,
    pub expected_canonical_revision: u64,
    pub kind: EcosystemProposalKindV1,
    pub world_layer: WorldLayerV1,
    pub valid_from_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub payload: EcosystemProposalPayloadV1,
    pub source_digest: Digest,
    pub semantic_idempotency_digest: Digest,
}

pub enum EcosystemProposalPayloadV1 {
    ExternalObservation { class: ExternalObservationClassV1, value: ExternalObservationValueV1, measurement: Option<BoundedMeasurementV1>, observed_at_utc_ms: u64 },
    ActivityOffer { activity_class: LivedActivityClassV1, goal_class: Option<LivedGoalClassV1>, duration_minutes: u16 },
    GoalProgressEvidence { goal_public_ref: Digest, progress_delta: Fixed },
}
```

`ExternalObservationClassV1`, `ExternalObservationValueV1` and `MeasurementUnitV1` are closed enums for the alpha3 weather/calendar/home/work observation set. `BoundedMeasurementV1` is only `{ value: Fixed, unit: MeasurementUnitV1 }`. Payload kind and enum variant must agree; there is no string, URL or arbitrary JSON field.

Add these integration records so later tasks do not alter v1 structs:

```rust
pub struct ContactIntentionBasisV1 {
    pub intention_id: Id128,
    pub relation_scope: Digest,
    pub purpose: ContactPurposeV1,
    pub cause_digest: Digest,
    pub cause_public_refs: Vec<Digest>,
    pub consent_epoch: u64,
    pub consent_revision: u64,
    pub created_at_utc_ms: u64,
    pub expires_at_utc_ms: u64,
    pub live: bool,
}

pub struct Alpha3WakeProposalV1 {
    pub legacy: WakeProposalV1,
    pub contact_updates: Vec<RelationContactProcessV1>,
    pub contact_bases: Vec<ContactIntentionBasisV1>,
    pub lived_day: Option<LivedDayStateV1>,
    pub world_anchor: Option<WorldAnchorV1>,
    pub dream_updates: Vec<DreamResidueV1>,
}

pub struct WakeClaimV2 {
    pub claim_token: Digest,
    pub event: TimeAdvanceV1,
    pub proposal: Alpha3WakeProposalV1,
    pub lease_deadline_utc_ms: u64,
}
```

Budget, readiness, gates and projection types are exact and independent of the v1 DTOs:

```rust
pub struct RelationBudgetPolicyV1 { pub schema_version: u16, pub relation_scope: Digest, pub timezone_id: String, pub daily_token_limit: u64, pub revision: u64, pub source_digest: Digest }
pub struct RelationBudgetLedgerV1 { pub relation_scope: Digest, pub day_start_utc_ms: u64, pub limit_tokens: u64, pub reserved_tokens: u64, pub charged_tokens: u64, pub used_tokens: u64, pub usage_known: bool, pub revision: u64 }
pub struct ProviderUsageV1 { pub known: bool, pub used_tokens: Option<u64> }
pub struct ExternalizationSettleV2 { pub claim_token: Digest, pub outcome: ExternalizationOutcomeV1, pub provider_usage: ProviderUsageV1, pub candidate_digest: Option<Digest>, pub candidate_ciphertext: Option<Vec<u8>>, pub caller_incarnation: Digest }

pub enum ReadinessItemKindV1 { GlobalSwitch, RelationConsent, TrustedTimezone, TargetEnvelope, SecretStore, Provider, AstrbotSend, Budget, SleepQuietHours, PolicyRevision }
pub enum ReadinessItemStatusV1 { Ready, UnavailableOnHost, NotInitialized, Stale, Revoked, Blocked }
pub struct ReadinessItemV1 { pub kind: ReadinessItemKindV1, pub status: ReadinessItemStatusV1, pub witness_revision: u64 }
pub struct ProactiveReadinessV1 { pub schema_version: u16, pub relation_scope: Digest, pub revision: u64, pub evaluated_at_utc_ms: u64, pub items: Vec<ReadinessItemV1>, pub witness_digest: Digest }

pub struct GateDecisionV2 { pub decision: GateDecisionKindV2, pub reason: GateReasonV2, pub evaluated_at_utc_ms: u64, pub retry_at_utc_ms: Option<u64>, pub consent_epoch: u64, pub consent_revision: u64, pub policy_revision: u64, pub budget_day_start_utc_ms: u64, pub intention_public_ref: Digest, pub cause_public_refs: Vec<Digest>, pub capability_snapshot_digest: Digest }
```

`ProjectionFieldV1<T>` is a tagged generic with `available(T)`, `unavailable_on_host`, `not_initialized`, `redacted`, and `inconsistent`. `ObserveSnapshotRequestV2` contains schema, persona scope, optional relation scope, committed-only mode and `Experience | Private | Developer`. `ObserveSnapshotV2` contains schema, generated UTC, one canonical high-water and a tagged `ExperienceProjectionV2 | PrivateProjectionV2 | DeveloperProjectionV2`.

Experience fields are world/lived/goal/sleep/notable/contact explanation/disclosure. Private fields are consent/contact/budget/why-contacted/dream/correction-export-delete status. Developer fields are build/schema/formula/revisions/projection health/readiness/gate plus bounded `PublicIntentionV2`, `PublicOutboundV2` and `PublicClaimV2`; these public types contain only domain-separated public refs, closed states/times/revisions and never internal IDs or ciphertext.

Define `Alpha3ErrorCodeV1` with exactly `schema_unsupported`, `scope_mismatch`, `source_untrusted`, `capability_denied`, `world_anchor_immutable`, `world_layer_forbidden`, `consent_required`, `consent_revoked`, `proposal_expired`, `proposal_stale`, `duplicate_proposal`, `budget_exhausted`, `readiness_unavailable`, `dream_not_reviewed`, `projection_incomplete`, and `migration_failed`.

Define `Alpha3RequestV1` and `Alpha3ResponseV1` as closed tagged enums with matching variants for apply interaction, wake-v2 claim/settle, budget policy, externalization/dispatch gate-and-claim/settle, snapshot v2, conversation context, dream review, ecosystem capability grant/observe/propose, and consent control. Every response variant carries either its exact DTO or `{ code: Alpha3ErrorCodeV1, retry_at_utc_ms: Option<u64> }`; it never returns an untyped JSON map.

- [ ] **Step 4: Add wire kind 10 without rewriting old bytes**

In `CanonicalEvent`, append `InteractionFactBatch(InteractionFactBatchV1)`; its event authority is `UserObserved`, while Task 2 still validates each fact's finer authority before mutation. Set `WIRE_SCHEMA_VERSION=5` and `KIND_INTERACTION_FACT_BATCH=10`. Existing kinds keep their numeric codes and existing v1–v4 decoding branches.

Add `encode_event_checked(&CanonicalEvent) -> Result<Vec<u8>, WireError>` for bounded/untrusted ingress; keep the existing `encode_event(&CanonicalEvent) -> Vec<u8>` signature as a compatibility wrapper that calls the checked encoder and treats failure as an internal pre-validation violation. All new FFI/runtime ingress validates and uses the checked form before persistence. Add `WireError::TooManyInteractionFacts`.

The v5 encoder writes schema, kind, event/scope/causal, a `u8` fact count, then each fixed-layout fact. Enum codes are explicit and never derived from Rust discriminants. Optional terms use a presence byte; vectors use a checked `u8` count; code strings use the existing length-prefixed codec plus a 64-byte alpha3 bound. `decode_event` accepts schema `1..=5`, permits kind 10 only at schema 5, and calls `Reader::finish()`.

```rust
match event {
    CanonicalEvent::InteractionFactBatch(batch) => {
        ensure_interaction_batch_bounds(batch)?;
        push_u8(&mut out, KIND_INTERACTION_FACT_BATCH);
        encode_interaction_batch(&mut out, batch);
    }
    // existing arms remain layout-identical
}
```

- [ ] **Step 5: Migrate v6 to v7 atomically and conservatively**

Keep both `MIGRATION_DIGEST_V6` and `MIGRATION_DIGEST_V7`; verify the installed v6 digest before any v7 DDL. `schema::migrate_alpha3_v7(tx, from_version)` creates the eight design tables plus `relation_consent_head`, `contact_intention_basis`, `relation_budget_policy`, `proactive_readiness`, and `gate_decision_latest`. Every body row has a matching scope/revision column, JSON byte bound, unique/indexed identity, and a validation pass before commit.

Migration pseudo-flow:

```rust
for persona_scope in distinct_existing_personas(tx)? {
    insert_world_anchor_if_absent(tx, migration_world_anchor(persona_scope))?;
}
for policy in legacy_relation_policies(tx)? {
    let state = if policy.proactive_enabled {
        RelationConsentStateV1::PendingReconfirmation
    } else {
        RelationConsentStateV1::Disabled
    };
    insert_consent_epoch_one(tx, policy.relation_scope, state)?;
    insert_contact_zero(tx, policy.relation_scope, policy.last_inbound_utc_ms)?;
}
charge_unknown_legacy_reservations(tx)?; // reserved=0, charged+=old, usage_known=false
verify_v7_rows_and_indexes(tx)?;
insert_migration_receipt(tx, 7, MIGRATION_DIGEST_V7)?;
```

The anchor ID, empty lore digest, reality-policy digest and migration source event ID are domain-separated hashes of the persona scope and fixed policy/version bytes. Do not backfill activity history, dream residue, relationship quality, outcome facts, ecosystem proposals, goals, or contact causes. An older binary sees version 7 and returns `database is newer than binary` before write.

- [ ] **Step 6: Compile and run the two focused GREEN targets, then commit**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-01'
cargo check --locked --offline -p ae-contracts -p ae-store
cargo test --locked --offline -p ae-contracts --test alpha3_contracts
cargo test --locked --offline -p ae-store --test autonomy_migration alpha3_v6_to_v7 -- --exact
git diff --check
git add crates/ae-contracts/src/alpha3.rs crates/ae-contracts/src/lib.rs crates/ae-contracts/tests/alpha3_contracts.rs crates/ae-store/src/alpha3 crates/ae-store/src/lib.rs crates/ae-store/src/autonomy.rs crates/ae-store/tests/autonomy_migration.rs
git commit -m "feat: add alpha3 authority contracts and v7 migration"
```

Expected: all commands exit 0; the migration is repeatable, failure injection rolls the whole v7 transaction back, and the commit contains only Task 1 paths.

### Task 2 (ALPHA3-02): Make interaction, consent, and contact relation-local authority

**Files:**
- Create: `crates/ae-agent/src/contact.rs`
- Modify: `crates/ae-agent/src/lib.rs` (export contact reducer)
- Create: `crates/ae-store/src/alpha3/interaction.rs`
- Modify: `crates/ae-store/src/alpha3/mod.rs`
- Modify: `crates/ae-store/src/autonomy.rs` (shared wake transaction accepts `Alpha3WakeProposalV1`)
- Create: `crates/ae-runtime/src/alpha3/mod.rs`
- Create: `crates/ae-runtime/src/alpha3/interaction.rs`
- Modify: `crates/ae-runtime/src/lib.rs` (declare alpha3 module)
- Modify: `crates/ae-runtime/src/autonomy.rs` (`claim_wake` v1 safety compatibility)
- Create: `crates/ae-runtime/tests/alpha3_contact.rs`

- [ ] **Step 1: Add one end-to-end Native contact contract**

The single focused test creates two relations for one persona and proves: ordinary inbound commits `inbound_observed` and atomically zeros only that relation's due/unanswered; arbitrarily long silence with no cause produces no intention; an explicit follow-up plus a grant creates exactly one stable live intention after eligibility; a second wake with the same `(relation,purpose,cause)` reuses the semantic ID; pause suppresses it; end terminates the epoch; model-candidate grant is rejected with `source_untrusted`.

```rust
assert_eq!(r1.contact_due_score, Fixed::ZERO);
assert_eq!(r1.consecutive_unanswered, 0);
assert_eq!(r2, r2_before);
assert!(wake_without_cause.intentions.is_empty());
assert_eq!(wake_with_follow_up.intentions.len(), 1);
assert_eq!(wake_again.intentions[0].intention_id, wake_with_follow_up.intentions[0].intention_id);
```

- [ ] **Step 2: Verify the focused target is RED**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-02-red'
cargo test --locked --offline -p ae-runtime --test alpha3_contact
```

Expected: compilation fails because the contact reducer and alpha3 runtime transaction do not exist.

- [ ] **Step 3: Implement the pure relation contact reducer**

`contact.rs` exposes `reduce_interaction_v1`, `advance_contact_due_v1`, and `form_contact_candidate_v1`. Use integer/fixed arithmetic only:

```text
inbound interval = observed_at - previous_last_inbound
cadence EMA       = old * 3/4 + interval * 1/4       (first interval initializes)
normalized error  = abs(interval - new_ema) / max(new_ema, 1 ms)
variation EMA     = clamp(old * 3/4 + error * 1/4, 0, 1)
inbound effect    = due=0, unanswered=0, last_inbound=observed_at

eligibility = scheduled_at
           or cause_time + max(min_cooldown, cadence_ema_or_unanswered_backoff)
due before eligibility = 0
due after eligibility  = clamp(overdue / max(cadence_ema_or_backoff, 1 hour), 0, 1)
```

Only `follow_up_requested` or an explicitly selected scheduled check-in/repair source creates `active_cause_digest`. Resolve, expiry or end clears it. Silence changes timing cost after a cause exists; it never creates a cause or consent. Candidate eligibility additionally requires granted consent, matching purpose/channel, valid epoch/time, due threshold, repetition limit and an active source list of at most eight same-relation facts.

Construct the semantic digest from `ae.contact-intention.semantic.v1`, relation scope, purpose code, cause digest and consent epoch; do not include wake revision. The existing `DurableIntentionV1` remains the outbox state record, with `action_class` restricted to the three purpose codes; `ContactIntentionBasisV1` is the typed immutable cause/consent sidecar.

- [ ] **Step 4: Commit fact, consent, contact, event, and intention changes in Native transactions**

`Store::apply_interaction_fact_batch_v1` validates relation scope, `causal.base_revision`, unique fact IDs, times, confidence, source combinations and same-relation subject refs before opening `BEGIN IMMEDIATE`. Within that transaction it inserts the canonical journal row and bounded facts, advances consent/contact, expires or suppresses affected live intentions, writes one notable/safety `InnerEventV1`, and updates the canonical high-water. Duplicate event/fact digests return the original receipt; a digest collision fails closed.

Consent transitions are exact:

```text
disabled|pending_reconfirmation -- explicit grant --> granted (same epoch)
granted -- explicit pause --> paused
paused -- explicit resume with unexpired same terms --> granted
any non-ended -- explicit end --> ended, revoke target, suppress not-started work
ended -- explicit grant --> new epoch starting at previous epoch + 1
ordinary inbound --> no consent transition
model_candidate --> no consent/boundary/follow-up/outcome transition
```

Refactor the existing wake commit once so `commit_autonomous_wake` delegates to a transaction helper and `commit_alpha3_wake` adds contact/basis rows before the same commit. Add `AstrRuntime::claim_wake_v2`/`settle_wake_v2` in `alpha3/mod.rs`; Task 3 will populate the already-defined lived fields.

For v1 safety compatibility, remove the `elapsed_since_inbound → affiliation_need → relationship_connection` branch from `AstrRuntime::claim_wake`. Leave legacy state fields decodable and replayable, but set no new contact intention from `affiliation_need`; v1 callers continue to receive a valid wake with zero silence-created outreach.

- [ ] **Step 5: Compile, run the focused contract, and commit**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-02'
cargo check --locked --offline -p ae-agent -p ae-store -p ae-runtime
cargo test --locked --offline -p ae-runtime --test alpha3_contact
git diff --check
git add crates/ae-agent/src/contact.rs crates/ae-agent/src/lib.rs crates/ae-store/src/alpha3/interaction.rs crates/ae-store/src/alpha3/mod.rs crates/ae-store/src/autonomy.rs crates/ae-runtime/src/alpha3 crates/ae-runtime/src/lib.rs crates/ae-runtime/src/autonomy.rs crates/ae-runtime/tests/alpha3_contact.rs
git commit -m "feat: make contact consent and causes relation local"
```

Expected: compile and the single contract exit 0; no test asserts affection, loneliness or silence-created cause.

### Task 3 (ALPHA3-03): Add the fixed world, deterministic lived day, and dream non-fact boundary

**Files:**
- Create: `crates/ae-autonomy/src/lived_world.rs`
- Modify: `crates/ae-autonomy/src/lib.rs` (export lived-world reducer)
- Create: `crates/ae-store/src/alpha3/lived_world.rs`
- Modify: `crates/ae-store/src/alpha3/mod.rs`
- Create: `crates/ae-runtime/src/alpha3/lived_world.rs`
- Modify: `crates/ae-runtime/src/alpha3/mod.rs` (fill lived/world/dream fields in wake v2)
- Create: `crates/ae-runtime/tests/alpha3_lived_world.rs`

- [ ] **Step 1: Add one deterministic lived-world and dream-boundary contract**

One test vector runs the same persona/world/day/frozen-time inputs through two fresh runtimes and a replay; assert equal world anchor, segment IDs, goal IDs, routine formula digest, transition times and catch-up summary. Advance 24 hours and assert the reducer exposes no Provider/Host callback. Insert one pending dream fixture through a test-only store helper and prove it cannot change goal/contact/intention/conversation context/outbox before review; after retained review it can create only a `reflect_on_theme` goal with `DreamNonFactReview` origin.

```rust
assert_eq!(first.lived_day, replayed.lived_day);
assert_eq!(first.lived_day.routine_formula_digest, replayed.lived_day.routine_formula_digest);
assert_eq!(first.lived_day.current_segment.activity_class, LivedActivityClassV1::PersonalCare);
assert!(pending_effects.contact_bases.is_empty());
assert_eq!(retained_goal.goal_class, LivedGoalClassV1::ReflectOnTheme);
assert_eq!(retained_goal.origin, LivedGoalOriginV1::DreamNonFactReview);
```

- [ ] **Step 2: Run the RED target**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-03-red'
cargo test --locked --offline -p ae-runtime --test alpha3_lived_world
```

Expected: compilation fails because the lived-world reducer and persisted projections are absent.

- [ ] **Step 3: Implement the pure B+C lived-day reducer**

`advance_lived_day_v1` takes committed temporal profile, immutable world anchor, prior lived state, accepted unexpired activity offers, active explicit follow-up source, sleep state, frozen time and SeedCode-derived digest. Selection order is sleep, due explicit follow-up, accepted proposal, routine template, then deterministic leisure/transition.

Build the awake template relative to `preferred_wake_local_minute`: 60 minutes personal care; 180 focused-project/learning; 60 maintenance; 180 learning/leisure; 60 social-availability; then leisure/transition until sleep. Select each `A|B` choice by `domain_hash("ae.lived-day.choice.v1", persona_scope, world_anchor_id, day_ordinal, slot_index, seed_digest) % variant_count`. Truncate from the tail when the awake window is shorter; never overlap sleep.

Segment/goal IDs use semantic inputs, not loop ordinal or process time. A long catch-up emits at most 64 explicit transitions; beyond that compute the closed-form current slot and append one `lived_day_catch_up` event carrying start/end and skipped count. Importance is routine for ordinary transitions, notable for day/goal/sleep/interaction/proposal, and safety-critical for consent/boundary/budget/dispatch/source conflicts.

`conversation_context_v1(mode)` returns only committed segment, goal and respectively 1/3/8 notable nodes under a fixed byte limit. `inner_activity_display=false` affects only Experience projection and never this reducer. No function in this module imports or accepts Provider, prompt, network, clock, random, or ecosystem callback objects.

- [ ] **Step 4: Persist immutable anchors, lived state, and dream review in the wake transaction**

For fresh personas, `ensure_world_anchor_v1` creates the same `mixed_main_world` shape as migration using committed persona/timezone refs and stable empty-lore/reality-policy digests. Any ordinary attempt to update an existing anchor returns `world_anchor_immutable`.

Validate every segment/goal/proposal layer. `external_observed` requires an unexpired attested source; fantasy/near-real never override it. Store no activity/goal label text—Host/i18n maps the closed codes.

Normal production has no `create_dream_residue` FFI, Host method or ecosystem capability. `review_dream_residue_v1` is Native-only and accepts only an awake state plus `reject`, `expire`, or `retain_non_fact`; `non_fact` must be true, tags/themes and sources are bounded, and retention can feed only a reflection goal. It cannot populate contact cause, interaction fact, prompt context, memory, world anchor, Persona, intention or outbox.

Extend `claim_wake_v2` to compute the world/lived contribution and pending dream review before the existing `commit_alpha3_wake` transaction. A fact-driven `complete_user_follow_up` goal creation/completion now occurs in that same transaction from the canonical interaction source retained by Task 2.

- [ ] **Step 5: Compile, run the focused vector, and commit**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-03'
cargo check --locked --offline -p ae-autonomy -p ae-store -p ae-runtime
cargo test --locked --offline -p ae-runtime --test alpha3_lived_world
git diff --check
git add crates/ae-autonomy/src/lived_world.rs crates/ae-autonomy/src/lib.rs crates/ae-store/src/alpha3/lived_world.rs crates/ae-store/src/alpha3/mod.rs crates/ae-runtime/src/alpha3/lived_world.rs crates/ae-runtime/src/alpha3/mod.rs crates/ae-runtime/tests/alpha3_lived_world.rs
git commit -m "feat: add deterministic lived world authority"
```

Expected: all commands exit 0; the checked vector is deterministic and the dream fixture proves zero pre-review influence.

### Task 4 (ALPHA3-04): Wire real budget, readiness, gate decisions, and `ObserveSnapshotV2`

**Files:**
- Create: `crates/ae-store/src/alpha3/projection.rs`
- Modify: `crates/ae-store/src/alpha3/mod.rs`
- Modify: `crates/ae-store/src/autonomy.rs` (v1 gate/settlement delegates and neutral compatibility behavior)
- Create: `crates/ae-runtime/src/alpha3/projection.rs`
- Modify: `crates/ae-runtime/src/alpha3/mod.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs` (v1 compatibility adapters)
- Modify: `crates/ae-pyo3/src/lib.rs` (one closed alpha3 dispatcher)
- Modify: `python/astrembodiment_core/__init__.py` (export `alpha3_call`)
- Create: `crates/ae-runtime/tests/alpha3_projection.rs`

- [ ] **Step 1: Add one focused budget/gate/projection contract**

The test configures a 100-token relation-local day, grants one explicit follow-up, and verifies both settlement branches: reserve 40 then settle known usage 17 → `reserved=0, charged=17, used=17, usage_known=true`; reserve 40 then settle without usage → `reserved=0, charged=57, used=17, usage_known=false`. An unsettled reservation returns `provider_usage_unsettled`; another claim cannot exceed the 100-token limit.

In the same target, read Experience, Private and Developer snapshots before/after state hashes. Assert one canonical high-water, no query mutation, real live intention/outbound/claim counts, current consent/contact/budget/readiness/latest gate, and no raw UMO, prompt, ciphertext, Provider identifier, secret or reversible user ID in serialized output. Requesting a field unavailable on the current Host must return an explicit unavailable tag rather than null/zero/delivery success.

```rust
let before = authoritative_fingerprint(&runtime);
let private = runtime.observe_snapshot_v2(private_request()).unwrap();
let after = authoritative_fingerprint(&runtime);
assert_eq!(before, after);
assert_eq!(private.budget.value().charged_tokens, 57);
assert!(!private.budget.value().usage_known);
assert!(!serde_json::to_string(&private).unwrap().contains("umo_ciphertext"));
```

- [ ] **Step 2: Run the RED target**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-04-red'
cargo test --locked --offline -p ae-runtime --test alpha3_projection
```

Expected: compilation fails because budget policy, gate v2 and snapshot v2 execution are not implemented.

- [ ] **Step 3: Make the relation-local token ledger authoritative**

`Store::upsert_relation_budget_policy_v1` accepts the Host-frozen `_conf_schema.json.inner_activity_token_daily_max`, relation timezone, source digest and monotonic revision. It never reads Python logs or Page configuration during a gate.

`gate_and_claim_externalization_v2` performs, in one immediate transaction: expire stale intention/basis; load consent/contact/cause; compute readiness; persist GateDecisionV2; reject if any gate fails; otherwise reserve `max_tokens` in the relation's frozen local day and insert the claim. `settle_externalization_v2` releases the reservation and charges exactly Provider usage when known, otherwise the full reservation. Validate `ProviderUsageV1` as `(known=true, used_tokens=Some(n))` or `(known=false, used_tokens=None)`; every other combination is `schema_unsupported`.

```text
available = limit - charged - reserved
claim allowed iff request <= available and no unsettled claim for relation
known settle   => charged += used; used += used; reserved -= reservation
unknown settle => charged += reservation; reserved -= reservation; usage_known=false
```

The day key is `FrozenTimeInputV1.budget_day_start_utc_ms`; on rollover, retain old ledgers for audit and create the new day from the current Native policy. Lived-day, Page reads, ecosystem gates, dream review and ordinary wakes never reserve budget.

Keep existing v1 methods callable. On v7 they delegate to v2 with conservative compatibility: no relation grant/cause means suppression; `ExternalizationSettleV1.used_tokens=None` means unknown/full charge. Do not reinterpret null as zero usage.

- [ ] **Step 4: Persist readiness and both gate evaluations**

Build `ProactiveReadinessV1` from ten typed items: global switch, relation consent, trusted timezone, target envelope, secret store, Provider, AstrBot send capability, budget, sleep/quiet-hours, and current policy revisions. Host-owned facts arrive only in a digest-verified frozen witness; Native combines them with its stored authority and assigns the readiness revision.

Before externalization and again before dispatch, write `GateDecisionV2` with decision/reason, evaluation/retry time, consent epoch/revision, policy revision, budget day, intention public ref, cause public refs and capability snapshot digest. Re-read consent, cause, target generation, budget and readiness in the dispatch transaction. A pause/end after claim cancels work that has not called the adapter; if the adapter call started, retain `dispatch_unknown` and conservative settlement.

Candidate uniqueness remains `(relation,purpose,cause)`; expiry atomically marks intention and sidecar non-live. Why-contacted data is projected only from this committed basis plus consent revision and latest gate.

- [ ] **Step 5: Implement three Native DTO variants and the strict FFI envelope**

`Store::observe_snapshot_v2` opens query-only mode, resolves relation ownership supplied by the caller, pins one canonical high-water and selects exactly one tagged variant:

- Experience: current segment/goal, sleep proxy, world layer, bounded notable nodes and neutral contact timing explanation; no fixed raw values or digests.
- Private: consent epoch/state/terms/validity, contact due/follow-up/repetition/unanswered, why-contacted public refs, budget, dream non-fact rows and correction/export/delete status.
- Developer: schema/formula/build digests, revisions, projection health, readiness witnesses, latest gate, intention/outbound/claim stages, migration diagnostics and legacy affiliation only as an explicitly labeled migration field.

If one required field is inconsistent, return the entire requested layer as `inconsistent`; never splice cache/state from different revisions. Keep `observe_snapshot_v1` callable, but alpha3 Pages do not use its `affiliation_need`/`想念` presentation.

Add one PyO3 function:

```rust
#[pyfunction]
fn alpha3_call(py: Python<'_>, request_json: &str) -> PyResult<String> {
    let request: Alpha3RequestV1 = serde_json::from_str(request_json).map_err(closed_schema)?;
    let response = py.detach(|| {
        let mut guard = core()?;
        let runtime = guard
            .as_mut()
            .ok_or_else(|| NativeCoreError::new_err("CLOSED::native core is not open"))?;
        runtime.handle_alpha3(request).map_err(map_error)
    })?;
    serde_json::to_string(&response)
        .map_err(|error| NativeCoreError::new_err(format!("ENCODING::{error}")))
}
```

`handle_alpha3` exhaustively matches the closed operation enum; operations whose authority layer has not landed return a closed unavailable/error response. Register/export `alpha3_call` without removing or renaming any existing native symbol.

- [ ] **Step 6: Compile, run the focused target, and commit**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-04'
cargo check --locked --offline -p ae-store -p ae-runtime -p ae-pyo3
cargo test --locked --offline -p ae-runtime --test alpha3_projection
$env:PYTHONPYCACHEPREFIX='G:\AstrEmbodiment\.codex-task-temp\alpha3-pycache\task-04'
py -3.12 -m compileall -q python\astrembodiment_core
git diff --check
git add crates/ae-store/src/alpha3/projection.rs crates/ae-store/src/alpha3/mod.rs crates/ae-store/src/autonomy.rs crates/ae-runtime/src/alpha3/projection.rs crates/ae-runtime/src/alpha3/mod.rs crates/ae-runtime/src/autonomy.rs crates/ae-pyo3/src/lib.rs python/astrembodiment_core/__init__.py crates/ae-runtime/tests/alpha3_projection.rs
git commit -m "feat: expose authoritative alpha3 gates and projections"
```

Expected: commands exit 0; v1 symbols still compile, v2 reads are query-only, and budget unknown usage is conservatively charged.

### Task 5 (ALPHA3-05): Connect AstrBot facts, controls, Provider settlement, and ecosystem broker

**Files:**
- Create: `crates/ae-store/src/alpha3/ecosystem.rs`
- Modify: `crates/ae-store/src/alpha3/mod.rs`
- Create: `crates/ae-runtime/src/alpha3/ecosystem.rs`
- Modify: `crates/ae-runtime/src/alpha3/mod.rs`
- Create: `crates/ae-runtime/tests/alpha3_ecosystem.rs`
- Create: `astr_embodiment/interaction.py`
- Create: `astr_embodiment/ecosystem.py`
- Create: `astr_embodiment/observatory_controls.py`
- Modify: `astr_embodiment/contracts.py` (closed Host payload builders)
- Modify: `astr_embodiment/bridge.py` (`alpha3_call` plus typed convenience wrappers)
- Modify: `astr_embodiment/autonomy.py` (wake-v2 supervisor and synchronous fact wake notification)
- Modify: `astr_embodiment/proactive.py` (usage and content safety)
- Modify: `main.py` (first-turn barrier, commands, controls, broker lifecycle)
- Modify: `_conf_schema.json` (control/developer switches and budget wording)
- Create: `tests/test_alpha3_host_boundaries.py`

- [ ] **Step 1: Add two narrow boundary tests**

The Rust test denies observation/proposal without a live capability; accepts one attested `activity_offer`; rejects stale revision, duplicate idempotency, forbidden world-layer promotion and every world/consent/dream/outbox/provider mutation kind; and proves an accepted proposal is still only a source until the next Native lived-day decision.

The Python test proves: every ordinary inbound emits one message-free `inbound_observed` batch before `UserStimulus`; exact stop/pause/end phrases can only pause/end; no phrase grants; Page controls require authenticated user, matching plugin/relation handle, configured admin, one-use CSRF nonce and idempotency nonce; absent Host caller identity leaves broker unavailable; known/unknown Provider usage maps exactly; manipulative candidate markers fail before encryption/dispatch.

- [ ] **Step 2: Run the RED targets**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-05-red'
cargo test --locked --offline -p ae-runtime --test alpha3_ecosystem
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
py -3.12 -m pytest -q tests\test_alpha3_host_boundaries.py -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-05-red'
```

Expected: both targets fail at missing ecosystem/Host adapters.

- [ ] **Step 3: Make normal inbound a synchronous source-bound fact barrier**

`build_interaction_batch_v1` accepts opaque `ScopeTokens`, turn/event IDs, current Native revision, observed UTC and an optional deterministic exact-control result. Its source digest covers only domain, opaque tokens, fact kind, time and closed value; it does not include message text or platform raw IDs. `extractor_digest` identifies `astrbot-inbound-v1` or the versioned exact phrase table.

The phrase table compares NFC-normalized, trimmed, whole-message values only. Freeze English `stop`, `goodbye`, `later` and Chinese `勿扰` plus explicit slash commands to pause/end semantics. Ambiguous values map to pause. Grant is reachable only through `/ae_contact_grant` or authenticated Page control with explicit purposes/validity; ordinary text and model output cannot grant.

Change `_run_genesis` ordering:

```text
ensure Genesis committed
read native revision
apply interaction_fact_batch (at least inbound_observed)
use returned revision as UserStimulus causal base
commit existing UserStimulus and inject the normal request context
notify supervisor only to re-evaluate schedules; do not call record_relation_inbound again
```

This preserves the existing v1 stimulus/action-contract path while making contact updates committed before the AstrBot Provider request.

Add relation-owner chat commands for grant/pause/resume/end. They derive relation scope from the current event, submit explicit-control facts, return Native audit receipt/public state, and never update a Python consent cache.

- [ ] **Step 4: Settle actual Provider usage and block manipulative candidates**

Add `provider_usage_v1(response)` that accepts only a non-negative integer from an observed `usage.total_tokens` or `usage.total_token_count`; if neither exact field exists, return `known=false, used_tokens=None`. Do not count characters or tokenizer estimates.

Call v2 settle on every post-Provider branch, including provider failure and candidate rejection. Unknown usage charges the reservation. Expand `_candidate_is_closed` with a case-folded, bounded phrase policy for jealousy, exclusivity, abandonment, guilt/FOMO and AI-needs-care claims in English and Chinese; a match returns terminal rejection before candidate encryption or adapter call. Keep prompt/system text bounded to the committed purpose/cause and the existing non-manipulation contract; do not include dream residue, raw facts or private source IDs.

Immediately before `submit_proactive_message`, call the v2 dispatch gate. Map a true AstrBot `send_message` result to `adapter_submitted`; only a separate observed platform receipt may advance to `platform_accepted` or `delivery_confirmed`.

- [ ] **Step 5: Implement authenticated Page controls without shadow consent**

`ObservatoryControlService` stores only short-lived authorization grants/nonces, never behavior state. `main.py` registers `/<plugin>/observatory/control/contact` as POST when Pages are available. The handler requires non-empty `request.username`, exact `request.plugin_name`, a username equal to configured `observatory_control_admin`, a relation-bound scope handle, a 128-bit one-use CSRF nonce issued in authenticated bootstrap, and a separate 128-bit idempotency nonce.

Accepted operations are `grant`, `pause`, `resume`, and `end`; grant requires a non-empty subset of the three purposes, channel exactly `astrbot_session`, and optional bounded validity. Consume the CSRF nonce before Native call, pass the idempotency nonce into the canonical source digest, and return only the Native audit receipt. Default `observatory_control_admin=""` makes Page writes unavailable; chat commands still let the relation owner act.

Add `developer_observatory_enabled=false`. Do not treat the Page asset token as API identity and do not expose Dashboard JWT handling in plugin code.

- [ ] **Step 6: Implement `EcosystemBrokerV1` as fail-closed Host transport plus Native admission**

Native `ecosystem_observe_v1` returns only persona public handle, canonical revision, world public ref/layer, local minute, sleep, activity, optional goal, allowed proposal kinds, capability digest and expiry. `ecosystem_propose_v1` validates the Host-frozen plugin installation digest, grant revision/digest, scope, TTL, expected canonical revision, world layer, 16 KiB envelope, eight source refs and semantic idempotency before returning accepted/rejected/deferred. Accepted proposals stay proposals and become eligible sources only in the next lived reducer.

Host `AstrBotEcosystemTransport.try_bind(context)` requires both callable `context.register_plugin_service` and `context.get_current_plugin_call_identity`. The call identity object must provide installation ID, name, version and manifest digest; broker derives its digests itself. If either method or any identity field is absent, expose `availability=unavailable_on_host` and register no transport.

This feature detection is intentional for the currently inspected AstrBot v4.26.7 source, which exposes `register_web_api` but no attested plugin-service caller interface. Never fall back to localhost HTTP, a global import registry, stack inspection, shared SQLite or a plugin-supplied name. Alpha3 ships no capability grant and no second ecosystem plugin by default.

- [ ] **Step 7: Compile, run both focused targets, and commit**

```powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\task-05'
cargo check --locked --offline -p ae-store -p ae-runtime -p ae-pyo3
cargo test --locked --offline -p ae-runtime --test alpha3_ecosystem
$env:PYTHONPYCACHEPREFIX='G:\AstrEmbodiment\.codex-task-temp\alpha3-pycache\task-05'
py -3.12 -m compileall -q main.py astr_embodiment
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
py -3.12 -m pytest -q tests\test_alpha3_host_boundaries.py -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-05'
git diff --check
git add crates/ae-store/src/alpha3/ecosystem.rs crates/ae-store/src/alpha3/mod.rs crates/ae-runtime/src/alpha3/ecosystem.rs crates/ae-runtime/src/alpha3/mod.rs crates/ae-runtime/tests/alpha3_ecosystem.rs astr_embodiment/interaction.py astr_embodiment/ecosystem.py astr_embodiment/observatory_controls.py astr_embodiment/contracts.py astr_embodiment/bridge.py astr_embodiment/autonomy.py astr_embodiment/proactive.py main.py _conf_schema.json tests/test_alpha3_host_boundaries.py
git commit -m "feat: connect alpha3 to AstrBot host boundaries"
```

Expected: all commands exit 0; the default broker/control state is unavailable/closed, and a 24-hour deterministic loop has no path to the Provider mock.

### Task 6 (ALPHA3-06): Render authority-separated Experience, Private, and Developer Pages

**Files:**
- Modify: `astr_embodiment/observatory.py` (v2 authorization and explicit fallback)
- Modify: `pages/observatory/index.html`
- Modify: `pages/observatory/app.js`
- Modify: `pages/observatory/style.css`
- Modify: `.astrbot-plugin/i18n/zh-CN.json`
- Modify: `.astrbot-plugin/i18n/en-US.json`
- Modify: `tests/test_observatory_page_api.py`
- Modify: `tests/test_release_contracts.py` (asset/view contract only; version assertions remain Task 7)
- Modify: `README.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: Add one Page/API separation contract**

Extend the existing Page API test with three requests and assert the service authorizes before calling `observe_snapshot_v2`. Experience cannot receive digests/fixed values/private relation data; Private cannot receive another relation or developer/build/claim internals; Developer requires authenticated configured admin plus `developer_observatory_enabled=true`. Unsupported Native/Host returns explicit unavailable and leaves core startup active.

Extend the release asset contract to require layer selectors, `apiPost` only for the contact-control endpoint, local relative assets, no CDN/iframe/arbitrary URL, no raw identifier in URL/storage, and the permanent computation/world-agent disclosure.

- [ ] **Step 2: Run the two RED Page targets**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
py -3.12 -m pytest -q tests\test_observatory_page_api.py -k 'alpha3 or layer or control' -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-06-red'
py -3.12 -m pytest -q tests\test_release_contracts.py -k 'page' -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-06-red'
```

Expected: failures show that the current service requests v1 and the current UI is one developer/read-only projection.

- [ ] **Step 3: Select Native DTOs only after Host authorization**

Add `layer=experience|private|developer` to the existing bootstrap/snapshot path while retaining the old v1 route. `ObservatoryProjectionService` validates username/plugin, scope handle and layer policy before constructing `ObserveSnapshotRequestV2`. Private and Developer relation handles remain server-issued, opaque, expiring and username-bound. A v2-unsupported Native returns a layer-level `unavailable_on_host`; do not infer consent, budget, gate or delivery from v1.

Do not request a Developer DTO and strip fields for other layers. Keep events committed-only and relation source filtering in Native; Host allowlists are a second boundary, not the primary isolation mechanism.

- [ ] **Step 4: Rework the shared Page shell into three layers**

Keep one `pages/observatory/` entry and the AstrBot bridge. Use hash routes `#/experience`, `#/private`, and `#/developer`; each layer may render its own subpanels from one exact DTO variant.

- Experience shows current lived segment, active goal, sleep scheduling proxy, world layer, notable nodes and a neutral why-contacted explanation. Hide this layer's lived panels when `inner_activity_display=false` without stopping polling or Native life.
- Private shows consent epoch/state/terms/expiry, grant/pause/resume/end controls, quiet hours, contact timing/follow-up/repetition/unanswered, source public refs, dream non-fact/review state, budget charged/reserved/known status, and correction/export/delete availability.
- Developer shows schema/build/formula digests, canonical/operational revisions, projection health, readiness witness revisions, GateDecisionV2, intention/outbound/claim stage, migration health and explicitly labeled legacy diagnostics.

Every layer displays “计算状态/角色世界代理，不是意识、生理、临床测量或现实经历” (and the equivalent English text). Replace all Experience/Private labels for `affiliation_need`, “想念”, “affect physiology”, and “contact tendency” with `contact_due_score`, “联系时机评分”, “computed signals”, and “contact timing”. Legacy wording appears only in Developer migration diagnostics.

Polling remains GET-only, five seconds or slower, paused while hidden/manual pause, bounded backoff and capped event memory. The polling pause control is visually and semantically separate from contact pause. Contact mutations use the Task 5 POST service and replace the one-use CSRF nonce after each response.

- [ ] **Step 5: Update i18n and accurate product documentation**

Add exact zh-CN/en-US keys for all closed activity/goal/layer/consent/purpose/gate/readiness/availability/dream codes. Unknown codes render as unavailable, not raw internal strings.

README and CHANGELOG describe: AstrBot is the only host; one fixed mixed main world; no Tavern/Card/Lorebook compatibility; deterministic B+C lived day; relation-level explicit consent above the global switch; zero-LLM default; ecosystem broker unavailable on hosts without caller identity; dream residue is non-fact and normally empty; v1 compatibility; Pages version requirement; and COMPILE/FOCUSED versus real Host integration claims. Do not claim consciousness, real sleep/dream/life, delivery, a second plugin, or a production ecosystem.

- [ ] **Step 6: Check JavaScript/Python, run the focused contracts, and commit**

```powershell
node --check pages\observatory\app.js
$env:PYTHONPYCACHEPREFIX='G:\AstrEmbodiment\.codex-task-temp\alpha3-pycache\task-06'
py -3.12 -m compileall -q astr_embodiment\observatory.py
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
py -3.12 -m pytest -q tests\test_observatory_page_api.py -k 'alpha3 or layer or control' -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-06'
py -3.12 -m pytest -q tests\test_release_contracts.py -k 'page' -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-06'
git diff --check
git add astr_embodiment/observatory.py pages/observatory .astrbot-plugin/i18n tests/test_observatory_page_api.py tests/test_release_contracts.py README.md CHANGELOG.md
git commit -m "feat: add alpha3 authority separated Pages"
```

Expected: commands exit 0; layer responses are structurally distinct and the browser contains no hidden higher-authority payload.

### Task 7 (ALPHA3-07): Version, package, and run the bounded delivery gates

**Files:**
- Modify: `Cargo.toml` (`workspace.package.version`)
- Modify: `Cargo.lock` (workspace package entries only)
- Modify: `pyproject.toml`
- Modify: `metadata.yaml`
- Modify: `crates/ae-pyo3/src/lib.rs` (`version()` marker)
- Modify: `scripts/package_plugin.py` (wheel/runtime versions and `alpha3_call` marker)
- Modify: `tests/test_release_contracts.py` (alpha3 version/API/package assertions)

- [ ] **Step 1: Change the release contract to alpha3 first**

Require Rust/metadata/native runtime `1.1.0-alpha3`, Python wheel `1.1.0a3`, `alpha3_call` in the initializer and native payload markers, both Windows/Linux ABI3 payloads, Page/i18n assets, archive size below 16 MiB, and exclusion of crates/tests/caches/databases/wheels/source maps/evidence. Existing v1 symbol markers remain required.

- [ ] **Step 2: Run the release contract to verify the version boundary is RED**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
py -3.12 -m pytest -q tests\test_release_contracts.py -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\task-07-red'
```

Expected: version and `alpha3_call` package-marker assertions fail against alpha2 constants; unrelated release assertions remain green or explicitly skip only when fresh platform wheels are absent.

- [ ] **Step 3: Bump all release identities without broad dependency changes**

Set:

```text
Cargo workspace/native runtime/metadata: 1.1.0-alpha3
PEP 440 project and wheel:              1.1.0a3
package archive stem:                   astrbot_plugin_astrembodiment-1.1.0-alpha3
```

Update only local workspace package versions in `Cargo.lock`; do not update third-party crates. Keep `astrbot_version: ">=4.16,<5"` for core compatibility and document that Pages remain feature-detected. Add `alpha3_call` to `NATIVE_API_MARKERS`; keep every old native marker.

- [ ] **Step 4: Run the compile-first final gate exactly once**

```powershell
$env:PYTHONPYCACHEPREFIX='G:\AstrEmbodiment\.codex-task-temp\alpha3-pycache\final'
py -3.12 -m compileall -q main.py astr_embodiment

$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\alpha3-cargo\final'
cargo check --locked --offline --workspace --all-targets

node --check pages\observatory\app.js
git diff --check
```

Expected: each command exits 0. If Cargo reports an unavailable offline crate, classify `ENVIRONMENTAL_DEPENDENCY_MISSING`; do not retry online and do not call it a source compile failure.

- [ ] **Step 5: Run only the final release contracts**

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
py -3.12 -m pytest -q tests\test_release_contracts.py -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\alpha3-pytest\final'
```

Expected: release contract exits 0. It proves source/package structure and mocked clean-namespace exports; it does not prove a real AstrBot Page, plugin-to-plugin identity, Provider usage, platform delivery or crash recovery.

- [ ] **Step 6: Inspect scope, commit the release metadata, and classify delivery honestly**

```powershell
git diff --check
git status --short
git diff --name-only HEAD~7
git add Cargo.toml Cargo.lock pyproject.toml metadata.yaml crates/ae-pyo3/src/lib.rs scripts/package_plugin.py tests/test_release_contracts.py
git commit -m "chore: prepare 1.1.0-alpha3 package"
git log --oneline --decorate -8
```

Expected: seven ordered alpha3 commits exist, no generated ZIP/wheel/cache/database/evidence is staged, and unrelated pre-existing paths remain untouched.

Final report `COMPILE/FOCUSED PASS` only when the compile gate and release contract have current exit-0 evidence. Real Host integration remains explicitly unverified by this plan. A later optional controlled AstrBot check should verify caller identity, three authenticated views, grant/pause/end, known/unknown Provider usage, one proactive adapter submission and crash recovery; even then adapter submission is not delivery confirmation.

## Spec coverage checklist for the implementing coordinator

| Design requirement | Owning task |
|---|---|
| AstrBot-only host, Native single writer, v1 compatibility, wire v5, DB v7 | Task 1 |
| Source-bound inbound/follow-up/boundary/outcome facts | Tasks 1–2 and Host barrier in Task 5 |
| Relation consent epochs, pause/resume/end, explicit cause, stable intention | Task 2 |
| Fixed mixed world, layer validation, deterministic B+C loop, importance/context modes | Task 3 |
| Dream contract empty by default, waking review, non-fact influence limit | Task 3 |
| Relation-local daily budget, actual/unknown usage, readiness and dual gate | Task 4 plus Provider adapter in Task 5 |
| Live intention/outbound/claim and explicit availability projection | Task 4 |
| Ecosystem observation/proposal, capability identity, bounds and Native admission | Task 5 |
| Authenticated relation controls, CSRF/idempotency, no Host shadow state | Task 5 |
| Experience/Private/Developer DTO and UI separation, neutral language | Task 6 |
| Version/package, compileall, offline Cargo check, node check, release contracts | Task 7 |
| No Tavern import, no independent server, no background narrative/dream LLM, no second plugin required | Execution contract, Tasks 3, 5 and 6 |

Plan complete and saved to `docs/superpowers/plans/2026-08-29-alpha3-lived-world-native-ecosystem-implementation.md`. Execute through the attested Fast Lane sequentially, one accepted commit per task; use inline execution only when the coordinator intentionally declares `DEGRADED_SKILL_ONLY` and disables concurrent writers.
