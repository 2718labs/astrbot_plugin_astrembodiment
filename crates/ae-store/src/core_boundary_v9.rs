//! Permanent core-boundary schema. Installation is owned exclusively by Store.
//! The source is frozen by AE-T11T12-CORE-BOUNDARY-R2; never perform request-path DDL.

use crate::StoreError;
use ae_contracts::{wire::domain_hash, Digest};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

pub(crate) const VERSION: u32 = 9;
pub(crate) const MIGRATION_DIGEST: &[u8] = b"ae.autonomy.db.v9.core-boundary-overlay.v1";
const SOURCE_SHA256: &str = "7b4a5bf22b8c3695f00a80bc126dd559b9abb71e1aa70688bd7ce6ca7b5d0c49";
const SQL_SHA256: &str = "fb195037e7a15e267d761949c20ee2272fff6a5469a194d3d197c7cf5335627c";
const OBJECT_COUNT: usize = 146;
const MAX_CATALOG_OBJECTS: usize = 4096;
const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
const MAX_RAW_ROWS: usize = 65_536;
const MAX_RAW_BYTES: usize = 64 * 1024 * 1024;
const SCHEMA_SOURCE: &str = r#"CREATE TABLE core_boundary_control_v1(
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
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SchemaObject {
    pub kind: String,
    pub name: String,
    pub table: String,
    pub sql: String,
}

pub(crate) struct Schema {
    pub sql: String,
    pub objects: Vec<SchemaObject>,
    pub retired_tables: Vec<String>,
}

fn invalid(code: &'static str) -> StoreError {
    StoreError::ContinuityFence(code)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Read the global catalog before trusting a discovered name. Lengths are
/// checked in SQLite before allocating any catalog text in Rust.
pub(crate) fn bounded_catalog(conn: &Connection) -> Result<Vec<SchemaObject>, StoreError> {
    let mut statement = conn.prepare(
        "SELECT type,
          CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=96 THEN name END,
          CASE WHEN typeof(tbl_name)='text' AND length(CAST(tbl_name AS BLOB))<=96 THEN tbl_name END,
          CASE WHEN sql IS NULL THEN '' WHEN typeof(sql)='text' AND length(CAST(sql AS BLOB))<=32768 THEN sql END
         FROM sqlite_schema LIMIT 4097"
    )?;
    let mut rows = statement.query([])?;
    let mut objects = Vec::new();
    let mut names = BTreeSet::new();
    let mut total = 0usize;
    while let Some(row) = rows.next()? {
        if objects.len() == MAX_CATALOG_OBJECTS {
            return Err(invalid("V9_CATALOG_LIMIT"));
        }
        let kind: String = row.get(0)?;
        if !matches!(kind.as_str(), "table" | "index" | "trigger" | "view") {
            return Err(invalid("V9_CATALOG_TYPE"));
        }
        let name = row
            .get::<_, Option<String>>(1)?
            .ok_or_else(|| invalid("V9_CATALOG_NAME"))?;
        let table = row
            .get::<_, Option<String>>(2)?
            .ok_or_else(|| invalid("V9_CATALOG_TABLE"))?;
        let sql = row
            .get::<_, Option<String>>(3)?
            .ok_or_else(|| invalid("V9_CATALOG_SQL"))?;
        total = total
            .checked_add(name.len())
            .and_then(|n| n.checked_add(table.len()))
            .and_then(|n| n.checked_add(sql.len()))
            .ok_or_else(|| invalid("V9_CATALOG_LIMIT"))?;
        if total > MAX_CATALOG_BYTES || !names.insert(name.clone()) {
            return Err(invalid("V9_CATALOG_LIMIT"));
        }
        objects.push(SchemaObject {
            kind,
            name,
            table,
            sql,
        });
    }
    Ok(objects)
}

pub(crate) fn render_schema() -> Result<Schema, StoreError> {
    if sha256_hex(SCHEMA_SOURCE.as_bytes()) != SOURCE_SHA256
        || SCHEMA_SOURCE.contains('\r')
        || !SCHEMA_SOURCE.ends_with('\n')
    {
        return Err(invalid("V9_SCHEMA_SOURCE_DIGEST"));
    }
    let mut sql = String::new();
    let mut retired_tables = Vec::new();
    let mut ordinals = BTreeSet::new();
    for line in SCHEMA_SOURCE.lines() {
        if line.is_empty() {
            return Err(invalid("V9_SCHEMA_SOURCE_LINE"));
        }
        if line.starts_with('@') {
            let parts: Vec<_> = line.split('|').collect();
            if parts.len() != 3
                || parts[0] != "@deny"
                || parts[1].len() != 2
                || !parts[1].bytes().all(|b| b.is_ascii_digit())
                || parts[2].is_empty()
                || parts[2].len() > 63
                || !parts[2].as_bytes()[0].is_ascii_lowercase()
                || !parts[2]
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                || !ordinals.insert(parts[1])
                || retired_tables.iter().any(|name| name == parts[2])
            {
                return Err(invalid("V9_SCHEMA_SOURCE_MACRO"));
            }
            let ordinal = parts[1];
            let table = parts[2];
            retired_tables.push(table.to_owned());
            for (operation, suffix) in [("INSERT", "i"), ("UPDATE", "u"), ("DELETE", "d")] {
                sql.push_str(&format!(
                    "CREATE TRIGGER core_boundary_retired_{ordinal}_{table}_{suffix}_v1\nBEFORE {operation} ON \"{table}\"\nBEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_LEGACY_WRITE_DENIED'); END;\n"
                ));
            }
        } else {
            sql.push_str(line);
            sql.push('\n');
        }
    }
    if sha256_hex(sql.as_bytes()) != SQL_SHA256 || retired_tables.len() != 28 {
        return Err(invalid("V9_SCHEMA_SQL_DIGEST"));
    }
    // Derive catalog identity with SQLite itself. This is an isolated memory
    // connection, never the user's database; no SQL normalization guesses.
    let model = Connection::open_in_memory()?;
    model.execute_batch(
        "CREATE TABLE schema_migrations(version INTEGER);
        CREATE TABLE active_bindings(dummy INTEGER);",
    )?;
    for table in &retired_tables {
        model.execute_batch(&format!("CREATE TABLE \"{table}\"(dummy INTEGER);"))?;
    }
    let baseline: BTreeSet<_> = bounded_catalog(&model)?
        .into_iter()
        .map(|o| o.name)
        .collect();
    model.execute_batch(&sql)?;
    let objects: Vec<_> = bounded_catalog(&model)?
        .into_iter()
        .filter(|object| !baseline.contains(&object.name))
        .collect();
    if objects.len() != OBJECT_COUNT {
        return Err(invalid("V9_SCHEMA_OBJECT_COUNT"));
    }
    Ok(Schema {
        sql,
        objects,
        retired_tables,
    })
}

pub(crate) fn verify_schema_catalog(
    catalog: &[SchemaObject],
    expected: &Schema,
) -> Result<(), StoreError> {
    for object in &expected.objects {
        if catalog.iter().find(|actual| actual.name == object.name) != Some(object) {
            return Err(invalid("V9_SCHEMA_INCOMPLETE"));
        }
    }
    for object in catalog {
        if (object.name.starts_with("core_boundary_")
            || object.name.starts_with("core_inbound_")
            || object.name.starts_with("core_delivery_")
            || object.name.starts_with("embodiment_"))
            && !expected.objects.iter().any(|expected| expected == object)
        {
            return Err(invalid("V9_SCHEMA_UNEXPECTED_OBJECT"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawRow {
    pub kind: u16,
    pub key: Vec<u8>,
    pub digest: Digest,
    pub columns: Vec<String>,
    pub cells: Vec<(u8, Vec<u8>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RawProof {
    pub rows: Vec<RawRow>,
    pub payload_bytes: u64,
    pub catalog_digest: Digest,
    pub root: Digest,
}

const PRIMARY_KEYS: &[&[&str]] = &[
    &["persona_scope"],
    &["relation_scope"],
    &["persona_scope"],
    &["persona_scope", "journal_revision"],
    &["work_scope"],
    &["intention_id"],
    &["persona_scope"],
    &["binding_digest"],
    &["outbound_id"],
    &["claim_token"],
    &["relation_scope", "budget_day_start_utc_ms"],
    &["claim_token"],
    &["sequence"],
    &["persona_scope"],
    &["persona_scope"],
    &["residue_id"],
    &["claim_token"],
    &["persona_scope"],
    &["relation_scope", "consent_epoch", "revision"],
    &["relation_scope"],
    &["relation_scope"],
    &["intention_id"],
    &["plugin_instance_digest", "revision"],
    &["proposal_id"],
    &["residue_id"],
    &["relation_scope"],
    &["relation_scope"],
    &["relation_scope", "gate_phase"],
];

fn hash(domain: &str, bytes: &[u8]) -> Digest {
    domain_hash(domain.as_bytes(), &[bytes])
}

fn lp(bytes: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    output.extend_from_slice(bytes);
}

fn cell_frame(ordinal: usize, class: u8, bytes: &[u8], output: &mut Vec<u8>) {
    output.extend_from_slice(&(ordinal as u16).to_le_bytes());
    output.push(class);
    lp(bytes, output);
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Classification-independent proof. No JSON decoder participates here, so
/// even malformed legacy authority can be sealed without changing its bytes.
pub(crate) fn scan_raw_legacy(conn: &Connection, schema: &Schema) -> Result<RawProof, StoreError> {
    let mut rows_out = Vec::new();
    let mut payload_bytes = 0usize;
    let mut catalog_bytes = Vec::new();
    let mut manifest_bytes = Vec::new();
    for (index, table) in schema.retired_tables.iter().enumerate() {
        let kind = (index + 1) as u16;
        manifest_bytes.extend_from_slice(&kind.to_le_bytes());
        lp(table.as_bytes(), &mut manifest_bytes);
        for key in PRIMARY_KEYS[index] {
            lp(key.as_bytes(), &mut manifest_bytes);
        }
        lp(table.as_bytes(), &mut catalog_bytes);
        let mut metadata = conn.prepare(&format!("SELECT cid,name,type,\"notnull\",pk,hidden FROM pragma_table_xinfo('{table}') ORDER BY cid LIMIT 257"))?;
        let mut column_rows = metadata.query([])?;
        let mut columns = Vec::new();
        let mut declared_keys = Vec::new();
        while let Some(row) = column_rows.next()? {
            if columns.len() == 256 {
                return Err(invalid("V9_LEGACY_COLUMN_LIMIT"));
            }
            let cid: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let sql_type: String = row.get(2)?;
            let not_null: i64 = row.get(3)?;
            let pk: i64 = row.get(4)?;
            let hidden: i64 = row.get(5)?;
            if cid != columns.len() as i64
                || !identifier(&name)
                || sql_type.len() > 96
                || !matches!(not_null, 0 | 1)
                || hidden != 0
                || pk < 0
                || pk > 256
            {
                return Err(invalid("V9_LEGACY_COLUMN_IDENTITY"));
            }
            catalog_bytes.extend_from_slice(&(cid as u16).to_le_bytes());
            lp(name.as_bytes(), &mut catalog_bytes);
            lp(sql_type.as_bytes(), &mut catalog_bytes);
            catalog_bytes.push(not_null as u8);
            catalog_bytes.extend_from_slice(&(pk as u16).to_le_bytes());
            if pk > 0 {
                declared_keys.push((pk, name.clone()));
            }
            columns.push(name);
        }
        if columns.is_empty() {
            return Err(invalid("V9_LEGACY_TABLE_MISSING"));
        }
        declared_keys.sort();
        if declared_keys
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            != PRIMARY_KEYS[index]
        {
            return Err(invalid("V9_LEGACY_PRIMARY_KEY"));
        }
        let key_ordinals: Vec<_> = PRIMARY_KEYS[index]
            .iter()
            .map(|name| columns.iter().position(|column| column == name).unwrap())
            .collect();
        // SQLite checks every variable-width cell before returning the row;
        // never allocate an oversized text/blob in the host process.
        let predicates: Vec<_> = columns.iter().map(|name|
            format!("(typeof(\"{name}\") IN ('text','blob') AND length(CAST(\"{name}\" AS BLOB))>{MAX_RAW_BYTES})")
        ).collect();
        let oversize: bool = conn.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM \"{table}\" WHERE {} LIMIT 1)",
                predicates.join(" OR ")
            ),
            [],
            |row| row.get(0),
        )?;
        if oversize {
            return Err(invalid("V9_RAW_PAYLOAD_LIMIT"));
        }
        let projection = columns
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(",");
        let mut statement =
            conn.prepare(&format!("SELECT {projection} FROM \"{table}\" LIMIT 65537"))?;
        let mut source_rows = statement.query([])?;
        while let Some(row) = source_rows.next()? {
            if rows_out.len() == MAX_RAW_ROWS {
                return Err(invalid("V9_RAW_ROW_LIMIT"));
            }
            let mut framed = Vec::new();
            let mut cells = Vec::new();
            for ordinal in 0..columns.len() {
                let value = row.get_ref(ordinal)?;
                let fixed;
                let (class, bytes): (u8, &[u8]) = match value {
                    ValueRef::Null => (0, &[]),
                    ValueRef::Integer(value) => {
                        fixed = value.to_le_bytes();
                        (1, &fixed)
                    }
                    ValueRef::Real(value) => {
                        fixed = value.to_bits().to_le_bytes();
                        (2, &fixed)
                    }
                    ValueRef::Text(value) => (3, value),
                    ValueRef::Blob(value) => (4, value),
                };
                payload_bytes = payload_bytes
                    .checked_add(11)
                    .and_then(|n| n.checked_add(bytes.len()))
                    .ok_or_else(|| invalid("V9_RAW_PAYLOAD_LIMIT"))?;
                if payload_bytes > MAX_RAW_BYTES {
                    return Err(invalid("V9_RAW_PAYLOAD_LIMIT"));
                }
                cell_frame(ordinal, class, bytes, &mut framed);
                cells.push((class, bytes.to_vec()));
            }
            let mut key = vec![1];
            key.extend_from_slice(&kind.to_le_bytes());
            key.extend_from_slice(&(key_ordinals.len() as u16).to_le_bytes());
            for &ordinal in &key_ordinals {
                cell_frame(ordinal, cells[ordinal].0, &cells[ordinal].1, &mut key);
            }
            let mut bytes = Vec::new();
            lp(&key, &mut bytes);
            lp(&framed, &mut bytes);
            let digest = hash(&format!("ae.core-boundary.raw-row.v1/{kind}"), &bytes);
            rows_out.push(RawRow {
                kind,
                key,
                digest,
                columns: columns.clone(),
                cells,
            });
        }
    }
    rows_out.sort_by(|left, right| {
        (left.kind.to_le_bytes(), &left.key, left.digest).cmp(&(
            right.kind.to_le_bytes(),
            &right.key,
            right.digest,
        ))
    });
    let catalog_digest = hash("ae.core-boundary.legacy-catalog.v1", &catalog_bytes);
    let manifest_digest = hash(
        "ae.core-boundary.immutable-legacy-manifest.v1",
        &manifest_bytes,
    );
    let mut seed = manifest_digest.to_vec();
    seed.extend_from_slice(&catalog_digest);
    let mut root = hash("ae.core-boundary.failure-preimage-root.v1", &seed);
    for (index, row) in rows_out.iter().enumerate() {
        let mut leaf = root.to_vec();
        leaf.extend_from_slice(&(index as u64).to_le_bytes());
        leaf.extend_from_slice(&row.kind.to_le_bytes());
        lp(&row.key, &mut leaf);
        leaf.extend_from_slice(&row.digest);
        root = hash("ae.core-boundary.failure-preimage-step.v1", &leaf);
    }
    Ok(RawProof {
        rows: rows_out,
        payload_bytes: payload_bytes as u64,
        catalog_digest,
        root,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpenRoute {
    Fresh,
    Legacy(u32),
    V9,
}

/// This is the first SQLite catalog statement on the production open path.
/// It deliberately does not change PRAGMAs, open a transaction or read time.
pub(crate) fn preflight(conn: &Connection) -> Result<OpenRoute, StoreError> {
    let catalog = bounded_catalog(conn)?;
    if catalog.is_empty() {
        return Ok(OpenRoute::Fresh);
    }
    let has_v9_object = catalog.iter().any(|object| {
        object.name.starts_with("core_boundary_")
            || object.name.starts_with("core_inbound_")
            || object.name.starts_with("core_delivery_")
            || object.name.starts_with("embodiment_")
    });
    let migration = catalog
        .iter()
        .find(|object| object.name == "schema_migrations");
    if migration.is_none() {
        return if has_v9_object {
            Err(invalid("V9_SCHEMA_INCOMPLETE"))
        } else {
            Ok(OpenRoute::Legacy(0))
        };
    }
    if migration.unwrap().kind != "table" {
        return Err(invalid("V9_MIGRATION_CATALOG"));
    }
    let mut statement = conn.prepare(
        "SELECT version,
         CASE WHEN typeof(digest)='blob' AND length(digest)<=128 THEN digest END,
         completed_at_ms FROM schema_migrations ORDER BY version LIMIT 11",
    )?;
    let mut rows = statement.query([])?;
    let mut newest = 0u32;
    let mut seen = BTreeSet::new();
    let mut has_v9_receipt = false;
    while let Some(row) = rows.next()? {
        let version: i64 = row.get(0)?;
        let digest: Option<Vec<u8>> = row.get(1)?;
        let time: i64 = row.get(2)?;
        if version <= 0
            || version > i64::from(VERSION)
            || !seen.insert(version)
            || digest.is_none()
            || time < 0
            || (version == i64::from(VERSION) && time == 0)
        {
            return Err(invalid("V9_MIGRATION_IDENTITY"));
        }
        newest = version as u32;
        if newest == VERSION {
            if digest.as_deref() != Some(MIGRATION_DIGEST) {
                return Err(invalid("V9_MIGRATION_IDENTITY"));
            }
            has_v9_receipt = true;
        }
    }
    if has_v9_receipt != has_v9_object {
        return Err(invalid("V9_SCHEMA_INCOMPLETE"));
    }
    if has_v9_receipt {
        Ok(OpenRoute::V9)
    } else {
        Ok(OpenRoute::Legacy(newest))
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CoreBoundaryClosedCode {
    LegacyAuthority,
    UnsupportedLegacyState,
}

impl CoreBoundaryClosedCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::LegacyAuthority => "CORE_BOUNDARY_LEGACY_AUTHORITY",
            Self::UnsupportedLegacyState => "CORE_BOUNDARY_UNSUPPORTED_LEGACY_STATE",
        }
    }
}

#[derive(Debug)]
pub(crate) enum V9UpgradeError {
    Closed(CoreBoundaryClosedCode),
    Fatal(StoreError),
}

impl From<StoreError> for V9UpgradeError {
    fn from(error: StoreError) -> Self {
        Self::Fatal(error)
    }
}

fn failure_receipt(proof: &RawProof, now: u64) -> (Vec<u8>, Digest) {
    let mut bytes = 1u16.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(proof.rows.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&proof.payload_bytes.to_le_bytes());
    bytes.extend_from_slice(&proof.catalog_digest);
    bytes.extend_from_slice(&proof.root);
    bytes.extend_from_slice(&now.to_le_bytes());
    let digest = hash("ae.core-boundary.failure-receipt.v1", &bytes);
    (bytes, digest)
}

fn schema_digest(schema: &Schema) -> Digest {
    hash("ae.autonomy.db.v9.schema-sql.v1", schema.sql.as_bytes())
}

/// Install only inside the caller's outer IMMEDIATE transaction. A returned
/// error must roll back that transaction; this function never commits.
pub(crate) fn install_pending(
    conn: &Connection,
    schema: &Schema,
    now: u64,
) -> Result<RawProof, StoreError> {
    if conn.is_autocommit() || now == 0 || now > i64::MAX as u64 {
        return Err(invalid("V9_INSTALL_TRANSACTION_REQUIRED"));
    }
    conn.execute_batch(&schema.sql)?;
    let proof = scan_raw_legacy(conn, schema)?;
    let (receipt, digest) = failure_receipt(&proof, now);
    conn.execute(
        "INSERT INTO schema_migrations(version,digest,completed_at_ms) VALUES(9,?1,?2)",
        rusqlite::params![MIGRATION_DIGEST, now as i64],
    )?;
    conn.execute(
        "INSERT INTO core_boundary_failure_receipt_v1 VALUES(1,8,?1,?2,?3,?4,?5,?6,?7)",
        rusqlite::params![proof.rows.len() as i64, proof.payload_bytes as i64,
            proof.catalog_digest.as_slice(), proof.root.as_slice(), now as i64,
            receipt, digest.as_slice()],
    )?;
    let pending = hash("ae.core-boundary.pending.v1", &[]);
    conn.execute(
        "INSERT INTO core_boundary_control_v1 VALUES(1,1,'pending',1,8,?1,?2,?3,?4,?5,?5,?5,NULL)",
        rusqlite::params![now as i64, schema_digest(schema).as_slice(), proof.root.as_slice(),
            digest.as_slice(), pending.as_slice()],
    )?;
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM (SELECT 1 FROM active_bindings LIMIT 65537)",
        [], |row| row.get(0),
    )?;
    if count > 65_536 {
        return Err(invalid("V9_LEGACY_OWNER_LIMIT"));
    }
    conn.execute(
        "INSERT INTO embodiment_inventory_head_v1(singleton,epoch,entry_count) VALUES(1,1,?1)",
        rusqlite::params![count],
    )?;
    Ok(proof)
}

/// Authenticating the raw failure proof is independent of semantic
/// classification and therefore also works for a terminal failed_closed DB.
pub(crate) fn verify_failure_receipt(
    conn: &Connection,
    schema: &Schema,
    now: u64,
) -> Result<RawProof, StoreError> {
    let proof = scan_raw_legacy(conn, schema)?;
    let (bytes, digest) = failure_receipt(&proof, now);
    let valid: bool = conn.query_row(
        "SELECT COUNT(*)=1 FROM core_boundary_failure_receipt_v1
         WHERE singleton=1 AND source_db_version=8 AND source_row_count=?1
         AND source_payload_bytes=?2 AND legacy_catalog_digest=?3 AND raw_preimage_root=?4
         AND authoritative_now_utc_ms=?5 AND receipt_bytes=?6 AND receipt_digest=?7",
        rusqlite::params![proof.rows.len() as i64, proof.payload_bytes as i64,
            proof.catalog_digest.as_slice(), proof.root.as_slice(), now as i64,
            bytes, digest.as_slice()], |row| row.get(0),
    )?;
    if !valid {
        return Err(invalid("V9_FAILURE_PREIMAGE_MISMATCH"));
    }
    Ok(proof)
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Owner(u8, Digest);

fn global_owner() -> Owner {
    Owner(2, hash("ae.core-boundary.owner.global-unowned.v1", &[]))
}

impl Owner {
    fn name(self) -> &'static str {
        if self.0 == 1 { "persona" } else { "global_unowned" }
    }
}

impl RawRow {
    fn integer(&self, name: &str) -> Option<i64> {
        let cell = self.cell(name)?;
        (cell.0 == 1).then(|| cell.1.as_slice().try_into().ok().map(i64::from_le_bytes)).flatten()
    }
    fn cell(&self, name: &str) -> Option<&(u8, Vec<u8>)> {
        self.columns.iter().position(|column| column == name).map(|index| &self.cells[index])
    }
    fn blob(&self, name: &str) -> Option<&[u8]> {
        self.cell(name).filter(|cell| cell.0 == 4).map(|cell| cell.1.as_slice())
    }
    fn text(&self, name: &str) -> Option<&str> {
        self.cell(name).filter(|cell| cell.0 == 3)
            .and_then(|cell| std::str::from_utf8(&cell.1).ok())
    }
    fn persona(&self) -> Option<Owner> {
        self.blob("persona_scope").and_then(|bytes| bytes.try_into().ok()).map(|key| Owner(1, key))
    }
}

struct Disposition<'a> {
    row: &'a RawRow,
    owner: Owner,
    name: &'static str,
    leaf: Digest,
    preimage_leaf: Digest,
}

#[derive(Clone)]
struct ClaimAuthority {
    owner: Owner,
    record: Vec<u8>,
    dispatch_started: bool,
    reservation: Option<(Digest, Option<u64>, u64)>,
}

fn closed_claim() -> V9UpgradeError {
    V9UpgradeError::Closed(CoreBoundaryClosedCode::LegacyAuthority)
}

fn decode_legacy<T: serde::de::DeserializeOwned>(row: &RawRow) -> Result<T, V9UpgradeError> {
    serde_json::from_str(row.text("body_json").ok_or_else(closed_claim)?).map_err(|_| closed_claim())
}

fn authenticated_intention(row: &RawRow) -> Result<ae_contracts::DurableIntentionV1, V9UpgradeError> {
    let value: ae_contracts::DurableIntentionV1 = decode_legacy(row)?;
    let state = serde_json::to_value(value.state).map_err(|_| closed_claim())?;
    if row.blob("intention_id") != Some(value.intention_id.as_slice())
        || row.blob("persona_scope") != Some(value.persona_scope.as_slice())
        || row.blob("relation_scope") != Some(value.relation_scope.as_slice())
        || row.blob("semantic_digest") != Some(value.semantic_idempotency_digest.as_slice())
        || row.text("state") != state.as_str()
        || row.integer("revision").is_none_or(|revision| revision < 0)
    { return Err(closed_claim()); }
    Ok(value)
}

fn authenticated_outbound(row: &RawRow) -> Result<ae_contracts::OutboundAttemptV1, V9UpgradeError> {
    let value: ae_contracts::OutboundAttemptV1 = decode_legacy(row)?;
    let state = serde_json::to_value(value.state).map_err(|_| closed_claim())?;
    if row.blob("outbound_id") != Some(value.outbound_id.as_slice())
        || row.blob("intention_id") != Some(value.intention_id.as_slice())
        || row.blob("target_digest") != Some(value.target.binding_digest.as_slice())
        || row.text("state") != state.as_str()
    { return Err(closed_claim()); }
    Ok(value)
}

/// One read-only authentication result supplies ownership, dispatch uncertainty
/// and reservation classification. No caller can infer any of these from an
/// unauthenticated raw record_id or from body JSON alone.
fn authenticate_claims(proof: &RawProof) -> Result<std::collections::BTreeMap<Digest, ClaimAuthority>, V9UpgradeError> {
    use ae_contracts::*;
    let mut output = std::collections::BTreeMap::new();
    for row in proof.rows.iter().filter(|row| row.kind == 10) {
        let token: Digest = row.blob("claim_token").and_then(|v| v.try_into().ok()).ok_or_else(closed_claim)?;
        let caller: Digest = row.blob("caller_incarnation").and_then(|v| v.try_into().ok()).ok_or_else(closed_claim)?;
        let record = row.blob("record_id").ok_or_else(closed_claim)?;
        let deadline = row.integer("lease_deadline_utc_ms").and_then(|v| u64::try_from(v).ok()).ok_or_else(closed_claim)?;
        let mut authority = ClaimAuthority { owner: global_owner(), record: record.to_vec(), dispatch_started: false, reservation: None };
        match row.text("claim_kind").ok_or_else(closed_claim)? {
            kind @ ("wake" | "wake_v2") => {
                let (claim_token,event,proposal,lease) = if kind == "wake" {
                    let claim: WakeClaimV1 = decode_legacy(row)?;
                    (claim.claim_token,claim.event,claim.proposal,claim.lease_deadline_utc_ms)
                } else {
                    let claim: WakeClaimV2 = decode_legacy(row)?;
                    let scope = wire::persona_scope_digest(&claim.event.scope.bot_token,&claim.event.scope.persona_token,None);
                    if claim.proposal.world_anchor.as_ref().is_some_and(|v|v.persona_scope!=scope)
                        || claim.proposal.lived_day.as_ref().is_some_and(|v|v.persona_scope!=scope)
                        || claim.proposal.dream_updates.iter().any(|v|v.persona_scope!=scope)
                    { return Err(closed_claim()); }
                    (claim.claim_token,claim.event,claim.proposal.legacy,claim.lease_deadline_utc_ms)
                };
                let scope = wire::persona_scope_digest(&event.scope.bot_token,&event.scope.persona_token,None);
                let domain: &[u8] = if kind=="wake" { b"wake" } else { b"wake-v2" };
                ae_autonomy::validate_frozen_time(&event.frozen).map_err(|_|closed_claim())?;
                let mut frozen_hash=Sha256::new();
                frozen_hash.update(b"ae.frozen-time-input.v1\0");
                frozen_hash.update(serde_json::to_vec(&event.frozen).map_err(|_|closed_claim())?);
                let expected_frozen: Digest=frozen_hash.finalize().into();
                if event.scope.relation_token.is_some() || record != event.event_id.as_slice()
                    || event.frozen_input_digest!=expected_frozen
                    || claim_token!=token || crate::autonomy::digest_claim(domain,&event.event_id,&caller)!=token
                    || lease!=deadline || deadline!=event.frozen.effective_now_utc_ms.saturating_add(120_000)
                    || proposal.state.persona_scope!=scope
                    || proposal.state.generation!=event.expected_generation.saturating_add(1)
                    || proposal.inner_events.iter().any(|v|v.persona_scope!=scope)
                    || proposal.intentions.iter().any(|v|v.persona_scope!=scope)
                { return Err(closed_claim()); }
                authority.owner=Owner(1,scope);
            }
            "externalization" | "externalization_v2" => {
                let intention_row=proof.rows.iter().find(|v|v.kind==6 && v.blob("intention_id")==Some(record)).ok_or_else(closed_claim)?;
                let intention=authenticated_intention(intention_row)?;
                authority.owner=Owner(1,intention.persona_scope);
                if let Ok(claim)=decode_legacy::<ExternalizationClaimV1>(row) {
                    if claim.claim_token!=token || claim.intention_id.as_slice()!=record
                        || claim.caller_incarnation!=caller || claim.lease_deadline_utc_ms!=deadline
                        || crate::autonomy::externalization_claim_token(&claim)!=token
                    { return Err(closed_claim()); }
                    authority.reservation=Some((intention.relation_scope,None,u64::from(claim.max_tokens)));
                } else {
                    let claim: ExternalizationClaimV2=decode_legacy(row)?;
                    let revision=intention_row.integer("revision").and_then(|v|u64::try_from(v).ok()).and_then(|v|v.checked_sub(1)).ok_or_else(closed_claim)?;
                    let expected=wire::domain_hash(b"ae.alpha3.externalization-claim.v2",&[
                        &intention.relation_scope,&intention.intention_id,&[claim.attempt_no],&claim.reserved_tokens.to_le_bytes(),
                        &revision.to_le_bytes(),&claim.policy_revision.to_le_bytes(),&claim.consent_revision.to_le_bytes(),
                        &claim.capability_snapshot_digest,&caller,&claim.budget_day_start_utc_ms.to_le_bytes()]);
                    if token!=claim.claim_token || token!=expected || deadline!=claim.lease_deadline_utc_ms
                        || (claim.intention_public_ref!=encode_intention_scoped_locator_v1(&intention.relation_scope,&intention.intention_id)
                            && !legacy_v2_intention_public_ref_matches(&intention.relation_scope,&intention.intention_id,&claim.intention_public_ref))
                    { return Err(closed_claim()); }
                    if let Some(gate)=&claim.gate_decision_snapshot { gate.validate().map_err(|_|closed_claim())?; }
                    authority.reservation=Some((intention.relation_scope,Some(claim.budget_day_start_utc_ms),claim.reserved_tokens));
                }
            }
            "dispatch" | "dispatch_v2" => {
                let outbound_row=proof.rows.iter().find(|v|v.kind==9 && v.blob("outbound_id")==Some(record)).ok_or_else(closed_claim)?;
                let outbound=authenticated_outbound(outbound_row)?;
                let intention=authenticated_intention(proof.rows.iter().find(|v|v.kind==6 && v.blob("intention_id")==Some(outbound.intention_id.as_slice())).ok_or_else(closed_claim)?)?;
                if wire::persona_scope_digest(&outbound.target.bot_token,&outbound.target.persona_token,None)!=intention.persona_scope
                    || wire::persona_scope_digest(&outbound.target.bot_token,&outbound.target.persona_token,Some(&outbound.target.relation_token))!=intention.relation_scope
                { return Err(closed_claim()); }
                authority.owner=Owner(1,intention.persona_scope);
                if let Ok(claim)=decode_legacy::<DispatchClaimV1>(row) {
                    if claim.claim_token!=token || claim.outbound_id.as_slice()!=record || claim.caller_incarnation!=caller
                        || claim.lease_deadline_utc_ms!=deadline || crate::autonomy::dispatch_claim_token(&claim)!=token
                        || claim.target!=outbound.target || claim.candidate_ciphertext!=outbound.candidate_ciphertext
                    { return Err(closed_claim()); }
                } else {
                    let claim: DispatchClaimV2=decode_legacy(row)?;
                    if claim.claim_token!=token || claim.lease_deadline_utc_ms!=deadline
                        || crate::alpha3::projection::dispatch_claim_token(&claim,&caller)!=token
                        || claim.target_binding_digest!=outbound.target.binding_digest
                        || (claim.outbound_public_ref!=encode_execution_scoped_locator_v1(&intention.relation_scope,&outbound.outbound_id)
                            && !legacy_v2_execution_public_ref_matches(&intention.relation_scope,&outbound.outbound_id,&claim.outbound_public_ref))
                    { return Err(closed_claim()); }
                    if let Some(gate)=&claim.gate_decision_snapshot { gate.validate().map_err(|_|closed_claim())?; }
                }
                // Claim acquisition is the historical durable adapter-start
                // boundary, even if a crash left the intention pending.
                authority.dispatch_started=true;
            }
            _ => return Err(closed_claim()),
        }
        if output.insert(token,authority).is_some() { return Err(closed_claim()); }
    }
    Ok(output)
}

fn reservation_proven(row: &RawRow, proof: &RawProof, claims: &std::collections::BTreeMap<Digest,ClaimAuthority>) -> bool {
    let Some(token)=row.blob("claim_token").and_then(|v| <Digest>::try_from(v).ok()) else { return false; };
    let Some((relation,claim_day,tokens))=claims.get(&token).and_then(|v|v.reservation) else { return false; };
    let Some(day)=row.integer("budget_day_start_utc_ms").and_then(|v|u64::try_from(v).ok()) else { return false; };
    if row.blob("relation_scope")!=Some(relation.as_slice()) || claim_day.is_some_and(|v|v!=day)
        || row.integer("reserved_tokens").and_then(|v|u64::try_from(v).ok())!=Some(tokens) { return false; }
    let Some(budget)=proof.rows.iter().find(|v|v.kind==11 && v.blob("relation_scope")==Some(relation.as_slice())
        && v.integer("budget_day_start_utc_ms")==Some(day as i64)) else { return false; };
    let (Some(reserved),Some(charged),Some(limit),Some(used),Some(known))=(budget.integer("reserved_tokens"),budget.integer("charged_tokens"),budget.integer("limit_tokens"),budget.integer("used_tokens"),budget.integer("usage_known")) else {return false;};
    if reserved<0 || charged<0 || used<0 || limit<0 || used>charged || !matches!(known,0|1)
        || charged.checked_add(reserved).is_none_or(|v|v>limit) { return false; }
    match row.integer("migrated_unknown_full_charge") {
        Some(1) => known==0 && u64::try_from(charged).is_ok_and(|v|v>=tokens),
        Some(0) => {
            let total=proof.rows.iter().filter(|v|v.kind==12 && v.blob("relation_scope")==Some(relation.as_slice()) && v.integer("budget_day_start_utc_ms")==Some(day as i64) && v.integer("migrated_unknown_full_charge")==Some(0))
                .try_fold(0u64,|sum,v|sum.checked_add(u64::try_from(v.integer("reserved_tokens")?).ok()?));
            total==u64::try_from(reserved).ok()
        }
        _=>false,
    }
}

fn disposition_code(row: &RawRow) -> Result<(u16, &'static str), V9UpgradeError> {
    let invalid_state = || V9UpgradeError::Closed(CoreBoundaryClosedCode::UnsupportedLegacyState);
    Ok(match row.kind {
        6 | 9 => match row.text("state").ok_or_else(invalid_state)? {
            "forming" | "ready" | "deferred" | "externalizing" => (1, "suppressed_core_boundary"),
            "dispatch_pending" => (2, "terminal_core_boundary"),
            "adapter_call_started" => (3, "dispatch_unknown_no_retry"),
            "adapter_submitted" | "platform_accepted" | "delivery_confirmed" | "dispatch_unknown" =>
                (4, "historical_fact_preserved_no_retry"),
            "suppressed" | "expired" | "terminal" => (5, "historical_terminal_preserved"),
            _ => return Err(invalid_state()),
        },
        10 => match row.text("claim_kind").ok_or_else(invalid_state)? {
            "wake" | "wake_v2" => (6, "retired_wake_claim"),
            "externalization" | "externalization_v2" => (7, "retired_externalization_claim"),
            "dispatch" | "dispatch_v2" => (8, "retired_dispatch_claim"),
            _ => return Err(invalid_state()),
        },
        12 => (9, "effective_charged_unknown_frozen"),
        8 => (11, "preserved_unreadable_by_core"),
        13 | 14 => (13, "historical_authority_preserved"),
        _ => (12, "preserved_ignored"),
    })
}

fn dispositions(proof: &RawProof) -> Result<Vec<Disposition<'_>>, V9UpgradeError> {
    let claims = authenticate_claims(proof)?;
    let mut relations = std::collections::BTreeMap::<Vec<u8>, Option<Owner>>::new();
    for row in proof.rows.iter().filter(|row| row.kind == 5) {
        if let (Some(relation), Some(persona)) = (row.blob("relation_scope"), row.persona()) {
            relations.entry(relation.to_vec()).and_modify(|old| {
                if *old != Some(persona) { *old = None; }
            }).or_insert(Some(persona));
        }
    }
    let resolve_relation = |row: &RawRow| row.blob("relation_scope")
        .and_then(|key| relations.get(key)).and_then(|owner| *owner);
    let mut output = Vec::with_capacity(proof.rows.len());
    for row in &proof.rows {
        if row.key.len() > 512 || row.cells.iter().any(|cell| cell.0 == 2) {
            return Err(V9UpgradeError::Closed(CoreBoundaryClosedCode::LegacyAuthority));
        }
        let owner = match row.kind {
            9 => {
                let outbound=authenticated_outbound(row)?;
                proof.rows.iter().find(|candidate|candidate.kind==6 && candidate.blob("intention_id")==Some(outbound.intention_id.as_slice()))
                    .map(authenticated_intention).transpose()?.map(|v|Owner(1,v.persona_scope))
            }
            10 => {
                row.blob("claim_token").and_then(|v| <Digest>::try_from(v).ok()).and_then(|token|claims.get(&token)).map(|v|v.owner)
            },
            6 => Some(Owner(1,authenticated_intention(row)?.persona_scope)),
            23 => None,
            _ => row.persona().or_else(|| resolve_relation(row)),
        }.unwrap_or_else(global_owner);
        if row.kind == 10 && owner.0 == 2 {
            return Err(V9UpgradeError::Closed(CoreBoundaryClosedCode::LegacyAuthority));
        }
        let (mut code, mut name) = disposition_code(row)?;
        if row.kind==12 {
            (code,name)=if reservation_proven(row,proof,&claims) {(9,"effective_charged_unknown_frozen")} else {(10,"frozen_unverified_no_refund")};
        }
        if matches!(row.kind,6|9) && row.text("state")==Some("dispatch_pending") {
            let started = proof.rows.iter().filter(|v|v.kind==9 && (row.kind==9 && v.key==row.key || row.kind==6 && v.blob("intention_id")==row.blob("intention_id")))
                .map(|outbound| {
                    let value=authenticated_outbound(outbound)?;
                    Ok(value.state==ae_contracts::IntentionStateV1::AdapterCallStarted || claims.values().any(|claim|claim.dispatch_started && claim.record==value.outbound_id))
                }).collect::<Result<Vec<bool>,V9UpgradeError>>()?.into_iter().any(|v|v);
            if started {(code,name)=(3,"dispatch_unknown_no_retry");}
        }
        let mut input = Vec::new();
        lp(&row.key, &mut input);
        input.extend_from_slice(&row.digest);
        let preimage_leaf = hash(&format!("ae.core-boundary.preimage-leaf.v1/{}", row.kind), &input);
        input.extend_from_slice(&code.to_le_bytes());
        input.push(owner.0);
        input.extend_from_slice(&owner.1);
        let leaf = hash(&format!("ae.core-boundary.disposition-leaf.v1/{}", row.kind), &input);
        output.push(Disposition { row, owner, name, leaf, preimage_leaf });
    }
    Ok(output)
}


#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnerReceipt {
    owner: Owner,
    journal_revision: u64,
    journal_anchor: Digest,
    operational_count: u64,
    operational_anchor: Digest,
    count: u64,
    preimage: Digest,
    disposition: Digest,
    bytes: Vec<u8>,
    digest: Digest,
}

fn prefix_anchors(
    conn: &Connection, owner: Owner, frozen: Option<&OwnerReceipt>,
) -> Result<(u64, Digest, u64, Digest), StoreError> {
    use rusqlite::OptionalExtension;
    if owner.0 == 2 {
        return Ok((0, hash("ae.core-boundary.global-unowned.journal-prefix-empty.v1", &[]),
            0, hash("ae.core-boundary.global-unowned.operational-prefix-empty.v1", &[])));
    }
    let journal: Option<(i64, Vec<u8>)> = if let Some(receipt) = frozen {
        if receipt.journal_revision == 0 {
            None
        } else {
            conn.query_row(
                "SELECT logical_revision,chain_digest FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
                rusqlite::params![owner.1.as_slice(), receipt.journal_revision as i64],
                |row| Ok((row.get(0)?, row.get(1)?)),
            ).optional()?
        }
    } else {
        conn.query_row(
            "SELECT logical_revision,chain_digest FROM journal WHERE scope_digest=?1 ORDER BY logical_revision DESC LIMIT 1",
            rusqlite::params![owner.1.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?
    };
    let (revision, chain) = match journal {
        Some((revision, chain)) if revision > 0 && chain.len() == 32 => (revision as u64, chain),
        None if frozen.is_none_or(|receipt| receipt.journal_revision == 0) => (0, vec![0; 32]),
        _ => return Err(invalid("V9_JOURNAL_PREFIX")),
    };
    let mut bytes = owner.1.to_vec();
    bytes.extend_from_slice(&revision.to_le_bytes());
    bytes.extend_from_slice(&chain);
    let journal_anchor = hash("ae.core-boundary.legacy-journal-prefix.v1", &bytes);
    let operational: Option<(i64, Vec<u8>)> = conn.query_row(
        "SELECT entry_count,head_digest FROM autonomy_operational_authority_head WHERE persona_scope=?1",
        rusqlite::params![owner.1.as_slice()], |row| Ok((row.get(0)?, row.get(1)?)),
    ).optional()?;
    let (count, chain) = match operational {
        Some((count, chain)) if count >= 0 && chain.len() == 32 => (count as u64, chain),
        None => (0, vec![0; 32]),
        _ => return Err(invalid("V9_OPERATIONAL_PREFIX")),
    };
    let mut bytes = owner.1.to_vec();
    bytes.extend_from_slice(&count.to_le_bytes());
    bytes.extend_from_slice(&chain);
    let operational_anchor = hash("ae.core-boundary.legacy-operational-prefix.v1", &bytes);
    Ok((revision, journal_anchor, count, operational_anchor))
}

fn owner_receipts(
    conn: &Connection, schema: &Schema, dispositions: &[Disposition<'_>], now: u64,
    frozen: Option<&[OwnerReceipt]>,
) -> Result<Vec<OwnerReceipt>, StoreError> {
    let mut owners = BTreeSet::from([global_owner()]);
    owners.extend(dispositions.iter().map(|entry| entry.owner));
    if let Some(frozen) = frozen {
        owners.extend(frozen.iter().map(|receipt| receipt.owner));
    } else {
        let mut statement = conn.prepare("SELECT bot_token,persona_token FROM active_bindings LIMIT 65537")?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let bot: [u8; 16] = row.get::<_, Vec<u8>>(0)?.try_into().map_err(|_| invalid("V9_BINDING_IDENTITY"))?;
            let persona: [u8; 16] = row.get::<_, Vec<u8>>(1)?.try_into().map_err(|_| invalid("V9_BINDING_IDENTITY"))?;
            owners.insert(Owner(1, ae_contracts::wire::persona_scope_digest(&bot, &persona, None)));
            if owners.len() > 65_536 {
                return Err(invalid("V9_OWNER_LIMIT"));
            }
        }
    }
    let schema_digest = schema_digest(schema);
    let mut grouped = std::collections::BTreeMap::<Owner, Vec<&Disposition<'_>>>::new();
    for entry in dispositions {
        grouped.entry(entry.owner).or_default().push(entry);
    }
    let mut output = Vec::new();
    for owner in owners {
        let previous = frozen.and_then(|receipts| receipts.iter().find(|receipt| receipt.owner == owner));
        if frozen.is_some() && previous.is_none() {
            return Err(invalid("V9_OWNER_SET_MISMATCH"));
        }
        let (journal_revision, journal_anchor, operational_count, operational_anchor) = prefix_anchors(conn, owner, previous)?;
        let mut seed = schema_digest.to_vec();
        seed.push(owner.0);
        seed.extend_from_slice(&owner.1);
        let mut preimage = hash("ae.core-boundary.owner-preimage-root.v1", &seed);
        let mut disposition = hash("ae.core-boundary.owner-disposition-root.v1", &seed);
        let entries = grouped.get(&owner).map(Vec::as_slice).unwrap_or(&[]);
        for (index, entry) in entries.iter().enumerate() {
            let mut bytes = preimage.to_vec();
            bytes.extend_from_slice(&(index as u64).to_le_bytes());
            lp(&entry.row.key, &mut bytes);
            bytes.extend_from_slice(&entry.preimage_leaf);
            preimage = hash("ae.core-boundary.owner-preimage-step.v1", &bytes);
            let mut bytes = disposition.to_vec();
            bytes.extend_from_slice(&(index as u64).to_le_bytes());
            lp(&entry.row.key, &mut bytes);
            bytes.extend_from_slice(&entry.leaf);
            disposition = hash("ae.core-boundary.owner-disposition-step.v1", &bytes);
        }
        let count = entries.len() as u64;
        let mut bytes = 1u16.to_le_bytes().to_vec();
        bytes.push(owner.0);
        bytes.extend_from_slice(&owner.1);
        bytes.extend_from_slice(&journal_revision.to_le_bytes());
        bytes.extend_from_slice(&journal_anchor);
        bytes.extend_from_slice(&operational_count.to_le_bytes());
        bytes.extend_from_slice(&operational_anchor);
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&preimage);
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&disposition);
        bytes.extend_from_slice(&now.to_le_bytes());
        let digest = hash("ae.core-boundary.owner-receipt.v1", &bytes);
        output.push(OwnerReceipt { owner, journal_revision, journal_anchor, operational_count,
            operational_anchor, count, preimage, disposition, bytes, digest });
    }
    Ok(output)
}

fn control_roots(schema: &Schema, receipts: &[OwnerReceipt]) -> [Digest; 3] {
    let digest = schema_digest(schema);
    let mut roots = [
        hash("ae.core-boundary.global-preimage-root.v1", &digest),
        hash("ae.core-boundary.global-disposition-root.v1", &digest),
        hash("ae.core-boundary.global-receipt-root.v1", &digest),
    ];
    for (index, receipt) in receipts.iter().enumerate() {
        for (lane, domain) in [
            "ae.core-boundary.global-preimage-step.v1",
            "ae.core-boundary.global-disposition-step.v1",
            "ae.core-boundary.global-receipt-step.v1",
        ].iter().enumerate() {
            let mut bytes = roots[lane].to_vec();
            bytes.extend_from_slice(&(index as u64).to_le_bytes());
            bytes.push(receipt.owner.0);
            bytes.extend_from_slice(&receipt.owner.1);
            if lane != 2 {
                bytes.extend_from_slice(&receipt.count.to_le_bytes());
            }
            bytes.extend_from_slice(&[receipt.preimage, receipt.disposition, receipt.digest][lane]);
            roots[lane] = hash(domain, &bytes);
        }
    }
    roots
}

fn persist_overlay(
    conn: &Connection, dispositions: &[Disposition<'_>], receipts: &[OwnerReceipt], now: u64,
) -> Result<(), StoreError> {
    for entry in dispositions {
        conn.execute(
            "INSERT INTO core_boundary_disposition_v1 VALUES(?1,?2,?3,?4,?5,?6,'core_boundary_upgrade',?7,?8)",
            rusqlite::params![entry.row.kind, entry.row.key, entry.owner.name(),
                entry.owner.1.as_slice(), entry.row.digest.as_slice(), entry.name, now as i64,
                entry.leaf.as_slice()],
        )?;
    }
    for receipt in receipts {
        conn.execute(
            "INSERT INTO core_boundary_persona_receipt_v1 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?7,?9,?10,?11,?12)",
            rusqlite::params![receipt.owner.name(), receipt.owner.1.as_slice(),
                receipt.journal_revision as i64, receipt.journal_anchor.as_slice(),
                receipt.operational_count as i64, receipt.operational_anchor.as_slice(),
                receipt.count as i64, receipt.preimage.as_slice(), receipt.disposition.as_slice(),
                now as i64, receipt.bytes, receipt.digest.as_slice()],
        )?;
    }
    Ok(())
}


fn read_owner_receipts(conn: &Connection) -> Result<Vec<OwnerReceipt>, StoreError> {
    let (count, bytes): (i64, i64) = conn.query_row(
        "SELECT COUNT(*),COALESCE(SUM(length(receipt_bytes)),0) FROM core_boundary_persona_receipt_v1",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if !(0..=65_537).contains(&count) || !(0..=67_108_864).contains(&bytes) {
        return Err(invalid("V9_OWNER_RECEIPT_LIMIT"));
    }
    let mut statement = conn.prepare(
        "SELECT owner_kind,owner_key,journal_prefix_revision,journal_prefix_anchor,
         legacy_operational_prefix_count,legacy_operational_prefix_anchor,source_row_count,
         preimage_root,disposition_count,disposition_root,receipt_bytes,receipt_digest
         FROM core_boundary_persona_receipt_v1
         ORDER BY CASE owner_kind WHEN 'persona' THEN 1 ELSE 2 END,owner_key LIMIT 65538",
    )?;
    let mut rows = statement.query([])?;
    let mut output = Vec::new();
    while let Some(row) = rows.next()? {
        let kind = match row.get::<_, String>(0)?.as_str() {
            "persona" => 1,
            "global_unowned" => 2,
            _ => return Err(invalid("V9_OWNER_KIND")),
        };
        let digest = |index| -> Result<Digest, StoreError> {
            row.get::<_, Vec<u8>>(index)?.try_into().map_err(|_| invalid("V9_RECEIPT_DIGEST"))
        };
        let unsigned = |index| -> Result<u64, StoreError> {
            row.get::<_, i64>(index)?.try_into().map_err(|_| invalid("V9_RECEIPT_COUNT"))
        };
        let count = unsigned(6)?;
        if count != unsigned(8)? {
            return Err(invalid("V9_RECEIPT_COUNT"));
        }
        let owner = Owner(kind, digest(1)?);
        if kind == 2 && owner != global_owner() {
            return Err(invalid("V9_GLOBAL_OWNER"));
        }
        output.push(OwnerReceipt { owner, journal_revision: unsigned(2)?,
            journal_anchor: digest(3)?, operational_count: unsigned(4)?,
            operational_anchor: digest(5)?, count, preimage: digest(7)?,
            disposition: digest(9)?, bytes: row.get(10)?, digest: digest(11)? });
    }
    Ok(output)
}

fn verify_overlay(
    conn: &Connection, entries: &[Disposition<'_>], receipts: &[OwnerReceipt], now: u64,
) -> Result<(), StoreError> {
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM core_boundary_disposition_v1", [], |row| row.get(0))?;
    if count != entries.len() as i64 {
        return Err(invalid("V9_DISPOSITION_COUNT"));
    }
    for entry in entries {
        let valid: bool = conn.query_row(
            "SELECT COUNT(*)=1 FROM core_boundary_disposition_v1
             WHERE record_kind=?1 AND record_key_bytes=?2 AND owner_kind=?3 AND owner_key=?4
             AND source_row_digest=?5 AND effective_disposition=?6 AND reason='core_boundary_upgrade'
             AND authoritative_now_utc_ms=?7 AND leaf_digest=?8",
            rusqlite::params![entry.row.kind, entry.row.key, entry.owner.name(), entry.owner.1.as_slice(),
                entry.row.digest.as_slice(), entry.name, now as i64, entry.leaf.as_slice()],
            |row| row.get(0),
        )?;
        if !valid {
            return Err(invalid("V9_DISPOSITION_MISMATCH"));
        }
    }
    if read_owner_receipts(conn)? != receipts {
        return Err(invalid("V9_OWNER_RECEIPT_MISMATCH"));
    }
    let correct_time: bool = conn.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM core_boundary_persona_receipt_v1 WHERE authoritative_now_utc_ms!=?1)",
        rusqlite::params![now as i64], |row| row.get(0),
    )?;
    if !correct_time {
        return Err(invalid("V9_OWNER_RECEIPT_TIME"));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerminalState { Applied, FailedClosed }

fn not_classified_roots(proof: &RawProof) -> [Digest; 3] {
    [proof.root,
        hash("ae.core-boundary.not-classified.disposition.v1", &[]),
        hash("ae.core-boundary.not-classified.owner-receipt.v1", &[])]
}

pub(crate) fn verify_terminal(conn: &Connection) -> Result<TerminalState, StoreError> {
    let schema = render_schema()?;
    verify_schema_catalog(&bounded_catalog(conn)?, &schema)?;
    let (state, now, code): (String, i64, Option<String>) = conn.query_row(
        "SELECT state,authoritative_now_utc_ms,failure_code FROM core_boundary_control_v1 WHERE singleton=1",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if now <= 0 {
        return Err(invalid("V9_CONTROL_TIME"));
    }
    if state == "pending" {
        return Err(invalid("CORE_BOUNDARY_PENDING_CORRUPT"));
    }
    let proof = verify_failure_receipt(conn, &schema, now as u64)?;
    let (terminal, roots) = match state.as_str() {
        "applied" if code.is_none() => {
            crate::autonomy::verify_autonomy_v8_read_only(conn)?;
            let entries = dispositions(&proof).map_err(|error| match error {
                V9UpgradeError::Closed(_) => invalid("V9_APPLIED_CLASSIFICATION"),
                V9UpgradeError::Fatal(error) => error,
            })?;
            let frozen = read_owner_receipts(conn)?;
            let receipts = owner_receipts(conn, &schema, &entries, now as u64, Some(&frozen))?;
            verify_overlay(conn, &entries, &receipts, now as u64)?;
            (TerminalState::Applied, control_roots(&schema, &receipts))
        },
        "failed_closed" if matches!(code.as_deref(),
            Some("CORE_BOUNDARY_LEGACY_AUTHORITY" | "CORE_BOUNDARY_UNSUPPORTED_LEGACY_STATE")) => {
            let empty: bool = conn.query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM core_boundary_disposition_v1)
                 AND NOT EXISTS(SELECT 1 FROM core_boundary_persona_receipt_v1)",
                [], |row| row.get(0),
            )?;
            if !empty { return Err(invalid("V9_FAILED_CLOSED_OVERLAY")); }
            (TerminalState::FailedClosed, not_classified_roots(&proof))
        },
        _ => return Err(invalid("V9_CONTROL_STATE")),
    };
    let (_, failure_digest) = failure_receipt(&proof, now as u64);
    let valid: bool = conn.query_row(
        "SELECT COUNT(*)=1 FROM core_boundary_control_v1 WHERE singleton=1
         AND boundary_revision=1 AND externalization_disabled=1 AND source_db_version=8
         AND schema_digest=?1 AND failure_preimage_root=?2 AND failure_receipt_digest=?3
         AND preimage_root=?4 AND disposition_root=?5 AND receipt_root=?6",
        rusqlite::params![schema_digest(&schema).as_slice(), proof.root.as_slice(), failure_digest.as_slice(),
            roots[0].as_slice(), roots[1].as_slice(), roots[2].as_slice()], |row| row.get(0),
    )?;
    let migration: bool = conn.query_row(
        "SELECT COUNT(*)=1 FROM schema_migrations WHERE version=9 AND digest=?1 AND completed_at_ms=?2",
        rusqlite::params![MIGRATION_DIGEST, now], |row| row.get(0),
    )?;
    if !valid || !migration {
        return Err(invalid("V9_TERMINAL_CLOSURE"));
    }
    Ok(terminal)
}

fn classified_upgrade(
    conn: &Connection, schema: &Schema, proof: &RawProof, now: u64,
) -> Result<[Digest; 3], V9UpgradeError> {
    if proof.rows.iter().any(|row| row.cells.iter().any(|cell| cell.0 == 2)) {
        return Err(V9UpgradeError::Closed(CoreBoundaryClosedCode::LegacyAuthority));
    }
    crate::autonomy::verify_autonomy_v8_read_only(conn).map_err(|error| match error {
        StoreError::AutonomyConflict(_) | StoreError::ContinuityFence(_) =>
            V9UpgradeError::Closed(CoreBoundaryClosedCode::LegacyAuthority),
        other => V9UpgradeError::Fatal(other),
    })?;
    let entries = dispositions(proof)?;
    let receipts = owner_receipts(conn, schema, &entries, now, None)?;
    persist_overlay(conn, &entries, &receipts, now)?;
    if scan_raw_legacy(conn, schema)? != *proof {
        return Err(V9UpgradeError::Fatal(invalid("V9_PREIMAGE_CHANGED")));
    }
    verify_overlay(conn, &entries, &receipts, now)?;
    Ok(control_roots(schema, &receipts))
}

pub(crate) fn upgrade(conn: &mut Connection) -> Result<TerminalState, StoreError> {
    let schema = render_schema()?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let now = crate::now_ms();
    let proof = install_pending(&tx, &schema, now)?;
    tx.execute_batch("SAVEPOINT core_boundary_classification")?;
    let (state, roots, code) = match classified_upgrade(&tx, &schema, &proof, now) {
        Ok(roots) => {
            tx.execute_batch("RELEASE core_boundary_classification")?;
            ("applied", roots, None)
        },
        Err(V9UpgradeError::Closed(code)) => {
            tx.execute_batch("ROLLBACK TO core_boundary_classification; RELEASE core_boundary_classification")?;
            if verify_failure_receipt(&tx, &schema, now)? != proof {
                return Err(invalid("V9_PREIMAGE_CHANGED"));
            }
            ("failed_closed", not_classified_roots(&proof), Some(code.as_str()))
        },
        Err(V9UpgradeError::Fatal(error)) => return Err(error),
    };
    tx.execute(
        "UPDATE core_boundary_control_v1 SET state=?1,preimage_root=?2,
         disposition_root=?3,receipt_root=?4,failure_code=?5 WHERE singleton=1 AND state='pending'",
        rusqlite::params![state, roots[0].as_slice(), roots[1].as_slice(), roots[2].as_slice(), code],
    )?;
    let terminal = verify_terminal(&tx)?;
    tx.commit()?;
    Ok(terminal)
}

/// A terminal reopen enforces query-only for all verification reads. Only the
/// applied route restores the connection flag; failed_closed stays read-only.
pub(crate) fn reopen(conn: &Connection) -> Result<TerminalState, StoreError> {
    let prior: bool = conn.pragma_query_value(None, "query_only", |row| row.get(0))?;
    let before = conn.total_changes();
    conn.pragma_update(None, "query_only", true)?;
    let state = verify_terminal(conn)?;
    if conn.total_changes() != before {
        return Err(invalid("V9_REOPEN_WROTE"));
    }
    if state == TerminalState::Applied {
        conn.pragma_update(None, "query_only", prior)?;
    }
    Ok(state)
}

// Every retained executor has its own closed tag so source enumeration can
// detect an added bypass. These tags are private migration compatibility only.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum RetiredOperationTagV1 {
    upsert_temporal_profile,
    upsert_relation_policy,
    record_relation_inbound,
    store_outbound_target,
    initialize_autonomous_state,
    upsert_autonomy_scope,
    list_autonomy_scopes,
    list_autonomy_scopes_for_persona,
    rebuild_autonomy_scope_bindings_from_journal,
    load_temporal_profile,
    load_relation_policy,
    load_intention,
    load_current_outbound_target,
    load_autonomous_state,
    list_autonomous_states,
    claim_wake,
    claim_wake_proposal,
    load_wake_claim,
    write_autonomy_snapshot,
    read_autonomy_snapshot,
    load_wake_time_settlement_v1,
    commit_autonomous_wake,
    commit_autonomous_wake_claim_v1,
    commit_alpha3_wake,
    gate_and_claim_externalization,
    settle_externalization,
    gate_and_claim_dispatch,
    settle_dispatch,
    load_outbound_for_intention,
    outbound_submission_metrics,
    recover_orphaned_dispatches,
    recover_orphaned_externalizations,
    externalization_reserved_tokens,
    list_pending_autonomy_work,
    list_pending_autonomy_work_for_relation,
    verify_autonomy_projection,
    has_autonomy_journal_delta,
    rebuild_autonomy_projection,
    load_relation_contact_v1,
    load_relation_consent_v1,
    load_contact_intention_basis_v1,
    load_interaction_facts_v1,
    recover_retired_wake_v2_claims,
    claim_wake_v2,
    load_wake_claim_v2,
    apply_interaction_fact_batch_v1,
    apply_interaction_fact_batch_with_appraisal_v1,
    upsert_relation_budget_policy_v1,
    relation_budget_ledger_v1,
    gate_and_claim_externalization_v2,
    settle_externalization_v2,
    observe_body_snapshot_v1,
    observe_execution_receipt_v1,
    observe_budget_summary_v1,
    observe_gate_reasons_v1,
    intention_scoped_locator_v1,
    execution_scoped_locator_v1,
    observe_snapshot_v2,
    alpha3_authoritative_fingerprint_v1,
    gate_and_claim_dispatch_v2,
    settle_dispatch_v2,
    grant_ecosystem_capability_v1,
    ecosystem_observe_v1,
    ecosystem_propose_v1,
    verify_lived_world_replay_v1,
    load_world_anchor_v1,
    load_lived_day_v1,
    load_dream_residues_v1,
    load_accepted_activity_proposals_v1,
    conversation_context_v1,
    review_dream_residue_v1,
    review_dream_residue_with_authority_v1,
    apply_event,
    ensure_relation_authority_tx,
    record_adapter_submission_tx,
    persist_alpha3_wake_tx,
    persist_lived_world_wake_tx,
    append_operational_authority,
}

pub(crate) fn enforce_retired(
    conn: &Connection,
    _operation: RetiredOperationTagV1,
) -> Result<(), StoreError> {
    // Do not trust a cached version: this connection could have been opened
    // before another process committed the permanent fence.
    if preflight(conn)? == OpenRoute::V9 {
        let schema = render_schema()?;
        verify_schema_catalog(&bounded_catalog(conn)?, &schema)?;
        let terminal: bool = conn.query_row(
            "SELECT COUNT(*)=1 FROM core_boundary_control_v1 AS c
             JOIN schema_migrations AS m ON m.version=9
             WHERE c.singleton=1 AND c.state IN ('applied','failed_closed')
             AND c.boundary_revision=1 AND c.externalization_disabled=1
             AND c.schema_digest=?1 AND m.digest=?2
             AND m.completed_at_ms=c.authoritative_now_utc_ms",
            rusqlite::params![schema_digest(&schema).as_slice(), MIGRATION_DIGEST],
            |row| row.get(0),
        )?;
        if !terminal { return Err(invalid("V9_TERMINAL_CLOSURE")); }
        return Err(invalid("UNSUPPORTED_CORE_BOUNDARY"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::Store::migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn frozen_schema_renders_all_objects_and_rejects_catalog_drift() {
        let schema = render_schema().unwrap();
        assert_eq!(schema.objects.len(), 146);
        assert_eq!(schema.retired_tables.len(), 28);
        verify_schema_catalog(&schema.objects, &schema).unwrap();
        let mut changed = schema.objects.clone();
        changed[0].sql.push(' ');
        assert!(verify_schema_catalog(&changed, &schema).is_err());
    }

    #[test]
    fn legacy_raw_scan_preserves_storage_classes_and_read_only_verifier() {
        let connection = legacy();
        let conn = &connection;
        let schema = render_schema().unwrap();
        let before = conn.total_changes();
        crate::autonomy::verify_autonomy_v8_read_only(conn).unwrap();
        let empty = scan_raw_legacy(conn, &schema).unwrap();
        assert!(empty.rows.is_empty());
        assert_eq!(conn.total_changes(), before);
        conn.execute(
            "INSERT INTO persona_temporal_profile VALUES(?1,1,?2)",
            rusqlite::params![[7u8; 32].as_slice(), " { invalid legacy json } "],
        )
        .unwrap();
        let preserved = scan_raw_legacy(conn, &schema).unwrap();
        assert_eq!(preserved.rows.len(), 1);
        assert_eq!(
            preserved.rows[0].cells[2],
            (3, b" { invalid legacy json } ".to_vec())
        );
        assert_ne!(empty.root, preserved.root);
        assert_eq!(scan_raw_legacy(conn, &schema).unwrap(), preserved);
    }

    #[test]
    fn first_upgrade_and_terminal_reopen_are_closed_and_zero_write() {
        let mut connection = legacy();
        let conn = &mut connection;
        let schema = render_schema().unwrap();
        let before = scan_raw_legacy(conn, &schema).unwrap();
        assert_eq!(upgrade(conn).unwrap(), TerminalState::Applied);
        assert_eq!(scan_raw_legacy(conn, &schema).unwrap(), before);
        let writes = conn.total_changes();
        assert_eq!(preflight(conn).unwrap(), OpenRoute::V9);
        assert_eq!(reopen(conn).unwrap(), TerminalState::Applied);
        assert_eq!(conn.total_changes(), writes);
        assert!(conn.execute("INSERT INTO persona_temporal_profile VALUES(?1,1,'{}')",
            rusqlite::params![[7u8; 32].as_slice()]).is_err());
        assert_eq!(conn.total_changes(), writes);
    }

    #[test]
    fn malformed_legacy_authority_is_preserved_in_failed_closed_reopen() {
        let mut conn = legacy();
        conn.execute("INSERT INTO durable_intention VALUES(?1,?2,?3,?4,'unknown',0,'not json')",
            rusqlite::params![[1u8;16].as_slice(),[2u8;32].as_slice(),[3u8;32].as_slice(),[4u8;32].as_slice()]).unwrap();
        let schema = render_schema().unwrap();
        let before = scan_raw_legacy(&conn, &schema).unwrap();
        assert_eq!(upgrade(&mut conn).unwrap(), TerminalState::FailedClosed);
        assert_eq!(scan_raw_legacy(&conn, &schema).unwrap(), before);
        let changes = conn.total_changes();
        assert_eq!(reopen(&conn).unwrap(), TerminalState::FailedClosed);
        assert_eq!(conn.total_changes(), changes);
        assert!(conn.execute("INSERT INTO meta VALUES('forbidden',X'00')", []).is_err());
    }

    #[test]
    fn visible_pending_is_corrupt_and_fatal_install_rolls_back_every_v9_object() {
        let mut conn = legacy();
        let schema = render_schema().unwrap();
        {
            let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).unwrap();
            install_pending(&tx, &schema, 42).unwrap();
            tx.commit().unwrap();
        }
        let changes = conn.total_changes();
        assert!(matches!(reopen(&conn), Err(StoreError::ContinuityFence("CORE_BOUNDARY_PENDING_CORRUPT"))));
        assert_eq!(conn.total_changes(), changes);
        let mut broken = legacy();
        broken.execute_batch("DROP TABLE outbound_target").unwrap();
        assert!(upgrade(&mut broken).is_err());
        let count: i64 = broken.query_row("SELECT COUNT(*) FROM sqlite_schema WHERE name LIKE 'core_boundary_%'",[],|r|r.get(0)).unwrap();
        assert_eq!(count, 0);
        assert_eq!(preflight(&broken).unwrap(), OpenRoute::Legacy(8));
    }

    #[test]
    fn nonempty_v8_claims_authenticate_wake_owners_and_both_reservation_dispositions() {
        use ae_contracts::*;
        use ae_fixed::Fixed;
        let mut conn=legacy();
        let scope=ScopeRef {bot_token:[1;16],persona_token:[2;16],relation_token:None,session_token:[3;16]};
        let persona=wire::persona_scope_digest(&scope.bot_token,&scope.persona_token,None);
        let frozen=FrozenTimeInputV1 {schema_version:1,observed_now_utc_ms:1000,effective_now_utc_ms:1000,
            persona_tzid:"UTC".into(),persona_utc_offset_seconds:0,persona_local_minute:0,persona_day_ordinal:0,
            relation_tzid:"UTC".into(),relation_utc_offset_seconds:0,relation_local_minute:0,relation_day_ordinal:0,
            budget_day_start_utc_ms:0,budget_next_day_start_utc_ms:86_400_000,next_timezone_transition_utc_ms:None,tzdb_fingerprint:[4;32]};
        let mut frozen_hash=Sha256::new();frozen_hash.update(b"ae.frozen-time-input.v1\0");frozen_hash.update(serde_json::to_vec(&frozen).unwrap());
        let event=TimeAdvanceV1 {event_id:[5;16],scope:scope.clone(),expected_generation:0,frozen,
            frozen_input_digest:frozen_hash.finalize().into(),stimulus:AutonomousStimulusV1::default()};
        let state=AutonomousRuntimeStateV1 {schema_version:1,persona_scope:persona,relation_scope:None,generation:1,state_revision:0,
            last_advanced_at_utc_ms:1000,next_wake_at_utc_ms:301000,wake_intensity:WakeIntensityV1::Micro,sleep_state:SleepStateV1::Awake,
            process_s:Fixed::ZERO,process_c:Fixed::ZERO,arousal:Fixed::ZERO,sleep_threshold_held_ms:0,circadian_phase_minutes:Fixed::ZERO,
            affiliation_need:Fixed::ZERO,unfinished_topic_salience:Fixed::ZERO,social_energy:Fixed::ONE,formula_digest:[6;32],mapping_digest:[7;32],workspace_residual:Fixed::ZERO};
        let proposal=WakeProposalV1 {state,inner_events:vec![],intentions:vec![]};
        for (kind,domain,id) in [("wake",b"wake".as_slice(),5u8),("wake_v2",b"wake-v2".as_slice(),8u8)] {
            let mut event=event.clone();event.event_id=[id;16];
            let caller=wire::domain_hash(b"ae.runtime.wake-caller.v2",&[&event.event_id,&event.scope.session_token]);
            let token=crate::autonomy::digest_claim(domain,&event.event_id,&caller);
            let body=if kind=="wake" {serde_json::to_string(&WakeClaimV1 {claim_token:token,event:event.clone(),proposal:proposal.clone(),lease_deadline_utc_ms:121000}).unwrap()}
                else {serde_json::to_string(&WakeClaimV2 {claim_token:token,event:event.clone(),proposal:Alpha3WakeProposalV1 {legacy:proposal.clone(),contact_updates:vec![],contact_bases:vec![],lived_day:None,world_anchor:None,dream_updates:vec![]},lease_deadline_utc_ms:121000}).unwrap()};
            conn.execute("INSERT INTO autonomy_claim VALUES(?1,?2,?3,?4,121000,?5)",rusqlite::params![token.as_slice(),kind,event.event_id.as_slice(),caller.as_slice(),body]).unwrap();
        }
        let intention=DurableIntentionV1 {schema_version:1,intention_id:[10;16],persona_scope:persona,relation_scope:[11;32],state:IntentionStateV1::Externalizing,
            action_class:"follow_up".into(),salience:Fixed::ONE,urgency:Fixed::ONE,confidence:Fixed::ONE,created_at_utc_ms:1,not_before_utc_ms:1,expires_at_utc_ms:999999,
            externalization_attempts:1,semantic_idempotency_digest:[12;32],workspace_mapping_digest:[13;32],workspace_residual:Fixed::ZERO,source_event_ids:vec![[14;16]]};
        conn.execute("INSERT INTO durable_intention VALUES(?1,?2,?3,?4,'externalizing',1,?5)",rusqlite::params![intention.intention_id.as_slice(),persona.as_slice(),intention.relation_scope.as_slice(),intention.semantic_idempotency_digest.as_slice(),serde_json::to_string(&intention).unwrap()]).unwrap();
        let mut claim=ExternalizationClaimV1 {claim_token:[0;32],intention_id:intention.intention_id,attempt_no:1,max_tokens:64,prompt_contract:"fixture".into(),prompt_contract_digest:[15;32],relation_policy_revision:1,target_binding_digest:[16;32],capability_snapshot_digest:[17;32],frozen_input_digest:[18;32],caller_incarnation:[19;32],lease_deadline_utc_ms:121000};
        claim.claim_token=crate::autonomy::externalization_claim_token(&claim);
        conn.execute("INSERT INTO autonomy_claim VALUES(?1,'externalization',?2,?3,121000,?4)",rusqlite::params![claim.claim_token.as_slice(),claim.intention_id.as_slice(),claim.caller_incarnation.as_slice(),serde_json::to_string(&claim).unwrap()]).unwrap();
        conn.execute("INSERT INTO externalization_budget(relation_scope,budget_day_start_utc_ms,reserved_tokens,limit_tokens,charged_tokens,used_tokens,usage_known,revision) VALUES(?1,0,64,128,0,0,1,1)",rusqlite::params![intention.relation_scope.as_slice()]).unwrap();
        conn.execute("INSERT INTO externalization_budget_claim VALUES(?1,?2,0,64,0)",rusqlite::params![claim.claim_token.as_slice(),intention.relation_scope.as_slice()]).unwrap();
        conn.execute("INSERT INTO externalization_budget_claim VALUES(?1,?2,86400000,32,0)",rusqlite::params![[99u8;32].as_slice(),intention.relation_scope.as_slice()]).unwrap();
        let schema=render_schema().unwrap();
        let before=scan_raw_legacy(&conn,&schema).unwrap();
        assert_eq!(authenticate_claims(&before).unwrap().len(),3);
        assert_eq!(upgrade(&mut conn).unwrap(),TerminalState::Applied);
        assert_eq!(scan_raw_legacy(&conn,&schema).unwrap(),before);
        let mut stmt=conn.prepare("SELECT effective_disposition FROM core_boundary_disposition_v1 WHERE record_kind=12 ORDER BY effective_disposition").unwrap();
        assert_eq!(stmt.query_map([],|r|r.get::<_,String>(0)).unwrap().collect::<Result<Vec<_>,_>>().unwrap(),vec!["effective_charged_unknown_frozen","frozen_unverified_no_refund"]);
        drop(stmt);
        let owners:i64=conn.query_row("SELECT COUNT(*) FROM core_boundary_disposition_v1 WHERE record_kind=10 AND owner_kind='persona' AND owner_key=?1",rusqlite::params![persona.as_slice()],|r|r.get(0)).unwrap();
        assert_eq!(owners,3);
        let changes=conn.total_changes();
        assert_eq!(reopen(&conn).unwrap(),TerminalState::Applied);
        assert_eq!(conn.total_changes(),changes);
        assert_eq!(scan_raw_legacy(&conn,&schema).unwrap(),before);
    }
}
