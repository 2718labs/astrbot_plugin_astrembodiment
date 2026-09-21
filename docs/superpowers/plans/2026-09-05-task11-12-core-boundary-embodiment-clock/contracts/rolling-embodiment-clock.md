# Rolling Embodiment Clock Contract

Status: frozen Task 11/12 shared contract

Authority: [top-level addendum](../../2026-09-05-task11-12-core-boundary-embodiment-clock.md) and [work index](../index.md).

This contract owns persona inventory, temporal profile, vendored timezone rules, deterministic time evolution and bounded one-head/64-receipt storage.

## 6. Persona inventory, profile and clock contracts

### 6.1 Inventory epoch/snapshot, not a high-water promise

V9 adds one singleton inventory head and three exact triggers over
`active_bindings`. Their sole SQL-byte authority is the one-time
[v9 schema manifest](v9-schema-manifest.md); 11B does not create or alter them.

Migration initializes `(epoch=1, entry_count=exact active_bindings count)`
before installing runtime access. The v9 schema verifier authenticates trigger
SQL exactly; no writer can change a binding without changing epoch.

The first `list_embodiment_personas_v1(limit<=64)` returns ordered
`PersonaScopeRef`, binding revision, persona-scope digest and an authenticated
snapshot token:

```text
token = H("ae.embodiment-inventory.snapshot.v1",
          schema_digest || U64LE(epoch) || U64LE(entry_count))
```

Every later page supplies the same token and checks epoch/count both before and
after its bounded query. Any change returns `INVENTORY_CHANGED` and no page.
Host discards the partial scan, starts a new epoch and deduplicates by
`PersonaScopeRef`. It never claims a high-water cursor prevents omissions.
A newly committed Genesis also notifies the running clock after the binding
transaction; the epoch protocol remains authoritative across restart/races.

### 6.2 Vendored timezone and schedule rules

Persona time never comes from AstrBot's or the user's timezone. Both wheels
ship the same immutable tzdb asset and manifest containing non-placeholder
`tzdb_release`, `format_version=1` and `content_sha256`. Host opens the vendored
bytes directly; it never falls back to the Windows registry, system zoneinfo or
locale. The source SHA, tzdb release and content SHA are embedded in both
wheels and the final package manifest.

Fixed-offset identifiers use only this canonical grammar:

```text
UTC
UTC[+-](0[0-9]|1[0-3]):[0-5][0-9]
UTC[+-]14:00
```

Zero is only `UTC`; `UTC+00:00` and `UTC-00:00` are rejected. Range is
inclusive `-14:00..+14:00`. A fixed zone has no next transition. IANA names
must resolve to a canonical entry in the vendored manifest; aliases are
canonicalized once before profile CAS. `UTC-07:00` therefore never acquires DST,
while `America/Los_Angeles` follows the vendored rules.

The active temporal profile is a new schedule-free contract; the legacy
`PersonaTemporalProfileV1` is preserved evidence and is never active input:

```rust
pub struct EmbodimentTemporalProfileV1 {
    pub schema_version: u16,             // 1
    pub persona_tzid: String,            // canonical fixed/IANA ID
    pub tzdb_release: String,
    pub tzdb_content_sha256: Digest,
    pub circadian_period_millis: u64,
    pub homeostatic_awake_gain_per_hour: Fixed,
    pub homeostatic_asleep_decay_per_hour: Fixed,
    pub drowsy_enter_threshold: Fixed,
    pub drowsy_exit_threshold: Fixed,
    pub endogenous_phase_hysteresis: Fixed,
    pub maximum_analytic_horizon_ms: u64, // exactly 604_800_000
}
```

It contains no chronotype, preferred sleep/wake minute, flex, entrainment or
mode. Those fields have one authority only: the separately versioned schedule.
Profile validation requires the embedded tzdb release/content SHA to equal the
vendored manifest; those exact identities are also columns in profile and
clock head and participate in both digests.

```rust
pub enum SleepScheduleModeV1 { Auto, Fixed }

pub struct EmbodimentSleepScheduleV1 {
    pub schema_version: u16,             // 1
    pub mode: SleepScheduleModeV1,
    pub chronotype: ChronotypeV1,
    pub preferred_sleep_local_minute: u16,
    pub preferred_wake_local_minute: u16,
    pub sleep_flex_minutes: u16,
    pub entrainment_rate_minutes_per_day: u16,
}
```

All minutes are `0..1439`; flex and entrainment are bounded by 720. `auto`
adapts only the committed circadian/sleep phase within those bounds. `fixed`
keeps the preferred local interval. Neither mode reads a user's quiet hours.

### 6.3 Conditional create, profile CAS and first anchor

Startup uses exactly two typed calls. First,
`get_embodiment_persona_v1(scope)` is a read-only authenticated lookup returning
`Present { first_creation_receipt, incarnation_digest, profile_revision,
schedule_revision, clock_head } | Missing { incarnation_digest }`. Only
`Missing` authorizes Host to capture creation time and call:

```rust
pub struct CreateEmbodimentPersonaIfMissingV1 {
    pub schema_version: u16,             // 1
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    pub incarnation_digest: Digest,
    pub profile_template: EmbodimentTemporalProfileV1,
    pub sleep_schedule: EmbodimentSleepScheduleV1,
    pub frozen_creation: FrozenPersonaTimeV1,
}
```

```text
operation_id = Trunc128(H("ae.embodiment.create.operation-id.v1",
                          persona_scope || incarnation_digest))
```

Create verifies v9 and active binding, validates the scope/formula/incarnation,
then queries the anchor by persona and creation operation before interpreting,
hashing or comparing `profile_template`, `sleep_schedule` or
`frozen_creation`. If the exact incarnation already exists, it returns the
first immutable creation receipt as zero-write `Existing`; a caller's fresh
creation payload is neither compared nor consumed. A different incarnation is
zero-write `PERSONA_INCARNATION_CONFLICT`. Only a missing anchor validates the
full payload, derives `request_digest`, and creates profile, schedule, head and
receipt once. This ordering also defines the insert-race loser: restart the
anchor lookup and return that first receipt; never surface a uniqueness error.

The first anchor binds binding revision/incarnation, profile, schedule, initial
clock state and the digest of any valid legacy local sleep state. The legacy
`autonomous_runtime_state`/profile row remains byte-identical. Missing legacy
state uses Genesis defaults; malformed legacy state fails closed rather than
being repaired. There is no `ensure_*` alias or combined read/write call.

`read_embodiment_profile_v1` returns profile plus profile/schedule revisions.
The only active timezone/sleep update request is:

```rust
pub struct CompareAndSwapEmbodimentProfileV1 {
    pub schema_version: u16,             // 1
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    pub expected_profile_revision: u64,
    pub expected_schedule_revision: u64,
    pub replacement_profile: EmbodimentTemporalProfileV1,
    pub replacement_schedule: EmbodimentSleepScheduleV1,
    pub frozen: FrozenPersonaTimeV1,
}
```

```text
operation_id = Trunc128(H("ae.embodiment.profile-cas.operation-id.v1",
  persona_scope || U64LE(expected_profile_revision) ||
  U64LE(expected_schedule_revision) || replacement_profile_digest ||
  replacement_schedule_digest))
request_digest = H("ae.embodiment.profile-cas.request.v1",
                   canonical request bytes)
event_id = Trunc128(H("ae.embodiment.profile-cas.event-id.v1",
  persona_scope || operation_id || request_digest))
```

`compare_and_swap_embodiment_profile_v1` validates that formula and accepts the
expected profile/schedule revisions plus full replacements. Exact retry request
bytes return the retained `Existing` result regardless of timing;
the caller must therefore retain and reuse the original frozen request. Reusing
the operation ID with different frozen or replacement bytes is
`IDEMPOTENCY_CONFLICT`; a different value with stale revision returns
`PROFILE_REVISION_CONFLICT`. A successful CAS uses its one frozen time to
project and update the clock head atomically, writes one rolling receipt with
bit 4 set, and advances both profile and schedule revisions in the same
transaction. It never creates a main-journal or semantic-history row.

The CAS transaction order is v9/binding verification, retained rolling-receipt
lookup and request-digest comparison, then (only for a new operation) rolling
closure and expected-revision validation, frozen-time projection and commit.
Thus an exact retry is resolved before the now-stale expected revisions are
examined. A retained same-operation/different-request collision and every
terminal validation error are zero-write.

### 6.4 Time request and deterministic IDs

```rust
pub struct FrozenPersonaTimeV1 {
    pub schema_version: u16,             // 1
    pub now_utc_ms: u64,                 // sole authoritative instant
    pub persona_tzid: String,
    pub persona_utc_offset_seconds: i32,
    pub persona_local_minute: u16,
    pub persona_day_ordinal: i32,
    pub next_timezone_transition_utc_ms: Option<u64>,
    pub tzdb_release: String,
    pub tzdb_content_sha256: Digest,
}

pub struct EmbodimentClockStatusRequestV1 {
    pub schema_version: u16,             // 1
    pub scope: PersonaScopeRef,
    pub profile_revision: u64,
    pub schedule_revision: u64,
    pub frozen: FrozenPersonaTimeV1,
}

pub struct EmbodimentTimeAdvanceRequestV1 {
    pub schema_version: u16,             // 1
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    pub profile_revision: u64,
    pub schedule_revision: u64,
    pub frozen: FrozenPersonaTimeV1,
}
```

Native never reads a wall clock in status or advance. Host captures and
canonicalizes the single `FrozenPersonaTimeV1` against the vendored tzdb, then
passes it to `embodiment_clock_status_v1`. `NotDue` returns only the committed
deadline/cadence/head. `Due` returns exact canonical
`EmbodimentTimeAdvanceRequestV1` bytes plus their digest. Host must pass those
bytes unchanged to `advance_embodiment_time_v1`; Native decodes and verifies
them and does not reconstruct a request from a second time sample.

The request has no relation/session, user timezone, Provider, Token, target,
outbox, stimulus, expected generation or Host causal base. Unknown fields fail
closed. Operation IDs are not arbitrary:

```text
operation_id = Trunc128(H("ae.embodiment-time.operation-id.v1",
  persona_scope || U64LE(now_utc_ms) || U64LE(profile_revision) ||
  U64LE(schedule_revision) || tzdb_content_sha256))
request_digest = H("ae.embodiment-time.request.v1", canonical request bytes)
event_id = Trunc128(H("ae.embodiment-time.event-id.v1",
  persona_scope || operation_id || request_digest))
```

This formula prevents reuse of a compacted old operation ID with a new instant.

### 6.5 Per-kind wire compatibility

Do not bump global `WIRE_SCHEMA_VERSION = 5`. Existing kinds 1 through 10 keep
their exact bytes and accepted envelope versions. Add
`KIND_EMBODIMENT_TIME_ADVANCE = 11`; its v5 envelope payload begins with
`EMBODIMENT_TIME_ADVANCE_WIRE_VERSION_V1 = 1`. Unknown per-kind versions fail
before allocation/mutation. Existing `InteractionFactBatchV1` v5 digests do
not change.

The bounded ledger has a separate closed discriminator, not another journal
kind: `EmbodimentClockMutationKindV1::Advance=1`,
`EmbodimentClockMutationKindV1::ProfileCas=2`, and the Store-private
`EmbodimentClockMutationKindV1::SemanticAnchor=3`. The discriminator is present in
result-core and receipt rows. Profile CAS is submitted only through its typed
API; no discriminator is accepted by generic `apply_event`.

Implementation authority correction: SemanticAnchor is emitted only inside a
new, successful perception settlement transaction after semantic rows and their
evidence binding exist. It has no Host or public API. It increments sequence
and state revision once, keeps time revision, last time, profile, schedule,
sleep state and deadline unchanged, and has zero elapsed/mask. The head update
guard requires the just-inserted receipt. The new matrix anchor revision must
strictly exceed its predecessor.

The immutable persona anchor records initial_semantic_revision. Every later
real perception commit requires exactly one immutable semantic_clock_anchor_proof_v1,
including zero-epoch inputs; a missing proof fails closed. Its bounded canonical
proof (<=524288 bytes) binds the old clock head, full epoch, anchor state/graph,
formula, projected input state and semantic commitment without copying the 16K
field or graph. Replay reconstructs from the persistent anchor/Genesis before
checking state_before. Proofs restrictively reference semantic_commits and are
not deleted by appraisal retention or rolling-window folding. The new internal
receipt binds proof digest and semantic commitment without a hash cycle.
Background Advance/ProfileCas still add zero main-journal/semantic-history rows.

Keep kind-8 `TimeAdvanceV1` bytes/digests verifiable across crates under the
[controlled historical-codec exception](retired-surface-release.md#811-controlled-task-12a-historical-codec-exception).
The existing public Rust DTO, decoder and pure encoding/hash helpers remain
available to continuum replay and Store historical verification. An opaque
`legacy_audit` module was not implemented. Pure encoding can represent kind 8;
no Host/Native/Runtime submission or current journal writer accepts it. The
typed embodiment-clock APIs remain the only active time-mutation route.

### 6.6 Exact embodiment tables and hash closure

The only table/index/trigger bytes, object order, storage checks and object
goldens are in [v9-schema-manifest.md](v9-schema-manifest.md). Card 11B consumes
those preinstalled objects without DDL. The profile row stores canonical
schedule-free `EmbodimentTemporalProfileV1` bytes, and profile/head both store
and authenticate the same vendored tzdb release/content SHA. The rolling row
stores `mutation_kind`, result-core and immutable commit-receipt fields
separately.
Bodies/digests use these domains:

```text
ae.embodiment.persona-anchor.v1
ae.embodiment.profile.v1
ae.embodiment.sleep-schedule.v1
ae.embodiment.clock-head.v1
ae.embodiment.clock-receipt.v1
ae.embodiment.clock-result-core.v1
ae.embodiment.clock-commit-receipt.v1
ae.embodiment.clock-receipt-leaf.v1
ae.embodiment.clock-chain-step.v1
```

This rolling clock is not the legacy relation-bearing kind-8 journal and is not
a new persona/semantic history. Kind 11 is the canonical advance request wire
stored in this bounded window/head commitment; generic `apply_event` cannot
submit it and no active encoder appends it to the existing journal.

### 6.7 Atomic transition and Runtime hot state

`advance_embodiment_time_v1` performs one `BEGIN IMMEDIATE`:

1. Verify v9 `applied`, persona anchor, active binding, profile/schedule and
   vendored tzdb closure.
2. Check the 64-row recent window by operation ID before projecting state. Same
   request returns the stored result as `Existing`; a different request digest
   is `IDEMPOTENCY_CONFLICT`. Both are zero-write.
3. Validate the operation-ID formula and the exact rolling closure. If the
   absent receipt's instant is at or before
   `compacted_through_now_utc_ms`, return `COMPACTED_OR_STALE`; otherwise
   `now_utc_ms` must exceed `last_now_utc_ms` or return
   `CLOCK_NOT_MONOTONIC`. Both are zero-write.
4. Analytically advance sleep/allostasis/circadian/matrix state once. Raw
   elapsed is recorded; applied elapsed is capped by
   `MATRIX_TIME_MAX_ELAPSED_MS = 604_800_000` (seven days). A longer restart gap
   is one bounded projection with `capped_gap=true`, never a catch-up loop.
5. Preserve the matrix anchor, full canonical `MatrixTimeEpochV1` (awake,
   drowsy and asleep ticks plus all three remainders) and formula digest in the
   clock head. The clock path may not rebase a rounded field, clear/reseed the
   epoch, or append a semantic transition. Only a separately journaled inbound
   semantic settlement may atomically install a new authenticated matrix anchor
   and epoch under the same persona lock.
6. Compute a meaningful-event mask: bit 0 is an actual sleep-state enum
   transition toward sleep, bit 1 an actual transition toward wake, bit 2 the
   one permitted dream consolidation for the current `sleep_episode_ordinal`,
   and bit 3 a hysteretic endogenous-phase enum boundary. Bit 4 is reserved for
   a successful timezone/tzdb/profile CAS and is never set by an ordinary time
   advance. Sleep and wake bits are mutually exclusive. Continuous numeric
   drift sets no bit; phase thresholds and their distinct enter/exit hysteresis
   values are bound by `formula_digest`. A capped restart summarizes missed
   intervals and emits at most the final boundary plus one consolidation for
   the current committed sleep episode.
7. Encode `result_core_bytes` with mutation discriminator and every committed
   outcome/state field except prior/leaf/chain/head and public-envelope fields;
   compute `result_core_digest` before any leaf. If `recent_count=64`,
   authenticate and fold exactly the oldest row at
   `sequence=compacted_count+1` into `compacted_chain_digest`, update
   `compacted_through_now_utc_ms`, and delete exactly that row by primary key.
   Then compute leaf/chain/head in order. Only after the new head digest exists,
   encode the immutable commit receipt; it may contain head/leaf commitments
   but does not participate in `result_core_digest`, leaf, chain or head. Insert
   the one rolling row, update the one head and require recent rows to stay
   contiguous.
8. Commit once, then return a Store-private outcome containing the public
   receipt, exact persona identity, authoritative committed head and either the
   fully hydrated Runtime state or an explicit `ReloadRequired` projection.

No step inserts into the existing persona journal, `inner_event`, semantic
history/authority, legacy kind-8 history or operational authority. A background
clock commit changes only the v9 clock head, at most 64 v9 clock receipts and
strictly persona-local cached/hydrated state represented by that head.

The immutable commit receipt contains operation ID, persona scope/digest,
sequence, mutation kind, profile/schedule/matrix/state/time revisions, frozen
instant, raw/applied elapsed, `capped_gap`, meaningful-event mask, sleep state,
`next_due_at_utc_ms`, cadence class (`active|asleep|calm`) and commitment
digests. A non-persisted call envelope adds `Committed | Existing`; exact replay
returns `Existing` around the same stored commit receipt. Neither contains a
relation/user/provider/target/outbox field.

The rolling chain is exact:

```text
chain_0 = H("ae.embodiment.clock-chain-root.v1",
            persona_scope || persona_anchor_digest)
leaf_n = H("ae.embodiment.clock-receipt-leaf.v1",
  U64LE(sequence_n) || U8(mutation_kind) || operation_id || event_id ||
  request_digest ||
  U64LE(frozen_now_utc_ms) || U64LE(raw_elapsed_ms) ||
  U64LE(applied_elapsed_ms) || U8(capped_gap) ||
  U32LE(discrete_event_mask) ||
  result_core_digest || resulting_state_digest)
chain_n = H("ae.embodiment.clock-chain-step.v1",
            chain_(n-1) || U64LE(sequence_n) || leaf_n)
head_digest = H("ae.embodiment.clock-head.v1",
  canonical_head_fields_except_chain_and_head_digests ||
  compacted_chain_digest || recent_chain_digest)
commit_receipt_digest = H("ae.embodiment.clock-commit-receipt.v1",
  result_core_digest || leaf_n || chain_n || head_digest ||
  LP64(commit_receipt_bytes))
```

`commit_receipt_bytes` excludes its own digest. `result_core_bytes` and
`commit_receipt_bytes` have independent canonical version-1 encoders; neither
contains the other. The per-call public envelope is encoded only after this
closure and is not stored or hashed into it. This order is the only permitted
construction and removes all circular hash choices.

For an empty window, both chain fields equal `chain_0`. Otherwise the first
recent row's `prior_chain_digest` equals `compacted_chain_digest`, every later
row links to its predecessor, and folding the final recent leaf yields
`recent_chain_digest`. Always
`head.sequence=head.compacted_count+head.recent_count`, recent sequences are
exactly `compacted_count+1..sequence`, and `recent_count<=64`.

While a receipt is retained, an equal request returns `Existing` and a different
digest for that operation ID returns `IDEMPOTENCY_CONFLICT`. Once folded, a
formula-valid request at or before `compacted_through_now_utc_ms` returns stable
`COMPACTED_OR_STALE` with zero writes and never reapplies. An invalid formula is
always `INVALID_OPERATION_ID`.

Runtime hot-state replacement and the cross-lane lock order are governed by
[core-ingress-delivery.md](core-ingress-delivery.md); the clock must not create a
second cache or lock discipline.

### 6.8 Zero-write recheck and bounded lifetime storage

`embodiment_clock_status_v1` is a pure authenticated projection over the exact
`EmbodimentClockStatusRequestV1`, including its Host-frozen time/tzdb bytes. It
returns `NotDue { next_due_at_utc_ms, cadence_class, clock_head_digest }` or
`Due { exact_advance_request_bytes, request_digest, next_due_at_utc_ms,
clock_head_digest }`. The Due bytes contain the same `frozen` bytes verbatim and
are the only accepted input for that Host wake's advance. No Native wall-clock
read or second Host sample is allowed. Either status result changes no DB row,
receipt, timestamp, sequence or revision. Polling is not a bodily event. The
Native due time is the earliest meaningful boundary or maximum analytic
horizon, not Host's polling frequency.

The 64-to-64 fold is part of the same successful advance transaction: starting
from 64 rows, authenticate one oldest leaf, advance `compacted_chain_digest` and
count, delete exactly one row, then insert exactly one row. Missing/gapped rows,
wrong prior/leaf/head digests, a noncontiguous sequence, or an affected-row count
other than one rolls back the whole transaction with
`CLOCK_ROLLING_CLOSURE_INVALID`. It does not discard or partially advance state.

Thus a persona owns exactly one head plus `min(sequence,64)` recent receipt
rows regardless of lifetime. Background clock activity adds zero physical rows
to the existing persona journal and semantic history, so it can never consume
their 65,536-row budgets. This claim is deliberately limited to background
clock activity. General archival/compaction of genuine inbound and ordinary
delivery history is outside Tasks 11/12 and is not smuggled in as a second
archive layer. Open, inventory, `NotDue`, status/recheck and `Existing` paths
are zero-write.

## 7. Host EmbodimentClock and simplified settings

Replace `AutonomousSupervisor` with one `EmbodimentClock` asyncio task per
plugin instance. It has no externalization callback, relation enumeration,
pending/recovery scan, Provider callback, target store or send adapter.

At startup it scans one authenticated inventory epoch. `INVENTORY_CHANGED`
discards partial pages and restarts with deduplication. It calls the read-only
persona lookup first and creates only a returned Missing persona, then keeps one
min-heap of Native deadlines; it does not create one task per persona. A new
Genesis/inbound binding notification performs the same lookup/create sequence
and refreshes that persona in the heap.

The scheduler participates in the single shared persona mutation order defined
by [core-ingress-delivery.md](core-ingress-delivery.md).

Native `next_due_at_utc_ms` is authoritative. Host uses a 60-second floor and
maximum recheck intervals of five minutes for `active`, one hour for `asleep`
and six hours for `calm`. Local error backoff is bounded at one hour and never
calls Provider. Stop/cancel is idempotent and awaited. Disabling the clock
stops only the Host task; it does not mutate persona state. Re-enable/restart
causes one bounded advance per persona, not repeated catch-up.

Those intervals are polling ceilings, not mutation cadences. Every scheduler
wake captures one frozen persona time and calls the pure Native status/recheck
path with it; `NotDue` only reschedules. On `Due`, it forwards Native's exact
advance request bytes unchanged. It calls `advance_embodiment_time_v1` only when
Native reports a meaningful boundary or maximum analytic horizon due.

Visible autonomy settings are only:

1. `embodiment_clock_enabled` (default `true`), a deterministic local clock
   that never spends Tokens;
2. `persona_timezone`, a canonical fixed/IANA zone applied through profile CAS;
3. `persona_sleep`, one object with `mode=auto|fixed` and fixed-mode sleep/wake
   times; advanced bounds live in the persisted profile;
4. `semantic_appraisal_token_daily_max`, the existing inbound semantic budget.

Identity/Genesis, native data directory, runtime envelope and semantic Provider
selection remain where required; they are not proactive settings. Existing
`proactive_*`, user quiet hours/timezone, contact TTL/cooldown, unanswered
backoff, emergency bypass, `inner_activity_token_daily_max` and
`autonomous_runtime_enabled` bytes remain in the user's saved config but are
absent from `_conf_schema.json`, excluded from active digests and never written
back. They are `legacy_preserved_ignored`; the old runtime flag cannot control
the new clock.

Default economy means adaptive local scheduling and zero clock
Provider/Token/network/send calls, not silently stopping biological time.
