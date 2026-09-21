# CyberHuman Autonomous Runtime v0 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build one persistent, adaptive CyberHuman runtime that advances without inbound messages, sleeps and wakes on its own persona clock, forms endogenous intentions through a deterministic 16K→2K→256→32 workspace, and can safely submit one proactive AstrBot message under explicit relation policy.

**Architecture:** Rust remains the only recoverable authority: `ae-contracts` freezes closed wire types, `ae-store` owns migrations and transactional state, `ae-autonomy` computes temporal/sleep dynamics, `ae-renorm` derives the multiscale pyramid, and `ae-runtime` owns claims, decisions, intentions, and outbox transitions. Python freezes host/time/capability facts, runs a cancellable adaptive supervisor, performs the one allowed LLM externalization, and calls AstrBot's verified `Context.send_message(session, MessageChain) -> bool`; Python never keeps a second recoverable brain. Wave 0 authority migration is mandatory, M1–M4 are internal dependency/commit/rollback gates, and the user receives one final integrated delivery rather than four approval pauses.

**Tech Stack:** Rust 2021, `ae-fixed`, serde closed schemas, rusqlite SQLite, PyO3 0.29, Python 3.12 `asyncio`/`zoneinfo`, AstrBot `Context` and `MessageChain`, Windows DPAPI plus AstrBot-provided `cryptography.hazmat.primitives.ciphers.aead.AESGCM`, Cargo, Ruff, pytest.

---

## Execution contract and current RED baseline

- Repository: `G:\AstrEmbodiment\AstrEmbodiment-1.0.0-MVP-Development-Kit\astrbot_plugin_astrembodiment`.
- Approved design: `docs/superpowers/specs/2026-08-28-cyberhuman-autonomous-runtime-design.md`.
- Baseline revision when this plan was written: `cd30563834fdcdcf3de7f754f883e889cded6b8d` on `release/1.0.0-alpha1`, ahead of origin by one commit.
- Existing CodeGraph was current when mapped: 32 files, 1,053 nodes, 3,302 edges. Important anchors are `TimeAdvance` at `crates/ae-contracts/src/lib.rs:557`, `AstrRuntime::apply_event` at `crates/ae-runtime/src/lib.rs:305`, `Store::commit_journal` at `crates/ae-store/src/lib.rs:1118`, `Workspace` at `crates/ae-renorm/src/lib.rs:9`, `NativeBridge.apply_event` at `astr_embodiment/bridge.py:236`, and plugin lifecycle at `main.py:115`/`:135`.
- Current RED facts: `TimeAdvance` contains only `elapsed_ms`; runtime commits `delta_bytes: vec![]` and `state_after == state_before`; `empty_workspace()` creates 32 zero tokens; `_scope_for()` omits `relation_token`; `initialize()` starts no scheduler; `terminate()` only closes the bridge; `proactive_enabled` is configuration text with no outbound path.
- AstrBot host source was verified at `G:\Bugfinders\AstrBot` revision `2efa31957`: `Context.send_message(self, session: str | MessageSesion, message_chain: MessageChain) -> bool`; `True` means a matching platform was found after `send_by_session`, not delivery/read confirmation.
- All task-owned build/cache/evidence directories must be under `G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\`. No worker may use the coordinator checkout as a Fast Lane worker worktree.
- Compile first. Each wave gets one workspace compile gate and only the focused invariant tests listed below; do not add broad combinatorial suites.

### Fast Lane scheduling rules

- The 2718lab Fast Lane compiler is an inert descriptor. Dispatch only exact `action="start"` assignments whose host-provided route, lease, worktree, `index_context`, and predecessor fences validate. Missing attestation is `NO_SAFE_WORK`; do not guess a model or fall back to a shared writer.
- Coordinator lane owns decomposition, ordered integration, rollback decisions, final tests, manual smoke, and acceptance. A worker returns only candidate commit, changed files, real command output, evidence hash, and blockers; it never merges or accepts its own work.
- One effective writer per exact path. Units whose write scopes overlap queue behind the current owner even if their logical work seems independent.
- Input CodeGraph query occurs once at dispatch; output query occurs once at the terminal boundary. A worker consumes the provided bounded `index_context` and does not rescan or poll the index.
- If durable workflow/MCP support is unavailable, mark `DEGRADED_SKILL_ONLY`, disable concurrent writers, and execute these units serially in the coordinator checkout. This is still one product delivery, but not a claim of Fast Lane lease/crash-recovery guarantees.

### Task DAG

```text
FL-00 baseline/read-only
  -> FL-01 authority + wire migration
       -> FL-02 store schema/transactions
       -> FL-03 temporal/sleep pure engine       (parallel with FL-02)
       -> FL-04 host frozen-time projection      (parallel with FL-02/03)
            FL-02 + FL-03 + FL-04 -> FL-05 M1 runtime/FFI/supervisor integration
              -> FL-06 renorm pyramid
              -> FL-07 workspace/intention core (FL-06 precedes runtime integration)
                 -> FL-08 relation identity + encrypted target
                 -> FL-09 native outbox/gates    (same store/runtime paths queue serially)
                    FL-08 + FL-09 -> FL-10 proactive AstrBot integration
                                      -> FL-11 display query
                                        -> FL-12 final integration acceptance + manual smoke
```

## File responsibility map

| Path | Responsibility |
|---|---|
| `crates/ae-contracts/src/autonomy.rs` | Closed autonomy, time, relation, intention, outbox, claim/settle types and stable schema enums. |
| `crates/ae-contracts/src/lib.rs` | Re-export autonomy types and extend canonical wire codecs; no business policy. |
| `crates/ae-authority/src/lib.rs` | Explicit derived-state authority lattice; residual authority remains unchanged. |
| `model/authority-matrix-v1.toml` | Machine-readable `time_advance.derived_allow` while retaining `time_advance.allow=[]`. |
| `crates/ae-store/src/autonomy.rs` | Autonomous schema migration, typed rows, atomic wake/intention/outbox transactions and read-only event queries. |
| `crates/ae-store/src/lib.rs` | Store lifecycle plus narrow delegation to the autonomy store module. |
| `crates/ae-autonomy/src/lib.rs` | Pure deterministic Process S/C, entrainment, sleep transitions, bounded recovery and next-wake calculation. |
| `crates/ae-renorm/src/lib.rs` | Deterministic 16K→2K→256→32 restriction, residual, winner selection and transient prolongation. |
| `crates/ae-agent/src/autonomy.rs` | Endogenous candidate scoring and closed operational intention transition rules. |
| `crates/ae-runtime/src/autonomy.rs` | Native orchestration of claims, commits, budget/gates, recovery and query projections. |
| `crates/ae-runtime/src/lib.rs` | Existing Genesis/event compatibility and delegation to autonomous runtime methods. |
| `crates/ae-pyo3/src/lib.rs` | Versioned JSON FFI only; no SQL or policy reimplementation. |
| `astr_embodiment/temporal.py` | UTC/IANA projection and frozen tzdb evidence. |
| `astr_embodiment/relation.py` | NFC length-prefixed private/group relation identity and inbound binding facts. |
| `astr_embodiment/secret_store.py` | Windows DPAPI-protected AES-256-GCM key; non-Windows v0 proactive capability fails closed. |
| `astr_embodiment/autonomy.py` | Cancellable adaptive supervisor, FFI claim/settle sequencing and economy-mode externalization. |
| `astr_embodiment/proactive.py` | AstrBot message-chain construction and one-shot adapter call. |
| `astr_embodiment/display.py` | Template-only mood card/timeline rendering from committed query rows. |
| `astr_embodiment/bridge.py` | Thin JSON methods for new PyO3 calls. |
| `astr_embodiment/contracts.py` | Host DTOs/builders; no recoverable state. |
| `astr_embodiment/tokens.py` | Existing opaque tokens plus canonical relation-token derivation. |
| `main.py` | Plugin lifecycle, inbound target binding, commands, and supervisor ownership. |
| `_conf_schema.json` | Explicit opt-in, persona/user timezones, quiet hours, economy/display controls. |

### Task 0 (FL-00): Freeze baseline and dispatch packets

**Files:**
- Read: `.codegraph/codegraph.db`
- Read: every file listed in the File responsibility map
- Evidence only: `G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\baseline\`

- [ ] **Step 1: Verify the repository identity and clean integration boundary**

Run:

```powershell
$repo = 'G:\AstrEmbodiment\AstrEmbodiment-1.0.0-MVP-Development-Kit\astrbot_plugin_astrembodiment'
git -C $repo rev-parse --show-toplevel
git -C $repo rev-parse HEAD
git -C $repo status --short --branch
```

Expected: top level equals `$repo`; HEAD is the coordinator-selected integration base; only previously accepted coordinator changes are present. Any unrelated dirty path blocks worker dispatch until the coordinator assigns an isolated worktree.

- [ ] **Step 2: Verify the existing index once**

Run:

```powershell
codegraph status .
codegraph explore "TimeAdvance apply_event Store Workspace NativeBridge AstrEmbodimentPlugin" --max-files 12
```

Expected: index reports `[OK] Index is up to date`; anchors match the RED facts above. If not current, the trusted host performs one `codegraph sync .` before dispatch and binds the resulting input snapshot to every strict assignment.

- [ ] **Step 3: Create task-local build roots**

Run:

```powershell
$aeTaskTemp = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0'
New-Item -ItemType Directory -Force -Path $aeTaskTemp, "$aeTaskTemp\cargo", "$aeTaskTemp\pytest", "$aeTaskTemp\evidence" | Out-Null
Get-Item -LiteralPath $aeTaskTemp | Select-Object FullName,Attributes
```

Expected: every returned path is an ordinary directory below the G-drive task root, not a reparse point.

- [ ] **Step 4: Register the DAG before spawning writers**

Register `FL-01` through `FL-12` with the exact dependency and write scopes in this document. Claim only the current ready wave, bind the actual returned agent target, and reject any assignment whose `host_dispatch.model`, `host_dispatch.reasoning_effort`, lease, worktree, predecessor hash, or `index_context` is missing.

Expected: `FL-01` is the only write-ready unit; `FL-02`–`FL-12` remain dependency-blocked.

### Task 1 (FL-01 / Wave 0): Migrate autonomy authority and canonical wire

**Files:**
- Create: `crates/ae-contracts/src/autonomy.rs`
- Modify: `crates/ae-contracts/src/lib.rs`
- Modify: `crates/ae-authority/src/lib.rs`
- Modify: `model/authority-matrix-v1.toml`
- Modify: `tests/test_static_contracts.py`

- [ ] **Step 1: Add the closed authority and state vocabulary**

Add these exact public shapes in `crates/ae-contracts/src/autonomy.rs`; every persisted enum uses `#[serde(rename_all = "snake_case", deny_unknown_fields)]` where serde permits it, and every record uses `#[serde(deny_unknown_fields)]`:

```rust
pub const AUTONOMY_SCHEMA_VERSION: u16 = 1;

pub enum DerivedMutationClass {
    TemporalState,
    SleepState,
    WorkspaceProjection,
    OperationalIntention,
    WakeSchedule,
    PermanentMemory,
    RelationCommitment,
    PersonaGenesis,
    DeliveryFact,
}

pub struct FrozenTimeInputV1 {
    pub schema_version: u16,
    pub observed_now_utc_ms: u64,
    pub effective_now_utc_ms: u64,
    pub persona_tzid: String,
    pub persona_utc_offset_seconds: i32,
    pub persona_local_minute: u16,
    pub persona_day_ordinal: i32,
    pub relation_tzid: String,
    pub relation_utc_offset_seconds: i32,
    pub relation_local_minute: u16,
    pub relation_day_ordinal: i32,
    pub budget_day_start_utc_ms: u64,
    pub budget_next_day_start_utc_ms: u64,
    pub next_timezone_transition_utc_ms: Option<u64>,
    pub tzdb_fingerprint: Digest,
}

pub struct TimeAdvanceV1 {
    pub event_id: Id128,
    pub scope: ScopeRef,
    pub expected_generation: u64,
    pub frozen: FrozenTimeInputV1,
    pub frozen_input_digest: Digest,
}

pub enum SleepStateV1 { Awake, Drowsy, Asleep }
pub enum WakeIntensityV1 { Micro, Associative, Ignition, Emergency }
pub enum IntentionStateV1 {
    Forming, Ready, Deferred, Externalizing, DispatchPending,
    AdapterCallStarted, AdapterSubmitted, PlatformAccepted,
    DeliveryConfirmed, DispatchUnknown, Suppressed, Expired, Terminal,
}
pub enum DispatchOutcomeV1 {
    AdapterRejectedTerminal, AdapterSubmitted, PlatformAccepted,
    DeliveryConfirmed, DispatchUnknown,
}
```

Also define `PersonaTemporalProfileV1`, `RelationTemporalPolicyV1`, `AutonomousRuntimeStateV1`, `InnerEventV1`, `DurableIntentionV1`, `OutboundTargetEnvelopeV1`, `WakeClaimV1`, `ExternalizationClaimV1`, `DispatchClaimV1`, `MoodCardV1`, and `InnerEventPageV1` with exactly the fields required by design sections 5–16. Use `Id128`/`Digest` for opaque identity, record ids, claim tokens, candidate/target/input digests; use `Fixed` for bounded dynamics; use UTC `u64` for instants; use closed enums rather than strings for state.

- [ ] **Step 2: Replace legacy `TimeAdvance` without a serde-default migration**

In `crates/ae-contracts/src/lib.rs`, re-export `autonomy::*`, change `CanonicalEvent::TimeAdvance(TimeAdvance)` to `CanonicalEvent::TimeAdvance(TimeAdvanceV1)`, and extend the canonical encoder/decoder with a new wire schema version and explicit legacy discriminator. A legacy body containing only `elapsed_ms` must return `WireError::UnsupportedSchema(0)` unless the store migration is decoding it through the dedicated legacy reader.

Expected code boundary:

```rust
pub mod autonomy;
pub use autonomy::*;

pub enum CanonicalEvent {
    UserStimulus(UserStimulus),
    UserReaction(UserReaction),
    CorrectionClaim(CorrectionClaim),
    CorrectionVerdict(CorrectionVerdictEvent),
    SelfActionCandidate(SelfActionCandidate),
    DeliveryOutcome(DeliveryOutcome),
    SettlementEvidence(SettlementEvidence),
    TimeAdvance(TimeAdvanceV1),
    AdminAction(AdminAction),
}
```

- [ ] **Step 3: Grant only reversible derived-state authority**

Add the following method to `AuthorityProjection`; do not alter `allows()` for residual coordinates:

```rust
pub fn allows_derived(
    source: SourceAuthority,
    class: DerivedMutationClass,
) -> bool {
    source == SourceAuthority::TimeAdvance
        && matches!(
            class,
            DerivedMutationClass::TemporalState
                | DerivedMutationClass::SleepState
                | DerivedMutationClass::WorkspaceProjection
                | DerivedMutationClass::OperationalIntention
                | DerivedMutationClass::WakeSchedule
        )
}
```

Update `model/authority-matrix-v1.toml` to retain `allow = []` and add:

```toml
[time_advance]
allow = []
derived_allow = [
  "temporal_state",
  "sleep_state",
  "workspace_projection",
  "operational_intention",
  "wake_schedule",
]
```

- [ ] **Step 4: Compile the authority contract before tests**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-01'
cargo check --locked -p ae-contracts -p ae-authority
```

Expected: exit code 0; no production state or SQLite file is opened.

- [ ] **Step 5: Run the two focused authority checks**

Add one Rust test proving all five reversible classes are allowed and all four irreversible classes are rejected, and extend `test_self_action_has_zero_residual_authority` coverage so `TimeAdvance` still has a zero residual bitmap.

Run:

```powershell
cargo test --locked -p ae-authority allows_only_reversible_time_derived_state -- --exact
cargo test --locked -p ae-authority self_critique_platform_time_and_admin_have_zero_authority -- --exact
```

Expected: both commands pass; `time_advance.allow=[]` remains true.

- [ ] **Step 6: Commit the Wave 0 contract boundary**

```powershell
git add crates/ae-contracts/src/autonomy.rs crates/ae-contracts/src/lib.rs crates/ae-authority/src/lib.rs model/authority-matrix-v1.toml tests/test_static_contracts.py
git commit -m "feat: migrate autonomous derived-state authority"
```

Expected: one scoped candidate commit; M1 remains blocked until FL-02/03/04 integrate against this exact contract hash.

### Task 2 (FL-02 / Wave 0): Add autonomous SQLite migration and atomic records

**Files:**
- Create: `crates/ae-store/src/autonomy.rs`
- Modify: `crates/ae-store/src/lib.rs`
- Create: `crates/ae-store/tests/autonomy_migration.rs`

- [ ] **Step 1: Add a monotonic schema ledger**

Extend `Store::migrate` to create `schema_migrations(version INTEGER PRIMARY KEY, digest BLOB NOT NULL, completed_at_ms INTEGER NOT NULL)` and reject a database version newer than the binary. Register one immediate transaction that creates tables `persona_temporal_profile`, `relation_temporal_policy`, `autonomous_runtime_state`, `inner_event`, `durable_intention`, `wake_schedule`, `outbound_attempt`, and `autonomy_claim`; include foreign keys/unique constraints on persona scope, relation scope, generation, semantic idempotency digest, outbound id, and live claim token.

Required migration entry point:

```rust
pub(crate) const AUTONOMY_DB_VERSION: u32 = 1;

pub(crate) fn migrate_autonomy(
    tx: &rusqlite::Transaction<'_>,
    from_version: u32,
) -> Result<u32, StoreError>;
```

The migration must not reinterpret old `TimeAdvance` event bytes with serde defaults. Existing journal bytes stay immutable; replay uses the legacy event decoder for schema 0 and emits no autonomous delta for those historical rows.

- [ ] **Step 2: Add typed store transactions**

Implement these exact `Store` methods by delegating to `autonomy.rs`:

```rust
pub fn load_autonomous_state(&self, persona_scope: &Digest)
    -> Result<Option<AutonomousRuntimeStateV1>, StoreError>;
pub fn claim_wake(&mut self, request: &WakeClaimRequestV1)
    -> Result<WakeClaimV1, StoreError>;
pub fn settle_wake(&mut self, token: &Digest, proposal: &WakeProposalV1)
    -> Result<AutonomousRuntimeStateV1, StoreError>;
pub fn claim_externalization(&mut self, request: &ExternalizationClaimRequestV1)
    -> Result<ExternalizationClaimV1, StoreError>;
pub fn settle_externalization(&mut self, request: &ExternalizationSettleV1)
    -> Result<DurableIntentionV1, StoreError>;
pub fn claim_dispatch(&mut self, request: &DispatchClaimRequestV1)
    -> Result<DispatchClaimV1, StoreError>;
pub fn settle_dispatch(&mut self, request: &DispatchSettleV1)
    -> Result<OutboundAttemptV1, StoreError>;
pub fn query_inner_events(&self, query: &InnerEventQueryV1)
    -> Result<InnerEventPageV1, StoreError>;
```

`settle_wake` commits state revision, inner events, intention transitions, and next wake in one `TransactionBehavior::Immediate` transaction. `claim_dispatch` moves `dispatch_pending → adapter_call_started` in the same transaction that creates the claim token; recovery classifies every orphaned `adapter_call_started` as `dispatch_unknown`.

- [ ] **Step 3: Compile store migration first**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-02'
cargo check --locked -p ae-store
```

Expected: exit code 0.

- [ ] **Step 4: Run one migration/recovery test target**

`crates/ae-store/tests/autonomy_migration.rs` must contain exactly three tests: legacy database migration preserves journal bytes; failed migration rolls back and reopens; orphaned `adapter_call_started` recovers to `dispatch_unknown` without creating a second outbound id.

Run:

```powershell
cargo test --locked -p ae-store --test autonomy_migration
```

Expected: 3 passed; no unrelated workspace tests run.

- [ ] **Step 5: Commit the store authority boundary**

```powershell
git add crates/ae-store/src/autonomy.rs crates/ae-store/src/lib.rs crates/ae-store/tests/autonomy_migration.rs
git commit -m "feat: persist autonomous runtime transactions"
```

### Task 3 (FL-03 / M1): Implement deterministic temporal and sleep dynamics

**Files:**
- Create: `crates/ae-autonomy/Cargo.toml`
- Create: `crates/ae-autonomy/src/lib.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`

- [ ] **Step 1: Add the pure autonomy crate**

Add workspace member/dependency `ae-autonomy`; its only dependencies are workspace `ae-fixed`, `ae-contracts`, `serde`, and `thiserror`. Do not add floating-point runtime dependencies.

- [ ] **Step 2: Implement the deterministic API**

Expose these signatures:

```rust
pub const MAX_OFFLINE_INTEGRATION_MS: u64 = 168 * 60 * 60 * 1_000;

pub fn validate_frozen_time(input: &FrozenTimeInputV1) -> Result<(), AutonomyError>;
pub fn advance_temporal_state(
    old: &AutonomousRuntimeStateV1,
    profile: &PersonaTemporalProfileV1,
    frozen: &FrozenTimeInputV1,
    stimulus_arousal: Fixed,
) -> Result<WakeProposalV1, AutonomyError>;
pub fn next_wake_utc_ms(
    state: &AutonomousRuntimeStateV1,
    intensity: WakeIntensityV1,
) -> u64;
```

Use fixed-point lookup tables checked into this file for the specified 18 h wake and 4 h sleep exponentials and the 1,440-minute cosine. Clamp elapsed time to 168 h in one closed-form update; never iterate missed ticks. Apply `effective_now=max(observed,last_committed)`, `A∈[0,0.20]`, `e=wrap_to_720(target_phase-current_phase)`, and entrainment `clamp(e, ±90*dt_days)`. Sleep transitions use the design thresholds with hysteresis and emit typed `InnerEventV1` rows.

- [ ] **Step 3: Compile the pure engine**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-03'
cargo check -p ae-autonomy
cargo check --locked -p ae-autonomy
```

Expected: the first command updates `Cargo.lock` only for the new local crate; the locked command then exits 0 without network dependency changes.

- [ ] **Step 4: Run bounded deterministic tests**

Add four unit tests in the crate: identical old state/input gives byte-identical proposal; clock rollback advances zero; 30-day gap performs one 168 h bounded update; Los Angeles→Shanghai travel changes TZ immediately while phase shift is at most 90 minutes/day.

Run:

```powershell
cargo test --locked -p ae-autonomy
```

Expected: 4 passed.

- [ ] **Step 5: Commit the pure temporal engine**

```powershell
git add Cargo.toml Cargo.lock crates/ae-autonomy
git commit -m "feat: add deterministic cyberhuman sleep dynamics"
```

### Task 4 (FL-04 / M1): Freeze Host UTC and IANA time evidence

**Files:**
- Create: `astr_embodiment/temporal.py`
- Modify: `astr_embodiment/contracts.py`
- Create: `tests/test_temporal_projection.py`

- [ ] **Step 1: Implement the host-only frozen projection**

Use these exact types/signature:

```python
@dataclass(frozen=True, slots=True)
class TemporalConfig:
    persona_tzid: str
    relation_tzid: str

def freeze_time_input(
    *,
    observed_now_utc_ms: int,
    last_committed_utc_ms: int,
    config: TemporalConfig,
) -> dict[str, object]:
    """Return astrembodiment.frozen-time-input.v1 with a canonical digest."""
```

Use only aware UTC datetimes and `zoneinfo.ZoneInfo`. Compute local minute/day ordinal, UTC offsets, user-local natural-day UTC boundaries, next transition by bounded binary search over the next 370 days, and `tzdb_fingerprint = SHA256(provider || tzpath || canonical_transition_slice)`. Invalid TZIDs raise `TemporalProjectionError`; do not fall back inside this function.

- [ ] **Step 2: Replace `build_time_advance_json`**

The builder takes `expected_generation` and the already frozen mapping; it does not calculate time or timezone values:

```python
def build_time_advance_json(
    *, scope: ScopeTokens, event_id: str,
    expected_generation: int,
    frozen: Mapping[str, object],
) -> dict[str, object]:
```

- [ ] **Step 3: Compile Python before tests**

Run:

```powershell
python -m compileall -q astr_embodiment\temporal.py astr_embodiment\contracts.py
python -m ruff check astr_embodiment\temporal.py astr_embodiment\contracts.py tests\test_temporal_projection.py
```

Expected: both commands exit 0.

- [ ] **Step 4: Run one focused projection module**

The test module contains three cases: LA persona/Shanghai relation projections differ correctly; DST transition keeps UTC monotonic and day bounds ordered; invalid TZID fails closed.

Run:

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD = '1'
python -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pytest\fl-04' tests\test_temporal_projection.py
```

Expected: 3 passed.

- [ ] **Step 5: Commit the Host time freezer**

```powershell
git add astr_embodiment/temporal.py astr_embodiment/contracts.py tests/test_temporal_projection.py
git commit -m "feat: freeze dual-timezone runtime inputs"
```

### Task 5 (FL-05 / M1): Integrate wake claim/settle and adaptive supervisor

**Files:**
- Create: `crates/ae-runtime/src/autonomy.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Modify: `crates/ae-runtime/Cargo.toml`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `astr_embodiment/bridge.py`
- Create: `astr_embodiment/autonomy.py`
- Modify: `main.py`
- Modify: `_conf_schema.json`
- Create: `tests/test_autonomous_supervisor.py`

- [ ] **Step 1: Add Native M1 runtime methods**

Expose on `AstrRuntime`:

```rust
pub fn bootstrap_autonomy(
    &mut self,
    scope: &ScopeRef,
    profile: &PersonaTemporalProfileV1,
    relation: Option<&RelationTemporalPolicyV1>,
) -> Result<AutonomousRuntimeStateV1, RuntimeError>;
pub fn claim_wake(
    &mut self,
    scope: &ScopeRef,
    event: &TimeAdvanceV1,
) -> Result<WakeClaimV1, RuntimeError>;
pub fn settle_wake(
    &mut self,
    claim_token: &Digest,
) -> Result<AutonomousRuntimeStateV1, RuntimeError>;
pub fn recover_autonomy(
    &mut self,
    scope: &ScopeRef,
    frozen: &FrozenTimeInputV1,
) -> Result<RecoveryReportV1, RuntimeError>;
```

`claim_wake` verifies frozen digest, generation, persona lease, authority class and mapping digest; it computes a proposal but does not advance authority. `settle_wake` atomically persists the proposal. Remove the G0 `TimeAdvance` no-op branch from `apply_event`; old inbound `UserStimulus` and `DeliveryOutcome` behavior remains compatible.

- [ ] **Step 2: Add closed PyO3/bridge methods**

Register JSON-in/JSON-out functions `bootstrap_autonomy`, `claim_wake`, `settle_wake`, `recover_autonomy`, and `autonomy_status`. Mirror them on `NativeBridge` with `dict[str, Any]` inputs/outputs. The FFI owns no scheduler and accepts no arbitrary SQL.

- [ ] **Step 3: Implement a cancellable adaptive supervisor**

Create `AutonomousSupervisor.__init__(*, bridge: NativeBridge, freeze_now: Callable[[dict[str, object]], dict[str, object]], on_externalization: Callable[[dict[str, object]], Awaitable[None]]) -> None`, `start() -> None`, `stop() -> None`, and `notify_inbound(binding: dict[str, object]) -> None` as async lifecycle methods. `__init__` allocates one `asyncio.Event`, sets `_task=None`, `_stopping=False`, and keeps only the latest non-authoritative inbound binding; `start` creates exactly one `_run()` task; `stop` sets `_stopping`, signals the event, awaits the task, and clears it; `notify_inbound` replaces the ephemeral binding and signals the event.

Use one task owned by the plugin, one `asyncio.Event` for earlier wakeups, and deadlines returned by Native. No fixed polling loop is allowed. `stop()` sets the stop flag, wakes the task, awaits it, then returns; no new claim may start afterward. `notify_inbound` may shorten a persisted next wake but is not the only start path.

- [ ] **Step 4: Wire plugin lifecycle and minimal configuration**

After `_bridge.open`, initialize/start the supervisor even when no inbound event exists. Before `_bridge.close`, await supervisor stop. Add closed config fields: `autonomous_runtime_enabled=true`, `persona_home_timezone`, `persona_current_timezone`, `persona_chronotype`, `preferred_sleep_local`, `preferred_wake_local`, `sleep_flex_minutes`, `entrainment_rate_minutes_per_day`, `user_timezone`, `user_timezone_source`, `quiet_hours_start`, `quiet_hours_end`, `inner_activity_mode="economy"`, and `inner_activity_display=false`. Keep `proactive_enabled=false`.

- [ ] **Step 5: Compile the integrated M1 slice**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-05'
cargo check --locked --workspace
python -m compileall -q main.py astr_embodiment
python -m ruff check main.py astr_embodiment tests\test_autonomous_supervisor.py
```

Expected: all three commands exit 0.

- [ ] **Step 6: Run the bounded M1 supervisor tests**

The test file contains four cases: start schedules without inbound; quiet micro wake performs zero LLM calls; stop prevents later claims; restart uses one bounded recovery rather than replaying ticks.

Run:

```powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD = '1'
python -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pytest\fl-05' tests\test_autonomous_supervisor.py
cargo test --locked -p ae-runtime autonomous_time_advance_changes_state_and_replays -- --exact
```

Expected: Python 4 passed; Rust focused test passed; recorded fake LLM count is zero.

- [ ] **Step 7: Commit the M1 integration boundary**

```powershell
git add crates/ae-runtime crates/ae-pyo3 astr_embodiment/bridge.py astr_embodiment/autonomy.py main.py _conf_schema.json tests/test_autonomous_supervisor.py
git commit -m "feat: run persistent adaptive sleep lifecycle"
```

### Task 6 (FL-06 / M2): Implement deterministic multiscale renormalization

**Files:**
- Modify: `crates/ae-renorm/Cargo.toml`
- Modify: `crates/ae-renorm/src/lib.rs`

- [ ] **Step 1: Replace the empty workspace scaffold**

Add `ae-neurofield` and `ae-contracts` dependencies and expose:

```rust
pub struct RenormPyramid {
    pub levels: [Vec<[Fixed; 8]>; 4],
    pub mapping_digest: Digest,
    pub consistency_residual: Fixed,
}

pub struct WorkspaceWinnerV1 {
    pub token_index: u8,
    pub score: Fixed,
    pub token: [Fixed; 8],
}

pub fn restrict(field: &NeuralField, formula_digest: &Digest)
    -> Result<RenormPyramid, RenormError>;
pub fn compete(pyramid: &RenormPyramid, threshold: Fixed)
    -> Option<WorkspaceWinnerV1>;
pub fn prolong_transient(
    winner: &WorkspaceWinnerV1,
    field: &mut NeuralField,
) -> Result<(), RenormError>;
```

Map the eight `NeuralField` vectors into L0 tokens. Every next level is the fixed-point mean of each consecutive group of eight, yielding exactly 16,384→2,048→256→32. Reconstruct by repeating parent values; residual is the maximum absolute fixed-point component error. `prolong_transient` may modify only `excitation` and `eligibility` for the winner's L0 receptive field and must not modify Genesis, relation, permanent memory, or stored baseline potential.

- [ ] **Step 2: Compile then run three focused invariants**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-06'
cargo check --locked -p ae-renorm
cargo test --locked -p ae-renorm
```

Expected: compile passes; tests prove exact level sizes, deterministic mapping digest/rebuild, and prolongation changes transient fields only.

- [ ] **Step 3: Commit renorm independently**

```powershell
git add crates/ae-renorm/Cargo.toml crates/ae-renorm/src/lib.rs
git commit -m "feat: add deterministic multiscale workspace"
```

### Task 7 (FL-07 / M2): Form and persist endogenous intentions

**Files:**
- Create: `crates/ae-agent/src/autonomy.rs`
- Modify: `crates/ae-agent/src/lib.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs`
- Modify: `crates/ae-runtime/src/lib.rs`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `astr_embodiment/bridge.py`

- [ ] **Step 1: Add a deterministic relationship-connection candidate**

Expose:

```rust
pub struct EndogenousSignalsV1 {
    pub affiliation_need: Fixed,
    pub unfinished_topic_salience: Fixed,
    pub social_energy: Fixed,
    pub repetition_penalty: Fixed,
}

pub fn form_endogenous_candidate(
    winner: &WorkspaceWinnerV1,
    signals: &EndogenousSignalsV1,
    now_utc_ms: u64,
) -> Option<DurableIntentionV1>;
```

Score is a fixed weighted sum frozen in schema v1; the semantic idempotency digest binds persona, relation, source InnerEvent ids, action class and semantic revision. No clock-hour/random greeting is permitted. State transitions are limited to the `IntentionStateV1` graph and TTL defaults to 24 h.

- [ ] **Step 2: Connect renorm to every settled wake**

After temporal advance, derive the pyramid from L0, reject externalization when residual exceeds the frozen threshold, compete L3, and persist either `workspace_ignited`, `workspace_suppressed`, `intention_formed`, or `intention_deferred` as committed InnerEvent rows. Persist only mapping digest/winner/operational intention; L1/L2/L3 remain rebuildable.

- [ ] **Step 3: Enforce Native externalization budget**

Implement `AstrRuntime::claim_externalization` and `settle_externalization`. Claim is allowed only for `Ready`, reserves the relation-user-local budget day, returns a minimal redacted prompt contract, allows one provider call per attempt and two attempts per intention only when the first settlement is `transient_provider_failure`. Candidate plaintext never enters Native logs; Native stores digest and encrypted candidate bytes supplied by Host.

- [ ] **Step 4: Compile and run one vertical M2 test**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-07'
cargo check --locked --workspace
cargo test --locked -p ae-runtime endogenous_wake_forms_one_deduplicated_intention -- --exact
```

Expected: compile passes; with no inbound event, a prepared persistent state produces one intention; repeating the same wake returns the same semantic intention id and no second outbound id.

- [ ] **Step 5: Commit M2 integration**

```powershell
git add crates/ae-agent crates/ae-runtime crates/ae-pyo3/src/lib.rs astr_embodiment/bridge.py
git commit -m "feat: form native endogenous outreach intentions"
```

### Task 8 (FL-08 / M3): Bind canonical relations and encrypted outbound targets

**Files:**
- Create: `astr_embodiment/relation.py`
- Create: `astr_embodiment/secret_store.py`
- Modify: `astr_embodiment/tokens.py`
- Modify: `main.py`
- Create: `tests/test_relation_binding.py`

- [ ] **Step 1: Implement canonical relation bytes**

Use NFC UTF-8 fields with `u32be(length) || bytes` and these exact functions:

```python
def canonical_relation_key(
    *, platform_id: str, bot_id: str, persona_id: str,
    target_kind: Literal["private", "group"], target_id: str,
) -> bytes:
    fields = ("v1", platform_id, bot_id, persona_id, target_kind, target_id)
    if target_kind not in {"private", "group"} or any(not value for value in fields):
        raise RelationBindingError("incomplete relation identity")
    out = bytearray()
    for value in fields:
        encoded = unicodedata.normalize("NFC", value).encode("utf-8")
        out.extend(len(encoded).to_bytes(4, "big"))
        out.extend(encoded)
    return bytes(out)

def relation_token_from_key(key: bytes) -> str:
    digest = hashlib.sha256(RELATION_TOKEN_DOMAIN + b"\x00" + key).digest()
    return digest[:16].hex()
```

Private fields are `["v1", platform_id, bot_id, persona_id, "private", user_id]`; group fields substitute `"group", group_id`. Missing/empty fields raise `RelationBindingError`; never derive from nickname or display name.

- [ ] **Step 2: Protect the UMO envelope**

Implement `WindowsDpapiAesGcmStore`: generate one 32-byte AES key, persist only `win32crypt.CryptProtectData` output under the AstrBot plugin data directory, and encrypt UMO with `AESGCM.encrypt(nonce, plaintext, aad)`. AAD is the canonical concatenation of schema version, target kind, all identity tokens, session token, binding generation and binding timestamp. Derive `umo_digest` using HMAC-SHA256 from a distinct HKDF label. Missing `win32crypt`, `cryptography`, key id mismatch, DPAPI failure, HMAC mismatch or non-Windows platform returns capability unavailable and disables proactive dispatch; it must not store plaintext or downgrade encryption.

- [ ] **Step 3: Bind only verified inbound targets**

Update `_scope_for()` to extract platform/bot/persona plus private `sender_id` or group `group_id` from the current verified event, derive a non-null relation token, and create a new target binding generation in Native through the bridge. Preserve raw UMO only long enough to encrypt it. A session rotation creates a new generation; private and group relation policies never share authorization.

- [ ] **Step 4: Compile and run focused relation tests**

Run:

```powershell
python -m compileall -q astr_embodiment\relation.py astr_embodiment\secret_store.py main.py
python -m ruff check astr_embodiment\relation.py astr_embodiment\secret_store.py astr_embodiment\tokens.py main.py tests\test_relation_binding.py
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD = '1'
python -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pytest\fl-08' tests\test_relation_binding.py
```

Expected: compile/lint pass; tests prove NFC/length-prefix stability, private/group separation, and ciphertext/AAD tamper rejection. If DPAPI is unavailable in the test host, the capability test must assert fail-closed rather than skip.

- [ ] **Step 5: Commit Host relation binding**

```powershell
git add astr_embodiment/relation.py astr_embodiment/secret_store.py astr_embodiment/tokens.py main.py tests/test_relation_binding.py
git commit -m "feat: bind encrypted proactive message targets"
```

### Task 9 (FL-09 / M3): Implement Native relation gates and transactional outbox

**Files:**
- Modify: `crates/ae-contracts/src/autonomy.rs`
- Modify: `crates/ae-store/src/autonomy.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `astr_embodiment/bridge.py`
- Create: `crates/ae-runtime/tests/proactive_outbox.rs`

- [ ] **Step 1: Freeze the nine gates in Native order**

Implement one function returning a typed permit/suppression reason:

```rust
pub fn evaluate_proactive_gates(
    state: &AutonomousRuntimeStateV1,
    intention: &DurableIntentionV1,
    policy: &RelationTemporalPolicyV1,
    target: &OutboundTargetEnvelopeV1,
    capability: &HostCapabilitySnapshotV1,
    frozen: &FrozenTimeInputV1,
) -> GateDecisionV1;
```

Order: explicit opt-in; valid target/capability; intention ready/TTL; residual pass; bot awake or emergency wake; quiet-hours rule; daily max; 6 h cooldown; unanswered backoff/hard stop. Emergency ≥0.90 may bypass only authorized quiet hours. Unknown user timezone/fallback too weak, deep sleep, max 2/day, or three unanswered submissions suppresses sending.

- [ ] **Step 2: Complete the transactional outbox state machine**

Externalization settlement inserts stable `dispatch_pending`. Preflight failures before `claim_dispatch` may retain it with a typed retry time. `claim_dispatch(outbound_id, expected_target_digest)` atomically writes `adapter_call_started` and returns target/candidate ciphertext plus token. Every exception/timeout/result loss after successful claim settles `DispatchUnknown`; recovery does the same. Only a provider/platform-declared idempotent API may have a future reconciliation path; v0 never automatic-resends.

- [ ] **Step 3: Compile and run the outbox boundary target**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-09'
cargo check --locked --workspace
cargo test --locked -p ae-runtime --test proactive_outbox
```

Expected: compile passes; tests cover permit, a representative suppression matrix, claim-before-call crash → `dispatch_unknown`, and duplicate wake → same outbound id. Keep this one integration target rather than creating a test per gate.

- [ ] **Step 4: Commit Native M3**

```powershell
git add crates/ae-contracts/src/autonomy.rs crates/ae-store/src/autonomy.rs crates/ae-runtime/src/autonomy.rs crates/ae-pyo3/src/lib.rs astr_embodiment/bridge.py crates/ae-runtime/tests/proactive_outbox.rs
git commit -m "feat: gate proactive dispatch with durable outbox"
```

### Task 10 (FL-10 / M3): Externalize once and call AstrBot proactively

**Files:**
- Create: `astr_embodiment/proactive.py`
- Modify: `astr_embodiment/autonomy.py`
- Modify: `main.py`
- Modify: `_conf_schema.json`
- Create: `tests/test_proactive_adapter.py`

- [ ] **Step 1: Implement the exact AstrBot adapter boundary**

```python
async def submit_proactive_message(
    *, context: Context, session_umo: str, text: str,
) -> bool:
    from astrbot.core.message.message_event_result import MessageChain
    chain = MessageChain().message(text)
    return await context.send_message(session_umo, chain)
```

The caller interprets `True` only as `adapter_submitted`. `False` is `adapter_rejected_terminal`. Exceptions after Native claim are settled as `dispatch_unknown`. Do not fabricate `platform_accepted` or `delivery_confirmed` without a verified platform receipt.

- [ ] **Step 2: Use one economy-mode LLM call after Native claim**

`AutonomousSupervisor` requests externalization only after Native gate approval. Call the configured provider once with the returned minimal prompt contract, no raw neural vectors, no unrelated memory, and no chat history. Reject empty, oversized, duplicate, unsafe, or target-leaking text before `settle_externalization`. The existing `_llm_generate` provider-selection rules remain the single host provider boundary.

- [ ] **Step 3: Execute claim/decrypt/call/settle in the safe order**

The only allowed order is:

```text
Native gate -> claim_externalization -> one LLM call -> settle_externalization
-> capability/target/key preflight -> claim_dispatch (Native transaction commits)
-> decrypt UMO and candidate -> Context.send_message -> settle_dispatch
```

Never decrypt before claim, never call AstrBot before `adapter_call_started` is committed, and zero/drop plaintext references in a `finally` block. A cancellation after claim settles unknown before supervisor exit.

- [ ] **Step 4: Add user-facing controls**

Update schema hints so `proactive_enabled=false` is explicit opt-in. Add `proactive_daily_max=2`, `min_proactive_cooldown_minutes=360`, `intention_ttl_minutes=1440`, `unanswered_backoff_base_minutes=360`, `unanswered_hard_stop=3`, `emergency_threshold=0.90`, `quiet_hours_emergency_bypass=true`, and `inner_activity_token_daily_max`; economy mode ignores the inner-activity budget except for an approved outbound candidate.

- [ ] **Step 5: Compile and run the bounded adapter tests**

Run:

```powershell
python -m compileall -q main.py astr_embodiment
python -m ruff check main.py astr_embodiment tests\test_proactive_adapter.py
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD = '1'
python -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pytest\fl-10' tests\test_proactive_adapter.py
```

Expected: tests prove successful adapter call settles only `adapter_submitted`; false settles terminal reject; exception/cancellation after claim settles unknown; gate suppression makes zero LLM and zero `send_message` calls.

- [ ] **Step 6: Commit the proactive Host slice**

```powershell
git add astr_embodiment/proactive.py astr_embodiment/autonomy.py main.py _conf_schema.json tests/test_proactive_adapter.py
git commit -m "feat: submit one economy-mode proactive message"
```

### Task 11 (FL-11 / M4): Expose truthful mood and inner-event views

**Files:**
- Modify: `crates/ae-store/src/autonomy.rs`
- Modify: `crates/ae-runtime/src/autonomy.rs`
- Modify: `crates/ae-pyo3/src/lib.rs`
- Modify: `astr_embodiment/bridge.py`
- Create: `astr_embodiment/display.py`
- Modify: `main.py`
- Create: `tests/test_inner_activity_display.py`

- [ ] **Step 1: Add read-only paginated queries**

Expose `query_inner_events(scope, after_event_id, limit, display_timezone)` with `1 <= limit <= 100` and stable `(committed_at_utc_ms,event_id)` ordering. Expose `mood_card(scope, frozen_time_input)` from the latest committed runtime state plus referenced event ids. Query methods use read transactions and cannot advance state, claim an intention, call LLM, or change next wake.

- [ ] **Step 2: Render with closed templates only**

Implement:

```python
EVENT_TEMPLATES = {
    "sleep_transition": "睡眠状态：{summary}",
    "memory_reference_surfaced": "记忆引用浮现：{summary}",
    "intention_formed": "形成意图：{summary}",
    "intention_deferred": "暂缓表达：{summary}",
    "intention_suppressed": "抑制表达：{summary}",
    "workspace_residual_rejected": "跨尺度一致性不足：{summary}",
    "outbox_stage": "外部行动阶段：{summary}",
}

def render_mood_card(card: Mapping[str, object]) -> str:
    return "心境：{mood}｜睡眠：{sleep}｜事件：{event_id}".format(**card)

def render_inner_event_page(page: Mapping[str, object]) -> str:
    lines: list[str] = []
    for row in page["items"]:
        template = EVENT_TEMPLATES.get(row["kind"])
        if template is None:
            raise DisplayContractError(f"unknown inner event: {row['kind']}")
        lines.append(f"{row['local_time']} {template.format(**row)} [{row['event_id']}]")
    return "\n".join(lines)
```

Map each known event enum to a fixed Chinese template such as sleep transition, memory-reference surfaced, intention formed/deferred/suppressed, workspace residual rejected, and outbox stage. Unknown schema/enum raises `DisplayContractError`; never ask an LLM to invent an explanation.

- [ ] **Step 3: Add `ae_mood` and `ae_mind` commands**

Both commands honor `inner_activity_display`; when disabled they return a short “展示已关闭，生命循环仍在运行” message. Deletion writes tombstones and removes readable rows/ciphertext while preserving minimal idempotency/outbox tombstones so old outbound work cannot revive.

- [ ] **Step 4: Compile and run one display target**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\fl-11'
cargo check --locked --workspace
python -m compileall -q main.py astr_embodiment\display.py
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD = '1'
python -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pytest\fl-11' tests\test_inner_activity_display.py
```

Expected: compile passes; tests prove every rendered row cites a committed event id, display-off does not stop wake revisions, query makes zero LLM calls, and tombstoned outbound semantics do not reappear.

- [ ] **Step 5: Commit M4**

```powershell
git add crates/ae-store/src/autonomy.rs crates/ae-runtime/src/autonomy.rs crates/ae-pyo3/src/lib.rs astr_embodiment/bridge.py astr_embodiment/display.py main.py tests/test_inner_activity_display.py
git commit -m "feat: expose committed inner activity views"
```

### Task 12 (FL-12): Integrate, verify once, and perform the real AstrBot smoke

**Files:**
- Modify only when a prior gate exposes a blocking defect: the owning task's existing write scope
- Evidence: `G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\evidence\`

- [ ] **Step 1: Integrate candidate commits in DAG order**

Order: FL-01, FL-02/03/04 after rebasing to FL-01, FL-05, FL-06, FL-07, FL-08, FL-09, FL-10, FL-11. Resolve shared-file changes only in the coordinator lane. Record candidate commit, base commit, integrated commit and evidence hash for each unit.

Expected: `git diff --check` is clean after each integration; a failure rolls back to the prior wave boundary without discarding unrelated user work.

- [ ] **Step 2: Run compile-first final gates**

Run:

```powershell
$env:CARGO_TARGET_DIR = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\cargo\final'
$env:PYTHONPYCACHEPREFIX = 'G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pycache'
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD = '1'
cargo check --locked --workspace
python -m compileall -q main.py astr_embodiment tests
python -m ruff check main.py astr_embodiment tests
python -c "import json; json.load(open('_conf_schema.json', encoding='utf-8')); print('config json: OK')"
```

Expected: all commands exit 0 and print no errors; JSON command prints `config json: OK`.

- [ ] **Step 3: Run only the affected regression set once**

Run:

```powershell
cargo test --locked --workspace
python -m pytest -q -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\cyberhuman-v0\pytest\final' tests\test_runtime_integration.py tests\test_temporal_projection.py tests\test_autonomous_supervisor.py tests\test_relation_binding.py tests\test_proactive_adapter.py tests\test_inner_activity_display.py
```

Expected: both commands exit 0. Do not rerun passing suites; if a failure occurs, fix only the owning boundary and rerun that failed target plus this final affected set once.

- [ ] **Step 4: Run the manual AstrBot smoke with a dedicated persona/session**

Use `America/Los_Angeles` persona time and `Asia/Shanghai` relation time. Enable autonomous runtime and display, leave proactive disabled for the first wake, and record UTC/persona/user local time, sleep state, Process S/C, generation, next wake and LLM call count. With no inbound message, observe one revision/InnerEvent increase and zero LLM calls. Prepare the controlled “relationship connection need + unfinished topic” fixture; verify one suppressed gate causes zero send. Then enable proactive, clear only that gate, and observe one LLM call, one stable outbound id and one message in the dedicated AstrBot session.

Repeat the crash points: before `claim_dispatch` may resume the same pending item after preflight; after claim but before call, and after call before settlement, must recover as `dispatch_unknown` with no automatic resend. Put persona asleep to suppress ordinary outreach, apply an allowed emergency wake and independently check user quiet-hour policy. Travel to Shanghai and verify wall time switches immediately while phase moves no faster than 90 minutes/day. Disable display and proactive separately and verify neither stops state evolution. Terminate plugin and verify no later write/generation occurs.

Expected evidence: repository/plugin revision, redacted persona/relation scope, UTC timeline, state revisions/generations, InnerEvent ids/types, LLM counts, outbound id, adapter return, any real platform receipt, and explicit classification `adapter_submitted` unless stronger evidence exists. Never retain message plaintext or raw UMO in evidence.

- [ ] **Step 5: Inspect final state and create the delivery commit**

Run:

```powershell
git diff --check
git status --short --branch
git log --oneline --decorate -15
codegraph status .
$changed = git diff --name-only HEAD~1 HEAD
codegraph affected $changed
```

Expected: no uncommitted implementation changes, index is current after the trusted host's single output sync, and affected paths match the regression set.

- [ ] **Step 6: Final coordinator acceptance**

Accept only when all of the following are backed by current exit/output evidence: authority migration preserved zero irreversible `TimeAdvance` authority; workspace compile passes; no-inbound state advances; restart is bounded; dual timezones and travel are correct; sleep/wake is adaptive; renorm and intention are deterministic; economy background uses zero LLM; suppressed gates send nothing; one real proactive message reaches the dedicated session; claim-unknown never auto-resends; display rows trace to committed events; terminate stops future writes.

Final report is one delivery with integrated commit/hash, dirty/clean status, exact compile/test/smoke evidence, and any honest platform limitations. A focused/static PASS without the real AstrBot smoke is `PARTIAL`, not CyberHuman v0 complete.

## Fast Lane unit summary

| Unit | Depends on | Write scope | Parallel eligibility |
|---|---|---|---|
| FL-01 | FL-00 | contracts, authority, matrix, static authority test | single writer |
| FL-02 | FL-01 | `ae-store` autonomy files/tests | parallel with FL-03/04 |
| FL-03 | FL-01 | new `ae-autonomy`, workspace Cargo files | parallel with FL-02/04; root Cargo integration by coordinator |
| FL-04 | FL-01 | temporal Host files/test | parallel with FL-02/03 |
| FL-05 | FL-02/03/04 | runtime/PyO3/bridge/supervisor/main/config | serialized integration owner |
| FL-06 | FL-05 | `ae-renorm` only | single writer |
| FL-07 | FL-06 | agent + runtime/PyO3/bridge | serialized integration owner |
| FL-08 | FL-07 | relation/secret/tokens/main/test | parallel with FL-09 except shared integration waits |
| FL-09 | FL-07 | contracts/store/runtime/PyO3/bridge/outbox test | parallel with FL-08; one Native writer |
| FL-10 | FL-08/09 | proactive/supervisor/main/config/test | serialized Host integration owner |
| FL-11 | FL-10 | store/runtime/PyO3/bridge/display/main/test | serialized final feature owner |
| FL-12 | FL-11 | coordinator integration/evidence only | no worker self-acceptance |

Plan complete and saved to `docs/superpowers/plans/2026-08-28-cyberhuman-autonomous-runtime-v0-implementation.md`. The approved execution mode is Fast Lane coordinated implementation: dispatch exact-route workers for disjoint units, queue same-path work, and return one final integrated delivery.
