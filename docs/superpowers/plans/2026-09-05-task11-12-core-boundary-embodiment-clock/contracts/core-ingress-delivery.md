# Core Ingress and Ordinary Delivery Contract

Status: frozen Task 11/12 shared contract

Authority: [top-level addendum](../../2026-09-05-task11-12-core-boundary-embodiment-clock.md) and [work index](../index.md).

This contract owns typed inbound observation, semantic appraisal authorization, ordinary DeliveryOutcome, replay, `_pending`, shared locks and Runtime hot-state coherence.

## 4. Core-only inbound semantic lane

### 4.1 Observation-only contracts

Do not reuse caller-supplied `InteractionFactBatchV1.causal.base_revision` as
active input. Keep its v5 bytes for historical decode. The new request is:

```rust
pub struct CoreInboundObservationV1 {
    pub schema_version: u16,             // 1
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,          // bot + persona only
    pub turn_id: Id128,
    pub observed_at_utc_ms: u64,
    pub message_digest: Digest,
    pub astrbot_event_identity_digest: Digest,
    pub astrbot_source_digest: Digest,
    pub extractor_digest: Digest,
    pub confidence: Fixed,
    pub relation_evidence_ref: Option<Digest>,
    pub session_evidence_ref: Option<Digest>,
}

pub struct CoreInboundAppraisalReservationV1 {
    pub daily_token_limit: u32,
    pub reserved_tokens: u32,
    pub provider_digest: Digest,
}

pub struct CommitCoreInboundV1 {
    pub schema_version: u16,             // 1
    pub observation: CoreInboundObservationV1,
    pub appraisal: Option<CoreInboundAppraisalReservationV1>,
}
```

The two optional evidence refs are non-enumerating commitments stored only in
the core receipt. No active code passes them into `ScopeRef` or uses them to
query relation/session state. Raw message text is not persisted. Unknown fields
fail closed.

Host derives `astrbot_event_identity_digest` only from AstrBot's immutable
adapter/account/conversation/message identifier tuple, using an explicitly
length-prefixed canonical encoding. It excludes raw message text, wall time,
arrival sequence and every Native revision. Host and Native both enforce:

```text
turn_id = Trunc128(H("ae.core-inbound.turn-id.v1",
                     persona_scope || astrbot_event_identity_digest))
operation_id = Trunc128(H("ae.core-inbound.operation-id.v1",
                          persona_scope || turn_id))
```

This is the only Host operation-ID construction. A mismatch returns
`INVALID_TURN_ID` or `INVALID_OPERATION_ID` before transaction begin.

Native derives, rather than accepts, identifiers:

```text
request_digest = H("ae.core-inbound.request.v1", canonical request bytes)
fact_id = Trunc128(H("ae.core-inbound.fact-id.v1", request_digest))
event_id = Trunc128(H("ae.core-inbound.event-id.v1",
                      persona_scope || operation_id || request_digest))
observation_digest = H("ae.core-inbound.observation.v1",
                       canonical observation bytes || fact_id || event_id)
```

To retain the existing compiled `InteractionFactBatchV1` codec without
restoring relation authority, Store constructs this exact internal projection
after it reads the current revision:

```rust
ScopeRef {
    bot_token: observation.scope.bot_token,
    persona_token: observation.scope.persona_token,
    relation_token: None,
    session_token: observation.turn_id,
}
CausalRef {
    turn_id: observation.turn_id,
    action_id: None,
    delivery_id: None,
    claim_id: None,
    base_revision: store_current_revision,
}
```

Native canonicalizes exactly one historical interaction fact with kind
`InboundObserved`, source authority `AstrbotMetadata`, and `None` for value
code, subject, consent, schedule and expiry. Its `source_digest` is the Native
`observation_digest`, which binds the message/source and both evidence refs.
The core-only Task 10B origin constructor/validator and attestation accept
precisely this relation-absent projection: `relation_present=false` and
`relation_scope=persona_scope`. They do not relax any other origin validation,
change historical kind-10 encoded bytes, or call the relation contact reducer.
The old relation-required constructor is not reachable from the core API and is
removed in Task 12.

### 4.2 Transaction receipt and Provider authority

`core_inbound_receipt_v1`, its persona/turn and event uniqueness indexes, and
all immutable/applied guards are created only by the exact one-time
[v9 schema manifest](v9-schema-manifest.md). The request path has no DDL and
11B may not alter those bytes.

The observation bytes retain message/source/evidence digests, never raw text.
The receipt stores the complete initial semantic-begin outcome including
`BudgetExhausted` and `CapacityDeferred`; each variable blob is capped at 64
KiB and included in the receipt digest.

The closed initial appraisal disposition is exactly:

```text
not_requested | claimed | budget_exhausted |
capacity_deferred | retry_expired_or_unknown
```

`BudgetExhausted` and `CapacityDeferred` are durable receipt states even though
they create no actionable Provider authority. Stored `initial_receipt_bytes`
are immutable: a first `claimed` commit records
`provider_authority_granted_initial=1` and never rewrites it. The per-call
`CoreInboundCommitOutcomeV1` is a separate, non-persisted projection. It returns
`commit_status=Committed, provider_authorized_now=true` only from the successful
first insert; every replay returns
`commit_status=Existing, provider_authorized_now=false` plus the authenticated
stored initial receipt. Existing therefore never calls Provider and never
pretends its projected bytes equal the immutable initial receipt. A crash after
commit sacrifices an appraisal rather than duplicating Provider usage.

`commit_core_inbound_v1` enters `BEGIN IMMEDIATE` and first verifies v9
`applied` plus the active persona binding. It then looks up
`(persona_scope,operation_id)` and compares the request digest before reading a
journal head, reconstructing event bytes, or invoking Task 10B maintenance.
Same operation plus same request returns the stored event/initial receipt as
zero-write `Existing`; same operation or derived event/turn ID plus a different
digest is zero-write `IDEMPOTENCY_CONFLICT`. Only a new operation reads the
current persona journal revision/chain head and authenticated clock-head digest
inside the transaction, builds the one canonical fact, commits its journal/
inner receipt with that clock-head binding, and optionally invokes the Task 10B
begin-claim helper before one commit. An intervening clock commit therefore
cannot perturb exact inbound replay.

Both `appraisal=None` and every closed appraisal result must leave every table
in the retired manifest byte-identical. Semantic budget exhaustion, capacity,
Provider error or retry-expired results never queue a background retry and
never block AstrBot's ordinary response.

The Host holds the persona lock for Native begin, releases it for the sole
Provider request, then reacquires it for Task 10B settlement. Settlement uses
the current matrix state inside Store and the existing rebase contract; it does
not resurrect a Host base revision.

Immediately after Native returns authenticated `Committed` or `Existing`, Host
installs the immutable `_pending` correlation for that durable inbound
operation before releasing the persona lock or attempting Provider work.
Provider refusal/error, semantic settlement failure and semantic expiry do not
discard or rewrite this correlation. Ordinary delivery must reference that
persisted `(persona_scope, inbound_operation_id, turn_id)` receipt; it never
falls back to a Host revision, reconstructs Genesis metadata, or synthesizes a
replacement inbound operation.

## 5. Typed ordinary DeliveryOutcome

`_pending` remains, but it no longer stores a causal base revision. It stores
only persona scope, turn ID, inbound operation ID, response contract/digest and
a deterministic delivery operation ID. Host and Native both enforce:

```text
delivery_operation_id = Trunc128(H("ae.core-delivery.operation-id.v1",
  persona_scope || turn_id || inbound_operation_id))
```

No revision, sequence, time, raw text or response payload participates in this
ID. Reusing the turn with a different delivery request is therefore a visible
idempotency conflict rather than a second outcome. A formula mismatch returns
`INVALID_OPERATION_ID` before transaction begin.

```rust
pub struct CommitCoreDeliveryOutcomeV1 {
    pub schema_version: u16,             // 1
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    pub turn_id: Id128,
    pub inbound_operation_id: Id128,
    pub delivered: bool,
    pub observed_at_utc_ms: u64,
    pub visible_action_digest: Digest,
}
```

The Store uses `BEGIN IMMEDIATE`, verifies v9/binding, and immediately checks
the delivery receipt/request digest. Exact replay returns the stored
event/receipt as zero-write `Existing`, before resolving the inbound reference,
reading a journal head or rebuilding event bytes. Only a new delivery verifies
the referenced `core_inbound_receipt_v1` has the same persona/turn, then reads
the current persona journal head and derives:

```text
request_digest = H("ae.core-delivery.request.v1", canonical request bytes)
event_id = Trunc128(H("ae.core-delivery.event-id.v1",
                      persona_scope || operation_id || request_digest))
```

`core_delivery_receipt_v1`, both uniqueness indexes and immutable/applied
guards are likewise installed only by the exact
[v9 schema manifest](v9-schema-manifest.md), never by a delivery request.

The transaction binds the current authenticated clock-head digest and commits
the typed ordinary `DeliveryOutcome`, journal receipt and idempotency receipt
once. Same operation/request returns `Existing`; any different reuse is
`IDEMPOTENCY_CONFLICT`. It shares the persona lock with the clock and semantic
lane.

This typed path never consults `GenesisCoordinator._applied` (the current
scope/event-ID memory cache) and that cache is deleted for delivery during the
Host cutover. A cache hit is not authority: only the Store receipt's exact
request digest may produce `Existing`. This closes the case where the same
event ID carries a different `delivered` bit or action digest.

A clock commit after `_pending` creation and before AstrBot delivery is an
explicit acceptance case: delivery succeeds against Store's current revision,
not the removed base. Retired-table raw digests before/after are identical.
Delivery never changes contact policy, unanswered counters, intentions,
targets, budgets or retry state.

After `Committed`, authenticated `Existing`, or any terminal delivery error,
Host clears the matching `_pending` entry. An error retains only a bounded
diagnostic record; `_pending` is never an implicit retry queue or outbox.

## Shared mutation ordering and Runtime hot state

The lock order is frozen and shared by inbound, settlement, delivery and clock:

```text
pop/read scheduler heap, then release heap lock
  -> Host asyncio.Lock(exact bot,persona)
  -> PyO3 global CORE mutex
  -> SQLite BEGIN IMMEDIATE
  -> v9/binding -> idempotency -> authoritative head -> derive/write -> COMMIT
  -> identity-checked hot replace or exact-scope eviction
  -> release CORE
  -> release persona lock
  -> reinsert deadline under the heap lock
```

The scheduler never holds the heap lock while waiting for a persona. No
Provider await or heap operation occurs inside the persona/CORE/SQLite chain.
Inbound installs `_pending` while the persona lock remains held after Native
returns, then releases it for Provider I/O. Semantic settlement and AstrBot's
after-message delivery callback reacquire that same exact persona lock.

The private projection is
`Hydrated { scope, authoritative_head, state } | ReloadRequired { scope,
authoritative_head }`. On either `Committed` or `Existing`, Runtime validates
the exact `(bot_token,persona_token)` identity and then atomically replaces only
that hot entry or evicts only that entry. It never copies a maximum revision or
new state into whichever `Option<HotBrain>` happens to be resident. A later
`hot_for(exact_scope)` performs the authenticated reload.

Commit/Existing success is fixed at the database commit boundary. If eager
hydration fails or returns `ReloadRequired` after the receipt is known durable,
Runtime evicts the exact entry and still returns the authenticated public
success; it never turns a committed operation into a post-commit failure. The
same private-outcome rule applies to core inbound and delivery receipts: their
exact identity and authoritative head come from Store, not a stale caller view.
