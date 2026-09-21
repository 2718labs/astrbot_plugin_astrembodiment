# Proactive Settings Simplification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the flat proactive-contact controls with four safe frequency modes, grouped quiet hours, and a truthful proactive-expression Token budget while preserving legacy behavior and failing closed on uncertain origin, configuration, or budget state.

**Architecture:** Python owns one pure stable-config resolver and transactional AstrBotConfig migration; it never chooses an auto runtime band. The existing Native gate remains the sole send authority and gains only the minimum backward-compatible fields and pure evaluator needed to bind contact, budget, frozen time, and effective frequency into its current authority snapshot. Existing consent, cause, sleep, quiet-hours, readiness, claim, settlement, and dispatch gates remain mandatory.

**Tech Stack:** Python 3.12, AstrBot plugin configuration schema, Rust 2021, serde closed contracts, rusqlite immediate transactions, PyO3 bridge, pytest, Cargo.

---

## Execution contract

- Worktree: G:\AstrEmbodiment\.codex-task-temp\astrembodiment-headless-body-alpha3-worktree.
- Frozen input: docs/superpowers/specs/2026-08-30-proactive-settings-simplification-design.md.
- Planning baseline: clean 3c89a228947c891e88699253c4574785116fdd7b on codex/astrembodiment-headless-body-alpha3.
- Execute Tasks 1-7 in order. Each implementation task starts from the accepted predecessor commit and touches only its listed files.
- Fast Lane registration returned DATA_ROOT_UNAVAILABLE / request rejected; no route, lease, or compiled index is attested. This is DEGRADED_SKILL_ONLY: use checked-out source evidence, do not guess routing authority, and do not claim Fast Lane acceptance.
- Do not use rg.exe. Use git grep, git ls-files, Get-ChildItem, and Select-String.
- Keep temporary roots below G:\AstrEmbodiment\.codex-task-temp\proactive-settings-*; set PYTEST_DISABLE_PLUGIN_AUTOLOAD=1 and a task-local pytest cache.
- Keep tests bounded: one table-driven Python resolver test, one Host migration/wiring test, one Rust evaluator table, one Native gate/budget integration test, then compile/static checks.
- Prepare the source increment after alpha3 as 1.1.0-alpha4 / 1.1.0a4. Never overwrite, relabel, or rebuild an alpha3 wheel/ZIP as alpha4.
- Focused contracts and compilation do not prove AstrBot WebUI rendering, offline Host load, dual-platform wheels, archive composition, or a real proactive send. Task 7 keeps these boundaries explicit.

### Host configuration API evidence gate

The checkout proves only a runtime-compatible save boundary: main.py::_persist_seed mutates the supplied config, prefers save_config_async, falls back to save_config, and rolls back on failure. AstrBot's public plugin-config contract documents a config mapping and save_config(); it does not promise a plugin-visible, pre-merge, schema-bound first-install attestation. Current upstream first_deploy is an internal detail and is not bound to plugin ID, config entity, schema digest, and this load.

Before editing production code, inspect the actually installed supported Host without mutation:

~~~powershell
python -c "import inspect; from astrbot.core.config.astrbot_config import AstrBotConfig; print(inspect.getsourcefile(AstrBotConfig)); print(inspect.signature(AstrBotConfig)); print([n for n in ('save_config_async','save_config') if callable(getattr(AstrBotConfig,n,None))])"
~~~

Expected: callable save methods are identified, or AstrBot is absent in the implementation environment. Neither proves fresh installation. Alpha4 production must pass origin unknown. Keep fresh-current-schema as a typed resolver input for a future attested Host adapter and its unit row, but never infer it from config values, Host files/timestamps/directories, SeedCode, DB existence, or first_deploy. If no save method is callable, migration persistence is unavailable; keep legal legacy runtime behavior and revision 0 for retry.

### Minimal Native change, not a new state machine

Python normalization alone cannot authorize auto safely: contact state, active ledger, today's submitted count, and claim state can change after Python reads them. crates/ae-store/src/alpha3/projection.rs::load_gate_authority already reads contact, temporal policy, budget policy/ledger, target, runtime state, and unsettled usage in the gate's TransactionBehavior::Immediate. Extend that transaction and snapshot. Do not add an auto scheduler, table, thread, or second policy authority.

The only new Native surface is:

1. defaulted auto_policy_version and next_claim_reservation_tokens on RelationTemporalPolicyV1;
2. closed EffectiveFrequencyV1 plus an optional/defaulted GateDecisionV2 field;
3. one pure evaluator in ae-agent::contact;
4. revision-0 allocation in the existing budget-policy upsert and active-ledger reconciliation.

Budget 0 closes in Python before Native/Provider work. Every positive budget must reach the real Native policy and current ledger before a claim; bridge/upsert failure re-bootstraps proactive_enabled=false for that relation.

## Stable invariants

1. Restrained is 2/360, moderate is 4/180, custom is the exact legal pair, and auto can produce only 2/360, 3/240, or 4/180.
2. Auto is only an upper-limit selector inside an already-valid gate. It cannot create cause/consent, wake, bypass any gate, invoke Provider, or send.
3. Missing/future/regressing auto evidence suppresses. Any unanswered contact selects restrained plus max(360 minutes, base*2^(n-1)); hard stop suppresses. Time alone cannot widen without a real inbound reset.
4. Revision 0 migrates from legacy. Revision 1 trusts only a complete new object. Invalid rev1, negative/non-int, or revision >1 fails closed; hidden legacy never resurrects it.
5. Migration writes only proactive_frequency, user_quiet_hours, and proactive_settings_revision. It never overwrites legacy fields or inner_activity_token_daily_max.
6. Digest is calculated after the final save outcome from one normalized policy. Auto binds its stable envelope/version, not a dynamic band. Fail-closed category and active TTL/backoff/hard-stop overrides are bound.
7. The complete claim reservation is one Python constant with value 256, stored in temporal policy, consumed by Native auto, used by the real externalization request, and verified before reservation.
8. Lowering budget never erases consumption: current ledger limit becomes max(new limit, charged+reserved), making remaining allowance zero until valid recovery/new day.
9. All Provider calls remain after Native allow+claim. Any preparation error leaves no Provider path.

## File responsibility map

| Path | Responsibility |
|---|---|
| astr_embodiment/proactive_settings.py | Pure strict parsing, migration proposal, normalized digest input; no Host I/O or runtime auto selection. |
| _conf_schema.json | Static object groups, unconditional custom fields, hidden compatibility keys, truthful budget label. |
| main.py | Save/rollback, final digest, relation temporal+budget preparation, supervisor ordering. |
| astr_embodiment/autonomy.py | Prepare current and recovered scopes before recovery/pending/claim. |
| astr_embodiment/proactive.py | Shared 256-token reservation used by the real claim. |
| astr_embodiment/bridge.py | Typed wrapper for existing upsert_budget_policy. |
| crates/ae-contracts/src/autonomy.rs | Defaulted temporal auto contract. |
| crates/ae-contracts/src/alpha3.rs | Closed effective-frequency decision value. |
| crates/ae-agent/src/contact.rs | Pure fixed/auto frequency evaluator. |
| crates/ae-store/src/alpha3/projection.rs | Same-transaction evidence, count, budget reconciliation, gate snapshot. |

### Task 1: Add the pure stable-config resolver

**Files:**
- Create: astr_embodiment/proactive_settings.py
- Create: tests/test_proactive_settings.py

- [ ] **Step 1: Write one table-driven RED test**

Create test_resolve_proactive_settings_matrix. Deep-copy every input, call resolve_proactive_settings, and assert the copy is unchanged. Cover these exact rows in one parameter table:

| Case | Input | Expected |
|---|---|---|
| trusted fresh default | rev0, typed fresh, complete auto and legacy 2/360 defaults | auto v1 envelope; migration patch |
| unknown/existing default | rev0, no valid origin, legacy 2/360 | restrained 2/360; compatibility notice; patch |
| forged origin | rev0 plus unverified dict/string evidence | treated unknown; restrained 2/360 |
| legacy custom | rev0, legacy 7/90 | exact custom 7/90; old input unchanged |
| legal zero | rev0, legacy 0/0 | exact custom 0/0 |
| invalid legacy | -1/360, bool, or numeric string | disabled, invalid-legacy, no patch |
| rev1 modes | auto/restrained/moderate/custom 5/120 | envelope / 2/360 / 4/180 / 5/120 |
| invalid rev1 | missing group, invalid custom, invalid quiet | disabled; no legacy fallback |
| future/bad revision | 2, negative, bool, or string | disabled; no migration |
| historical budget | inner_activity_token_daily_max=2048 | budget exactly 2048 |
| zero/invalid budget | 0; negative/bool/string/overflow | zero disables; invalid fails closed |
| hidden policy | legal non-default TTL/backoff/hard-stop | exact values retained and digested |

Use these public types/signature:

~~~python
class OriginKind(StrEnum):
    FRESH_CURRENT_SCHEMA = "fresh-current-schema"
    EXISTING = "existing"
    UNKNOWN = "unknown"

class FrequencyMode(StrEnum):
    AUTO = "auto"
    RESTRAINED = "restrained"
    MODERATE = "moderate"
    CUSTOM = "custom"

@dataclass(frozen=True)
class MigrationPatch:
    frequency: Mapping[str, object]
    quiet_hours: Mapping[str, object]
    revision: int

@dataclass(frozen=True)
class ResolvedProactivePolicy:
    effective_enabled: bool
    configured_mode: FrequencyMode | None
    auto_policy_version: int | None
    auto_envelope: tuple[int, int, int, int] | None  # daily min/max, cooldown-ms min/max
    fixed_daily_max: int | None
    fixed_cooldown_ms: int | None
    quiet_start_minute: int
    quiet_end_minute: int
    allow_authorized_emergency_bypass: bool
    intention_ttl_ms: int
    unanswered_backoff_base_ms: int
    unanswered_hard_stop: int
    emergency_threshold_fxp6: int
    expression_token_daily_max: int
    source_kind: str
    failure_reason: str | None
    migration_patch: MigrationPatch | None
    normalized_digest_input: Mapping[str, object]

resolve_proactive_settings(
    config: Mapping[str, object],
    *,
    origin_kind: OriginKind = OriginKind.UNKNOWN,
    migration_status: Literal["not-attempted", "saved", "save-failed"] = "not-attempted",
) -> ResolvedProactivePolicy
~~~

~~~powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_proactive_settings.py::test_resolve_proactive_settings_matrix -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-01-red'
~~~

Expected RED: collection fails because astr_embodiment.proactive_settings is absent.

- [ ] **Step 2: Implement strict parsing and migration proposal**

Use AUTO_POLICY_VERSION=1, AUTO_PAIRS=((2,21600000),(3,14400000),(4,10800000)), AUTO_ENVELOPE=(2,4,10800000,21600000), U16_MAX=65535, and U64_MAX=18446744073709551615. _strict_int rejects bool and non-int; checked minutes*60000 must fit u64; HH:MM must be two decimal components in range. proactive_enabled and both emergency-bypass values must be actual bool values rather than Python truthiness conversions.

Resolution order:

~~~text
revision invalid, <0, or >1 -> fail closed, no patch
revision 1                 -> validate only new groups; ignore legacy frequency
revision 0 + typed fresh   -> require complete default auto surface and legacy 2/360
revision 0 + other origin  -> legacy fallback 2/360; restrained if equal, else custom
invalid legacy/fresh mismatch -> fail closed, no patch
budget 0                  -> effective_enabled false with budget-disabled
~~~

For rev0 malformed legacy quiet hours, preserve raw old keys, propose safe 00:30/08:30/true in the new group, and record legacy-quiet-invalid. Malformed rev1 quiet hours fails closed. Legal custom daily/cooldown include zero. Preserve legal non-default hidden TTL/backoff/hard-stop.

normalized_digest_input contains only typed values. Auto includes mode/version/restrained/balanced/moderate; fixed modes include one pair. Always include enabled, source/failure category, quiet hours, TTL, backoff, hard stop, threshold, and expression budget. Never include duplicate raw inputs or a dynamic auto band.

- [ ] **Step 3: Run GREEN and commit**

~~~powershell
python -m pytest -q tests/test_proactive_settings.py::test_resolve_proactive_settings_matrix -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-01-green'
git add astr_embodiment/proactive_settings.py tests/test_proactive_settings.py
git commit -m "feat: resolve simplified proactive settings"
~~~

Expected: the named table test passes and the commit contains only resolver/test.

### Task 2: Add static schema and transactional migration/digest

**Files:**
- Modify: _conf_schema.json
- Modify: tests/test_static_contracts.py
- Modify: main.py (__init__, initialize, add _prepare_proactive_settings, _persist_proactive_migration, _refresh_autonomy_config_identity)
- Modify: tests/test_runtime_integration.py (FakeConfig; add one migration/save test)

- [ ] **Step 1: Extend the schema test and add one Host migration RED test**

Extend test_config_schema_parses to assert:

- proactive_frequency and user_quiet_hours are objects with default {};
- options/labels are exactly auto/restrained/moderate/custom and 自动/克制/适中/自定义;
- custom inputs are unconditional static items, default 2/360, with custom-only hints;
- revision default is 0 and all nine legacy/internal keys are invisible;
- the Token storage key is unchanged and description is 主动表达每日 Token 预算.

Add test_proactive_settings_migration_save_and_rollback with three subcases: successful async save advances only the three new top-level values to rev1; save_config_async returning False restores touched values and uses unknown-origin legacy 2/360 for this run; exception after concurrent replacement restores only values still equal to this patch and never stomps replacement. Assert legacy frequency/quiet/budget remain unchanged.

~~~powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_static_contracts.py::test_config_schema_parses tests/test_runtime_integration.py::test_proactive_settings_migration_save_and_rollback -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-02-red'
~~~

Expected RED: groups and migration helpers are absent.

- [ ] **Step 2: Define static AstrBot objects without invented visibility APIs**

proactive_frequency.items always contains mode, custom_daily_max, custom_cooldown_minutes. user_quiet_hours.items contains start, end, allow_authorized_emergency_bypass. Use design hints for auto evidence/envelope, preset pairs, custom-only effect, and emergency limitations. Do not invent conditional visibility.

Set invisible=true on exactly: proactive_daily_max, min_proactive_cooldown_minutes, quiet_hours_start, quiet_hours_end, quiet_hours_emergency_bypass, intention_ttl_minutes, unanswered_backoff_base_minutes, unanswered_hard_stop, emergency_threshold. Keep proactive_enabled and budget visible.

- [ ] **Step 3: Persist only the migration patch through runtime-compatible methods**

In __init__, preserve self.config, copy values, resolve with OriginKind.UNKNOWN, and remove the current raw config hash. In initialize, call _prepare_proactive_settings before NativeBridge.open and supervisor construction.

_persist_proactive_migration must:

1. snapshot only the three touched top-level keys using a private missing sentinel;
2. write deep plain-dict patch copies to self.config;
3. await callable save_config_async and treat explicit False as failure; otherwise call save_config; missing methods fail without file I/O;
4. on failure restore/delete only if current value still equals this task's patch, preserving concurrent replacement;
5. refresh self._config_values=dict(self.config) after success/rollback;
6. rerun resolver with saved or save-failed. Failed fresh disables; unknown/existing retains legal legacy rev0.

Log only origin, validation, chosen mode/fail-closed, and save result; never config bodies, targets, messages, secrets, or relation IDs.

- [ ] **Step 4: Replace raw digest with normalized final identity**

~~~python
def _refresh_autonomy_config_identity(self) -> None:
    canonical = {
        key: value for key, value in self._config_values.items()
        if key not in PROACTIVE_INPUT_KEYS
    }
    canonical["proactive_policy_v1"] = dict(
        self._proactive_policy.normalized_digest_input
    )
    encoded = json.dumps(
        canonical, sort_keys=True, separators=(",", ":"), default=str
    ).encode()
    digest = hashlib.sha256(b"ae.host-config-source.v2\0" + encoded).digest()
    self._autonomy_config_source_digest = digest.hex()
    self._autonomy_config_revision = int.from_bytes(digest[:8], "big") or 1
~~~

Call only after final migration resolution and before Native/supervisor. PROACTIVE_INPUT_KEYS removes new groups/revision and all legacy proactive fields before inserting normalized policy. Thus rev1 hidden frequency duplicates cannot compete, while active hidden safety values/failure category remain bound.

- [ ] **Step 5: Run GREEN and commit**

~~~powershell
python -m pytest -q tests/test_static_contracts.py::test_config_schema_parses tests/test_runtime_integration.py::test_proactive_settings_migration_save_and_rollback -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-02-green'
python -m py_compile main.py astr_embodiment/proactive_settings.py
git add _conf_schema.json main.py tests/test_static_contracts.py tests/test_runtime_integration.py
git commit -m "feat: migrate proactive settings safely"
~~~

Expected: both nodes pass, py_compile exits 0, and rollback changes no legacy input.

### Task 3: Add the minimal Native frequency contract and pure evaluator

**Files:**
- Modify: crates/ae-contracts/src/autonomy.rs (RelationTemporalPolicyV1)
- Modify: crates/ae-contracts/src/alpha3.rs (EffectiveFrequencyBandV1, FrequencySelectionReasonV1, EffectiveFrequencyV1, GateDecisionV2)
- Modify: every tracked Rust RelationTemporalPolicyV1 literal found by git grep -n -F "RelationTemporalPolicyV1 {" -- "*.rs"
- Modify: crates/ae-agent/src/contact.rs
- Create: crates/ae-agent/tests/proactive_frequency.rs

- [ ] **Step 1: Write one evaluator-table RED test**

Name it frequency_policy_is_closed_and_unanswered_never_widens. The single table asserts fixed restrained/moderate/custom passthrough and auto rows:

- inactive -> 2/360;
- token ceiling 2 -> 2/360;
- active, no outbound, ceiling 4 -> 3/240;
- active, inbound newer than outbound, ceiling 4 -> 4/180;
- active, ceiling 3 -> 3/240;
- unanswered 1/2 -> restrained with 6h/12h cooldown;
- hard stop -> suppression;
- future inbound/outbound, ledger underflow, or zero reservation -> suppression.

A second call with later time but no new inbound must never produce a wider pair than an unanswered call.

~~~powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-cargo\task-03-red'
cargo test --locked --offline -p ae-agent --test proactive_frequency frequency_policy_is_closed_and_unanswered_never_widens -- --exact
~~~

Expected RED: contracts/evaluator do not exist.

- [ ] **Step 2: Add backward-compatible contract fields and result types**

Append to RelationTemporalPolicyV1:

~~~rust
#[serde(default)]
pub auto_policy_version: u16, // 0=fixed, 1=only supported auto contract
#[serde(default)]
pub next_claim_reservation_tokens: u64,
~~~

Set all Rust literals deliberately: 0/0 for historical compatibility fixtures that must not gain authority; 1/256 for new auto fixtures; 0/256 for current fixed policy fixtures.

In alpha3.rs add closed snake-case bands Restrained, Balanced, Moderate, Custom and reasons FixedPolicy, Unanswered, InactiveRelation, BudgetCeiling, RecentReply, ActiveRelation, ConservativeDefault. Define:

~~~rust
pub struct EffectiveFrequencyV1 {
    pub band: EffectiveFrequencyBandV1,
    pub daily_max: u16,
    pub cooldown_ms: u64,
    pub selection_reason: FrequencySelectionReasonV1,
    #[serde(with = "crate::hex::d32")]
    pub evidence_digest: Digest,
}
~~~

Add #[serde(default)] pub effective_frequency: Option<EffectiveFrequencyV1> to GateDecisionV2 and set existing literals to None. None remains decodable for historical decisions and pre-frequency suppression, but a newly produced allowed claim and its current dispatch verification must require Some. Some rejects zero evidence digest; Restrained/Balanced/Moderate require their exact pairs, while Custom accepts the already-validated u16/u64 pair. Defaults preserve old JSON decoding without letting an old decision gain new send authority.

- [ ] **Step 3: Implement the pure evaluator in ae-agent::contact**

Define FrequencyEvidenceV1 with references to temporal policy, RelationContactProcessV1, RelationBudgetLedgerV1, frozen effective time, and authoritative daily_submitted. Validate checked arithmetic and implement exactly:

~~~text
activity_window = ema ? clamp(2*ema, 24h, 14d) : 7d
active = inbound exists AND now>=inbound AND now-inbound<=activity_window
remaining = limit-charged-reserved
remaining_claims = remaining/next_claim_reservation_tokens
token_ceiling = min(4, daily_submitted+remaining_claims)
hard stop -> suppress before a band
unanswered>0 -> restrained, cooldown=max(6h*2^(n-1), 6h)
inactive or ceiling<=2 -> restrained
active and outbound exists and inbound>outbound and ceiling>=4 -> moderate
active and ceiling>=3 -> balanced
otherwise -> restrained
~~~

For auto_policy_version=0, derive fixed restrained/moderate/custom from the stored pair with a config-fixed evidence digest. Reject other auto versions, zero reservation, overflow/underflow, contact time after frozen now, or illegal policy values. Hash full typed input+result with domain ae.alpha4.effective-frequency-evidence.v1; this is evidence, not persisted state.

- [ ] **Step 4: Run GREEN and commit**

~~~powershell
cargo test --locked --offline -p ae-agent --test proactive_frequency frequency_policy_is_closed_and_unanswered_never_widens -- --exact
cargo check --locked --offline -p ae-contracts -p ae-agent
git add crates/ae-contracts/src/autonomy.rs crates/ae-contracts/src/alpha3.rs crates/ae-agent/src/contact.rs crates/ae-agent/tests/proactive_frequency.rs
git add -- $(git grep -l -F "RelationTemporalPolicyV1 {" -- "*.rs")
git commit -m "feat: define native proactive frequency policy"
~~~

Expected: named test and checks exit 0; no DB schema/background state was added.

### Task 4: Bind effective frequency and positive budgets inside the existing gate

**Files:**
- Modify: crates/ae-store/src/alpha3/projection.rs (GateAuthority, ensure_budget_ledger, load_gate_authority, gate_reason, make_gate_decision, producer_gate_authority_snapshot, upsert_relation_budget_policy_v1)
- Modify: crates/ae-runtime/tests/alpha3_projection.rs

- [ ] **Step 1: Add one Native authority RED test**

Add auto_frequency_and_budget_policy_share_gate_authority. Reuse existing genesis/consent/contact/target/readiness helpers. The test proves:

1. budget upsert revision 0 stores revision 1; identical stable fields are idempotent; 2048->128 allocates next revision;
2. a current-day ledger with 256 charged reconciles to limit 256 after the 128 policy, leaving zero available without violating invariants;
3. active/no outbound/1024 remaining yields balanced 3/240; later inbound newer than submitted outbound yields moderate 4/180;
4. unanswered yields restrained+backoff and hard stop suppresses;
5. today's count comes from submitted outbound_attempt rows, not stale temporal_policy.daily_submitted;
6. externalization max_tokens unequal to stored reservation is rejected before reservation;
7. changing contact, budget, frozen time, count, or result changes authority snapshot digest, and dispatch recomputes current authority.

~~~powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-cargo\task-04-red'
cargo test --locked --offline -p ae-runtime --test alpha3_projection auto_frequency_and_budget_policy_share_gate_authority -- --exact
~~~

Expected RED: revision 0 is rejected and decision has no effective-frequency evidence.

- [ ] **Step 2: Allocate budget revision in the existing immediate transaction**

Keep the explicit positive-revision CAS path for old callers. For input revision 0, compare stable fields (relation_scope, timezone_id, daily_token_limit, source_digest): no row -> revision 1; equal -> current unchanged; changed -> current.revision.checked_add(1) and CAS. Continue rejecting zero limit/digest and timezone mismatch. Return actual stored revision.

Change ensure_budget_ledger to load/create then reconcile the current day:

~~~rust
let consumed = ledger.charged_tokens.checked_add(ledger.reserved_tokens)?;
let target_limit = policy.daily_token_limit.max(consumed);
if ledger.limit_tokens != target_limit {
    let previous = ledger.revision;
    ledger.limit_tokens = target_limit;
    ledger.revision = ledger.revision.checked_add(1)?;
    save_budget_ledger(tx, previous, &ledger)?;
}
~~~

This tightens immediately without erasing charges/reservations. Do not touch historical days or add another ledger.

- [ ] **Step 3: Replace stale temporal counters with same-transaction authority**

Add a private helper that parses submitted outbound_attempt rows for the same relation and frozen.budget_day_start_utc_ms. Count only AdapterSubmitted, PlatformAccepted, DeliveryConfirmed, DispatchUnknown, matching crates/ae-store/src/autonomy.rs authoritative legacy logic. Use authority.contact.last_outbound_submitted_utc_ms and contact.consecutive_unanswered; never temporal compatibility copies.

Store daily_submitted in GateAuthority. Externalization uses request max_tokens; dispatch uses temporal_policy.next_claim_reservation_tokens, never request zero. Reject zero stored reservation or mismatch before readiness/claim.

- [ ] **Step 4: Evaluate and bind the pair in the current gate snapshot**

Preserve current ordering through global/consent/cause/timezone/target/provider/send/sleep/quiet/unsettled/budget/policy gates. Hard stop precedes frequency. Auto itself never bypasses sleep or quiet hours: only the existing authorized emergency wake path may first produce an Awake frozen snapshot, and quiet bypass is used only where the current gate can prove that existing authority; absence of proof suppresses. Then call the pure evaluator and enforce its daily max/effective cooldown plus unanswered backoff. Earlier suppression/evaluator error gives effective_frequency=None and no claim.

Add authoritative daily count, reservation, and effective-frequency commitment to GateAuthoritySnapshotV1. Keep the existing ae.alpha3.gate-authority-snapshot.v1 commitment; no second table/digest authority. Put the same value in GateDecisionV2. Dispatch recomputes inside its own immediate transaction.

- [ ] **Step 5: Run GREEN and commit**

~~~powershell
cargo test --locked --offline -p ae-runtime --test alpha3_projection auto_frequency_and_budget_policy_share_gate_authority -- --exact
cargo check --locked --offline -p ae-store -p ae-runtime
git add crates/ae-store/src/alpha3/projection.rs crates/ae-runtime/tests/alpha3_projection.rs
git commit -m "feat: enforce proactive frequency in native gate"
~~~

Expected: named test/checks pass; SQLite schema/table list is unchanged.

### Task 5: Prepare every relation policy/budget before recovery or Provider work

**Files:**
- Modify: astr_embodiment/bridge.py (upsert_relation_budget_policy_v1)
- Modify: astr_embodiment/proactive.py (PROACTIVE_CLAIM_RESERVATION_TOKENS)
- Modify: astr_embodiment/autonomy.py (AutonomousSupervisor.__init__, _run)
- Modify: main.py (_autonomy_binding, _prepare_autonomy_scope, _on_autonomous_externalization, direct bootstrap callers, supervisor construction)
- Modify: tests/test_runtime_integration.py (native fake; one preparation test)

- [ ] **Step 1: Add one Host wiring RED test**

Add test_proactive_budget_is_prepared_before_recovery_or_provider with a recording fake:

- rev1 auto + budget 2048: temporal bootstrap first with envelope 4/180, v1, reservation 256; upsert second with 2048/revision0/normalized digest; only then recover/pending/claim; actual externalization uses max_tokens 256;
- budget 0 or upsert exception: bootstrap disabled, no pending externalization/generate/send, retry positive preparation later;
- every work_scope from autonomy_status is prepared before recovery/pending, not only _latest_binding; persona wake may continue only after each relation is either positively prepared or proven disabled.

~~~powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_runtime_integration.py::test_proactive_budget_is_prepared_before_recovery_or_provider -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-05-red'
~~~

Expected RED: no budget wrapper/preparation callback exists.

- [ ] **Step 2: Add thin bridge and one reservation constant**

~~~python
def upsert_relation_budget_policy_v1(
    self, request: dict[str, Any]
) -> dict[str, Any]:
    return self.alpha3_call("upsert_budget_policy", request)
~~~

Define PROACTIVE_CLAIM_RESERVATION_TOKENS=256 in proactive.py, replace local max_tokens=256, and import it in main.py. Do not duplicate 256 elsewhere in Python.

- [ ] **Step 3: Make _autonomy_binding consume only resolved policy**

Remove direct reads of all new/old proactive inputs and _local_minute fallbacks from relation policy construction. Auto sends envelope daily=4/cooldown=180m, auto version 1; fixed sends exact resolved pair, version 0. Always send resolved quiet/TTL/backoff/hard-stop/threshold and reservation 256. Keep temporal contact copies zero/None only for wire compatibility; Task 4 no longer trusts them.

Use the normalized post-migration digest/revision. _on_autonomous_externalization checks self._proactive_policy.effective_enabled, never raw proactive_enabled.

- [ ] **Step 4: Add fail-closed preparation and supervisor ordering**

_prepare_autonomy_scope(scope_mapping) returns the closed string status ready or disabled and raises if it cannot establish either state:

1. reconstructs ScopeTokens and bootstraps resolved temporal policy;
2. if disabled or budget 0, re-bootstraps proactive_enabled=false and returns disabled;
3. derives budget source digest from domain ae.host-budget-policy.v1 plus canonical timezone, positive limit, and normalized policy digest;
4. upserts schema1/relation/timezone/limit/revision0/source digest;
5. verifies returned scope/timezone/limit/digest; any error/mismatch immediately bootstraps disabled and returns disabled; if that disabling bootstrap also fails, raise;
6. returns ready only after both Native authorities match.

Pass this synchronous callback to AutonomousSupervisor. Each _run cycle prepares latest binding, every recovered runtime scope, and every relation work_scope before recover_autonomy, pending_autonomy_work, or claim_wake_v2. Disabled skips that relation's pending externalization but still proves a closed policy; an exception skips recovery/claim for that cycle and uses existing bounded retry. Persona wake is not blocked by a successfully disabled relation. Do not await between temporal bootstrap and budget upsert/disable.

Replace direct bootstrap_autonomy(_autonomy_binding(scope)) callers in contact control and inbound processing with _prepare_autonomy_scope. Continue recording valid inbound facts and explicit consent controls after a ready or disabled result; budget only suppresses externalization. Raise only when neither current policy nor disabled policy could be established. Start supervisor only after Task 2 final resolution/digest.

- [ ] **Step 5: Run GREEN and commit**

~~~powershell
python -m pytest -q tests/test_runtime_integration.py::test_proactive_budget_is_prepared_before_recovery_or_provider tests/test_proactive_settings.py::test_resolve_proactive_settings_matrix -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-05-green'
python -m py_compile main.py astr_embodiment/proactive.py astr_embodiment/proactive_settings.py astr_embodiment/autonomy.py astr_embodiment/bridge.py
git add main.py astr_embodiment/bridge.py astr_embodiment/proactive.py astr_embodiment/autonomy.py tests/test_runtime_integration.py
git commit -m "feat: wire proactive budget before autonomous work"
~~~

Expected: named tests pass, compile exits 0, and fake records no Provider/send on budget zero/upsert failure.

### Task 6: Document and version the next source increment

**Files:**
- Modify: README.md
- Modify: CHANGELOG.md
- Modify: Cargo.toml
- Modify: Cargo.lock
- Modify: pyproject.toml
- Modify: uv.lock
- Modify: metadata.yaml
- Modify: .github/workflows/ci.yml
- Modify: scripts/package_plugin.py
- Modify: tests/test_release_contracts.py
- Modify: tests/test_runtime_integration.py (native version assertion)

- [ ] **Step 1: Make the release-version contract RED**

Change test_release_versions_and_required_files_are_present expectations first to Rust/native/metadata 1.1.0-alpha4 and Python/wheel 1.1.0a4. Mechanically update alpha3 package filenames in release fixtures to alpha4, while leaving protocol/module identifiers such as alpha3_call, crates/ae-contracts/src/alpha3.rs, DB contracts, and wire names unchanged.

~~~powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
python -m pytest -q tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-06-red'
~~~

Expected RED: manifests still report alpha3.

- [ ] **Step 2: Bump every authoritative version source to alpha4**

Set workspace/native/metadata/package constants/CI to 1.1.0-alpha4; project/lock/wheel constants to 1.1.0a4; update runtime integration's native version assertion. Refresh Cargo.lock and uv.lock with the repository's established lock commands if manifest edits do not update them. Do not rename alpha3 APIs and do not build wheels/ZIPs.

- [ ] **Step 3: Update README and CHANGELOG without overstating acceptance**

README must document the four visible concepts, exact preset table, auto envelope and balanced band, strict custom ranges, unknown-origin to restrained behavior, hidden legacy audit behavior, and unchanged historical Token storage key. State alpha4 source wiring is compile/focused-contract ready only; Host render/save/reload, dual-platform wheels, offline install, and send are unverified until their gates run.

Add heading [1.1.0-alpha4] - 2026-08-30 below Unreleased. Record configuration simplification, conservative migration, Native-frozen auto evidence, real budget policy/ledger wiring, and limitations. Preserve historical alpha3 text verbatim; never claim alpha3 artifacts include these changes.

- [ ] **Step 4: Run GREEN and commit**

~~~powershell
python -m pytest -q tests/test_release_contracts.py::test_release_versions_and_required_files_are_present tests/test_static_contracts.py::test_config_schema_parses -o cache_dir='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\task-06-green'
git add README.md CHANGELOG.md Cargo.toml Cargo.lock pyproject.toml uv.lock metadata.yaml .github/workflows/ci.yml scripts/package_plugin.py tests/test_release_contracts.py tests/test_runtime_integration.py
git commit -m "chore: prepare proactive settings alpha4 source"
~~~

Expected: both nodes pass; no wheel, archive, native binary, cache, DB, or evidence artifact is staged.

### Task 7: Run bounded verification and hand off manual/package gates

**Files:**
- Verify only; do not create an empty commit.

- [ ] **Step 1: Collect before running Python tests**

~~~powershell
$env:PYTEST_DISABLE_PLUGIN_AUTOLOAD='1'
$pytestCache='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-pytest\final'
python -m pytest --collect-only -q tests/test_proactive_settings.py::test_resolve_proactive_settings_matrix tests/test_runtime_integration.py::test_proactive_settings_migration_save_and_rollback tests/test_runtime_integration.py::test_proactive_budget_is_prepared_before_recovery_or_provider tests/test_static_contracts.py::test_config_schema_parses tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir=$pytestCache
~~~

Expected: five named nodes collect with exit 0. Collection failure is not test PASS.

- [ ] **Step 2: Run the minimum high-value Python and compile checks**

~~~powershell
python -m pytest -q tests/test_proactive_settings.py::test_resolve_proactive_settings_matrix tests/test_runtime_integration.py::test_proactive_settings_migration_save_and_rollback tests/test_runtime_integration.py::test_proactive_budget_is_prepared_before_recovery_or_provider tests/test_static_contracts.py::test_config_schema_parses tests/test_release_contracts.py::test_release_versions_and_required_files_are_present -o cache_dir=$pytestCache
python -m py_compile main.py astr_embodiment/proactive.py astr_embodiment/proactive_settings.py astr_embodiment/autonomy.py astr_embodiment/bridge.py
~~~

Expected: five nodes pass and py_compile exits 0.

- [ ] **Step 3: Run two Rust behavior targets and one workspace compile**

~~~powershell
$env:CARGO_TARGET_DIR='G:\AstrEmbodiment\.codex-task-temp\proactive-settings-cargo\final'
cargo fmt --all -- --check
cargo test --locked --offline -p ae-agent --test proactive_frequency frequency_policy_is_closed_and_unanswered_never_widens -- --exact
cargo test --locked --offline -p ae-runtime --test alpha3_projection auto_frequency_and_budget_policy_share_gate_authority -- --exact
cargo check --locked --offline --workspace
~~~

Expected: formatting, both named tests, and workspace check exit 0. Missing offline cache is ENVIRONMENTAL_HARNESS_FAILURE, not behavior PASS and not permission for unplanned network fallback.

- [ ] **Step 4: Inspect scope and type/version consistency**

~~~powershell
git status --short
git diff HEAD~6..HEAD --check
git grep -n -E '1\.1\.0-alpha3|1\.1\.0a3' -- ':!docs/superpowers/specs/**' ':!docs/superpowers/plans/**' ':!CHANGELOG.md'
$unfinished=@(('TO'+'DO'),('TB'+'D'),('PLACE'+'HOLDER'),('fill'+' in'),('as '+'appropriate'))
foreach($marker in $unfinished){ git grep -n -F $marker -- astr_embodiment/proactive_settings.py main.py _conf_schema.json crates/ae-agent/src/contact.rs crates/ae-store/src/alpha3/projection.rs }
git grep -n -F 'RelationTemporalPolicyV1 {' -- '*.rs'
~~~

Expected: clean worktree; diff check clean; old versions only in retained historical prose; no unfinished markers; every temporal-policy literal supplies deliberate auto/reservation fields.

- [ ] **Step 5: Run the AstrBot manual boundary on an expendable config**

With supported AstrBot >=4.16,<5 and a backed-up disposable plugin config:

1. open plugin settings and verify only enable, frequency object, quiet-hours object, and proactive-expression budget are visible;
2. verify all four modes and always-visible custom inputs, then save/reload each;
3. load legacy 2/360 and verify restrained without attested origin; load 7/90 and verify exact custom; confirm old keys unchanged;
4. force save failure and verify revision remains 0 with no half-migration, then restore Host and retry;
5. set budget 0 and verify no Provider call; set 2048 and inspect Native policy/ledger for 2048 before claim;
6. with explicit relation grant/cause and safe target, verify asleep/quiet/unanswered/hard-stop/budget suppression, then observe at most one permitted send. Record adapter-submitted separately from platform-accepted/delivery-confirmed.

Expected: only a recorded run earns HOST MANUAL PASS. Without it report compile/focused-contract status.

- [ ] **Step 6: Keep packaging as a separate alpha4 release gate**

Do not run scripts/package_plugin.py for implementation acceptance. A later release task must build fresh Windows x64 and Linux x86_64 1.1.0a4 wheels from the accepted alpha4 SHA, verify hashes/ABI/API, run full archive contracts, create a new astrbot_plugin_astrembodiment-1.1.0-alpha4-universal.zip, and clean-load it offline on both platforms. Never reuse alpha3 wheel members or overwrite alpha3 ZIP.

Final classification:

- PASS_SOURCE_FOCUSED only if Steps 1-4 pass;
- add PASS_HOST_MANUAL only with Step 5 receipts;
- add PASS_PACKAGE only after fresh-wheel/ZIP/offline-host gate;
- otherwise report exact partial/environmental blocker.

## Spec coverage audit

- [ ] Four modes, exact presets, balanced auto band, fixed envelope, custom zero/range/overflow/bool/string rules have one typed source.
- [ ] Auto consumes frozen sleep/contact/count/budget/reservation in the existing immediate gate; missing/future/regressing evidence suppresses.
- [ ] Unanswered only tightens to restrained+backoff, hard stop suppresses, and no inbound means no time-only widening.
- [ ] Current Host cannot prove fresh, so production passes unknown; attested fresh is a future typed gate, not guessed behavior.
- [ ] Rev0/rev1/future priority, 2/360 restrained, legal legacy custom, invalid fail-closed, no old-key overwrite, and normalized post-save digest are covered.
- [ ] Schema is static/unconditional, hides exactly nine internal keys, and retains historical Token key with truthful text.
- [ ] Positive budget reaches real Native policy/ledger before work; zero and upsert/reconcile failure close before Provider.
- [ ] Save uses supported runtime methods; false/exception/missing method fails; rollback preserves concurrent replacement.
- [ ] Alpha4 source/docs are distinct from alpha3 artifacts; manual/package boundaries are explicit.
- [ ] No Native auto state machine/table/thread, invented conditional UI/origin API, direct Host config-file edit, or broad low-value suite.

## Plan self-review checklist

Before implementation handoff:

~~~powershell
$plan='docs\superpowers\plans\2026-08-30-proactive-settings-simplification.md'
$unfinished=@(('TO'+'DO'),('TB'+'D'),('PLACE'+'HOLDER'),('fill'+' in'),('as '+'appropriate'))
foreach($marker in $unfinished){ Select-String -LiteralPath $plan -SimpleMatch $marker }
Select-String -LiteralPath $plan -Pattern '^### Task [0-9]+:'
git diff --check -- $plan
~~~

Expected: no unfinished-marker matches, exactly seven task headings, and clean diff. Reread the frozen spec against the coverage audit; verify u16 daily max, checked u64 milliseconds/tokens, Python bool rejection, 256 reservation equality, and alpha4/a4 spelling.
