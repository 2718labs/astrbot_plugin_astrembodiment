# Retired Surface and Release Contract

Status: frozen Task 11/12 shared contract, with the controlled Task 12A historical-codec revision in Section 8.1.1.

Authority: [top-level addendum](../../2026-09-05-task11-12-core-boundary-embodiment-clock.md) and [work index](../index.md).

This contract owns the closed unsupported-operation surface, Host/UI/module removal and same-source cross-platform artifact proof.

## 8. Exhaustive Task 12 retirement

### 8.1 Closed manifests and bounded pre-deserialization classifier

The independently adjudicated `RETIRED_OPERATION_NAME_MANIFEST_V1` is exactly
these 35 ASCII strings in this order:

```text
_autonomy_call
alpha3_call
apply_event
apply_interaction
apply_interaction_v1
autonomy_status
begin_semantic_appraisal_v1
bind_outbound_target
bootstrap_autonomy
claim_wake
claim_wake_v2
gate_and_claim_dispatch
gate_and_claim_dispatch_v2
gate_and_claim_externalization
gate_and_claim_externalization_v2
gate_externalization
host_readiness_witness_digest_v1
integration_availability_v1
observe_body_snapshot_v1
observe_budget_summary_v1
observe_execution_receipt_v1
observe_gate_reasons_v1
pending_autonomy_work
record_relation_inbound
recover_autonomy
scope_digests
settle_dispatch
settle_dispatch_v2
settle_externalization
settle_externalization_v2
settle_wake
settle_wake_v2
upsert_budget_policy
verify_autonomy_projection
wake_caller_incarnation_v2
```

`apply_interaction_v1` is the Python alias for `apply_interaction`;
`alpha3_call` and `_autonomy_call` are the two generic dispatch aliases.
Every v1/v2 wake, externalization and dispatch name is listed separately; there
is no “all generations” wildcard. Old semantic begin is retired because
`commit_core_inbound_v1` owns begin/reservation atomically; semantic settle
remains allowed.

The generic-event classifier has this exact kind manifest. Route code 1 means
no active submission; 2 means typed core delivery only; 3 means historical
decode/verify only under Section 8.1.1; 4 means Store-owned core inbound projection
only; 5 means typed embodiment-clock input only. Every kind is unsupported
through `apply_event`.

| code | exact historical/type name | route code |
| ---: | --- | ---: |
| 1 | `UserStimulusV1` | 1 |
| 2 | `UserReactionV1` | 1 |
| 3 | `CorrectionClaimV1` | 1 |
| 4 | `CorrectionVerdictV1` | 1 |
| 5 | `SelfActionCandidateV1` | 1 |
| 6 | `DeliveryOutcomeV1` | 2 |
| 7 | `SettlementEvidenceV1` | 1 |
| 8 | `TimeAdvanceV1` | 3 |
| 9 | `AdminActionV1` | 1 |
| 10 | `InteractionFactBatchV1` | 4 |
| 11 | `EmbodimentTimeAdvanceRequestV1` | 5 |

The exact retired Host command/callback registry is these 14 identifiers in
this order:

```text
_commit_contact_control
_persist_proactive_migration
_prepare_proactive_settings
/ae_contact_end
/ae_contact_pause
ae_contact_end
ae_contact_pause
ae_wake
AutonomousSupervisor
contact_end_command
contact_pause_command
emergency_wake_command
execute_proactive_intention
submit_proactive_message
```

The Store/Runtime bypass manifest is a closed set of exact Rust symbol suffixes:
`apply_interaction_fact_batch_v1`, `handle_alpha3`,
`ensure_relation_authority_tx`, `load_relation_contact_v1`,
`load_relation_consent_v1`, `relation_contact_v1`,
`relation_consent_v1`, `upsert_relation_policy`,
`store_outbound_target`, `load_current_outbound_target`,
`upsert_autonomy_scope`, `list_autonomy_scopes`,
`list_autonomy_scopes_for_persona`,
`rebuild_autonomy_scope_bindings_from_journal`,
`claim_wake_proposal`, `load_wake_claim`, `load_wake_claim_v2`,
`load_wake_time_settlement_v1`, `commit_autonomous_wake`,
`commit_autonomous_wake_claim_v1`, `commit_alpha3_wake`,
`recover_retired_wake_v2_claims`, `write_autonomy_snapshot`,
`read_autonomy_snapshot`, `pending_outbound`,
`autonomy_work_scopes`, `rebuild_autonomy_projection`,
`recover_orphaned_dispatches`, `recover_orphaned_externalizations`,
`externalization_reserved_tokens`, `list_pending_autonomy_work`,
`list_pending_autonomy_work_for_relation`,
`load_outbound_for_intention`, `outbound_submission_metrics`,
`has_autonomy_journal_delta`, `relation_budget_ledger_v1`, and
`alpha3_authoritative_fingerprint_v1`. Together with the 35 externally
addressable names, this is the complete input to
`RetiredOperationTagV1`; additions fail an exhaustive match.

The compatibility classifier accepts at most 64 ASCII bytes for an operation
name and, for `apply_event`, only the fixed envelope prefix needed to read its
one-byte kind. A manifest hit returns exactly `UNSUPPORTED_CORE_BOUNDARY`
before full JSON/event deserialization, `core()`, DB open, target read or
transaction. Unknown names return `UNKNOWN_OPERATION`. Direct retired
PyO3/Python symbols are absent; any crate-local executor that remains during
11A invokes the central v9 guard as its literal first statement and is deleted
atomically in 12A.

The scanner allows retired strings inside this manifest, bounded classifier,
migration-only tag table, the exact historical codec roles in Section 8.1.1,
and their tests. It rejects retired execution exports, dispatcher reachability,
active Host imports, command/settings registration and retired Python archive
members. A source-text occurrence in a historical codec is not itself an
active capability; Rust visibility alone does not decide execution authority.

### 8.1.1 Controlled Task 12A historical-codec exception

Decision recorded on 2026-09-08 against implementation commit
`0784ecebbc3878a8e7231a3666c38526cd95676a`: preserve the existing public Rust
historical DTO/codec boundary used by continuum replay and Store migration
verification. Independent execution-boundary review reported APPROVE with no
P0/P1 findings. This exception replaces the former opaque-only/private-only
kind-8 packaging requirement; it does not authorize an event submission path.
No `ae_contracts::legacy_audit` opaque module has been implemented, and delivery
reports must not claim otherwise.

The exact kind-8 symbols retained for this role are:

| Symbol | Historical role and limit |
| --- | --- |
| `ae_contracts::TimeAdvanceV1` (`ae_contracts::autonomy::TimeAdvanceV1`) | Public serializable historical DTO; constructing a value grants no submission authority. |
| `ae_contracts::FrozenTimeInputV1`, `ae_contracts::AutonomousStimulusV1` | Existing field DTOs of `TimeAdvanceV1`; data only, without an executor. |
| `ae_contracts::CanonicalEvent::TimeAdvance` | Historical discriminant retained for byte/digest verification. |
| `ae_contracts::wire::KIND_TIME_ADVANCE` | Historical kind code 8; route code remains 3. |
| `ae_contracts::wire::decode_event` | Pure historical bytes-to-DTO decoder. It does not receive a Store or Runtime handle. |
| `ae_contracts::wire::encode_event_checked`, `ae_contracts::wire::encode_event`, `ae_contracts::wire::event_digest` | Pure canonical encoding/hash helpers used to verify historical identity. They may represent kind 8 in memory, but cannot persist or execute it. |

The public decoder returns a concrete `CanonicalEvent`, not an opaque object.
`ae_continuum::verify_replay` consumes stored `JournalRow` bytes, decodes them,
checks event digests/receipts/hash chains and returns a report without writes.
Store-private historical verification, including
`ae_store::autonomy::verify_autonomy_v8_read_only`, retains the existing bounded
row/catalog checks. This exception does not turn the general wire decoder into
a Native compatibility endpoint or exempt its callers from storage bounds.

The absence of an execution path is enforced separately: Task 12A removed the
Runtime autonomy/alpha3 execution modules, Runtime generic `apply_event`, the
old Store public execution methods and generic `commit_journal`, the alpha3
request/response dispatcher enums, and the retired PyO3/Host callables. Native
and wrapper exports remain the exact 19-method manifest. The pure Host request
compiler rejects retired operation names before body parsing and has no DB
capability. New time mutations enter only through typed embodiment-clock APIs.
Private legacy migration constructors remain subject to the existing v9 router
and fence; this revision does not relax the zero-write exact-v9 reopen or
immutable-legacy-table requirements.

Future review must reject any new Host/Native/Runtime submission route that
accepts this historical DTO. Reintroducing target/secret access, proactive
generation, contact intent, recipient selection, dispatch or send is outside
this exception. The wire bytes, manifest counts/goldens, v9 schema, and protected
historical fixtures are unchanged by this documentation revision.

The separate [Task 12A test migration handoff](../tasks/12a-test-migration.md)
records outstanding legacy test wiring. Execution-boundary approval is not a
claim that the old test suite compiles or that matrix regression coverage has
already been migrated.

### 8.2 Atomic surface deletion

After all Host callers use the new typed APIs, one Task 12A commit removes the
entire old surface across contracts request enums, Runtime, Store public
methods, PyO3 registration, `astr_embodiment/bridge.py`,
`astr_embodiment/__init__.py`, `python/astrembodiment_core/__init__.py`,
`main.py` and tests. Do not leave a commit where the wrapper imports a removed
Native symbol or Host calls an already deleted method.

Remove from source import graph and final archive:

- `astr_embodiment/proactive.py`;
- `astr_embodiment/proactive_settings.py`;
- `astr_embodiment/secret_store.py`;
- `astr_embodiment/autonomy.py` (the retired `AutonomousSupervisor`);
- supervisor/contact commands and every active target/secret/outbound package
  marker.

Deleting the secret-store module never opens/deletes an installed key file.
Removing UI keys never rewrites saved config. `main.py::_pending`, typed
ordinary DeliveryOutcome, Task 10B semantic reservations and Genesis remain.

The final direct Native/PyO3 callable manifest is exactly the following 19
`name@contract_version` entries in this order; every version is integer 1:

```text
advance_embodiment_time_v1@1
build_info_v1@1
commit_core_delivery_outcome_v1@1
commit_core_inbound_v1@1
compare_and_swap_embodiment_profile_v1@1
compile_core_host_request_v1@1
create_embodiment_persona_if_missing_v1@1
embodiment_clock_status_v1@1
ensure_genesis@1
flush_and_close@1
get_embodiment_persona_v1@1
health@1
inspect@1
list_embodiment_personas_v1@1
open@1
read_embodiment_profile_v1@1
settle_semantic_appraisal_v1@1
verify_replay@1
version@1
```

The Python bridge may add only local `loaded` and `close` adapters over
`health`/`flush_and_close`; they are not extension exports or generic Native
dispatchers. Extension `dir()`, registration, `__all__`, bridge callable
inspection and package scanning fail closed on any additional callable.

## 9. Same-SHA cross-platform provenance

Windows and Linux artifacts are not equivalent merely because filenames or
Python versions match. A clean build injects the exact 40-hex Git source SHA
and a closed core API digest into the Native `build_info_v1` response and each
wheel's immutable manifest. Build fails if the injected SHA differs from
`git rev-parse HEAD` or the worktree is dirty.

`CORE_PUBLIC_METHOD_MANIFEST_V1_BYTES` is `U16LE(count=19)` followed in the
shown order by `U16LE(UTF8-name-length) || UTF8(name) || U16LE(version)`. Its
fixed raw SHA-256 golden is
`800e4dccb2a29b6edbaa2ac7cc46c34f466ece682e0ccf1a272577aacb3790c4`.

The Task 11C pure `compile_core_host_request_v1` compiler is the nineteenth
entry. It has no DB or Runtime capability; its closed operation switch rejects
retired names and unknown operations before decoding a request body.

`RETIRED_SURFACE_MANIFEST_V1_BYTES` is `U16LE(35)` plus each shown operation as
`LP16(UTF8 name)`, then `U16LE(11)` plus each kind row as
`U8(code)||LP16(UTF8 type_name)||U8(route_code)`, then `U16LE(14)` plus each
shown Host identifier as `LP16(UTF8 name)`. Its fixed raw SHA-256 golden is
`6a28d0e925be34a108b636c47b4ea8ff1a69b763d7de14ba91d4a44127dc291a`.
No locale sort or normalization occurs; the displayed order is canonical.

The build API digest is:

```text
H("ae.core-public-api.v1",
  LP64(CORE_PUBLIC_METHOD_MANIFEST_V1_BYTES) ||
  CORE_PUBLIC_METHOD_MANIFEST_V1_SHA256 ||
  LP64(RETIRED_SURFACE_MANIFEST_V1_BYTES) ||
  RETIRED_SURFACE_MANIFEST_V1_SHA256 ||
  AUTONOMY_SCHEMA_V9_SQL_SHA256 || tzdb_content_sha256)
```

Both manifest compilers check count, lengths and fixed goldens before build;
the scanner consumes these checked-in bytes but is not their author.

The universal manifest records, for each real platform, wheel filename/hash,
Native binary filename/hash/build ID, source SHA, API digest, v9 schema digest,
tzdb release/content SHA and imported `build_info_v1`. Both wheels must be
built from the same clean source SHA and imported on actual Windows x64 and
Linux x86_64. The imported direct-method set and API digest must match exactly.

Only after both real imports pass may the universal ZIP be assembled. The ZIP
records both wheel hashes and its source/API/schema/tzdb identities, excludes
proactive/secret modules and active symbols/settings, and contains no
AstrCyberHuman dependency. Repacking an old wheel or a static member-list check
is not platform evidence.

## 11. Final evidence gate

Tasks 11/12 pass only with fresh evidence from one exact source SHA:

1. all migration routes behave exactly as Section 3 specifies; v9 reopen is
   zero-write and every legacy row remains byte-identical;
2. core inbound records all closed appraisal states, Existing never calls
   Provider, and no retired table changes;
3. typed ordinary delivery is idempotent, Store-revision-bound and succeeds
   after an intervening clock commit;
4. inventory epoch changes force a complete deduplicated rescan;
5. persona conditional-create/profile/schedule/time IDs, hashes, DDL, vendored tzdb,
   monotonicity, hydrated hot state and bounded restart pass;
6. zero-write recheck plus the authenticated one-head/64-receipt rolling clock
   keep background operation within frozen limits while adding zero rows to the
   existing persona journal and semantic history;
7. the exhaustive old-operation manifest is rejected before parse/core/open,
   direct old symbols are absent, and internal executors touch no target/write;
8. four Rust packages and changed Python layers compile/import;
9. real Windows/Linux wheels from the same clean SHA report identical source,
   API, schema and tzdb identities before the ZIP is assembled;
10. independent review finds no active proactive generation, contact intent,
    recipient selection, target/secret access, dispatch or send path.

A source compile, fixture-only test, static scanner, one-platform wheel or ZIP
preflight is not release approval. Until both real platform imports and final
independent review pass, the release gate remains `NO_GO`.
