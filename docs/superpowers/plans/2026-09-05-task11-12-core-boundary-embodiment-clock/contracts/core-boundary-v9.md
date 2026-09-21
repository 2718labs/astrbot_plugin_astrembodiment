# Core Boundary V9 Contract

Status: frozen Task 11/12 shared contract

Authority: [top-level addendum](../../2026-09-05-task11-12-core-boundary-embodiment-clock.md) and [work index](../index.md).

This contract owns migration routing, the permanent overlay fence, raw legacy preimages and downgrade resistance.

## 3. Autonomy DB v9 permanent fence

### 3.1 Explicit versions and read-only route selection

Use explicit constants so a future current-version change cannot make v8
validation accidentally accept v9:

```rust
const AUTONOMY_DB_VERSION_V6: u32 = 6;
const AUTONOMY_DB_VERSION_V7: u32 = 7;
const AUTONOMY_DB_VERSION_V8: u32 = 8;
const AUTONOMY_DB_VERSION_V9: u32 = 9;
pub const AUTONOMY_DB_VERSION: u32 = AUTONOMY_DB_VERSION_V9;
const MIGRATION_DIGEST_V9: &[u8] =
    b"ae.autonomy.db.v9.core-boundary-overlay.v1";
```

The connection-open path in `Store::open` invokes the v9 router immediately
after SQLite opens and before any wall-clock read, mutating PRAGMA, migration
transaction, table-specific PRAGMA, `Store::migrate` prologue,
`autonomy::migrate_autonomy`, `semantic::migrate_schema`, common metadata
UPSERT or `CREATE IF NOT EXISTS`. The router runs one read-only global
catalog/version preflight. Its first catalog statement is an unfiltered
`sqlite_schema` query with
`LIMIT MAX_V9_GLOBAL_SCHEMA_OBJECTS + 1`.
`MAX_V9_GLOBAL_SCHEMA_OBJECTS` is 4,096; object name and per-object SQL text
limits are 96 and 32,768 bytes, and cumulative catalog name/SQL bytes are
capped at 16 MiB before allocation. Validate storage classes, duplicate names,
object types, schema-version rows and migration-receipt identities before using
a discovered name in another statement.

The exact-v9 branch returns from this connection-open router. It is not a
branch inside `Store::migrate`/`migrate_autonomy` and cannot fall through a
shared migration prologue. In
particular, an existing v9 database never reaches the current-version metadata
UPSERT or the semantic migration helper. Its dedicated verifier enables
query-only enforcement before its first follow-up query.

The preflight chooses exactly one route:

| Preflight result | Route |
| --- | --- |
| fresh DB | run existing base/autonomy v1-v8 and semantic constructors in their frozen order; after the combined legacy schema is committed and exact, enter first v9 upgrade |
| authenticated version `< 8` | run only the already defined sequential autonomy and semantic legacy migrations to exact v8/current semantic schema; after they commit, enter first v9 upgrade |
| authenticated exact v8 | run only any still-required existing semantic migration; do not repair/rebuild autonomy; verify the combined legacy schema, commit it, then enter first v9 upgrade |
| exact v9 receipt plus exact control `applied` or `failed_closed` | use the dedicated v9 read-only verifier and return with zero migration writes |
| v9 control `pending` visible after reopen | corruption (`CORE_BOUNDARY_PENDING_CORRUPT`); no resume and no write |
| v9 receipt/control missing, duplicated, mismatched or partially installed | corruption; no repair and no write |
| version `> 9` or unknown catalog identity | newer/unsupported DB; no write |

Legacy or semantic migration failure before reaching the exact combined legacy
schema rolls back and fails open; it must not create a v9 fence over an unknown
schema. V9 installation is always the final migration, never an intermediate
step followed by semantic DDL. Existing v9 verification runs with SQLite
query-only enforcement and does not refresh a timestamp, recompute-and-store a
digest or retry a `failed_closed` upgrade. After a successful `applied`
verification, Store restores only the connection's prior `query_only` flag so
typed core mutations may run; this is not a database write and must leave
`total_changes` unchanged. A `failed_closed` Store exposes no Native mutation
surface.

### 3.2 Sole complete v9 schema authority and first-operation guard

[V9 schema manifest](v9-schema-manifest.md) is the sole byte authority for
`AUTONOMY_SCHEMA_V9_SQL`. It freezes the complete ordered input, canonical
renderer, output object sequence and both golden SHA-256 values. It includes
boundary control/failure/disposition/owner receipts, core inbound and ordinary
delivery receipts, inventory, persona anchor/profile/schedule, rolling clock,
all explicit indexes, immutable/CAS/fold guards and every legacy I/U/D deny
trigger.

Card 11A installs that complete schema once as the final migration. Cards 11B
and 11C may use its empty tables but may not execute `CREATE`, `ALTER`,
`DROP`, request-path DDL, add an index/trigger, reorder an object or change a
catalog SQL byte. A missing object is `V9_SCHEMA_INCOMPLETE`, never authority
for a later lazy repair.

The exact schema identity remains:

```text
schema_sql_sha256 = SHA256(exact rendered AUTONOMY_SCHEMA_V9_SQL bytes)
schema_digest = H("ae.autonomy.db.v9.schema-sql.v1",
                  exact rendered AUTONOMY_SCHEMA_V9_SQL bytes)
schema_migrations_v9_row =
  (version=9, digest=exact_bytes(MIGRATION_DIGEST_V9),
   completed_at_ms=authoritative_now_utc_ms)
```

This follows the existing source schema: `schema_migrations.digest` stores the
literal `MIGRATION_DIGEST_V9` bytes, not `schema_digest`.
`completed_at_ms` uses the one migration instant captured only after the
read-only route admits a first v9 upgrade. The control row stores
`schema_digest` and links the failure receipt plus terminal overlay roots.

The v9 verifier checks the complete ordered object-name/type/table/normalized-
catalog-SQL tuple against the manifest, the renderer source/output goldens,
schema receipt and control. Alias, shadow, missing, extra v9-prefixed object or
SQL-byte mismatch is corruption. Every catalog/row scan uses the frozen
sentinel and aggregate byte bounds; there is no background repair or `VACUUM`.

One Store-private function,
`enforce_core_boundary_v9_retired(RetiredOperationTagV1)`, is the only
retired-executor gate. It reads and verifies the already-open Store's terminal
v9 fence without parsing an operation body or opening another connection.
Until Card 12A deletes the callers, its invocation is literally the first
statement in every retired entry point located in:

- `crates/ae-store/src/lib.rs` for generic event/interaction dispatch;
- `crates/ae-store/src/autonomy.rs`;
- `crates/ae-store/src/alpha3/mod.rs`;
- `crates/ae-store/src/alpha3/interaction.rs`;
- `crates/ae-store/src/alpha3/projection.rs`;
- `crates/ae-store/src/alpha3/ecosystem.rs`;
- `crates/ae-store/src/alpha3/lived_world.rs`.

The guard returns `UNSUPPORTED_CORE_BOUNDARY` before JSON/event-body decode,
target/secret/relation read, transaction begin or write. A path that cannot
place this guard first is deleted in 11A rather than temporarily retained.
Tests enumerate every function in those files against the centralized tag
manifest; a new unclassified executor fails the gate.

### 3.3 First v9 transaction and failure semantics

Only after the connection-open router admits a first v9 upgrade does one outer
`BEGIN IMMEDIATE` run:

1. Capture exactly one `authoritative_now_utc_ms`; every v9 migration row uses
   it and no helper reads the wall clock again.
2. Execute the complete schema manifest in order and insert the exact version-9
   `schema_migrations` receipt.
3. Before opening a savepoint, run the classification-independent bounded
   `RawFailurePreimageV1` scan over every table in
   `IMMUTABLE_LEGACY_TABLE_MANIFEST_V1`. Persist its row count, framed-byte
   count, catalog digest and raw root in
   `core_boundary_failure_receipt_v1`; insert `core_boundary_control_v1` as
   `pending` and bind that receipt/root. A scan limit, allocation, catalog,
   SQLite or I/O failure here is Fatal and rolls back the whole outer
   transaction; no unverifiable `failed_closed` state may be committed.
4. Open a savepoint. Inside it, run the new
   `verify_autonomy_v8_read_only`. It may select, frame, hash and classify; it
   may not repair, backfill, rebuild, use `INSERT OR IGNORE`/DDL, invoke a
   legacy migration or read time.
5. Build dispositions and owner receipts only in new v9 tables. Do not update a
   legacy row.
6. Re-run both the classification-independent raw scan and the classified
   owner scan before finalization. The first must match the persisted failure
   receipt exactly; the second must match every disposition/owner receipt.
7. On success, release the savepoint, set control to `applied` with final
   classified preimage/disposition/receipt roots, retain the linked raw failure
   proof, and commit once.
8. On a typed/classifiable v8 data-authority failure after step 3, roll back the
   savepoint. Confirm that disposition/owner tables are empty, keep the
   persisted raw failure root/receipt, set control to `failed_closed` with the
   stable failure code and domain-separated `not_classified` disposition/
   owner-receipt roots, then commit. Those two constants explicitly mean “no
   classification committed”; they are never presented as legacy-byte proof.
9. On unexpected SQLite, I/O, catalog, limit plumbing or durability failure,
   roll back the entire outer transaction and fail `Store::open`. No Host
   clock starts.

The typed split is
`V9UpgradeError::Closed(CoreBoundaryClosedCode)` versus
`V9UpgradeError::Fatal(StoreOpenError)`; error strings never choose a commit
path. A committed `failed_closed` database still has
`MAX(schema_migrations)=9`, exact schema/control/failure receipts and all
legacy write-deny triggers, but exposes no Native mutation surface. AstrBot may
continue an ordinary reply only without claiming a Native mutation.

A committed `pending` row is never resumable. SQLite atomicity means ordinary
interruption exposes either no v9 transaction or a terminal state; visible
pending is `CORE_BOUNDARY_PENDING_CORRUPT` and zero-write.

On exact-v9 reopen, the query-only verifier branches by terminal state before
any downstream migration prologue:

- `applied`: verify schema receipt/catalog, control, failure receipt and a
  fresh identical `RawFailurePreimageV1`, then verify every disposition,
  owner receipt and all three classified control roots.
- `failed_closed`: verify schema receipt/catalog, control, failure receipt and
  a fresh identical raw root/count/byte count; require zero disposition/owner
  rows and the exact `not_classified` roots/failure code.
- either branch: no timestamp refresh, metadata UPSERT, repair or retry;
  SQL trace and `total_changes` remain zero.

The failure receipt is therefore an authenticated closure over the old logical
SQLite bytes even when semantic v8 classification fails. Raw database-file
hashes are not the gate because WAL connection bookkeeping is outside the
logical migration contract.

### 3.4 Overlay-only retirement matrix

Legacy rows are never renamed, settled, expired, zeroed, rewritten or deleted.
The following are effective v9 dispositions only:

| Legacy record | Authenticated legacy state | Immutable v9 disposition |
| --- | --- | --- |
| intention | `Forming`, `Ready`, `Deferred`, `Externalizing` | `suppressed_core_boundary` |
| intention/outbound | `DispatchPending`, no adapter-call-start evidence | `terminal_core_boundary` |
| intention/outbound | `AdapterCallStarted` or authenticated started dispatch claim | `dispatch_unknown_no_retry` |
| intention/outbound | `AdapterSubmitted`, `PlatformAccepted`, `DeliveryConfirmed`, `DispatchUnknown` | `historical_fact_preserved_no_retry` |
| intention/outbound | existing `Suppressed`, `Expired`, `Terminal` | `historical_terminal_preserved` |
| claim kind `wake`, `wake_v2` | valid row | `retired_wake_claim` |
| claim kind `externalization`, `externalization_v2` | valid row | `retired_externalization_claim` |
| claim kind `dispatch`, `dispatch_v2` | valid row | `retired_dispatch_claim` |
| consistent externalization reservation | reservation/budget pair fully authenticates | `effective_charged_unknown_frozen` |
| unprovable reservation | missing/mismatched authority | `frozen_unverified_no_refund` |
| target/ciphertext/key reference | any authenticated state | `preserved_unreadable_by_core` |
| consent/contact/basis/readiness/gate/policy | any authenticated state | `preserved_ignored` |
| operational-authority row/head | any raw-authenticated state | `historical_authority_preserved` |

`effective_charged_unknown_frozen` is audit semantics in the overlay. Physical
`reserved_tokens`, `charged_tokens`, `used_tokens`, claim rows and marker bytes
remain unchanged. Since all active proactive accounting is retired, no code can
charge, refund, reserve or reuse them. This avoids alias collisions and double
charging that a physical claim-kind rename or settlement would create.

The sole full set is `IMMUTABLE_LEGACY_TABLE_MANIFEST_V1` in
[v9-schema-manifest.md](v9-schema-manifest.md). Every listed table is whole-
table immutable and receives exact INSERT/UPDATE/DELETE deny triggers. In
particular it includes `autonomy_operational_authority` and
`autonomy_operational_authority_head`; v9 never appends, checkpoints or repairs
either table. Each manifest entry fixes table name, ordinal/record kind,
primary-key column order, owner resolver and disposition function. Every row,
including an unknown claim/proposal state, enters the raw proof; a state that
cannot be classified makes the upgrade `failed_closed` rather than escaping
the overlay.

Future mutable core tables are not part of this raw retirement preimage. The
legacy local sleep/profile rows are preserved and separately anchored during
persona initialization; new profile/clock tables are also excluded.

### 3.5 Exact raw-SQLite preimage and v9-only chain

The migration hashes SQLite values without parsing or reserializing JSON/text.
For every allowlisted row, columns are read in frozen `table_xinfo.cid` order.
Each value is framed as:

```text
column_ordinal: U16LE
storage_class: U8       # null=0, integer=1, real=2, text=3, blob=4
byte_length: U64LE
raw_value_bytes
```

`NULL` has length zero; INTEGER is signed two's-complement I64LE with length
eight; REAL is its SQLite `f64::to_bits()` U64LE with length eight; TEXT/BLOB
use the exact bytes returned by SQLite. The classification-independent failure
scan frames every storage class before interpretation. The later v8 semantic
verifier rejects REAL in retired authority as a classifiable `failed_closed`,
but the already persisted raw proof still authenticates that value. The
catalog digest binds table name and every column name/type/not-null/PK ordinal,
so the same bytes cannot be reinterpreted under another schema. In every
formula, `LP64(x) = U64LE(byte_length(x)) || x`.

Before semantic classification, all rows are sorted by `(record_kind U16LE,
record_key_bytes,row_digest)` and identical ties are retained as repeated
leaves. The failure proof is:

```text
failure_0 = H("ae.core-boundary.failure-preimage-root.v1",
  immutable_legacy_manifest_digest || legacy_catalog_digest)
failure_n = H("ae.core-boundary.failure-preimage-step.v1",
  failure_(n-1) || U64LE(n-1) || U16LE(record_kind) ||
  LP64(record_key_bytes) || row_digest)
failure_receipt_bytes = U16LE(1) || U64LE(source_row_count) ||
  U64LE(source_payload_bytes) || legacy_catalog_digest || failure_n ||
  U64LE(authoritative_now_utc_ms)
failure_receipt_digest = H("ae.core-boundary.failure-receipt.v1",
                           failure_receipt_bytes)
```

Counts and bytes use checked U64 addition. The complete scan is limited by
`MAX_V9_ROWS=65_536` and 64 MiB framed payload. Exceeding either limit is Fatal
and produces no v9 commit; a truncated root is never persisted.

Composite record keys use the same frames for primary-key fields and begin
with `(key_schema_version: U8=1, record_kind: U16LE,
component_count: U16LE)`. Each component retains its frozen column ordinal,
storage class, U64LE length and exact bytes. The domain is unique per record
kind. Kind-domain suffixes are the ASCII decimal `record_kind` without signs or leading zeroes,
and `owner_kind_code` is `U8(1)` for persona or `U8(2)` for global/unowned.
The checked-in manifest assigns these unsigned 16-bit disposition codes:
`suppressed_core_boundary=1`, `terminal_core_boundary=2`,
`dispatch_unknown_no_retry=3`, `historical_fact_preserved_no_retry=4`,
`historical_terminal_preserved=5`, `retired_wake_claim=6`,
`retired_externalization_claim=7`, `retired_dispatch_claim=8`,
`effective_charged_unknown_frozen=9`, `frozen_unverified_no_refund=10`,
`preserved_unreadable_by_core=11`, `preserved_ignored=12`, and
`historical_authority_preserved=13`.

```text
record_key_bytes = framed composite key
row_digest = H("ae.core-boundary.raw-row.v1/<kind>",
               LP64(record_key_bytes) || LP64(framed_all_columns))
preimage_leaf = H("ae.core-boundary.preimage-leaf.v1/<kind>",
                  LP64(record_key_bytes) || row_digest)
disposition_leaf = H("ae.core-boundary.disposition-leaf.v1/<kind>",
  LP64(record_key_bytes) || row_digest || U16LE(disposition_code) ||
  owner_kind_code || owner_key)
```

Rows are ordered lexicographically by `(record_kind U16LE,
record_key_bytes)`, never by locale/text collation. For each owner, the two
distinct chains are:

```text
preimage_0 = H("ae.core-boundary.owner-preimage-root.v1",
               schema_digest || owner_kind_code || owner_key)
preimage_n = H("ae.core-boundary.owner-preimage-step.v1",
  preimage_(n-1) || U64LE(n-1) || LP64(record_key_bytes) || preimage_leaf)
disposition_0 = H("ae.core-boundary.owner-disposition-root.v1",
                  schema_digest || owner_kind_code || owner_key)
disposition_n = H("ae.core-boundary.owner-disposition-step.v1",
  disposition_(n-1) || U64LE(n-1) || LP64(record_key_bytes) ||
  disposition_leaf)
```

Each row has exactly one owner. An authenticated path from relation/intention/
binding to one persona uses that persona digest. Missing, ambiguous, conflicting
or genuinely global ownership uses the single constant
`H("ae.core-boundary.owner.global-unowned.v1", empty)`; ownership is never
guessed and a record is never duplicated across receipts.

Owner kind codes are `persona=1` and `global_unowned=2`. The global bucket uses
the fixed owner key above; `(owner_kind,owner_key)` prevents collision with a
persona digest. The owner set is exactly every authenticated legacy persona plus
one global/unowned owner, even when an owner has zero retired rows. For every
owner, `receipt_bytes` is this canonical value and `receipt_digest` hashes it:

The global/unowned receipt has exactly `journal_prefix_revision=0` and
`legacy_operational_prefix_count=0`; its empty anchors are respectively
`H("ae.core-boundary.global-unowned.journal-prefix-empty.v1", empty)` and
`H("ae.core-boundary.global-unowned.operational-prefix-empty.v1", empty)`.
It never borrows a persona or current global head.

```text
receipt_bytes = U16LE(1) || owner_kind_code || owner_key ||
  U64LE(journal_prefix_revision) || journal_prefix_anchor ||
  U64LE(legacy_operational_prefix_count) ||
  legacy_operational_prefix_anchor || U64LE(source_row_count) ||
  preimage_root || U64LE(disposition_count) || disposition_root ||
  U64LE(authoritative_now_utc_ms)
receipt_digest = H("ae.core-boundary.owner-receipt.v1", receipt_bytes)
```

Source-row count must equal disposition count. Owner receipts are ordered by
`(owner_kind_code,owner_key)` and folded into three distinct control roots:

```text
global_preimage_0 = H("ae.core-boundary.global-preimage-root.v1", schema_digest)
global_preimage_n = H("ae.core-boundary.global-preimage-step.v1",
  global_preimage_(n-1) || U64LE(n-1) || owner_kind_code || owner_key ||
  U64LE(source_row_count) || preimage_root)
global_disposition_0 = H("ae.core-boundary.global-disposition-root.v1", schema_digest)
global_disposition_n = H("ae.core-boundary.global-disposition-step.v1",
  global_disposition_(n-1) || U64LE(n-1) || owner_kind_code || owner_key ||
  U64LE(disposition_count) || disposition_root)
global_receipt_0 = H("ae.core-boundary.global-receipt-root.v1", schema_digest)
global_receipt_n = H("ae.core-boundary.global-receipt-step.v1",
  global_receipt_(n-1) || U64LE(n-1) || owner_kind_code || owner_key ||
  receipt_digest)
```

The final `_n` values are the control row's `preimage_root`,
`disposition_root` and `receipt_root`. They authenticate every manifest row
exactly once, every legacy persona prefix anchor, and the global/unowned group.

For each persona, v9 records the legacy journal prefix that existed at the
migration instant: `(through_logical_revision, chain_digest_at_through)` under
`ae.core-boundary.legacy-journal-prefix.v1`. Future inbound/delivery rows may
append without changing this prefix receipt. The same rule freezes the legacy
operational-authority prefix count/head. V9 never appends to or repairs
`autonomy_operational_authority`; every retirement leaf uses only the new v9
chain.

The v9 schema receipt makes downgrade fail before a v8 executor can run: the
v8 binary sees `MAX(schema_migrations)=9 > AUTONOMY_DB_VERSION_V8` and rejects
open. A forged removal/mutation of that receipt breaks the v9 control/schema/
preimage closure.
