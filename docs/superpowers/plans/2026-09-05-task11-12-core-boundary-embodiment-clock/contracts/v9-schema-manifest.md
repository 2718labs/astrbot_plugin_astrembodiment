# V9 Schema Manifest

Status: frozen sole byte authority for Card 11A

Authority: [top-level addendum](../../2026-09-05-task11-12-core-boundary-embodiment-clock.md), [work index](../index.md) and DESIGN_AUTHORITY_ID AE-T11T12-CORE-BOUNDARY-R2.

## One-time ownership

This file is the only authority for AUTONOMY_SCHEMA_V9_SQL. Card 11A renders
and installs every object below in one final v9 migration. Cards 11B and 11C
may not change this source, add or lazily create an object, or repair a catalog
difference. Missing, extra or byte-different v9 schema is
V9_SCHEMA_INCOMPLETE.

The source contains every boundary, failure, disposition, owner, inbound,
delivery, inventory, persona profile/schedule and rolling-clock object. All
tables are WITHOUT ROWID and every secondary uniqueness rule is an explicit
named index, so SQLite creates no implicit v9 autoindex. schema_migrations and
the pre-v9 tables are pre-existing and are not counted as v9-created objects.

## Canonical renderer and goldens

V9_SCHEMA_SOURCE_V1 is the exact byte content between the source markers below,
excluding the marker lines. It is UTF-8 without BOM, uses LF only, starts with
the first C of CREATE and ends with one LF after the last @deny line.

The renderer reads lines without Unicode normalization:

1. A line not beginning with @ is copied with its LF unchanged.
2. A line @deny|NN|TABLE must have two ASCII digits NN, an ASCII identifier
   TABLE matching [a-z][a-z0-9_]{0,62}, and a unique ordinal/table.
3. That line itself emits no bytes. It expands INSERT, UPDATE, DELETE, in that
   order, using the exact template below. Each rendered statement ends LF.
4. Any blank line, CR, comment, unknown macro, missing final LF, duplicate
   object name or trailing byte rejects generation.
5. AUTONOMY_SCHEMA_V9_SQL is the rendered output. Catalog SQL for each object is
   compared with its exact rendered statement minus the final semicolon and LF.
   No whitespace or semantically equivalent normalization is allowed.

~~~text
CREATE TRIGGER core_boundary_retired_{NN}_{TABLE}_i_v1
BEFORE INSERT ON "{TABLE}"
BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_LEGACY_WRITE_DENIED'); END;
CREATE TRIGGER core_boundary_retired_{NN}_{TABLE}_u_v1
BEFORE UPDATE ON "{TABLE}"
BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_LEGACY_WRITE_DENIED'); END;
CREATE TRIGGER core_boundary_retired_{NN}_{TABLE}_d_v1
BEFORE DELETE ON "{TABLE}"
BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_LEGACY_WRITE_DENIED'); END;
~~~

The two SHA-256 values below are lowercase hex and are replaced only by a
design-authority revision that also replaces the source block:

- v9_schema_source_sha256: `7b4a5bf22b8c3695f00a80bc126dd559b9abb71e1aa70688bd7ce6ca7b5d0c49`
- autonomy_schema_v9_sql_sha256: `fb195037e7a15e267d761949c20ee2272fff6a5469a194d3d197c7cf5335627c`

11B implementation erratum (approved before release): matrix anchor revision
permits zero because the authenticated Genesis semantic origin uses revision
zero. The column remains nonnegative; this does not synthesize a semantic row.

The runtime schema_digest remains the domain-separated digest defined by the
core-boundary contract; these raw SHA-256 goldens independently catch renderer
or source drift.

## Complete canonical input

BEGIN V9_SCHEMA_SOURCE_V1
CREATE TABLE core_boundary_control_v1(
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  boundary_revision INTEGER NOT NULL CHECK(boundary_revision=1),
  state TEXT NOT NULL CHECK(state IN ('pending','applied','failed_closed')),
  externalization_disabled INTEGER NOT NULL CHECK(externalization_disabled=1),
  source_db_version INTEGER NOT NULL CHECK(source_db_version=8),
  authoritative_now_utc_ms INTEGER NOT NULL CHECK(authoritative_now_utc_ms>0),
  schema_digest BLOB NOT NULL CHECK(typeof(schema_digest)='blob' AND length(schema_digest)=32),
  failure_preimage_root BLOB NOT NULL CHECK(typeof(failure_preimage_root)='blob' AND length(failure_preimage_root)=32),
  failure_receipt_digest BLOB NOT NULL CHECK(typeof(failure_receipt_digest)='blob' AND length(failure_receipt_digest)=32),
  preimage_root BLOB NOT NULL CHECK(typeof(preimage_root)='blob' AND length(preimage_root)=32),
  disposition_root BLOB NOT NULL CHECK(typeof(disposition_root)='blob' AND length(disposition_root)=32),
  receipt_root BLOB NOT NULL CHECK(typeof(receipt_root)='blob' AND length(receipt_root)=32),
  failure_code TEXT CHECK(failure_code IS NULL OR (typeof(failure_code)='text' AND length(CAST(failure_code AS BLOB)) BETWEEN 1 AND 64)),
  CHECK((state IN ('pending','applied') AND failure_code IS NULL) OR (state='failed_closed' AND failure_code IS NOT NULL))
) WITHOUT ROWID;
CREATE TABLE core_boundary_failure_receipt_v1(
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  source_db_version INTEGER NOT NULL CHECK(source_db_version=8),
  source_row_count INTEGER NOT NULL CHECK(source_row_count BETWEEN 0 AND 65536),
  source_payload_bytes INTEGER NOT NULL CHECK(source_payload_bytes BETWEEN 0 AND 67108864),
  legacy_catalog_digest BLOB NOT NULL CHECK(typeof(legacy_catalog_digest)='blob' AND length(legacy_catalog_digest)=32),
  raw_preimage_root BLOB NOT NULL CHECK(typeof(raw_preimage_root)='blob' AND length(raw_preimage_root)=32),
  authoritative_now_utc_ms INTEGER NOT NULL CHECK(authoritative_now_utc_ms>0),
  receipt_bytes BLOB NOT NULL CHECK(typeof(receipt_bytes)='blob' AND length(receipt_bytes) BETWEEN 1 AND 65536),
  receipt_digest BLOB NOT NULL CHECK(typeof(receipt_digest)='blob' AND length(receipt_digest)=32)
) WITHOUT ROWID;
CREATE TABLE core_boundary_disposition_v1(
  record_kind INTEGER NOT NULL CHECK(record_kind BETWEEN 1 AND 31),
  record_key_bytes BLOB NOT NULL CHECK(typeof(record_key_bytes)='blob' AND length(record_key_bytes) BETWEEN 1 AND 512),
  owner_kind TEXT NOT NULL CHECK(owner_kind IN ('persona','global_unowned')),
  owner_key BLOB NOT NULL CHECK(typeof(owner_key)='blob' AND length(owner_key)=32),
  source_row_digest BLOB NOT NULL CHECK(typeof(source_row_digest)='blob' AND length(source_row_digest)=32),
  effective_disposition TEXT NOT NULL CHECK(typeof(effective_disposition)='text' AND length(CAST(effective_disposition AS BLOB)) BETWEEN 1 AND 64),
  reason TEXT NOT NULL CHECK(reason='core_boundary_upgrade'),
  authoritative_now_utc_ms INTEGER NOT NULL CHECK(authoritative_now_utc_ms>0),
  leaf_digest BLOB NOT NULL CHECK(typeof(leaf_digest)='blob' AND length(leaf_digest)=32),
  PRIMARY KEY(record_kind,record_key_bytes)
) WITHOUT ROWID;
CREATE TABLE core_boundary_persona_receipt_v1(
  owner_kind TEXT NOT NULL CHECK(owner_kind IN ('persona','global_unowned')),
  owner_key BLOB NOT NULL CHECK(typeof(owner_key)='blob' AND length(owner_key)=32),
  journal_prefix_revision INTEGER NOT NULL CHECK(journal_prefix_revision>=0),
  journal_prefix_anchor BLOB NOT NULL CHECK(typeof(journal_prefix_anchor)='blob' AND length(journal_prefix_anchor)=32),
  legacy_operational_prefix_count INTEGER NOT NULL CHECK(legacy_operational_prefix_count>=0),
  legacy_operational_prefix_anchor BLOB NOT NULL CHECK(typeof(legacy_operational_prefix_anchor)='blob' AND length(legacy_operational_prefix_anchor)=32),
  source_row_count INTEGER NOT NULL CHECK(source_row_count>=0),
  preimage_root BLOB NOT NULL CHECK(typeof(preimage_root)='blob' AND length(preimage_root)=32),
  disposition_count INTEGER NOT NULL CHECK(disposition_count>=0),
  disposition_root BLOB NOT NULL CHECK(typeof(disposition_root)='blob' AND length(disposition_root)=32),
  authoritative_now_utc_ms INTEGER NOT NULL CHECK(authoritative_now_utc_ms>0),
  receipt_bytes BLOB NOT NULL CHECK(typeof(receipt_bytes)='blob' AND length(receipt_bytes) BETWEEN 1 AND 65536),
  receipt_digest BLOB NOT NULL CHECK(typeof(receipt_digest)='blob' AND length(receipt_digest)=32),
  PRIMARY KEY(owner_kind,owner_key)
) WITHOUT ROWID;
CREATE TABLE embodiment_inventory_head_v1(
  singleton INTEGER PRIMARY KEY CHECK(singleton=1),
  epoch INTEGER NOT NULL CHECK(epoch>0),
  entry_count INTEGER NOT NULL CHECK(entry_count>=0)
) WITHOUT ROWID;
CREATE TABLE core_inbound_receipt_v1(
  persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  operation_id BLOB NOT NULL CHECK(typeof(operation_id)='blob' AND length(operation_id)=16),
  turn_id BLOB NOT NULL CHECK(typeof(turn_id)='blob' AND length(turn_id)=16),
  request_digest BLOB NOT NULL CHECK(typeof(request_digest)='blob' AND length(request_digest)=32),
  observation_bytes BLOB NOT NULL CHECK(typeof(observation_bytes)='blob' AND length(observation_bytes) BETWEEN 1 AND 65536),
  observation_digest BLOB NOT NULL CHECK(typeof(observation_digest)='blob' AND length(observation_digest)=32),
  fact_id BLOB NOT NULL CHECK(typeof(fact_id)='blob' AND length(fact_id)=16),
  event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
  inbound_revision INTEGER NOT NULL CHECK(inbound_revision>0),
  clock_head_digest BLOB NOT NULL CHECK(typeof(clock_head_digest)='blob' AND length(clock_head_digest)=32),
  event_bytes BLOB NOT NULL CHECK(typeof(event_bytes)='blob' AND length(event_bytes) BETWEEN 1 AND 65536),
  event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
  initial_disposition TEXT NOT NULL CHECK(initial_disposition IN ('not_requested','claimed','budget_exhausted','capacity_deferred','retry_expired_or_unknown')),
  budget_receipt_bytes BLOB NOT NULL CHECK(typeof(budget_receipt_bytes)='blob' AND length(budget_receipt_bytes)<=65536),
  settlement_challenge_digest BLOB CHECK(settlement_challenge_digest IS NULL OR (typeof(settlement_challenge_digest)='blob' AND length(settlement_challenge_digest)=32)),
  provider_authority_granted_initial INTEGER NOT NULL CHECK(provider_authority_granted_initial IN (0,1)),
  initial_receipt_bytes BLOB NOT NULL CHECK(typeof(initial_receipt_bytes)='blob' AND length(initial_receipt_bytes) BETWEEN 1 AND 65536),
  initial_receipt_digest BLOB NOT NULL CHECK(typeof(initial_receipt_digest)='blob' AND length(initial_receipt_digest)=32),
  PRIMARY KEY(persona_scope,operation_id)
) WITHOUT ROWID;
CREATE TABLE core_delivery_receipt_v1(
  persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  operation_id BLOB NOT NULL CHECK(typeof(operation_id)='blob' AND length(operation_id)=16),
  turn_id BLOB NOT NULL CHECK(typeof(turn_id)='blob' AND length(turn_id)=16),
  inbound_operation_id BLOB NOT NULL CHECK(typeof(inbound_operation_id)='blob' AND length(inbound_operation_id)=16),
  request_digest BLOB NOT NULL CHECK(typeof(request_digest)='blob' AND length(request_digest)=32),
  event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
  committed_revision INTEGER NOT NULL CHECK(committed_revision>0),
  clock_head_digest BLOB NOT NULL CHECK(typeof(clock_head_digest)='blob' AND length(clock_head_digest)=32),
  event_bytes BLOB NOT NULL CHECK(typeof(event_bytes)='blob' AND length(event_bytes) BETWEEN 1 AND 65536),
  event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
  receipt_bytes BLOB NOT NULL CHECK(typeof(receipt_bytes)='blob' AND length(receipt_bytes) BETWEEN 1 AND 65536),
  receipt_digest BLOB NOT NULL CHECK(typeof(receipt_digest)='blob' AND length(receipt_digest)=32),
  PRIMARY KEY(persona_scope,operation_id)
) WITHOUT ROWID;
CREATE TABLE embodiment_persona_anchor_v1(
  persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  bot_token BLOB NOT NULL CHECK(typeof(bot_token)='blob' AND length(bot_token)=16),
  persona_token BLOB NOT NULL CHECK(typeof(persona_token)='blob' AND length(persona_token)=16),
  create_operation_id BLOB NOT NULL CHECK(typeof(create_operation_id)='blob' AND length(create_operation_id)=16),
  binding_revision INTEGER NOT NULL CHECK(binding_revision>0),
  initial_semantic_revision INTEGER NOT NULL CHECK(initial_semantic_revision>=0),
  incarnation_digest BLOB NOT NULL CHECK(typeof(incarnation_digest)='blob' AND length(incarnation_digest)=32),
  legacy_sleep_anchor_digest BLOB NOT NULL CHECK(typeof(legacy_sleep_anchor_digest)='blob' AND length(legacy_sleep_anchor_digest)=32),
  profile_digest BLOB NOT NULL CHECK(typeof(profile_digest)='blob' AND length(profile_digest)=32),
  schedule_digest BLOB NOT NULL CHECK(typeof(schedule_digest)='blob' AND length(schedule_digest)=32),
  initial_state_digest BLOB NOT NULL CHECK(typeof(initial_state_digest)='blob' AND length(initial_state_digest)=32),
  anchored_at_utc_ms INTEGER NOT NULL CHECK(anchored_at_utc_ms>0),
  request_digest BLOB NOT NULL CHECK(typeof(request_digest)='blob' AND length(request_digest)=32),
  receipt_bytes BLOB NOT NULL CHECK(typeof(receipt_bytes)='blob' AND length(receipt_bytes) BETWEEN 1 AND 65536),
  receipt_digest BLOB NOT NULL CHECK(typeof(receipt_digest)='blob' AND length(receipt_digest)=32)
) WITHOUT ROWID;
CREATE TABLE embodiment_profile_v1(
  persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  profile_schema_version INTEGER NOT NULL CHECK(profile_schema_version=1),
  revision INTEGER NOT NULL CHECK(revision>0),
  persona_tzid TEXT NOT NULL CHECK(typeof(persona_tzid)='text' AND length(CAST(persona_tzid AS BLOB)) BETWEEN 1 AND 128),
  tzdb_release TEXT NOT NULL CHECK(typeof(tzdb_release)='text' AND length(CAST(tzdb_release AS BLOB)) BETWEEN 1 AND 32),
  tzdb_content_sha256 BLOB NOT NULL CHECK(typeof(tzdb_content_sha256)='blob' AND length(tzdb_content_sha256)=32),
  profile_bytes BLOB NOT NULL CHECK(typeof(profile_bytes)='blob' AND length(profile_bytes) BETWEEN 1 AND 16384),
  profile_digest BLOB NOT NULL CHECK(typeof(profile_digest)='blob' AND length(profile_digest)=32)
) WITHOUT ROWID;
CREATE TABLE embodiment_sleep_schedule_v1(
  persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  schedule_schema_version INTEGER NOT NULL CHECK(schedule_schema_version=1),
  revision INTEGER NOT NULL CHECK(revision>0),
  profile_revision INTEGER NOT NULL CHECK(profile_revision>0),
  mode TEXT NOT NULL CHECK(mode IN ('auto','fixed')),
  schedule_bytes BLOB NOT NULL CHECK(typeof(schedule_bytes)='blob' AND length(schedule_bytes) BETWEEN 1 AND 4096),
  schedule_digest BLOB NOT NULL CHECK(typeof(schedule_digest)='blob' AND length(schedule_digest)=32)
) WITHOUT ROWID;
CREATE TABLE embodiment_clock_head_v1(
  persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  sequence INTEGER NOT NULL CHECK(sequence>=0),
  compacted_count INTEGER NOT NULL CHECK(compacted_count>=0),
  recent_count INTEGER NOT NULL CHECK(recent_count BETWEEN 0 AND 64),
  time_revision INTEGER NOT NULL CHECK(time_revision>0),
  state_revision INTEGER NOT NULL CHECK(state_revision>0),
  profile_revision INTEGER NOT NULL CHECK(profile_revision>0),
  schedule_revision INTEGER NOT NULL CHECK(schedule_revision>0),
  tzdb_release TEXT NOT NULL CHECK(typeof(tzdb_release)='text' AND length(CAST(tzdb_release AS BLOB)) BETWEEN 1 AND 32),
  tzdb_content_sha256 BLOB NOT NULL CHECK(typeof(tzdb_content_sha256)='blob' AND length(tzdb_content_sha256)=32),
  last_now_utc_ms INTEGER NOT NULL CHECK(last_now_utc_ms>0),
  next_due_at_utc_ms INTEGER NOT NULL CHECK(next_due_at_utc_ms>last_now_utc_ms),
  compacted_through_now_utc_ms INTEGER NOT NULL CHECK(compacted_through_now_utc_ms>=0 AND compacted_through_now_utc_ms<=last_now_utc_ms),
  sleep_state TEXT NOT NULL CHECK(sleep_state IN ('awake','drowsy','asleep')),
  sleep_episode_ordinal INTEGER NOT NULL CHECK(sleep_episode_ordinal>=0),
  dream_consolidated_episode_ordinal INTEGER NOT NULL CHECK(dream_consolidated_episode_ordinal>=0 AND dream_consolidated_episode_ordinal<=sleep_episode_ordinal),
  endogenous_phase_code INTEGER NOT NULL CHECK(endogenous_phase_code BETWEEN 0 AND 31),
  matrix_anchor_semantic_revision INTEGER NOT NULL CHECK(matrix_anchor_semantic_revision>=0),
  matrix_anchor_state_digest BLOB NOT NULL CHECK(typeof(matrix_anchor_state_digest)='blob' AND length(matrix_anchor_state_digest)=32),
  matrix_anchor_graph_digest BLOB NOT NULL CHECK(typeof(matrix_anchor_graph_digest)='blob' AND length(matrix_anchor_graph_digest)=32),
  matrix_epoch_bytes BLOB NOT NULL CHECK(typeof(matrix_epoch_bytes)='blob' AND length(matrix_epoch_bytes) BETWEEN 1 AND 4096),
  matrix_epoch_digest BLOB NOT NULL CHECK(typeof(matrix_epoch_digest)='blob' AND length(matrix_epoch_digest)=32),
  formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
  state_bytes BLOB NOT NULL CHECK(typeof(state_bytes)='blob' AND length(state_bytes) BETWEEN 1 AND 262144),
  state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
  compacted_chain_digest BLOB NOT NULL CHECK(typeof(compacted_chain_digest)='blob' AND length(compacted_chain_digest)=32),
  recent_chain_digest BLOB NOT NULL CHECK(typeof(recent_chain_digest)='blob' AND length(recent_chain_digest)=32),
  head_digest BLOB NOT NULL CHECK(typeof(head_digest)='blob' AND length(head_digest)=32),
  CHECK(sequence=compacted_count+recent_count)
) WITHOUT ROWID;
CREATE TABLE embodiment_clock_receipt_v1(
  persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  sequence INTEGER NOT NULL CHECK(sequence>0),
  mutation_kind INTEGER NOT NULL CHECK(mutation_kind IN (1,2,3)),
  operation_id BLOB NOT NULL CHECK(typeof(operation_id)='blob' AND length(operation_id)=16),
  event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
  request_digest BLOB NOT NULL CHECK(typeof(request_digest)='blob' AND length(request_digest)=32),
  frozen_now_utc_ms INTEGER NOT NULL CHECK(frozen_now_utc_ms>0),
  raw_elapsed_ms INTEGER NOT NULL CHECK(raw_elapsed_ms>=0),
  applied_elapsed_ms INTEGER NOT NULL CHECK(applied_elapsed_ms>=0 AND applied_elapsed_ms<=604800000),
  capped_gap INTEGER NOT NULL CHECK(capped_gap IN (0,1)),
  discrete_event_mask INTEGER NOT NULL CHECK(discrete_event_mask BETWEEN 0 AND 31),
  prior_chain_digest BLOB NOT NULL CHECK(typeof(prior_chain_digest)='blob' AND length(prior_chain_digest)=32),
  leaf_digest BLOB NOT NULL CHECK(typeof(leaf_digest)='blob' AND length(leaf_digest)=32),
  result_core_bytes BLOB NOT NULL CHECK(typeof(result_core_bytes)='blob' AND length(result_core_bytes) BETWEEN 1 AND 65536),
  result_core_digest BLOB NOT NULL CHECK(typeof(result_core_digest)='blob' AND length(result_core_digest)=32),
  resulting_state_digest BLOB NOT NULL CHECK(typeof(resulting_state_digest)='blob' AND length(resulting_state_digest)=32),
  commit_receipt_bytes BLOB NOT NULL CHECK(typeof(commit_receipt_bytes)='blob' AND length(commit_receipt_bytes) BETWEEN 1 AND 65536),
  commit_receipt_digest BLOB NOT NULL CHECK(typeof(commit_receipt_digest)='blob' AND length(commit_receipt_digest)=32),
  PRIMARY KEY(persona_scope,sequence)
) WITHOUT ROWID;
CREATE TABLE semantic_clock_anchor_proof_v1(
  persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
  semantic_revision INTEGER NOT NULL CHECK(semantic_revision>0),
  proof_version INTEGER NOT NULL CHECK(proof_version=1),
  semantic_commitment_digest BLOB NOT NULL CHECK(typeof(semantic_commitment_digest)='blob' AND length(semantic_commitment_digest)=32),
  proof_bytes BLOB NOT NULL CHECK(typeof(proof_bytes)='blob' AND length(proof_bytes) BETWEEN 1 AND 524288),
  proof_digest BLOB NOT NULL CHECK(typeof(proof_digest)='blob' AND length(proof_digest)=32),
  PRIMARY KEY(persona_scope,semantic_revision),
  FOREIGN KEY(persona_scope,semantic_revision) REFERENCES semantic_commits(persona_scope,semantic_revision) ON DELETE RESTRICT
) WITHOUT ROWID;
CREATE INDEX core_boundary_disposition_owner_v1 ON core_boundary_disposition_v1(owner_kind,owner_key,record_kind,record_key_bytes);
CREATE UNIQUE INDEX core_inbound_receipt_turn_v1 ON core_inbound_receipt_v1(persona_scope,turn_id);
CREATE UNIQUE INDEX core_inbound_receipt_event_v1 ON core_inbound_receipt_v1(persona_scope,event_id);
CREATE UNIQUE INDEX core_delivery_receipt_inbound_v1 ON core_delivery_receipt_v1(persona_scope,inbound_operation_id);
CREATE UNIQUE INDEX core_delivery_receipt_event_v1 ON core_delivery_receipt_v1(persona_scope,event_id);
CREATE UNIQUE INDEX embodiment_persona_anchor_identity_v1 ON embodiment_persona_anchor_v1(bot_token,persona_token);
CREATE UNIQUE INDEX embodiment_clock_receipt_operation_v1 ON embodiment_clock_receipt_v1(persona_scope,operation_id);
CREATE UNIQUE INDEX embodiment_clock_receipt_event_v1 ON embodiment_clock_receipt_v1(persona_scope,event_id);
CREATE TRIGGER core_boundary_control_no_delete_v1 BEFORE DELETE ON core_boundary_control_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_control_terminal_immutable_v1 BEFORE UPDATE ON core_boundary_control_v1 WHEN OLD.state IN ('applied','failed_closed') BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_control_pending_transition_v1 BEFORE UPDATE ON core_boundary_control_v1 WHEN OLD.state='pending' AND (NEW.state NOT IN ('applied','failed_closed') OR NEW.singleton!=OLD.singleton OR NEW.boundary_revision!=OLD.boundary_revision OR NEW.externalization_disabled!=OLD.externalization_disabled OR NEW.source_db_version!=OLD.source_db_version OR NEW.authoritative_now_utc_ms!=OLD.authoritative_now_utc_ms OR NEW.schema_digest IS NOT OLD.schema_digest OR NEW.failure_preimage_root IS NOT OLD.failure_preimage_root OR NEW.failure_receipt_digest IS NOT OLD.failure_receipt_digest) BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_INVALID_TRANSITION'); END;
CREATE TRIGGER core_boundary_schema_receipt_no_update_v1 BEFORE UPDATE ON schema_migrations WHEN OLD.version=9 OR NEW.version=9 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_schema_receipt_no_delete_v1 BEFORE DELETE ON schema_migrations WHEN OLD.version=9 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_failure_receipt_no_update_v1 BEFORE UPDATE ON core_boundary_failure_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_failure_receipt_no_delete_v1 BEFORE DELETE ON core_boundary_failure_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_disposition_pending_insert_v1 BEFORE INSERT ON core_boundary_disposition_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='pending')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_PENDING'); END;
CREATE TRIGGER core_boundary_disposition_no_update_v1 BEFORE UPDATE ON core_boundary_disposition_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_disposition_no_delete_v1 BEFORE DELETE ON core_boundary_disposition_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_receipt_pending_insert_v1 BEFORE INSERT ON core_boundary_persona_receipt_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='pending')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_PENDING'); END;
CREATE TRIGGER core_boundary_receipt_no_update_v1 BEFORE UPDATE ON core_boundary_persona_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER core_boundary_receipt_no_delete_v1 BEFORE DELETE ON core_boundary_persona_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_IMMUTABLE'); END;
CREATE TRIGGER embodiment_inventory_insert_v1 AFTER INSERT ON active_bindings BEGIN UPDATE embodiment_inventory_head_v1 SET epoch=epoch+1,entry_count=entry_count+1 WHERE singleton=1; END;
CREATE TRIGGER embodiment_inventory_update_v1 AFTER UPDATE ON active_bindings BEGIN UPDATE embodiment_inventory_head_v1 SET epoch=epoch+1 WHERE singleton=1; END;
CREATE TRIGGER embodiment_inventory_delete_v1 AFTER DELETE ON active_bindings BEGIN UPDATE embodiment_inventory_head_v1 SET epoch=epoch+1,entry_count=entry_count-1 WHERE singleton=1; END;
CREATE TRIGGER embodiment_inventory_head_no_delete_v1 BEFORE DELETE ON embodiment_inventory_head_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_INVENTORY_IMMUTABLE'); END;
CREATE TRIGGER core_inbound_receipt_applied_insert_v1 BEFORE INSERT ON core_inbound_receipt_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_APPLIED'); END;
CREATE TRIGGER core_inbound_receipt_no_update_v1 BEFORE UPDATE ON core_inbound_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_RECEIPT_IMMUTABLE'); END;
CREATE TRIGGER core_inbound_receipt_no_delete_v1 BEFORE DELETE ON core_inbound_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_RECEIPT_IMMUTABLE'); END;
CREATE TRIGGER core_delivery_receipt_applied_insert_v1 BEFORE INSERT ON core_delivery_receipt_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_APPLIED'); END;
CREATE TRIGGER core_delivery_receipt_no_update_v1 BEFORE UPDATE ON core_delivery_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_RECEIPT_IMMUTABLE'); END;
CREATE TRIGGER core_delivery_receipt_no_delete_v1 BEFORE DELETE ON core_delivery_receipt_v1 BEGIN SELECT RAISE(ABORT,'CORE_RECEIPT_IMMUTABLE'); END;
CREATE TRIGGER embodiment_persona_anchor_applied_insert_v1 BEFORE INSERT ON embodiment_persona_anchor_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_APPLIED'); END;
CREATE TRIGGER embodiment_persona_anchor_no_update_v1 BEFORE UPDATE ON embodiment_persona_anchor_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_ANCHOR_IMMUTABLE'); END;
CREATE TRIGGER embodiment_persona_anchor_no_delete_v1 BEFORE DELETE ON embodiment_persona_anchor_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_ANCHOR_IMMUTABLE'); END;
CREATE TRIGGER embodiment_profile_applied_insert_v1 BEFORE INSERT ON embodiment_profile_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_APPLIED'); END;
CREATE TRIGGER embodiment_profile_update_guard_v1 BEFORE UPDATE ON embodiment_profile_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 OR NEW.persona_scope IS NOT OLD.persona_scope OR NEW.profile_schema_version!=OLD.profile_schema_version OR NEW.revision!=OLD.revision+1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_PROFILE_CAS_REQUIRED'); END;
CREATE TRIGGER embodiment_profile_no_delete_v1 BEFORE DELETE ON embodiment_profile_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_PROFILE_IMMUTABLE'); END;
CREATE TRIGGER embodiment_sleep_schedule_applied_insert_v1 BEFORE INSERT ON embodiment_sleep_schedule_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_APPLIED'); END;
CREATE TRIGGER embodiment_sleep_schedule_update_guard_v1 BEFORE UPDATE ON embodiment_sleep_schedule_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 OR NEW.persona_scope IS NOT OLD.persona_scope OR NEW.schedule_schema_version!=OLD.schedule_schema_version OR NEW.revision!=OLD.revision+1 OR NEW.profile_revision!=(SELECT revision FROM embodiment_profile_v1 WHERE persona_scope=OLD.persona_scope) BEGIN SELECT RAISE(ABORT,'EMBODIMENT_SCHEDULE_CAS_REQUIRED'); END;
CREATE TRIGGER embodiment_sleep_schedule_no_delete_v1 BEFORE DELETE ON embodiment_sleep_schedule_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_SCHEDULE_IMMUTABLE'); END;
CREATE TRIGGER embodiment_clock_head_applied_insert_v1 BEFORE INSERT ON embodiment_clock_head_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 BEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_NOT_APPLIED'); END;
CREATE TRIGGER embodiment_clock_head_update_guard_v1 BEFORE UPDATE ON embodiment_clock_head_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 OR NEW.persona_scope IS NOT OLD.persona_scope OR NEW.sequence!=OLD.sequence+1 OR NEW.state_revision!=OLD.state_revision+1 OR NEW.profile_revision!=(SELECT revision FROM embodiment_profile_v1 WHERE persona_scope=OLD.persona_scope) OR NEW.schedule_revision!=(SELECT revision FROM embodiment_sleep_schedule_v1 WHERE persona_scope=OLD.persona_scope) OR NOT EXISTS(SELECT 1 FROM embodiment_clock_receipt_v1 AS r WHERE r.persona_scope=NEW.persona_scope AND r.sequence=NEW.sequence AND ((r.mutation_kind IN (1,2) AND NEW.time_revision=OLD.time_revision+1 AND NEW.last_now_utc_ms>OLD.last_now_utc_ms) OR (r.mutation_kind=3 AND NEW.time_revision=OLD.time_revision AND NEW.last_now_utc_ms=OLD.last_now_utc_ms AND NEW.next_due_at_utc_ms=OLD.next_due_at_utc_ms AND NEW.profile_revision=OLD.profile_revision AND NEW.schedule_revision=OLD.schedule_revision AND NEW.sleep_state=OLD.sleep_state AND NEW.sleep_episode_ordinal=OLD.sleep_episode_ordinal AND NEW.dream_consolidated_episode_ordinal=OLD.dream_consolidated_episode_ordinal AND NEW.endogenous_phase_code=OLD.endogenous_phase_code AND NEW.matrix_anchor_semantic_revision>OLD.matrix_anchor_semantic_revision AND r.raw_elapsed_ms=0 AND r.applied_elapsed_ms=0 AND r.discrete_event_mask=0))) BEGIN SELECT RAISE(ABORT,'EMBODIMENT_CLOCK_CAS_REQUIRED'); END;
CREATE TRIGGER embodiment_clock_head_no_delete_v1 BEFORE DELETE ON embodiment_clock_head_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_CLOCK_IMMUTABLE'); END;
CREATE TRIGGER embodiment_clock_receipt_insert_guard_v1 BEFORE INSERT ON embodiment_clock_receipt_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 OR NEW.sequence!=(SELECT sequence+1 FROM embodiment_clock_head_v1 WHERE persona_scope=NEW.persona_scope) BEGIN SELECT RAISE(ABORT,'EMBODIMENT_CLOCK_RECEIPT_INVALID'); END;
CREATE TRIGGER embodiment_clock_receipt_no_update_v1 BEFORE UPDATE ON embodiment_clock_receipt_v1 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_CLOCK_RECEIPT_IMMUTABLE'); END;
CREATE TRIGGER embodiment_clock_receipt_fold_only_delete_v1 BEFORE DELETE ON embodiment_clock_receipt_v1 WHEN OLD.sequence!=(SELECT compacted_count+1 FROM embodiment_clock_head_v1 WHERE persona_scope=OLD.persona_scope) OR (SELECT recent_count FROM embodiment_clock_head_v1 WHERE persona_scope=OLD.persona_scope)!=64 BEGIN SELECT RAISE(ABORT,'EMBODIMENT_CLOCK_FOLD_REQUIRED'); END;
CREATE TRIGGER semantic_clock_proof_applied_insert_v1 BEFORE INSERT ON semantic_clock_anchor_proof_v1 WHEN (SELECT COUNT(*) FROM core_boundary_control_v1 WHERE singleton=1 AND state='applied')!=1 OR NOT EXISTS(SELECT 1 FROM semantic_commits AS s JOIN embodiment_persona_anchor_v1 AS a ON a.persona_scope=s.persona_scope WHERE s.persona_scope=NEW.persona_scope AND s.semantic_revision=NEW.semantic_revision AND s.semantic_revision>a.initial_semantic_revision AND s.commitment_digest=NEW.semantic_commitment_digest) BEGIN SELECT RAISE(ABORT,'SEMANTIC_CLOCK_PROOF_INVALID'); END;
CREATE TRIGGER semantic_clock_proof_no_update_v1 BEFORE UPDATE ON semantic_clock_anchor_proof_v1 BEGIN SELECT RAISE(ABORT,'SEMANTIC_CLOCK_PROOF_IMMUTABLE'); END;
CREATE TRIGGER semantic_clock_proof_no_delete_v1 BEFORE DELETE ON semantic_clock_anchor_proof_v1 BEGIN SELECT RAISE(ABORT,'SEMANTIC_CLOCK_PROOF_IMMUTABLE'); END;
@deny|01|persona_temporal_profile
@deny|02|relation_temporal_policy
@deny|03|autonomous_runtime_state
@deny|04|autonomy_snapshot
@deny|05|autonomy_scope_binding
@deny|06|durable_intention
@deny|07|wake_schedule
@deny|08|outbound_target
@deny|09|outbound_attempt
@deny|10|autonomy_claim
@deny|11|externalization_budget
@deny|12|externalization_budget_claim
@deny|13|autonomy_operational_authority
@deny|14|autonomy_operational_authority_head
@deny|15|endogenous_intent_state_v1
@deny|16|local_dream_residue_v1
@deny|17|wake_time_settlement_v1
@deny|18|lived_day_state
@deny|19|relation_consent
@deny|20|relation_consent_head
@deny|21|relation_contact_process
@deny|22|contact_intention_basis
@deny|23|ecosystem_capability_grant
@deny|24|ecosystem_proposal
@deny|25|dream_residue
@deny|26|relation_budget_policy
@deny|27|proactive_readiness
@deny|28|gate_decision_latest
END V9_SCHEMA_SOURCE_V1

## Complete object order

The explicit catalog manifest is the statement order of the rendered output:
13 tables, eight indexes, 41 fixed triggers and 84 generated retired-table
triggers, for exactly 146 v9-created objects. The renderer counts statements
and verifies each extracted object name is unique before SQLite executes any
statement. The expected names are the CREATE object names in source order,
followed for each @deny row by
core_boundary_retired_NN_TABLE_i_v1,
core_boundary_retired_NN_TABLE_u_v1 and
core_boundary_retired_NN_TABLE_d_v1.

IMMUTABLE_LEGACY_TABLE_MANIFEST_V1 is exactly the 28 @deny rows in that order.
Every row of every listed table is included in the raw failure preimage and is
write-denied. The classification manifest may assign a typed disposition or
failed_closed, but it may not filter a row out of the raw proof.

The manifest's key and owner rules are also closed. `pk(...)` means the listed
SQLite primary-key values, framed in that order; `persona(c)` uses the exact
32-byte value in column `c`; `relation(c)` follows only authenticated pre-v9
binding evidence and otherwise resolves to `global_unowned`; `via(t,c)` follows
that one authenticated legacy row and otherwise resolves to
`global_unowned`. No JSON parse may supply a missing key or owner.

| kind | table | record key | owner rule |
| ---: | --- | --- | --- |
| 1 | `persona_temporal_profile` | `pk(persona_scope)` | `persona(persona_scope)` |
| 2 | `relation_temporal_policy` | `pk(relation_scope)` | `relation(relation_scope)` |
| 3 | `autonomous_runtime_state` | `pk(persona_scope)` | `persona(persona_scope)` |
| 4 | `autonomy_snapshot` | `pk(persona_scope,journal_revision)` | `persona(persona_scope)` |
| 5 | `autonomy_scope_binding` | `pk(work_scope)` | `persona(persona_scope)` |
| 6 | `durable_intention` | `pk(intention_id)` | `persona(persona_scope)` |
| 7 | `wake_schedule` | `pk(persona_scope)` | `persona(persona_scope)` |
| 8 | `outbound_target` | `pk(binding_digest)` | `relation(relation_scope)` |
| 9 | `outbound_attempt` | `pk(outbound_id)` | `via(durable_intention,intention_id)` |
| 10 | `autonomy_claim` | `pk(claim_token)` | `via(claim_kind,record_id)` |
| 11 | `externalization_budget` | `pk(relation_scope,budget_day_start_utc_ms)` | `relation(relation_scope)` |
| 12 | `externalization_budget_claim` | `pk(claim_token)` | `relation(relation_scope)` |
| 13 | `autonomy_operational_authority` | `pk(sequence)` | `persona(persona_scope)` |
| 14 | `autonomy_operational_authority_head` | `pk(persona_scope)` | `persona(persona_scope)` |
| 15 | `endogenous_intent_state_v1` | `pk(persona_scope)` | `persona(persona_scope)` |
| 16 | `local_dream_residue_v1` | `pk(residue_id)` | `persona(persona_scope)` |
| 17 | `wake_time_settlement_v1` | `pk(claim_token)` | `persona(persona_scope)` |
| 18 | `lived_day_state` | `pk(persona_scope)` | `persona(persona_scope)` |
| 19 | `relation_consent` | `pk(relation_scope,consent_epoch,revision)` | `relation(relation_scope)` |
| 20 | `relation_consent_head` | `pk(relation_scope)` | `relation(relation_scope)` |
| 21 | `relation_contact_process` | `pk(relation_scope)` | `relation(relation_scope)` |
| 22 | `contact_intention_basis` | `pk(intention_id)` | `relation(relation_scope)` |
| 23 | `ecosystem_capability_grant` | `pk(plugin_instance_digest,revision)` | `global_unowned` |
| 24 | `ecosystem_proposal` | `pk(proposal_id)` | `persona(persona_scope)` |
| 25 | `dream_residue` | `pk(residue_id)` | `persona(persona_scope)` |
| 26 | `relation_budget_policy` | `pk(relation_scope)` | `relation(relation_scope)` |
| 27 | `proactive_readiness` | `pk(relation_scope)` | `relation(relation_scope)` |
| 28 | `gate_decision_latest` | `pk(relation_scope,gate_phase)` | `relation(relation_scope)` |

Kind 10's `via` resolver is a closed typed switch: both historical wake writers
store a 16-byte event ID in record_id, not a persona digest. Their bounded typed
claim body is admitted only after the event ID, caller-bound token, lease,
frozen-time commitment and proposal persona/generation agree; only then may its
event scope resolve the persona. This authenticated claim exception does not
permit an arbitrary JSON field to invent a missing owner. Externalization/dispatch claims resolve through their exact
intention/outbound foreign identity, and an unknown/missing/ambiguous link is
`global_unowned` plus a classification failure. The disposition function is
the state matrix in `core-boundary-v9.md`; any value outside that matrix is a
typed Closed failure, never an omitted row.

The surviving pre-v9 writer allowlist is the exact existing base plus the
current Task 10B semantic schema verified before v9. It contains only
`schema_migrations`, `meta`, `genesis_manifests`, `genesis_leases`,
`incarnations`, `graph_commits`, `context_commits`,
`legacy_semantic_formula_upgrades`, `field_migration_preimage_backups`,
`semantic_migration_authority_v1`, `journal`,
`applied_events`, `snapshots`,
`active_bindings`, `inner_event`, `inner_event_manifest`, `interaction_fact`,
`world_anchor`, and objects named by Task 10B's frozen semantic catalog
verifier. Anything outside that authenticated combined catalog, the 28-table
immutable manifest, or this file's v9 object list rejects upgrade. Future core
tables do not silently enter the old-byte preimage.

Implementation erratum: the Genesis and graph/context tables above are
mandatory base objects created by `Store::migrate` in `crates/ae-store/src/lib.rs`.
Their authenticated indexes/triggers and SQLite's `sqlite_sequence` bookkeeping
for existing AUTOINCREMENT tables belong to the combined legacy catalog too.
This corrects the formerly incomplete enumeration; no v9 source/output SQL byte
or golden changes, and no new mutable table is authorized.

The implementation also enumerates each retained Store helper in the private
RetiredOperationTagV1 guard (including temporal/state loaders, ecosystem and
lived-world adapters reached by the retired dispatchers). These are private
suboperations of the existing retired surface, not new public method names.
The standalone semantic settlement and Store-private appraisal claim helper
remain outside that guard. The guard verifies only the catalog and terminal
control/version identity before rejection; it never scans target/ciphertext.

Pre-v9 historical migration receipts permit completed_at_ms=0, as used by
existing v2/v5 fixtures; only the newly introduced v9 receipt requires a
strictly positive timestamp. Existing migration digests remain unchanged.

The existing migration-test-hooks feature exposes a test-only pre-v9
constructor. The historical autonomy_migration integration suite must run with
--features migration-test-hooks; the constructor calls the legacy migration
directly, refuses any v9 database, and never removes a permanent fence.

## Installation and verification

Card 11A renders in memory, checks both SHA-256 goldens and the 146-object
count, then executes the one output in its outer transaction. It initializes
embodiment_inventory_head_v1 exactly once from the bounded active_bindings
count. The v9 schema receipt and raw failure receipt are inserted only after all
objects exist.

Exact-v9 reopen derives the same source/output bytes from compiled constants,
checks both goldens, and compares all 146 catalog rows by name/type/table/exact
SQL before any mutation path. There is no IF NOT EXISTS, per-feature migration,
request-path DDL or later schema extension in Tasks 11/12.
