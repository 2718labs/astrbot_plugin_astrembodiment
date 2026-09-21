# Task 10B Semantic Appraisal Retention and Capacity Implementation Addendum

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound semantic-appraisal retention without weakening accounting, and commit healthy capacity degradation with the inbound turn while making zero Provider calls.

**Architecture:** SQLite remains the sole writer and `semantic_appraisal_claim` remains the sole pending/terminal claim table. Terminals retain exact replay for five minutes from settlement, then each claim folds into its budget and each eligible old budget folds into the singleton before bounded point deletion.

**Tech Stack:** Rust, rusqlite/SQLite, serde, PyO3, Python/AstrBot.

---

## Authority and file scope

This addendum is authoritative for Task 10B. It supersedes any conflicting 24-hour retention, `AlreadyObserved`, caller-minted origin/nonce, or raw/untyped appraisal-apply assumption without rewriting historical plans. No limit increase, background sweep/retry, `VACUUM`, Task 11/12 work, or release claim is authorized.

Modify only existing implementation surfaces: `crates/ae-contracts/src/emotion_matrix.rs`, `crates/ae-store/src/{lib.rs,semantic.rs,alpha3/interaction.rs}`, `crates/ae-runtime/src/{lib.rs,alpha3/interaction.rs}`, `crates/ae-pyo3/src/lib.rs`, `astr_embodiment/{bridge.py,__init__.py}`, and `main.py`. Extend existing tests in `crates/ae-store/src/semantic.rs`, `crates/ae-runtime/tests/perception_origin_authority.rs`, and `tests/test_runtime_integration.py`; create no new test file.

## Frozen persistence contract

- Store appraisal schema version `X'03'`.
- Keep the current V2 `semantic_appraisal_claim` columns/checks exactly; do not add terminal or tombstone tables.
- Add nonnegative integer `compacted_claim_rows`, nonnegative integer `compacted_charged_tokens`, and 32-byte `compacted_chain_digest` to `semantic_appraisal_budget`.
- Require `compacted_charged_tokens <= charged_tokens`. Zero compacted rows requires zero tokens and `zeroblob(32)` root; positive rows require a nonzero root.
- Add singleton `semantic_appraisal_rollup(singleton=1, compacted_budget_rows, compacted_claim_rows, compacted_charged_tokens, compacted_chain_digest, last_authoritative_now_ms)`, with nonnegative counters/time and the same zero/nonzero root invariant.
- Freeze the active V3 writer predicate `valid_semantic_appraisal_terminal_outcome_code_v1` to `('success','provider_error','timeout','malformed','expired')`. The V3 table arm and stored-row migration/audit/fold predicate additionally accept `budget_exhausted`, because baseline V2 legitimately persisted fully receipted terminals with that code. It is migration-only compatibility: V1/V2 copy preserves it and its leaf commits it, but an active V3 writer never creates it. `abandoned`, `binding_lost`, and every other code remain invalid legacy state.
- Use exactly:

```sql
CREATE INDEX semantic_appraisal_claim_retention_v3
  ON semantic_appraisal_claim(settled_at_ms,created_at_ms,request_nonce_digest);
CREATE INDEX semantic_appraisal_claim_budget_v3
  ON semantic_appraisal_claim(persona_scope,utc_day,settled_at_ms,request_nonce_digest);
CREATE INDEX semantic_appraisal_budget_retention_v3
  ON semantic_appraisal_budget(utc_day,persona_scope);
```

The reference V3 catalog contains exactly nine objects: tables `semantic_appraisal_budget`, `semantic_appraisal_claim`, and `semantic_appraisal_rollup`; autoindexes `sqlite_autoindex_semantic_appraisal_budget_1`, `sqlite_autoindex_semantic_appraisal_claim_1`, and `sqlite_autoindex_semantic_appraisal_claim_2`; and the three explicit indexes above. Define `semantic_appraisal_rollup.singleton` as `INTEGER PRIMARY KEY CHECK(singleton=1)`: it aliases `rowid`, so the sole row has `rowid=singleton=1` and creates no fourth autoindex. Partial, extra, or SQL-mismatched appraisal objects fail closed.

Use one encoder only. `U64(x)` is eight little-endian bytes; required bytes/text are `U64(byte_length) || bytes` (text is exact UTF-8); and `Option<T>` is tag `00` for `None` or `01 || encode(T)` for `Some`. Concatenate fields in the declared order into one `receipt`, then call `wire::domain_hash(domain, &[receipt.as_slice()])`. The outer function therefore adds its normal `U64(receipt_length)` single-field frame; do not pass encoded fields as multiple hash fields.

For a terminal claim, the claim-leaf receipt follows claim-table column order exactly:

```text
bytes(request_nonce_digest) || bytes(persona_scope) || U64(utc_day) ||
bytes(origin_event_digest) || bytes(origin_digest) || bytes(provider_digest) ||
U64(reserved_tokens) || U64(created_at_ms) || Some(U64(settled_at_ms)) ||
Some(U64(charged_tokens)) || Some(bytes(outcome_code)) ||
Some(U64(canonical_revision)) || Option<U64>(semantic_revision) ||
Option<U64>(usage_known) || Option<U64>(usage_tokens) ||
Option<bytes>(proposal_identity_digest) || Option<bytes>(settlement_identity_digest) ||
Option<bytes>(reply_affect_bytes) || Option<bytes>(reply_affect_digest) ||
Option<bytes>(terminal_receipt_bytes) || Option<bytes>(terminal_receipt_digest)
```

Hash it with `astr-embodiment/semantic-appraisal-claim-compaction-leaf-v1`. Advance the budget root with a literal 72-byte transition `prior_root || U64(old_compacted_claim_rows) || claim_leaf`, passed as the sole field to domain `astr-embodiment/semantic-appraisal-claim-compaction-chain-v1`. In the same CAS set `compacted_claim_rows=old+1` and `compacted_charged_tokens=old+claim.charged_tokens`.

For an eligible budget, the budget-leaf receipt follows budget-table column order exactly:

```text
bytes(persona_scope) || U64(utc_day) || U64(daily_token_limit) ||
U64(charged_tokens) || U64(reserved_tokens) || U64(blocked) || U64(updated_at_ms) ||
U64(compacted_claim_rows) || U64(compacted_charged_tokens) || bytes(compacted_chain_digest)
```

Hash it with `astr-embodiment/semantic-appraisal-budget-compaction-leaf-v1`. Advance the singleton root with the 72-byte transition `prior_root || U64(old_compacted_budget_rows) || budget_leaf`, passed as the sole field to domain `astr-embodiment/semantic-appraisal-budget-compaction-chain-v1`; atomically add one budget plus the budget's compacted claim/token counters to the rollup.

`semantic::tests::appraisal_compaction_hash_vectors_are_stable` freezes the one approved canonical fixture and these full expected hashes; changing any field order, option tag, length frame, domain, or single-field invocation must fail:

```text
claim leaf:   92ce31f17f148c810b55e6fbfcfe89821c5414cbc16d8e503a354812a981a201
claim chain:  a5682f0d19770aa08cd52d12ae2578ba09736062beed27a53c4e7e67ff94552d
               (prior_root=0x11 repeated 32 bytes, old_compacted_claim_rows=7)
budget leaf:  2de82fd3f526411c04807e4c8101a17fe281f1078267c16daddce3583f41cebd
budget chain: 606b0d7c4077e8ac0cdab815f5d12bda86cc29503006c09f499b9f317a906cde
               (prior_root=0x22 repeated 32 bytes, old_compacted_budget_rows=3)
```

Compaction is strictly hierarchical. Fold a terminal into its owning budget by CAS-updating counters/root and `updated_at_ms`, then point-delete it by `rowid + request_nonce_digest + settled_at_ms` in the same Immediate transaction. Only after the claim phase may a now-eligible old budget fold into the singleton; CAS-update singleton totals/root, then point-delete the budget by `rowid + persona_scope + utc_day + frozen counters/root`. A claim fold and subsequent owning-budget fold are two logical mutations. Counter, overflow, hash, CAS, or delete-count failure prevents deletion and rolls back.

For every live budget:

```text
reserved_tokens = SUM(retained pending reserved_tokens)
charged_tokens  = compacted_charged_tokens + SUM(retained terminal charged_tokens)
```

## Time, retention, and capacity

```text
PENDING_TTL_MS                 = 300000
TERMINAL_EXACT_RETRY_MS        = 300000
SEMANTIC_APPRAISAL_UTC_DAY_MS  = 86400000
RETENTION_MUTATIONS_PER_WRITE  = 64
```

Each begin/settle Immediate transaction reads wall time exactly once, computes `authoritative_now_ms = max(wall_now_ms, rollup.last_authoritative_now_ms)`, persists it, and passes that same value into target maintenance, generic maintenance, admission, and settlement. A transaction failure rolls the time update back. Migration does not read wall time: it seeds the singleton from the checked bounded maximum of budget `updated_at_ms`, claim `created_at_ms`, and non-null `settled_at_ms`, or zero when all three sets are empty; challenge expiry is never a time-authority input.

- Pending is fresh only while `now < created_at_ms + 300000`. At equality, atomically settle `expired`, charge the full reservation, and point-delete its challenge. Never directly prune pending.
- Terminal exact retry is available only while `now < settled_at_ms + 300000`. At equality return `retry_expired_or_unknown`, even if bounded maintenance has not yet reached the row.
- A budget is old only when `utc_day < floor(now / 86400000)` and folds only with zero reservation and no retained claim. `updated_at_ms` age is irrelevant; there is no 24-hour rule.
- Maintenance runs only inside normal begin/settle writes and spends at most 64 logical mutations. First inspect the requested target: begin uses `origin_event_digest` through `sqlite_autoindex_semantic_appraisal_claim_2`; settle uses `request_nonce_digest` through `sqlite_autoindex_semantic_appraisal_claim_1`. Expiring or folding that target consumes one mutation, leaving 63; a fresh/no-op target leaves all 64. The returned result is derived from the post-target state, so a generic backlog cannot hide expiry of the requested origin/nonce.
- Spend the remainder in claim-before-budget order. Expire `settled_at_ms IS NULL AND created_at_ms <= now-300000` rows through `semantic_appraisal_claim_retention_v3` with `ORDER BY settled_at_ms,created_at_ms,request_nonce_digest LIMIT remaining`; then fold `settled_at_ms IS NOT NULL AND settled_at_ms <= now-300000` rows through that same index/order/limit. Only then scan `utc_day < floor(now/86400000)` budgets through `semantic_appraisal_budget_retention_v3` with `ORDER BY utc_day,persona_scope LIMIT remaining`, point-probe `semantic_appraisal_claim_budget_v3`, and fold only a budget with zero reservation and no retained claim. Compute cutoffs with checked arithmetic; when `now < 300000`, the corresponding claim phase is empty. Every pending settlement, terminal-to-budget fold/delete, or budget-to-singleton fold/delete counts as one logical mutation, even though its atomic SQL sequence touches multiple rows. No `OFFSET`, range delete, background worker, or unbounded payload fetch is permitted.

Preserve the 8 MiB replay-payload cap:

```text
worst pending = 32 + 2 * 16384 = 32800 bytes
255 pending   = 8364000 bytes (valid; 24608 remain)
256 pending   = 8396800 bytes (8192 over 8388608)
```

The 4,096 row limit is a structural corruption bound, not pending capacity. Pending counts 32,800 bytes; terminal counts exact outcome/reply/receipt lengths. Fixed compact counters/roots are separately bounded by at most 4,096 live budgets plus one singleton and do not consume the variable replay allowance.

The boundary fixture must distribute valid pending claims as `64 + 64 + 64 + 63` across four personas. This produces 255 rows without tripping the 64-per-persona challenge quota. The next claim is the fourth persona's 64th: it is otherwise valid but would be row 256 and exceed only the 8 MiB replay allowance, so it returns `CapacityDeferred(retention_capacity_unavailable)`, inserts no budget/claim/challenge/reservation, and still commits that inbound interaction. Raising the byte, row, or per-persona limits is not an implementation.

Admission after valid-state preflight and maintenance is ordered: duplicate origin -> prospective budget row/persona -> actual budget exhaustion -> prospective claim/payload/challenge bounds -> entropy/insert/reserve. An already corrupt/over-limit state, SQLite/I/O/entropy failure, or stored closure error rolls back; only prospective inability to retain a new claim is healthy capacity.

`BudgetExhausted` and `CapacityDeferred` both create no terminal claim/challenge/reservation. Budget exhaustion requires a valid authoritative budget. Capacity uses reason `retention_capacity_unavailable` and does not insert a new budget solely for the failed claim.

## Duplicate, migration, open, and audit

`applied_events(scope_digest,event_digest)` remains the exact durable origin watermark and is not compacted here. Therefore no `AlreadyObserved` wire value is needed:

| Case | Result |
|---|---|
| Original budget/capacity no-claim | Return the exact begin outcome in that committing call; no claim ID exists. A repeated begin gets `retry_expired_or_unknown` because no per-origin outcome was persisted. |
| Duplicate fresh pending | Exact matching provider/reservation identity returns the same `Claimed` challenge and budget; a mismatch is identity conflict. The duplicate creates no second claim, challenge, or reservation. |
| Modern retained active-code terminal inside five minutes | An identity-equal settle replays the exact persisted payload; a mismatched settle is invalid. A repeated begin gets `retry_expired_or_unknown`. |
| Migrated `budget_exhausted` terminal, fully receipted or legacy-NULL | Validate and preserve it as stored history, but never exact-replay it: settle and repeated begin return `retry_expired_or_unknown`. At its horizon it folds normally, with the compatibility code committed in the claim leaf. |
| Migrated legacy terminal whose entire V2 replay extension is NULL | Exact payload reconstruction is impossible, so settle and repeated begin return `retry_expired_or_unknown` even before the horizon; this is not corruption. The row remains eligible for normal terminal folding at its horizon. |
| Terminal at/after horizon, already folded, or owning budget already rolled up | Settle and repeated begin get `retry_expired_or_unknown`; `applied_events` still proves origin membership. |

Store must commit maintenance/time before returning a private `RetryExpiredOrUnknown` outcome. Runtime then maps it to `RuntimeError::SemanticAppraisalRetryExpiredOrUnknown`, and PyO3 emits exactly `SEMANTIC_APPRAISAL_RETRY_EXPIRED_OR_UNKNOWN`. A Store error raised before commit would incorrectly roll back expiry settlement.

Migration accepts only: empty -> V3; exact V1 budget+claim -> preflight then rebuild/copy with a NULL replay extension and zero compact fields; exact V2 budget+claim -> preflight then rebuild/copy, byte-preserving every replay field including a valid `budget_exhausted` terminal, with zero compact fields; exact V3 triple -> verify only. Every partial table/meta/catalog combination rolls back. For V1/V2, complete all checks below against the live legacy tables before the first appraisal `DROP`, `ALTER`, `CREATE`, or `INSERT ... SELECT`; never depend on constraints added by the destination table.

Both that pre-DDL permit and the final V3 verifier must close, with bounded keyset scans:

- Exact rowsets, positive rowids, SQLite storage classes/lengths, global and per-persona counts, and the 8 MiB aggregate allowance; every claim has exactly one budget key.
- `claim.utc_day = floor(claim.created_at_ms / 86400000)`, `settled_at_ms >= created_at_ms` when present, and `budget.utc_day <= floor(budget.updated_at_ms / 86400000)`, all with checked integer conversions/arithmetic.
- For each budget, the pending reservation and retained-terminal charge equations above (plus compacted counters in V3), the daily limit, and the sticky blocked invariant. Counter overflow or an unexplained clearing of a required blocked state is corruption.
- Every pending claim has the nonce-addressed challenge with identical persona, origin-event digest, origin digest, and creation time; `expires_at_ms = created_at_ms + 300000`; its commitment/nonce and canonical origin bytes rederive. A terminal has no challenge with its nonce.
- Every claim closes through exact `applied_events(persona_scope,origin_event_digest)`, its referenced canonical `interaction_fact_batch` journal row, the single materialized `InboundObserved/AstrbotMetadata` fact, and a rederived origin commitment/digest.
- Every stored terminal uses the six-code compatibility set: the five active codes plus migration-only `budget_exhausted`. A modern terminal, including a fully receipted V2 `budget_exhausted`, rederives usage/charge, proposal and settlement identities, canonical `ReplyAffect` bytes/digest, and canonical terminal receipt bytes/digest/content. The fully NULL legacy replay extension is the only validation exception; a partial extension is corruption. Audit and fold accept the compatibility code, while active V3 settlement rejects it as an emitted outcome.
- V3 has exactly one whole-table rollup row with `rowid=singleton=1`; compact counters/root invariants hold; and `rollup.last_authoritative_now_ms` is at least every budget `updated_at_ms`, claim `created_at_ms`, and non-null claim `settled_at_ms`.

After copy, run the full V3 verifier and require zero `foreign_key_check` rows, then update appraisal meta to `X'03'` as the last migration write. Any failure rolls the transaction back. Exact V3 open is verify-only: it does not advance time or opportunistically compact.

The no-wall-time promise here is scoped to the Task 10B appraisal migration/open path, not unrelated Store subsystems: that path performs no wall-clock read, compaction, TTL settlement, or time-based challenge cleanup. Public `audit_semantic_integrity_v1` takes a fresh Deferred snapshot and runs the same V3 closures over current data without time advancement, repair, cleanup, backfill, cached results, or any write.

## Closed cross-layer ordering

Add only:

```rust
enum SemanticAppraisalBeginStatusV1 { Claimed, BudgetExhausted, CapacityDeferred }
enum SemanticAppraisalCapacityReasonV1 { RetentionCapacityUnavailable }
```

Add `capacity_reason: Option<_>` and make the begin result's budget optional. Enforce: claimed has nonce/challenge/budget and no capacity reason; budget-exhausted has budget but no nonce/challenge/reason; capacity-deferred has no nonce/challenge, exact reason, and optional budget.

Implement in order: Contracts -> Store/private committed outcome -> Runtime validation/error -> PyO3 code -> typed `bridge.py` exception -> `main.py`. Capacity makes zero semantic Provider generation/network/token calls, schedules no retry, keeps the transaction's inbound revision, and lets the main reply continue. `retry_expired_or_unknown` also makes zero begin Provider calls and bypasses the current second settle attempt. Neither path performs a raw second apply.

## Narrow implementation gate

- [ ] Freeze the four hashes above in `semantic::tests::appraisal_compaction_hash_vectors_are_stable`; add the `64+64+64+63`, then fourth-persona-64th capacity/no-claim case and one `+299999/+300000` target-first expiry/replay/hierarchical-fold case in existing Rust test surfaces.
- [ ] Cover duplicate-pending exact replay; fully receipted and legacy-NULL migrated `budget_exhausted` as validated-but-never-replayed history; legacy-all-NULL active-code retry classification; exact V1/V2/V3 migration; open no-time-mutation; read-only audit; and one pre-DDL/V3 closure corruption rollback without a new test file.
- [ ] Add one Python capacity/retry classification case to `tests/test_runtime_integration.py`.
- [ ] Run only the focused semantic Rust filters, `cargo check --locked --offline -p ae-contracts -p ae-store -p ae-runtime -p ae-pyo3`, the two existing Python nodes below plus the new focused case, and `python -m py_compile main.py astr_embodiment/bridge.py`.

Existing Python regression nodes:

```text
tests/test_runtime_integration.py::test_semantic_inbound_calls_provider_once_and_submits_exact_six_key_proposal
tests/test_runtime_integration.py::test_semantic_budget_exhaustion_skips_provider_and_keeps_normal_reply_open
```

Passing this gate is Task 10B evidence only. Task 11/12 active proactive Host/Native surfaces and release/platform gates remain outstanding.
