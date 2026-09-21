//! Persona-scoped semantic state authority.
//!
//! The journal is an ordered record of canonical events.  This module owns a
//! different cursor: one semantic revision stream per persona.  Evidence is
//! additionally keyed by relation, so two relationships may reuse an event ID
//! without sharing private evidence.  A semantic transition and its canonical
//! journal event are committed by one SQLite `BEGIN IMMEDIATE` transaction.

use ae_continuum::{CommitEnvelope, JournalRow};
use ae_contracts::{
    phase0_canonical_formula_digest_v1, phase0_semantic_route_digest_v1, wire,
    ApplyInteractionResultV1, AutonomousRuntimeStateV1, CanonicalEvent, CausalRef, CommitStatus,
    Digest, EvidenceVector, Id128, InteractionFactKindV1, InteractionFactV1,
    InteractionSourceAuthorityV1, MatrixSleepPhaseV1, MatrixTimeEpochV1, PerceptionChallengeV1,
    PerceptionOriginCommitmentV1, PerceptionProposalV1, PersonaScopeRef, ReplyAffectV1, ScopeRef,
    SemanticAppraisalBeginStatusV1, SemanticAppraisalBudgetReceiptV1,
    SemanticAppraisalCapacityReasonV1, SemanticAppraisalOutcomeV1,
    SemanticAppraisalProviderUsageV1, SemanticAppraisalSettleRequestV1, SemanticEstimate,
    SemanticTimeAuthorityV1, SemanticTransitionKindV1, SleepStateV1, SourceAuthority,
    TimeAdvanceV1, TransitionReceipt, UserStimulus,
};
use ae_fixed::Fixed;
use ae_neurofield::{graph_digest, state_digest, NeuralField, SparseGraph};
use ae_semantic_core::{
    advance_matrix_time_v1,
    decode_canonical_semantic_snapshot_v3 as decode_core_semantic_snapshot_v3,
    decode_legacy_time_snapshot_v1, decode_time_snapshot_v1, decode_transition_receipt_v2,
    derive_user_stimulus_transition_v1, encode_canonical_graph_v1, encode_time_snapshot_v1,
    matrix_time_formula_digest_v1, DecodedTimeSnapshotV1, MatrixTimeInputV1, SemanticCoreError,
    UserStimulusTransitionInputV1,
};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use super::{
    blob, bounded_typed_value, enforce_byte_budget, enforce_read_budget, now_ms,
    query_bounded_journal_row, query_bounded_snapshot_row, sqlite_length, stored_typed_digest,
    JournalCommitLane, JournalRevision, Store, StoreError, MAX_JOURNAL_EVENT_BYTES,
    MAX_JOURNAL_RECEIPT_BYTES, MAX_JOURNAL_ROWS_PER_SCOPE, MAX_SNAPSHOT_STATE_BYTES,
};
use crate::semantic_field_attestation::{
    active_identity, decode_canonical_aesem3_blocks, derive_canonical_semantic_sidecar_v1,
    verify_committed_semantic_migration_rows, ActiveSemanticIdentityV1,
};

const SEMANTIC_EVIDENCE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-evidence-authority-v1";
const SEMANTIC_COMMITMENT_DOMAIN_V1: &[u8] = b"astr-embodiment/persona-semantic-commitment-v1";
const SEMANTIC_ORIGIN_DOMAIN_V1: &[u8] = b"astr-embodiment/persona-semantic-origin-v1";
const LEGACY_SEMANTIC_LANE_NAMESPACE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-lane-namespace-v1";
const SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-snapshot-wire-v1";
const SEMANTIC_GRAPH_WIRE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-graph-wire-v1";
const SEMANTIC_RECEIPT_WIRE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-receipt-wire-v1";
const SEMANTIC_TELEMETRY_WIRE_DOMAIN_V1: &[u8] = b"astr-embodiment/semantic-telemetry-wire-v1";
/// The current `UserStimulus` wire has no externally supplied request nonce.
/// This Store-owned nonce binds a transition to the complete immutable event,
/// the active incarnation and the independent semantic cursor without
/// reinterpreting the legacy event schema.
const CANONICAL_SEMANTIC_NONCE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/canonical-semantic-request-nonce-v1";
/// This commitment binds (but does not claim to authenticate) the estimator
/// identity carried by the legacy `UserStimulus` wire. A future FFI proposal
/// lane must validate its external nonce before constructing that event.
const CANONICAL_ESTIMATOR_BINDING_DOMAIN_V1: &[u8] =
    b"astr-embodiment/canonical-semantic-estimator-binding-v1";
const PERCEPTION_CHALLENGE_NONCE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/perception-challenge-nonce-v1";
const PERCEPTION_CHALLENGE_SECRET_COMMITMENT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/perception-challenge-secret-commitment-v1";
const PERCEPTION_PROVIDER_DOMAIN_V1: &[u8] = b"astr-embodiment/perception-provider-v1";
const PERCEPTION_CHALLENGE_TTL_MS: u64 = 5 * 60 * 1_000;
const MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA: u64 = 64;
const MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL: u64 = 4_096;
const MAX_PERCEPTION_ORIGIN_BYTES: u64 = 4_096;
const MAX_PERCEPTION_CHALLENGE_PERSONAS_GLOBAL: u64 = 4_096;
const MAX_PERCEPTION_CHALLENGE_BYTES_GLOBAL: u64 =
    MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL * MAX_PERCEPTION_ORIGIN_BYTES;
const PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1: u64 = 128;

const MAX_SEMANTIC_ROWS_PER_PERSONA: u64 = MAX_JOURNAL_ROWS_PER_SCOPE;
const MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA: u64 = 512 * 1024 * 1024;
const MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL: u64 = 4_096;
const MAX_SEMANTIC_HISTORY_SCAN_ROWS_PER_PERSONA: u64 = MAX_SEMANTIC_ROWS_PER_PERSONA * 7;
const MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL: u64 = 4 * 1024 * 1024;
const MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL: u64 = 4 * 1024 * 1024 * 1024;
const MAX_SEMANTIC_GRAPH_BYTES: u64 = MAX_SNAPSHOT_STATE_BYTES;
const MAX_SEMANTIC_RECEIPT_BYTES: u64 = MAX_JOURNAL_RECEIPT_BYTES;
const MAX_SEMANTIC_TELEMETRY_BYTES: u64 = MAX_JOURNAL_RECEIPT_BYTES;
const MAX_SEMANTIC_OBSERVER_SCOPE_BYTES: u64 = 4_096;
const SEMANTIC_APPRAISAL_SCHEMA_VERSION_V1: u8 = 1;
const SEMANTIC_APPRAISAL_SCHEMA_VERSION_V2: u8 = 2;
const SEMANTIC_APPRAISAL_SCHEMA_VERSION_V3: u8 = 3;
const SEMANTIC_APPRAISAL_UTC_DAY_MS: u64 = 86_400_000;
const SEMANTIC_APPRAISAL_PENDING_TTL_MS: u64 = 300_000;
const SEMANTIC_APPRAISAL_TERMINAL_EXACT_RETRY_MS: u64 = 300_000;
const SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE: u64 = 64;
const MAX_SEMANTIC_APPRAISAL_CLAIMS_GLOBAL: u64 = 4_096;
const MAX_SEMANTIC_APPRAISAL_PERSONAS_GLOBAL: u64 = 4_096;
const MAX_SEMANTIC_APPRAISAL_BUDGET_ROWS_GLOBAL: u64 = 4_096;
const MAX_SEMANTIC_APPRAISAL_AGGREGATE_BYTES_GLOBAL: u64 = 8 * 1024 * 1024;
const MAX_SEMANTIC_APPRAISAL_REPAIR_ROWS_PER_PERSONA: u64 = 64;
const MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES: u64 = 16 * 1024;
const MAX_SEMANTIC_APPRAISAL_OUTCOME_BYTES: u64 = 32;
const MAX_SEMANTIC_APPRAISAL_PENDING_PAYLOAD_BYTES: u64 =
    MAX_SEMANTIC_APPRAISAL_OUTCOME_BYTES + 2 * MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES;
const SEMANTIC_APPRAISAL_SETTLEMENT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-settlement-v1";
const SEMANTIC_APPRAISAL_REPLY_AFFECT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-reply-affect-v1";
const SEMANTIC_APPRAISAL_TERMINAL_RECEIPT_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-terminal-receipt-v1";
const SEMANTIC_APPRAISAL_CLAIM_COMPACTION_LEAF_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-claim-compaction-leaf-v1";
const SEMANTIC_APPRAISAL_CLAIM_COMPACTION_CHAIN_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-claim-compaction-chain-v1";
const SEMANTIC_APPRAISAL_BUDGET_COMPACTION_LEAF_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-budget-compaction-leaf-v1";
const SEMANTIC_APPRAISAL_BUDGET_COMPACTION_CHAIN_DOMAIN_V1: &[u8] =
    b"astr-embodiment/semantic-appraisal-budget-compaction-chain-v1";
const SEMANTIC_APPRAISAL_BUDGET_RETENTION_CANDIDATES_V1_SQL: &str =
    "SELECT rowid,persona_scope,utc_day
     FROM semantic_appraisal_budget INDEXED BY semantic_appraisal_budget_retention_v3
     WHERE utc_day<?1
     ORDER BY utc_day,persona_scope LIMIT ?2";
const SEMANTIC_APPRAISAL_BUDGET_RETENTION_ELIGIBLE_V1_SQL: &str =
    "SELECT reserved_tokens=0 AND NOT EXISTS(
       SELECT 1 FROM semantic_appraisal_claim
         INDEXED BY semantic_appraisal_claim_budget_v3
        WHERE persona_scope=?2 AND utc_day=?3
     )
     FROM semantic_appraisal_budget
     WHERE rowid=?1 AND persona_scope=?2 AND utc_day=?3";

fn semantic_appraisal_pending_expired_v1(created_at_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(created_at_ms) >= SEMANTIC_APPRAISAL_PENDING_TTL_MS
}

fn semantic_appraisal_terminal_retry_live_v1(settled_at_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(settled_at_ms) < SEMANTIC_APPRAISAL_TERMINAL_EXACT_RETRY_MS
}

pub(crate) const SEMANTIC_APPRAISAL_SCHEMA_V1_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_appraisal_budget (
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
    daily_token_limit INTEGER NOT NULL CHECK(typeof(daily_token_limit)='integer' AND daily_token_limit>=0 AND daily_token_limit<=1000000),
    charged_tokens INTEGER NOT NULL CHECK(typeof(charged_tokens)='integer' AND charged_tokens>=0),
    reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0),
    blocked INTEGER NOT NULL CHECK(typeof(blocked)='integer' AND blocked IN (0,1)),
    updated_at_ms INTEGER NOT NULL CHECK(typeof(updated_at_ms)='integer' AND updated_at_ms>0),
    PRIMARY KEY(persona_scope,utc_day)
);
CREATE TABLE IF NOT EXISTS semantic_appraisal_claim (
    request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
    origin_event_digest BLOB NOT NULL UNIQUE CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
    origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
    provider_digest BLOB NOT NULL CHECK(typeof(provider_digest)='blob' AND length(provider_digest)=32),
    reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0 AND reserved_tokens<=1000000),
    created_at_ms INTEGER NOT NULL CHECK(typeof(created_at_ms)='integer' AND created_at_ms>0),
    settled_at_ms INTEGER,
    charged_tokens INTEGER,
    outcome_code TEXT,
    canonical_revision INTEGER,
    semantic_revision INTEGER,
    CHECK(
      (settled_at_ms IS NULL AND reserved_tokens>=768 AND charged_tokens IS NULL AND outcome_code IS NULL AND canonical_revision IS NULL AND semantic_revision IS NULL)
      OR
      (typeof(settled_at_ms)='integer' AND settled_at_ms>=created_at_ms
       AND typeof(charged_tokens)='integer' AND charged_tokens>=0
       AND typeof(outcome_code)='text' AND length(CAST(outcome_code AS BLOB)) BETWEEN 1 AND 32
       AND typeof(canonical_revision)='integer' AND canonical_revision>=0
       AND (semantic_revision IS NULL OR (typeof(semantic_revision)='integer' AND semantic_revision>0)))
    ),
    FOREIGN KEY(persona_scope,utc_day)
      REFERENCES semantic_appraisal_budget(persona_scope,utc_day)
);
CREATE INDEX IF NOT EXISTS semantic_appraisal_claim_pending_v1
  ON semantic_appraisal_claim(persona_scope,settled_at_ms);
"#;

pub(crate) const SEMANTIC_APPRAISAL_SCHEMA_V2_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_appraisal_budget (
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
    daily_token_limit INTEGER NOT NULL CHECK(typeof(daily_token_limit)='integer' AND daily_token_limit>=0 AND daily_token_limit<=1000000),
    charged_tokens INTEGER NOT NULL CHECK(typeof(charged_tokens)='integer' AND charged_tokens>=0),
    reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0),
    blocked INTEGER NOT NULL CHECK(typeof(blocked)='integer' AND blocked IN (0,1)),
    updated_at_ms INTEGER NOT NULL CHECK(typeof(updated_at_ms)='integer' AND updated_at_ms>0),
    PRIMARY KEY(persona_scope,utc_day)
);
CREATE TABLE IF NOT EXISTS semantic_appraisal_claim (
    request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
    origin_event_digest BLOB NOT NULL UNIQUE CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
    origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
    provider_digest BLOB NOT NULL CHECK(typeof(provider_digest)='blob' AND length(provider_digest)=32),
    reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0 AND reserved_tokens<=1000000),
    created_at_ms INTEGER NOT NULL CHECK(typeof(created_at_ms)='integer' AND created_at_ms>0),
    settled_at_ms INTEGER,
    charged_tokens INTEGER,
    outcome_code TEXT,
    canonical_revision INTEGER,
    semantic_revision INTEGER,
    usage_known INTEGER,
    usage_tokens INTEGER,
    proposal_identity_digest BLOB,
    settlement_identity_digest BLOB,
    reply_affect_bytes BLOB,
    reply_affect_digest BLOB,
    terminal_receipt_bytes BLOB,
    terminal_receipt_digest BLOB,
    CHECK(
      (settled_at_ms IS NULL AND reserved_tokens>=768 AND charged_tokens IS NULL
       AND outcome_code IS NULL AND canonical_revision IS NULL AND semantic_revision IS NULL
       AND usage_known IS NULL AND usage_tokens IS NULL AND proposal_identity_digest IS NULL
       AND settlement_identity_digest IS NULL AND reply_affect_bytes IS NULL
       AND reply_affect_digest IS NULL AND terminal_receipt_bytes IS NULL
       AND terminal_receipt_digest IS NULL)
      OR
      (typeof(settled_at_ms)='integer' AND settled_at_ms>=created_at_ms
       AND typeof(charged_tokens)='integer' AND charged_tokens>=0
       AND typeof(outcome_code)='text' AND length(CAST(outcome_code AS BLOB)) BETWEEN 1 AND 32
       AND typeof(canonical_revision)='integer' AND canonical_revision>=0
       AND (semantic_revision IS NULL OR (typeof(semantic_revision)='integer' AND semantic_revision>0))
       AND (
         (usage_known IS NULL AND usage_tokens IS NULL AND proposal_identity_digest IS NULL
          AND settlement_identity_digest IS NULL AND reply_affect_bytes IS NULL
          AND reply_affect_digest IS NULL AND terminal_receipt_bytes IS NULL
          AND terminal_receipt_digest IS NULL)
         OR
         (typeof(usage_known)='integer' AND usage_known IN (0,1)
          AND ((usage_known=0 AND usage_tokens IS NULL)
               OR (usage_known=1 AND typeof(usage_tokens)='integer' AND usage_tokens>=0 AND usage_tokens<=1000000))
          AND (proposal_identity_digest IS NULL
               OR (typeof(proposal_identity_digest)='blob' AND length(proposal_identity_digest)=32))
          AND typeof(settlement_identity_digest)='blob' AND length(settlement_identity_digest)=32
          AND typeof(reply_affect_bytes)='blob' AND length(reply_affect_bytes)<=16384
          AND typeof(reply_affect_digest)='blob' AND length(reply_affect_digest)=32
          AND typeof(terminal_receipt_bytes)='blob' AND length(terminal_receipt_bytes)<=16384
          AND typeof(terminal_receipt_digest)='blob' AND length(terminal_receipt_digest)=32)
       ))
    ),
    FOREIGN KEY(persona_scope,utc_day)
      REFERENCES semantic_appraisal_budget(persona_scope,utc_day)
);
CREATE INDEX IF NOT EXISTS semantic_appraisal_claim_pending_v1
  ON semantic_appraisal_claim(persona_scope,settled_at_ms);
"#;

const SEMANTIC_APPRAISAL_SCHEMA_V3_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS semantic_appraisal_budget (
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
    daily_token_limit INTEGER NOT NULL CHECK(typeof(daily_token_limit)='integer' AND daily_token_limit>=0 AND daily_token_limit<=1000000),
    charged_tokens INTEGER NOT NULL CHECK(typeof(charged_tokens)='integer' AND charged_tokens>=0),
    reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0),
    blocked INTEGER NOT NULL CHECK(typeof(blocked)='integer' AND blocked IN (0,1)),
    updated_at_ms INTEGER NOT NULL CHECK(typeof(updated_at_ms)='integer' AND updated_at_ms>0),
    compacted_claim_rows INTEGER NOT NULL CHECK(typeof(compacted_claim_rows)='integer' AND compacted_claim_rows>=0),
    compacted_charged_tokens INTEGER NOT NULL CHECK(typeof(compacted_charged_tokens)='integer' AND compacted_charged_tokens>=0 AND compacted_charged_tokens<=charged_tokens),
    compacted_chain_digest BLOB NOT NULL CHECK(typeof(compacted_chain_digest)='blob' AND length(compacted_chain_digest)=32),
    CHECK(
      (compacted_claim_rows=0 AND compacted_charged_tokens=0 AND compacted_chain_digest=zeroblob(32))
      OR
      (compacted_claim_rows>0 AND compacted_chain_digest<>zeroblob(32))
    ),
    PRIMARY KEY(persona_scope,utc_day)
);
CREATE TABLE IF NOT EXISTS semantic_appraisal_claim (
    request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
    persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
    utc_day INTEGER NOT NULL CHECK(typeof(utc_day)='integer' AND utc_day>=0),
    origin_event_digest BLOB NOT NULL UNIQUE CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
    origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
    provider_digest BLOB NOT NULL CHECK(typeof(provider_digest)='blob' AND length(provider_digest)=32),
    reserved_tokens INTEGER NOT NULL CHECK(typeof(reserved_tokens)='integer' AND reserved_tokens>=0 AND reserved_tokens<=1000000),
    created_at_ms INTEGER NOT NULL CHECK(typeof(created_at_ms)='integer' AND created_at_ms>0),
    settled_at_ms INTEGER,
    charged_tokens INTEGER,
    outcome_code TEXT,
    canonical_revision INTEGER,
    semantic_revision INTEGER,
    usage_known INTEGER,
    usage_tokens INTEGER,
    proposal_identity_digest BLOB,
    settlement_identity_digest BLOB,
    reply_affect_bytes BLOB,
    reply_affect_digest BLOB,
    terminal_receipt_bytes BLOB,
    terminal_receipt_digest BLOB,
    CHECK(
      (settled_at_ms IS NULL AND reserved_tokens>=768 AND charged_tokens IS NULL
       AND outcome_code IS NULL AND canonical_revision IS NULL AND semantic_revision IS NULL
       AND usage_known IS NULL AND usage_tokens IS NULL AND proposal_identity_digest IS NULL
       AND settlement_identity_digest IS NULL AND reply_affect_bytes IS NULL
       AND reply_affect_digest IS NULL AND terminal_receipt_bytes IS NULL
       AND terminal_receipt_digest IS NULL)
      OR
      (typeof(settled_at_ms)='integer' AND settled_at_ms>=created_at_ms
       AND typeof(charged_tokens)='integer' AND charged_tokens>=0
       AND typeof(outcome_code)='text' AND length(CAST(outcome_code AS BLOB)) BETWEEN 1 AND 32
       AND typeof(canonical_revision)='integer' AND canonical_revision>=0
       AND (semantic_revision IS NULL OR (typeof(semantic_revision)='integer' AND semantic_revision>0))
       AND (
         (usage_known IS NULL AND usage_tokens IS NULL AND proposal_identity_digest IS NULL
          AND settlement_identity_digest IS NULL AND reply_affect_bytes IS NULL
          AND reply_affect_digest IS NULL AND terminal_receipt_bytes IS NULL
          AND terminal_receipt_digest IS NULL)
         OR
         (typeof(usage_known)='integer' AND usage_known IN (0,1)
          AND ((usage_known=0 AND usage_tokens IS NULL)
               OR (usage_known=1 AND typeof(usage_tokens)='integer' AND usage_tokens>=0 AND usage_tokens<=1000000))
          AND (proposal_identity_digest IS NULL
               OR (typeof(proposal_identity_digest)='blob' AND length(proposal_identity_digest)=32))
          AND typeof(settlement_identity_digest)='blob' AND length(settlement_identity_digest)=32
          AND typeof(reply_affect_bytes)='blob' AND length(reply_affect_bytes)<=16384
          AND typeof(reply_affect_digest)='blob' AND length(reply_affect_digest)=32
          AND typeof(terminal_receipt_bytes)='blob' AND length(terminal_receipt_bytes)<=16384
          AND typeof(terminal_receipt_digest)='blob' AND length(terminal_receipt_digest)=32)
       ))
    ),
    FOREIGN KEY(persona_scope,utc_day)
      REFERENCES semantic_appraisal_budget(persona_scope,utc_day)
);
CREATE TABLE IF NOT EXISTS semantic_appraisal_rollup (
    singleton INTEGER PRIMARY KEY CHECK(typeof(singleton)='integer' AND singleton=1),
    compacted_budget_rows INTEGER NOT NULL CHECK(typeof(compacted_budget_rows)='integer' AND compacted_budget_rows>=0),
    compacted_claim_rows INTEGER NOT NULL CHECK(typeof(compacted_claim_rows)='integer' AND compacted_claim_rows>=0),
    compacted_charged_tokens INTEGER NOT NULL CHECK(typeof(compacted_charged_tokens)='integer' AND compacted_charged_tokens>=0),
    compacted_chain_digest BLOB NOT NULL CHECK(typeof(compacted_chain_digest)='blob' AND length(compacted_chain_digest)=32),
    last_authoritative_now_ms INTEGER NOT NULL CHECK(typeof(last_authoritative_now_ms)='integer' AND last_authoritative_now_ms>=0),
    CHECK(
      (compacted_budget_rows=0 AND compacted_claim_rows=0 AND compacted_charged_tokens=0 AND compacted_chain_digest=zeroblob(32))
      OR
      (compacted_budget_rows>0 AND compacted_chain_digest<>zeroblob(32))
    )
);
INSERT OR IGNORE INTO semantic_appraisal_rollup(
    singleton,compacted_budget_rows,compacted_claim_rows,compacted_charged_tokens,
    compacted_chain_digest,last_authoritative_now_ms
) VALUES(1,0,0,0,zeroblob(32),0);
CREATE INDEX semantic_appraisal_claim_retention_v3
  ON semantic_appraisal_claim(settled_at_ms,created_at_ms,request_nonce_digest);
CREATE INDEX semantic_appraisal_claim_budget_v3
  ON semantic_appraisal_claim(persona_scope,utc_day,settled_at_ms,request_nonce_digest);
CREATE INDEX semantic_appraisal_budget_retention_v3
  ON semantic_appraisal_budget(utc_day,persona_scope);
"#;

#[cfg(test)]
thread_local! {
    static FULL_REPLAY_ROW_VISITS_V1: Cell<u64> = const { Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_full_replay_row_visits_v1() {
    FULL_REPLAY_ROW_VISITS_V1.with(|visits| visits.set(0));
}

#[cfg(test)]
pub(crate) fn full_replay_row_visits_v1() -> u64 {
    FULL_REPLAY_ROW_VISITS_V1.with(Cell::get)
}

#[cfg(test)]
fn note_full_replay_row_v1() {
    FULL_REPLAY_ROW_VISITS_V1.with(|visits| visits.set(visits.get().saturating_add(1)));
}

/// Candidate produced by the semantic runtime.  Every identity-bearing field
/// is only a claim: `Store` derives it again from the canonical event, active
/// Genesis identity, frozen formula/route and strict AESEM3 sidecars.
#[derive(Clone, Debug)]
pub(crate) struct PairedSemanticCommitV1 {
    pub journal: CommitEnvelope,
    pub persona_scope: Digest,
    pub relation_scope: Option<Digest>,
    pub semantic_base_revision: u64,
    pub event_id: Id128,
    pub event_digest: Digest,
    pub evidence_digest: Digest,
    pub estimator_digest: Digest,
    pub incarnation_id: Digest,
    pub manifest_digest: Digest,
    pub route_digest: Digest,
    pub formula_digest: Digest,
    pub state_digest: Digest,
    pub snapshot_bytes: Vec<u8>,
    pub graph_digest: Digest,
    pub receipt_bytes: Vec<u8>,
    pub telemetry_bytes: Vec<u8>,
}

/// The immutable bridge from Genesis (or an attested Task-5 legacy suffix)
/// into the persona semantic namespace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticOriginV1 {
    pub persona_scope: Digest,
    pub source_scope_digest: Digest,
    pub source_revision: u64,
    pub legacy_migrated: bool,
    pub incarnation_id: Digest,
    pub manifest_digest: Digest,
    pub route_digest: Digest,
    pub formula_digest: Digest,
    pub state_digest: Digest,
    pub graph_digest: Digest,
    pub origin_digest: Digest,
}

/// A row reconstructed after commit from the durable journal and all semantic
/// sidecar tables.  Returning this type therefore never returns uncommitted
/// caller bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedSemanticV1 {
    pub journal_revision: u64,
    pub semantic_revision: u64,
    pub persona_scope: Digest,
    pub relation_scope: Option<Digest>,
    pub event_id: Id128,
    pub event_digest: Digest,
    pub transition_kind: SemanticTransitionKindV1,
    pub evidence_digest: Option<Digest>,
    pub estimator_digest: Option<Digest>,
    pub incarnation_id: Digest,
    pub manifest_digest: Digest,
    pub route_digest: Digest,
    pub formula_digest: Digest,
    pub state_digest: Digest,
    pub graph_before: Digest,
    pub graph_digest: Digest,
    pub snapshot_bytes: Vec<u8>,
    pub receipt_bytes: Option<Vec<u8>>,
    pub telemetry_bytes: Option<Vec<u8>>,
    pub time_authority: Option<SemanticTimeAuthorityV1>,
    pub commitment_digest: Digest,
    pub origin: SemanticOriginV1,
    pub journal: JournalRow,
}

/// Fully hydrated current semantic state. Time revisions are reconstructed
/// from one authenticated perception/Genesis anchor plus the cursor epoch;
/// their compact rows never contain a duplicate field or graph.
#[derive(Clone, Debug)]
pub struct HydratedSemanticStateV1 {
    pub semantic_revision: u64,
    pub formula_digest: Digest,
    pub state_digest: Digest,
    pub graph_digest: Digest,
    pub field: NeuralField,
    pub graph: SparseGraph,
}

/// Store-internal material for a read-only observer. Both fields are rebuilt
/// from authenticated semantic rows inside the caller's transaction; no
/// relation-scoped evidence or node identity leaves this boundary.
pub(crate) struct AttestedAffectProjectionInputV1 {
    pub(crate) semantic_revision: u64,
    pub(crate) personality_revision: u64,
    pub(crate) formula_digest: Digest,
    pub(crate) state_digest: Digest,
    pub(crate) graph_digest: Digest,
    pub(crate) previous_field: NeuralField,
    pub(crate) current_field: NeuralField,
    pub(crate) graph: SparseGraph,
    pub(crate) confidence_fxp6: u32,
    pub(crate) semantic_dynamics_renormalization_residual_fxp6: i64,
}

/// Direct Store result for a proposal commit.  Callers never infer replay
/// from a cursor sampled before the transaction; SQLite's serialized writer
/// decides this status while resolving the event identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticCommitDispositionV1 {
    Inserted,
    Existing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticCommitResultV1 {
    pub disposition: SemanticCommitDispositionV1,
    pub committed: CommittedSemanticV1,
}

/// Store-owned result appended to the interaction transaction by the
/// appraisal begin lane.  No caller-selected origin or nonce crosses this
/// boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticAppraisalClaimV1 {
    pub status: SemanticAppraisalBeginStatusV1,
    pub challenge: Option<PerceptionChallengeV1>,
    pub capacity_reason: Option<SemanticAppraisalCapacityReasonV1>,
    pub budget: Option<SemanticAppraisalBudgetReceiptV1>,
    pub reply_affect: Option<ReplyAffectV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticAppraisalBeginStoreOutcomeV1 {
    Completed {
        interaction: ApplyInteractionResultV1,
        claim: SemanticAppraisalClaimV1,
    },
    RetryExpiredOrUnknown {
        interaction: ApplyInteractionResultV1,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticAppraisalStoreSettlementV1 {
    Committed {
        result: SemanticCommitResultV1,
        charged_tokens: u64,
        reply_affect: Option<ReplyAffectV1>,
    },
    ZeroMutation {
        canonical_revision: u64,
        charged_tokens: u64,
        reply_affect: Option<ReplyAffectV1>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SemanticAppraisalSettlementStoreOutcomeV1 {
    Completed(SemanticAppraisalStoreSettlementV1),
    RetryExpiredOrUnknown,
}

/// Closed integration-test crash seams.  This type and every branch that
/// reads it are absent unless the non-default test feature is enabled.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SemanticFaultPoint {
    AfterJournalInsert,
    AfterSemanticInsert,
}

#[derive(Clone)]
struct DerivedSemanticCommitV1 {
    relation_present: bool,
    relation_storage_scope: Digest,
    semantic_revision: u64,
    state_before: Digest,
    graph_before: Digest,
    authority_digest: Digest,
    telemetry_digest: Digest,
    snapshot_wire_digest: Digest,
    graph_wire_digest: Digest,
    receipt_wire_digest: Digest,
    telemetry_wire_digest: Digest,
    graph_bytes: Vec<u8>,
    commitment_digest: Digest,
    origin: SemanticOriginV1,
}

#[derive(Clone)]
struct StoredCommitColumns {
    semantic_revision: i64,
    journal_revision: i64,
    relation_present: i64,
    relation_scope: Vec<u8>,
    event_id: Vec<u8>,
    event_digest: Vec<u8>,
    evidence_digest: Vec<u8>,
    estimator_digest: Vec<u8>,
    incarnation_id: Vec<u8>,
    manifest_digest: Vec<u8>,
    route_digest: Vec<u8>,
    formula_digest: Vec<u8>,
    state_before: Vec<u8>,
    state_digest: Vec<u8>,
    graph_before: Vec<u8>,
    graph_digest: Vec<u8>,
    authority_digest: Vec<u8>,
    telemetry_digest: Vec<u8>,
    snapshot_wire_digest: Vec<u8>,
    graph_wire_digest: Vec<u8>,
    receipt_wire_digest: Vec<u8>,
    telemetry_wire_digest: Vec<u8>,
    origin_digest: Vec<u8>,
    commitment_digest: Vec<u8>,
    transition_kind: String,
}

fn is_nonzero<const N: usize>(value: &[u8; N]) -> bool {
    value.iter().any(|byte| *byte != 0)
}

fn digest_from_vec(value: Vec<u8>, field: &'static str) -> Result<Digest, StoreError> {
    let actual = u64::try_from(value.len()).unwrap_or(u64::MAX);
    value
        .try_into()
        .map_err(|_| StoreError::InvalidStoredDigest { field, actual })
}

fn id_from_vec(value: Vec<u8>, field: &'static str) -> Result<Id128, StoreError> {
    let actual = u64::try_from(value.len()).unwrap_or(u64::MAX);
    value
        .try_into()
        .map_err(|_| StoreError::StorageBudgetExceeded {
            resource: field,
            limit: 16,
            actual,
        })
}

fn semantic_revision_from_sql(raw: i64) -> Result<u64, StoreError> {
    let revision = JournalRevision::try_from(raw)?.get();
    if revision == 0 {
        return Err(StoreError::ContinuityFence("semantic_revision_zero"));
    }
    Ok(revision)
}

fn evidence_values(dimensions: &EvidenceVector) -> [Fixed; 15] {
    [
        dimensions.positive,
        dimensions.affiliation,
        dimensions.harm,
        dimensions.boundary,
        dimensions.repair,
        dimensions.repetition,
        dimensions.new_information,
        dimensions.constraint_instability,
        dimensions.epistemic_conflict,
        dimensions.self_responsibility,
        dimensions.other_responsibility,
        dimensions.hostility,
        dimensions.publicness,
        dimensions.engagement,
        dimensions.rejection,
    ]
}

fn validated_stimulus(event: &CanonicalEvent) -> Result<&UserStimulus, StoreError> {
    let CanonicalEvent::UserStimulus(stimulus) = event else {
        return Err(StoreError::SemanticInvalid("user_stimulus_required"));
    };
    if event.authority() != SourceAuthority::UserObserved
        || !is_nonzero(&stimulus.event_id)
        || !is_nonzero(&stimulus.causal.turn_id)
        || stimulus.observed_at_ms == 0
        || stimulus.evidence.schema_version != 1
        || !is_nonzero(&stimulus.evidence.estimator_digest)
        || !(Fixed::ZERO < stimulus.evidence.estimator_confidence
            && stimulus.evidence.estimator_confidence <= Fixed::ONE)
        || evidence_values(&stimulus.evidence.dimensions)
            .iter()
            .any(|value| *value < Fixed::ZERO || *value > Fixed::ONE)
    {
        return Err(StoreError::SemanticInvalid("canonical_evidence"));
    }
    Ok(stimulus)
}

fn canonical_semantic_nonce_v1(
    event_bytes: &[u8],
    persona_scope: &Digest,
    relation_storage_scope: &Digest,
    incarnation_id: &Digest,
    semantic_base_revision: u64,
) -> Digest {
    let semantic_base = semantic_base_revision.to_le_bytes();
    wire::domain_hash(
        CANONICAL_SEMANTIC_NONCE_DOMAIN_V1,
        &[
            event_bytes,
            persona_scope,
            relation_storage_scope,
            incarnation_id,
            &semantic_base,
        ],
    )
}

fn canonical_estimator_binding_v1(
    event_bytes: &[u8],
    supplied_estimator_digest: &Digest,
    canonical_nonce: &Digest,
    incarnation_id: &Digest,
    semantic_base_revision: u64,
) -> Digest {
    let semantic_base = semantic_base_revision.to_le_bytes();
    wire::domain_hash(
        CANONICAL_ESTIMATOR_BINDING_DOMAIN_V1,
        &[
            event_bytes,
            supplied_estimator_digest,
            canonical_nonce,
            incarnation_id,
            &semantic_base,
        ],
    )
}

fn valid_perception_scope(scope: &ScopeRef) -> bool {
    is_nonzero(&scope.bot_token)
        && is_nonzero(&scope.persona_token)
        && is_nonzero(&scope.session_token)
        && scope
            .relation_token
            .as_ref()
            .map(is_nonzero)
            .unwrap_or(true)
}

fn perception_challenge_nonce_v1(
    secret_commitment: &Digest,
    origin_digest: &Digest,
    created_at_ms: u64,
    expires_at_ms: u64,
) -> Digest {
    let created_at = created_at_ms.to_le_bytes();
    let expires_at = expires_at_ms.to_le_bytes();
    wire::domain_hash(
        PERCEPTION_CHALLENGE_NONCE_DOMAIN_V1,
        &[secret_commitment, origin_digest, &created_at, &expires_at],
    )
}

fn perception_challenge_secret_commitment_v1(secret: &Digest) -> Digest {
    wire::domain_hash(PERCEPTION_CHALLENGE_SECRET_COMMITMENT_DOMAIN_V1, &[secret])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PerceptionChallengeStorageLayoutV1 {
    V2,
    V3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PerceptionChallengeStorageBudgetV1 {
    rows: u64,
    personas: u64,
    aggregate_bytes: u64,
}

impl PerceptionChallengeStorageBudgetV1 {
    fn admit_row(&mut self) -> Result<(), StoreError> {
        let actual = self
            .rows
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "perception_challenge.rows",
                limit: MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "perception_challenge.rows",
            actual,
            MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL,
        )?;
        self.rows = actual;
        Ok(())
    }

    fn admit_persona(&mut self) -> Result<(), StoreError> {
        let actual = self
            .personas
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "perception_challenge.personas",
                limit: MAX_PERCEPTION_CHALLENGE_PERSONAS_GLOBAL,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "perception_challenge.personas",
            actual,
            MAX_PERCEPTION_CHALLENGE_PERSONAS_GLOBAL,
        )?;
        self.personas = actual;
        Ok(())
    }

    fn admit_payload(&mut self, bytes: u64) -> Result<(), StoreError> {
        let actual =
            self.aggregate_bytes
                .checked_add(bytes)
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "perception_challenge.aggregate_bytes",
                    limit: MAX_PERCEPTION_CHALLENGE_BYTES_GLOBAL,
                    actual: u64::MAX,
                })?;
        enforce_byte_budget(
            "perception_challenge.aggregate_bytes",
            actual,
            MAX_PERCEPTION_CHALLENGE_BYTES_GLOBAL,
        )?;
        self.aggregate_bytes = actual;
        Ok(())
    }
}

const PERCEPTION_CHALLENGE_PHASE_ONE_V3_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(request_nonce_digest)='blob'
                 THEN length(request_nonce_digest) ELSE -1 END,
            CASE WHEN typeof(challenge_secret)='blob' THEN length(challenge_secret) ELSE -1 END,
            CASE WHEN typeof(origin_digest)='blob' THEN length(origin_digest) ELSE -1 END,
            CASE WHEN typeof(origin_bytes)='blob' THEN length(origin_bytes) ELSE -1 END,
            CASE WHEN typeof(bot_token)='blob' THEN length(bot_token) ELSE -1 END,
            CASE WHEN typeof(persona_token)='blob' THEN length(persona_token) ELSE -1 END,
            CASE WHEN typeof(origin_event_digest)='blob'
                 THEN length(origin_event_digest) ELSE -1 END,
            CASE WHEN typeof(origin_journal_revision)='integer'
                 THEN origin_journal_revision END,
            CASE WHEN typeof(base_revision)='integer' THEN base_revision END,
            CASE WHEN typeof(incarnation_id)='blob' THEN length(incarnation_id) ELSE -1 END,
            CASE WHEN typeof(manifest_digest)='blob' THEN length(manifest_digest) ELSE -1 END,
            CASE WHEN typeof(created_at_ms)='integer' THEN created_at_ms END,
            CASE WHEN typeof(expires_at_ms)='integer' THEN expires_at_ms END
     FROM perception_challenges NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const PERCEPTION_CHALLENGE_PHASE_ONE_V2_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(request_nonce_digest)='blob'
                 THEN length(request_nonce_digest) ELSE -1 END,
            CASE WHEN typeof(challenge_secret)='blob' THEN length(challenge_secret) ELSE -1 END,
            CASE WHEN typeof(scope_digest)='blob' THEN length(scope_digest) ELSE -1 END,
            CASE WHEN typeof(scope_bytes)='blob' THEN length(scope_bytes) ELSE -1 END,
            CASE WHEN typeof(event_id)='blob' THEN length(event_id) ELSE -1 END,
            CASE WHEN typeof(turn_id)='blob' THEN length(turn_id) ELSE -1 END,
            CASE WHEN typeof(base_revision)='integer' THEN base_revision END,
            CASE WHEN typeof(incarnation_id)='blob' THEN length(incarnation_id) ELSE -1 END,
            CASE WHEN typeof(status)='integer' THEN status END,
            CASE WHEN consumed_event_digest IS NULL THEN -2
                 WHEN typeof(consumed_event_digest)='blob'
                 THEN length(consumed_event_digest) ELSE -1 END,
            CASE WHEN consumed_estimator_digest IS NULL THEN -2
                 WHEN typeof(consumed_estimator_digest)='blob'
                 THEN length(consumed_estimator_digest) ELSE -1 END,
            CASE WHEN consumed_semantic_revision IS NULL THEN NULL
                 WHEN typeof(consumed_semantic_revision)='integer'
                 THEN consumed_semantic_revision END,
            CASE WHEN consumed_semantic_revision IS NULL THEN -2
                 WHEN typeof(consumed_semantic_revision)='integer' THEN 1 ELSE -1 END
     FROM perception_challenges NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

fn required_challenge_blob_length_v1(
    length: i64,
    expected: u64,
    field: &'static str,
    type_fence: &'static str,
) -> Result<(), StoreError> {
    if length < 0 {
        return Err(StoreError::ContinuityFence(type_fence));
    }
    let actual = u64::try_from(length).unwrap_or(u64::MAX);
    if actual != expected {
        return Err(StoreError::InvalidStoredDigest { field, actual });
    }
    Ok(())
}

fn challenge_nonnegative_integer_v1(
    value: Option<i64>,
    fence: &'static str,
) -> Result<u64, StoreError> {
    value
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(StoreError::ContinuityFence(fence))
}

fn scan_perception_challenges_phase_one_v1(
    conn: &Connection,
    layout: PerceptionChallengeStorageLayoutV1,
) -> Result<PerceptionChallengeStorageBudgetV1, StoreError> {
    if conn
        .query_row(
            "SELECT 1 FROM perception_challenges NOT INDEXED
             WHERE rowid<=0 ORDER BY rowid LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some()
    {
        return Err(StoreError::ContinuityFence(
            "perception_challenge_rowid_domain",
        ));
    }

    let sql = match layout {
        PerceptionChallengeStorageLayoutV1::V2 => PERCEPTION_CHALLENGE_PHASE_ONE_V2_SQL,
        PerceptionChallengeStorageLayoutV1::V3 => PERCEPTION_CHALLENGE_PHASE_ONE_V3_SQL,
    };
    let mut budget = PerceptionChallengeStorageBudgetV1::default();
    let mut personas = BTreeSet::new();
    let mut rows_by_persona = BTreeMap::<Digest, u64>::new();
    let mut cursor = 0_i64;
    loop {
        let remaining = MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL.saturating_sub(budget.rows);
        let limit = i64::try_from(
            remaining
                .checked_add(1)
                .unwrap_or(u64::MAX)
                .min(PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1),
        )
        .map_err(|_| StoreError::ContinuityFence("perception_challenge_scan_limit"))?;
        let mut statement = conn.prepare(sql)?;
        let mut rows = statement.query(params![cursor, limit])?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            budget.admit_row()?;
            let rowid: i64 = row.get(0)?;
            let persona_scope = stored_typed_digest(
                row.get(1)?,
                row.get(2)?,
                "perception_challenge.persona",
                "perception_challenge_persona_type",
            )?;
            if !personas.contains(&persona_scope) {
                budget.admit_persona()?;
                personas.insert(persona_scope);
            }
            let persona_rows = rows_by_persona.entry(persona_scope).or_default();
            let actual = persona_rows
                .checked_add(1)
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "perception_challenge.persona_rows",
                    limit: MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA,
                    actual: u64::MAX,
                })?;
            enforce_byte_budget(
                "perception_challenge.persona_rows",
                actual,
                MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA,
            )?;
            *persona_rows = actual;

            required_challenge_blob_length_v1(
                row.get(3)?,
                32,
                "perception_challenge.nonce",
                "perception_challenge_nonce_type",
            )?;
            required_challenge_blob_length_v1(
                row.get(4)?,
                32,
                "perception_challenge.secret_commitment",
                "perception_challenge_secret_type",
            )?;
            match layout {
                PerceptionChallengeStorageLayoutV1::V3 => {
                    required_challenge_blob_length_v1(
                        row.get(5)?,
                        32,
                        "perception_challenge.origin",
                        "perception_challenge_origin_type",
                    )?;
                    let payload_len: i64 = row.get(6)?;
                    if payload_len < 0 {
                        return Err(StoreError::ContinuityFence(
                            "perception_challenge_origin_wire",
                        ));
                    }
                    let payload_len = u64::try_from(payload_len).unwrap_or(u64::MAX);
                    budget.admit_payload(payload_len)?;
                    enforce_byte_budget(
                        "perception_challenge.origin_bytes",
                        payload_len,
                        MAX_PERCEPTION_ORIGIN_BYTES,
                    )?;
                    for (index, expected, field, fence) in [
                        (
                            7,
                            16,
                            "perception_challenge.bot",
                            "perception_challenge_bot_type",
                        ),
                        (
                            8,
                            16,
                            "perception_challenge.persona_token",
                            "perception_challenge_persona_token_type",
                        ),
                        (
                            9,
                            32,
                            "perception_challenge.origin_event",
                            "perception_challenge_origin_event_type",
                        ),
                        (
                            12,
                            32,
                            "perception_challenge.incarnation",
                            "perception_challenge_incarnation_type",
                        ),
                        (
                            13,
                            32,
                            "perception_challenge.manifest",
                            "perception_challenge_manifest_type",
                        ),
                    ] {
                        required_challenge_blob_length_v1(row.get(index)?, expected, field, fence)?;
                    }
                    let origin_revision = challenge_nonnegative_integer_v1(
                        row.get(10)?,
                        "perception_challenge_origin_revision",
                    )?;
                    if origin_revision == 0 {
                        return Err(StoreError::ContinuityFence(
                            "perception_challenge_origin_revision",
                        ));
                    }
                    challenge_nonnegative_integer_v1(
                        row.get(11)?,
                        "perception_challenge_base_revision",
                    )?;
                    let created = challenge_nonnegative_integer_v1(
                        row.get(14)?,
                        "perception_challenge_created",
                    )?;
                    let expires = challenge_nonnegative_integer_v1(
                        row.get(15)?,
                        "perception_challenge_expiry",
                    )?;
                    if created == 0
                        || expires.checked_sub(created) != Some(PERCEPTION_CHALLENGE_TTL_MS)
                    {
                        return Err(StoreError::ContinuityFence("perception_challenge_expiry"));
                    }
                }
                PerceptionChallengeStorageLayoutV1::V2 => {
                    required_challenge_blob_length_v1(
                        row.get(5)?,
                        32,
                        "perception_challenge.scope",
                        "perception_challenge_scope_type",
                    )?;
                    let payload_len: i64 = row.get(6)?;
                    if payload_len < 0 {
                        return Err(StoreError::ContinuityFence(
                            "perception_challenge_scope_wire",
                        ));
                    }
                    let payload_len = u64::try_from(payload_len).unwrap_or(u64::MAX);
                    budget.admit_payload(payload_len)?;
                    if !(49..=65).contains(&payload_len) {
                        return Err(StoreError::ContinuityFence(
                            "perception_challenge_scope_wire",
                        ));
                    }
                    required_challenge_blob_length_v1(
                        row.get(7)?,
                        16,
                        "perception_challenge.event",
                        "perception_challenge_event_type",
                    )?;
                    required_challenge_blob_length_v1(
                        row.get(8)?,
                        16,
                        "perception_challenge.turn",
                        "perception_challenge_turn_type",
                    )?;
                    challenge_nonnegative_integer_v1(
                        row.get(9)?,
                        "perception_challenge_base_revision",
                    )?;
                    required_challenge_blob_length_v1(
                        row.get(10)?,
                        32,
                        "perception_challenge.incarnation",
                        "perception_challenge_incarnation_type",
                    )?;
                    let status = challenge_nonnegative_integer_v1(
                        row.get(11)?,
                        "perception_challenge_status",
                    )?;
                    let consumed_event_len: i64 = row.get(12)?;
                    let consumed_estimator_len: i64 = row.get(13)?;
                    let consumed_revision: Option<i64> = row.get(14)?;
                    let consumed_revision_marker: i64 = row.get(15)?;
                    match status {
                        0 if consumed_event_len == -2
                            && consumed_estimator_len == -2
                            && consumed_revision.is_none()
                            && consumed_revision_marker == -2 => {}
                        1 => {
                            required_challenge_blob_length_v1(
                                consumed_event_len,
                                32,
                                "perception_challenge.consumed_event",
                                "perception_challenge_consumed_event_type",
                            )?;
                            required_challenge_blob_length_v1(
                                consumed_estimator_len,
                                32,
                                "perception_challenge.consumed_estimator",
                                "perception_challenge_consumed_estimator_type",
                            )?;
                            let revision = challenge_nonnegative_integer_v1(
                                consumed_revision,
                                "perception_challenge_consumed_revision",
                            )?;
                            if revision == 0 || consumed_revision_marker != 1 {
                                return Err(StoreError::ContinuityFence(
                                    "perception_challenge_consumed_revision",
                                ));
                            }
                        }
                        _ => {
                            return Err(StoreError::ContinuityFence("perception_challenge_status"))
                        }
                    }
                }
            }
            visited += 1;
            last_rowid = Some(rowid);
        }
        drop(rows);
        drop(statement);
        if visited < limit {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence("perception_challenge_rowset"))?;
    }
    Ok(budget)
}

#[derive(Clone, Debug)]
struct PerceptionChallengeClosedTokenV1 {
    rowid: i64,
    request_nonce_digest: Digest,
    challenge: Option<StoredPerceptionChallengeV1>,
}

#[derive(Clone, Debug)]
struct PerceptionChallengePermitV1 {
    layout: PerceptionChallengeStorageLayoutV1,
    budget: PerceptionChallengeStorageBudgetV1,
    tokens: Vec<PerceptionChallengeClosedTokenV1>,
}

const PERCEPTION_CHALLENGE_PHASE_TWO_V3_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32
                 THEN request_nonce_digest END,
            CASE WHEN typeof(challenge_secret)='blob' AND length(challenge_secret)=32
                 THEN challenge_secret END,
            CASE WHEN typeof(origin_digest)='blob' AND length(origin_digest)=32
                 THEN origin_digest END,
            CASE WHEN typeof(origin_bytes)='blob' AND length(origin_bytes)<=?3
                 THEN origin_bytes END,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(bot_token)='blob' AND length(bot_token)=16 THEN bot_token END,
            CASE WHEN typeof(persona_token)='blob' AND length(persona_token)=16
                 THEN persona_token END,
            CASE WHEN typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32
                 THEN origin_event_digest END,
            CASE WHEN typeof(origin_journal_revision)='integer'
                 THEN origin_journal_revision END,
            CASE WHEN typeof(base_revision)='integer' THEN base_revision END,
            CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32
                 THEN incarnation_id END,
            CASE WHEN typeof(manifest_digest)='blob' AND length(manifest_digest)=32
                 THEN manifest_digest END,
            CASE WHEN typeof(created_at_ms)='integer' THEN created_at_ms END,
            CASE WHEN typeof(expires_at_ms)='integer' THEN expires_at_ms END
     FROM perception_challenges NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const PERCEPTION_CHALLENGE_PHASE_TWO_V2_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32
                 THEN request_nonce_digest END,
            CASE WHEN typeof(challenge_secret)='blob' AND length(challenge_secret)=32
                 THEN challenge_secret END,
            CASE WHEN typeof(scope_digest)='blob' AND length(scope_digest)=32
                 THEN scope_digest END,
            CASE WHEN typeof(scope_bytes)='blob' AND length(scope_bytes) BETWEEN 49 AND 65
                 THEN scope_bytes END,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(event_id)='blob' AND length(event_id)=16 THEN event_id END,
            CASE WHEN typeof(turn_id)='blob' AND length(turn_id)=16 THEN turn_id END,
            CASE WHEN typeof(base_revision)='integer' THEN base_revision END,
            CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32
                 THEN incarnation_id END,
            CASE WHEN typeof(status)='integer' THEN status END,
            CASE WHEN consumed_event_digest IS NULL THEN NULL
                 WHEN typeof(consumed_event_digest)='blob' AND length(consumed_event_digest)=32
                 THEN consumed_event_digest END,
            CASE WHEN consumed_estimator_digest IS NULL THEN NULL
                 WHEN typeof(consumed_estimator_digest)='blob'
                      AND length(consumed_estimator_digest)=32
                 THEN consumed_estimator_digest END,
            CASE WHEN consumed_semantic_revision IS NULL THEN NULL
                 WHEN typeof(consumed_semantic_revision)='integer'
                 THEN consumed_semantic_revision END
     FROM perception_challenges NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

fn required_materialized_digest_v1(
    value: Option<Vec<u8>>,
    field: &'static str,
    fence: &'static str,
) -> Result<Digest, StoreError> {
    digest_from_vec(value.ok_or(StoreError::ContinuityFence(fence))?, field)
}

fn required_materialized_id_v1(
    value: Option<Vec<u8>>,
    field: &'static str,
    fence: &'static str,
) -> Result<Id128, StoreError> {
    id_from_vec(value.ok_or(StoreError::ContinuityFence(fence))?, field)
}

fn decode_legacy_perception_scope_v1(bytes: &[u8]) -> Result<ScopeRef, StoreError> {
    let relation_present = bytes.get(32).copied().ok_or(StoreError::ContinuityFence(
        "perception_challenge_scope_wire",
    ))?;
    let expected_len = match relation_present {
        0 => 49,
        1 => 65,
        _ => {
            return Err(StoreError::ContinuityFence(
                "perception_challenge_scope_wire",
            ))
        }
    };
    if bytes.len() != expected_len {
        return Err(StoreError::ContinuityFence(
            "perception_challenge_scope_wire",
        ));
    }
    let bot_token: Id128 = bytes[0..16]
        .try_into()
        .map_err(|_| StoreError::ContinuityFence("perception_challenge_scope_wire"))?;
    let persona_token: Id128 = bytes[16..32]
        .try_into()
        .map_err(|_| StoreError::ContinuityFence("perception_challenge_scope_wire"))?;
    let (relation_token, session_offset) = if relation_present == 1 {
        (
            Some(
                bytes[33..49]
                    .try_into()
                    .map_err(|_| StoreError::ContinuityFence("perception_challenge_scope_wire"))?,
            ),
            49,
        )
    } else {
        (None, 33)
    };
    let session_token: Id128 = bytes[session_offset..session_offset + 16]
        .try_into()
        .map_err(|_| StoreError::ContinuityFence("perception_challenge_scope_wire"))?;
    let scope = ScopeRef {
        bot_token,
        persona_token,
        relation_token,
        session_token,
    };
    if !valid_perception_scope(&scope) || wire::encode_scope(&scope) != bytes {
        return Err(StoreError::ContinuityFence(
            "perception_challenge_scope_wire",
        ));
    }
    Ok(scope)
}

fn legacy_perception_challenge_nonce_v1(
    secret: &Digest,
    scope_bytes: &[u8],
    scope_digest: &Digest,
    event_id: &Id128,
    turn_id: &Id128,
    base_revision: u64,
    incarnation_id: &Digest,
) -> Digest {
    let base_revision = base_revision.to_le_bytes();
    wire::domain_hash(
        PERCEPTION_CHALLENGE_NONCE_DOMAIN_V1,
        &[
            secret,
            scope_bytes,
            scope_digest,
            event_id,
            turn_id,
            &base_revision,
            incarnation_id,
        ],
    )
}

fn scan_perception_challenges_phase_two_v1(
    conn: &Connection,
    layout: PerceptionChallengeStorageLayoutV1,
    expected: PerceptionChallengeStorageBudgetV1,
) -> Result<Vec<PerceptionChallengeClosedTokenV1>, StoreError> {
    let sql = match layout {
        PerceptionChallengeStorageLayoutV1::V2 => PERCEPTION_CHALLENGE_PHASE_TWO_V2_SQL,
        PerceptionChallengeStorageLayoutV1::V3 => PERCEPTION_CHALLENGE_PHASE_TWO_V3_SQL,
    };
    let mut tokens = Vec::with_capacity(usize::try_from(expected.rows).unwrap_or(0));
    let mut cursor = 0_i64;
    while u64::try_from(tokens.len()).unwrap_or(u64::MAX) < expected.rows {
        let remaining = expected
            .rows
            .saturating_sub(u64::try_from(tokens.len()).unwrap_or(u64::MAX));
        let limit = i64::try_from(remaining.min(PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1))
            .map_err(|_| StoreError::ContinuityFence("perception_challenge_scan_limit"))?;
        let mut statement = conn.prepare(sql)?;
        let params: &[&dyn rusqlite::ToSql] = match layout {
            PerceptionChallengeStorageLayoutV1::V2 => &[&cursor, &limit],
            PerceptionChallengeStorageLayoutV1::V3 => {
                &[&cursor, &limit, &MAX_PERCEPTION_ORIGIN_BYTES]
            }
        };
        let mut rows = statement.query(params)?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            if rowid <= cursor {
                return Err(StoreError::ContinuityFence("perception_challenge_rowset"));
            }
            let nonce = required_materialized_digest_v1(
                row.get(1)?,
                "perception_challenge.nonce",
                "perception_challenge_nonce_type",
            )?;
            let secret = required_materialized_digest_v1(
                row.get(2)?,
                "perception_challenge.secret_commitment",
                "perception_challenge_secret_type",
            )?;
            let challenge = match layout {
                PerceptionChallengeStorageLayoutV1::V3 => {
                    let origin_digest = required_materialized_digest_v1(
                        row.get(3)?,
                        "perception_challenge.origin",
                        "perception_challenge_origin_type",
                    )?;
                    let origin_bytes: Vec<u8> =
                        row.get::<_, Option<Vec<u8>>>(4)?
                            .ok_or(StoreError::ContinuityFence(
                                "perception_challenge_origin_wire",
                            ))?;
                    let persona_scope = required_materialized_digest_v1(
                        row.get(5)?,
                        "perception_challenge.persona",
                        "perception_challenge_persona_type",
                    )?;
                    let bot_token = required_materialized_id_v1(
                        row.get(6)?,
                        "perception_challenge.bot",
                        "perception_challenge_bot_type",
                    )?;
                    let persona_token = required_materialized_id_v1(
                        row.get(7)?,
                        "perception_challenge.persona_token",
                        "perception_challenge_persona_token_type",
                    )?;
                    let origin_event_digest = required_materialized_digest_v1(
                        row.get(8)?,
                        "perception_challenge.origin_event",
                        "perception_challenge_origin_event_type",
                    )?;
                    let origin_journal_revision =
                        semantic_revision_from_sql(row.get::<_, Option<i64>>(9)?.ok_or(
                            StoreError::ContinuityFence("perception_challenge_origin_revision"),
                        )?)?;
                    let base_revision = challenge_nonnegative_integer_v1(
                        row.get(10)?,
                        "perception_challenge_base_revision",
                    )?;
                    let incarnation_id = required_materialized_digest_v1(
                        row.get(11)?,
                        "perception_challenge.incarnation",
                        "perception_challenge_incarnation_type",
                    )?;
                    let manifest_digest = required_materialized_digest_v1(
                        row.get(12)?,
                        "perception_challenge.manifest",
                        "perception_challenge_manifest_type",
                    )?;
                    let created_at_ms = challenge_nonnegative_integer_v1(
                        row.get(13)?,
                        "perception_challenge_created",
                    )?;
                    let expires_at_ms = challenge_nonnegative_integer_v1(
                        row.get(14)?,
                        "perception_challenge_expiry",
                    )?;
                    let origin: PerceptionOriginCommitmentV1 =
                        serde_json::from_slice(&origin_bytes).map_err(|_| {
                            StoreError::ContinuityFence("perception_challenge_origin_wire")
                        })?;
                    let canonical_origin = serde_json::to_vec(&origin).map_err(|_| {
                        StoreError::ContinuityFence("perception_challenge_origin_wire")
                    })?;
                    if canonical_origin != origin_bytes
                        || !(origin.validate_v1() || origin.validate_core_v1())
                        || origin.origin_digest != origin_digest
                        || origin.persona_scope != persona_scope
                        || origin.scope.bot_token != bot_token
                        || origin.scope.persona_token != persona_token
                        || origin.origin_event_digest != origin_event_digest
                        || origin.origin_journal_revision != origin_journal_revision
                        || origin.canonical_base_revision != base_revision
                        || origin.incarnation_id != incarnation_id
                        || origin.manifest_digest != manifest_digest
                        || created_at_ms == 0
                        || expires_at_ms.checked_sub(created_at_ms)
                            != Some(PERCEPTION_CHALLENGE_TTL_MS)
                    {
                        return Err(StoreError::ContinuityFence("perception_challenge_origin"));
                    }
                    let expected_nonce = perception_challenge_nonce_v1(
                        &secret,
                        &origin_digest,
                        created_at_ms,
                        expires_at_ms,
                    );
                    if !constant_time_eq::constant_time_eq_n(&nonce, &expected_nonce) {
                        return Err(StoreError::ContinuityFence("perception_challenge_nonce"));
                    }
                    let authoritative = committed_perception_origin_v1(
                        conn,
                        origin_event_digest,
                        Some(persona_scope),
                    )
                    .map_err(|_| StoreError::ContinuityFence("perception_challenge_binding"))?;
                    if authoritative.origin != origin {
                        return Err(StoreError::ContinuityFence("perception_challenge_origin"));
                    }
                    Some(StoredPerceptionChallengeV1 {
                        secret_commitment: secret,
                        origin,
                        origin_bytes,
                        created_at_ms,
                        expires_at_ms,
                    })
                }
                PerceptionChallengeStorageLayoutV1::V2 => {
                    let scope_digest = required_materialized_digest_v1(
                        row.get(3)?,
                        "perception_challenge.scope",
                        "perception_challenge_scope_type",
                    )?;
                    let scope_bytes: Vec<u8> =
                        row.get::<_, Option<Vec<u8>>>(4)?
                            .ok_or(StoreError::ContinuityFence(
                                "perception_challenge_scope_wire",
                            ))?;
                    let scope = decode_legacy_perception_scope_v1(&scope_bytes)?;
                    let persona_scope = required_materialized_digest_v1(
                        row.get(5)?,
                        "perception_challenge.persona",
                        "perception_challenge_persona_type",
                    )?;
                    let event_id = required_materialized_id_v1(
                        row.get(6)?,
                        "perception_challenge.event",
                        "perception_challenge_event_type",
                    )?;
                    let turn_id = required_materialized_id_v1(
                        row.get(7)?,
                        "perception_challenge.turn",
                        "perception_challenge_turn_type",
                    )?;
                    let base_revision = challenge_nonnegative_integer_v1(
                        row.get(8)?,
                        "perception_challenge_base_revision",
                    )?;
                    let incarnation_id = required_materialized_digest_v1(
                        row.get(9)?,
                        "perception_challenge.incarnation",
                        "perception_challenge_incarnation_type",
                    )?;
                    let status = challenge_nonnegative_integer_v1(
                        row.get(10)?,
                        "perception_challenge_status",
                    )?;
                    let consumed_event = row
                        .get::<_, Option<Vec<u8>>>(11)?
                        .map(|value| digest_from_vec(value, "perception_challenge.consumed_event"))
                        .transpose()?;
                    let consumed_estimator = row
                        .get::<_, Option<Vec<u8>>>(12)?
                        .map(|value| {
                            digest_from_vec(value, "perception_challenge.consumed_estimator")
                        })
                        .transpose()?;
                    let consumed_revision = row
                        .get::<_, Option<i64>>(13)?
                        .map(semantic_revision_from_sql)
                        .transpose()?;
                    if wire::scope_digest(&scope) != scope_digest
                        || wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None)
                            != persona_scope
                    {
                        return Err(StoreError::ContinuityFence(
                            "perception_challenge_scope_wire",
                        ));
                    }
                    let expected_nonce = legacy_perception_challenge_nonce_v1(
                        &secret,
                        &scope_bytes,
                        &scope_digest,
                        &event_id,
                        &turn_id,
                        base_revision,
                        &incarnation_id,
                    );
                    if !constant_time_eq::constant_time_eq_n(&nonce, &expected_nonce)
                        || (status == 0
                            && (consumed_event.is_some()
                                || consumed_estimator.is_some()
                                || consumed_revision.is_some()))
                        || (status == 1
                            && (consumed_event.is_none()
                                || consumed_estimator.is_none()
                                || consumed_revision.is_none()))
                        || status > 1
                    {
                        return Err(StoreError::ContinuityFence(
                            "perception_challenge_consumption",
                        ));
                    }
                    None
                }
            };
            tokens.push(PerceptionChallengeClosedTokenV1 {
                rowid,
                request_nonce_digest: nonce,
                challenge,
            });
            visited += 1;
            last_rowid = Some(rowid);
        }
        drop(rows);
        drop(statement);
        if visited != limit && u64::try_from(tokens.len()).unwrap_or(u64::MAX) < expected.rows {
            return Err(StoreError::ContinuityFence("perception_challenge_rowset"));
        }
        if let Some(last_rowid) = last_rowid {
            cursor = last_rowid;
        }
    }
    // Phase one admitted the complete closed rowset. An extra row now signals
    // either a snapshot violation or an implementation bug; never widen the
    // permit after materialization.
    if conn
        .query_row(
            "SELECT 1 FROM perception_challenges NOT INDEXED WHERE rowid>?1 LIMIT 1",
            params![cursor],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some()
    {
        return Err(StoreError::ContinuityFence("perception_challenge_rowset"));
    }
    Ok(tokens)
}

fn preflight_perception_challenges_v1(
    conn: &Connection,
    layout: PerceptionChallengeStorageLayoutV1,
) -> Result<PerceptionChallengePermitV1, StoreError> {
    let budget = scan_perception_challenges_phase_one_v1(conn, layout)?;
    let tokens = scan_perception_challenges_phase_two_v1(conn, layout, budget)?;
    Ok(PerceptionChallengePermitV1 {
        layout,
        budget,
        tokens,
    })
}

fn closed_perception_challenge_token_v1(
    conn: &Connection,
    request_nonce_digest: Digest,
) -> Result<Option<PerceptionChallengeClosedTokenV1>, StoreError> {
    let permit = preflight_perception_challenges_v1(conn, PerceptionChallengeStorageLayoutV1::V3)?;
    Ok(permit
        .tokens
        .into_iter()
        .find(|token| token.request_nonce_digest == request_nonce_digest))
}

#[derive(Clone, Debug)]
struct StoredPerceptionChallengeV1 {
    secret_commitment: Digest,
    origin: PerceptionOriginCommitmentV1,
    origin_bytes: Vec<u8>,
    created_at_ms: u64,
    expires_at_ms: u64,
}

#[derive(Clone, Debug)]
struct AttestedPerceptionOriginV1 {
    origin: PerceptionOriginCommitmentV1,
    current_journal_revision: u64,
}

fn read_perception_challenge_v1(
    conn: &Connection,
    request_nonce_digest: Digest,
) -> Result<Option<StoredPerceptionChallengeV1>, StoreError> {
    type RawChallenge = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        i64,
        Option<i64>,
        Option<i64>,
    );
    let raw: Option<RawChallenge> = conn
        .query_row(
            "SELECT CASE WHEN typeof(challenge_secret)='blob' AND length(challenge_secret)=32
                         THEN challenge_secret END,
                    CASE WHEN typeof(origin_digest)='blob' AND length(origin_digest)=32
                         THEN origin_digest END,
                    CASE WHEN typeof(origin_bytes)='blob' AND length(origin_bytes)<=?2
                         THEN origin_bytes END,
                    CASE WHEN typeof(origin_bytes)='blob' THEN length(origin_bytes) ELSE -1 END,
                    CASE WHEN typeof(created_at_ms)='integer' THEN created_at_ms END,
                    CASE WHEN typeof(expires_at_ms)='integer' THEN expires_at_ms END
             FROM perception_challenges WHERE request_nonce_digest=?1",
            params![blob(request_nonce_digest), MAX_PERCEPTION_ORIGIN_BYTES],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let secret_commitment = required_materialized_digest_v1(
        raw.0,
        "perception_challenge.secret_commitment",
        "perception_challenge_secret_type",
    )?;
    let origin_digest = required_materialized_digest_v1(
        raw.1,
        "perception_challenge.origin",
        "perception_challenge_origin_type",
    )?;
    let origin_bytes = bounded_typed_value(
        raw.2,
        raw.3,
        MAX_PERCEPTION_ORIGIN_BYTES,
        "perception_challenge.origin_bytes",
        "perception_challenge_origin_wire",
    )?;
    let origin: PerceptionOriginCommitmentV1 = serde_json::from_slice(&origin_bytes)
        .map_err(|_| StoreError::ContinuityFence("perception_challenge_origin_wire"))?;
    let canonical_origin = serde_json::to_vec(&origin)
        .map_err(|_| StoreError::ContinuityFence("perception_challenge_origin_wire"))?;
    if canonical_origin != origin_bytes
        || !(origin.validate_v1() || origin.validate_core_v1())
        || origin.origin_digest != origin_digest
    {
        return Err(StoreError::ContinuityFence("perception_challenge_origin"));
    }
    let created_at_ms = challenge_nonnegative_integer_v1(raw.4, "perception_challenge_created")?;
    let expires_at_ms = challenge_nonnegative_integer_v1(raw.5, "perception_challenge_expiry")?;
    if created_at_ms == 0
        || expires_at_ms <= created_at_ms
        || expires_at_ms.checked_sub(created_at_ms) != Some(PERCEPTION_CHALLENGE_TTL_MS)
    {
        return Err(StoreError::ContinuityFence("perception_challenge_expiry"));
    }
    Ok(Some(StoredPerceptionChallengeV1 {
        secret_commitment,
        origin,
        origin_bytes,
        created_at_ms,
        expires_at_ms,
    }))
}

/// Resolve perception authority exclusively from an InteractionFactBatch that
/// already passed the Host interaction transaction.  The event digest is an
/// opaque locator, not a caller-authored scope or identity claim.
fn committed_perception_origin_v1(
    conn: &Connection,
    origin_event_digest: Digest,
    expected_persona_scope: Option<Digest>,
) -> Result<AttestedPerceptionOriginV1, StoreError> {
    let (scope_bytes, revision_raw): (Option<Vec<u8>>, i64) =
        if let Some(persona_scope) = expected_persona_scope {
            conn.query_row(
                "SELECT CASE WHEN typeof(scope_digest)='blob' AND length(scope_digest)=32
                             THEN scope_digest END,
                        CASE WHEN typeof(revision)='integer' THEN revision ELSE -1 END
                 FROM applied_events
             WHERE scope_digest=?1 AND event_digest=?2",
                params![blob(persona_scope), blob(origin_event_digest)],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(StoreError::SemanticInvalid(
                "perception_origin_not_committed",
            ))?
        } else {
            // The public Host bridge begins with only the committed event digest.
            // Bound that one-time locator scan to two rows. Once the challenge has
            // stored the persona, proposal append/retry uses the PK-qualified path
            // above and remains O(1) in journal history.
            let mut applied_statement = conn.prepare(
                "SELECT CASE WHEN typeof(scope_digest)='blob' AND length(scope_digest)=32
                             THEN scope_digest END,
                        CASE WHEN typeof(revision)='integer' THEN revision ELSE -1 END
                 FROM applied_events
             WHERE event_digest=?1 ORDER BY scope_digest LIMIT 2",
            )?;
            let mut applied_rows = applied_statement.query(params![blob(origin_event_digest)])?;
            let Some(applied) = applied_rows.next()? else {
                return Err(StoreError::SemanticInvalid(
                    "perception_origin_not_committed",
                ));
            };
            let found = (applied.get(0)?, applied.get(1)?);
            if applied_rows.next()?.is_some() {
                return Err(StoreError::SemanticInvalid(
                    "perception_origin_not_committed",
                ));
            }
            found
        };
    let persona_scope = required_materialized_digest_v1(
        scope_bytes,
        "perception_origin.scope",
        "perception_origin_scope_type",
    )?;
    let origin_journal_revision = semantic_revision_from_sql(revision_raw)?;
    let journal = query_bounded_journal_row(
        conn,
        &persona_scope,
        JournalRevision::new(origin_journal_revision),
    )?
    .ok_or(StoreError::ContinuityFence("perception_origin_journal"))?;
    if journal.event_digest != origin_event_digest || journal.event_kind != "interaction_fact_batch"
    {
        return Err(StoreError::ContinuityFence("perception_origin_journal"));
    }
    let decoded = wire::decode_event(&journal.event_bytes)
        .map_err(|_| StoreError::ContinuityFence("perception_origin_event_wire"))?;
    if wire::encode_event(&decoded) != journal.event_bytes
        || wire::event_digest(&decoded) != origin_event_digest
    {
        return Err(StoreError::ContinuityFence("perception_origin_event_wire"));
    }
    let CanonicalEvent::InteractionFactBatch(batch) = decoded else {
        return Err(StoreError::SemanticInvalid("perception_origin_not_inbound"));
    };
    ae_contracts::validate_interaction_fact_batch(&batch)
        .map_err(|_| StoreError::ContinuityFence("perception_origin_interaction"))?;
    let [fact] = batch.facts.as_slice() else {
        return Err(StoreError::SemanticInvalid("perception_origin_ambiguous"));
    };
    if fact.kind != InteractionFactKindV1::InboundObserved
        || fact.source_authority != InteractionSourceAuthorityV1::AstrbotMetadata
        || fact.observed_at_utc_ms == 0
        || !is_nonzero(&fact.source_digest)
        || !is_nonzero(&fact.extractor_digest)
        || (batch.scope.relation_token.is_none()
            && (batch.scope.session_token != batch.causal.turn_id
                || batch.causal.action_id.is_some() || batch.causal.delivery_id.is_some()
                || batch.causal.claim_id.is_some() || fact.value_code.is_some()
                || fact.subject_public_ref.is_some() || fact.consent_terms.is_some()
                || fact.scheduled_at_utc_ms.is_some() || fact.expires_at_utc_ms.is_some()))
    {
        return Err(StoreError::SemanticInvalid("perception_origin_authority"));
    }
    let expected_persona =
        wire::persona_scope_digest(&batch.scope.bot_token, &batch.scope.persona_token, None);
    let relation_scope = wire::persona_scope_digest(
        &batch.scope.bot_token,
        &batch.scope.persona_token,
        batch.scope.relation_token.as_ref(),
    );
    if persona_scope != expected_persona {
        return Err(StoreError::ContinuityFence("perception_origin_scope"));
    }

    type RawFact = (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<String>,
        i64,
    );
    let materialized: RawFact = conn.query_row(
        "SELECT
                CASE WHEN typeof(fact_id)='blob' AND length(fact_id)=16 THEN fact_id END,
                CASE WHEN typeof(event_id)='blob' AND length(event_id)=16 THEN event_id END,
                CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                     THEN persona_scope END,
                CASE WHEN typeof(relation_scope)='blob' AND length(relation_scope)=32
                     THEN relation_scope END,
                CASE WHEN typeof(observed_at_utc_ms)='integer'
                     THEN observed_at_utc_ms ELSE -1 END,
                CASE WHEN typeof(source_digest)='blob' AND length(source_digest)=32
                     THEN source_digest END,
                CASE WHEN typeof(revision)='integer' THEN revision ELSE -1 END,
                CASE WHEN typeof(body_json)='text' AND length(CAST(body_json AS BLOB))<=?2
                     THEN body_json END,
                CASE WHEN typeof(body_json)='text' THEN length(CAST(body_json AS BLOB)) ELSE -1 END
         FROM interaction_fact WHERE fact_id=?1",
        params![blob(fact.fact_id), 262_144_i64],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        },
    )?;
    let body_json = bounded_typed_value(
        materialized.7,
        materialized.8,
        262_144,
        "perception_origin.fact_body",
        "perception_origin_fact_wire",
    )?;
    let body: InteractionFactV1 = serde_json::from_str(&body_json)
        .map_err(|_| StoreError::ContinuityFence("perception_origin_fact_wire"))?;
    let materialized_fact_id = id_from_vec(
        materialized.0.ok_or(StoreError::ContinuityFence(
            "perception_origin_fact_id_type",
        ))?,
        "perception_origin.fact_id",
    )?;
    let materialized_event_id = id_from_vec(
        materialized.1.ok_or(StoreError::ContinuityFence(
            "perception_origin_event_id_type",
        ))?,
        "perception_origin.event_id",
    )?;
    let materialized_persona = digest_from_vec(
        materialized.2.ok_or(StoreError::ContinuityFence(
            "perception_origin_persona_scope_type",
        ))?,
        "perception_origin.persona_scope",
    )?;
    let materialized_relation = digest_from_vec(
        materialized.3.ok_or(StoreError::ContinuityFence(
            "perception_origin_relation_scope_type",
        ))?,
        "perception_origin.relation_scope",
    )?;
    let materialized_source = digest_from_vec(
        materialized.5.ok_or(StoreError::ContinuityFence(
            "perception_origin_source_digest_type",
        ))?,
        "perception_origin.source_digest",
    )?;
    if materialized_fact_id != fact.fact_id
        || materialized_event_id != batch.event_id
        || materialized_persona != persona_scope
        || materialized_relation != relation_scope
        || JournalRevision::try_from(materialized.4)?.get() != fact.observed_at_utc_ms
        || materialized_source != fact.source_digest
        || semantic_revision_from_sql(materialized.6)? != origin_journal_revision
        || body != *fact
    {
        return Err(StoreError::ContinuityFence("perception_origin_fact"));
    }

    let identity = active_identity_for_scope_tx(conn, &batch.scope)?;
    let (current_journal_revision, _) =
        attest_persona_journal_tx(conn, persona_scope, identity.initial_snapshot_digest)?;
    if current_journal_revision < origin_journal_revision {
        return Err(StoreError::ContinuityFence("perception_origin_revision"));
    }
    let provider_code = [wire::interaction_source_authority_code(
        fact.source_authority,
    )];
    let scope_digest = wire::scope_digest(&batch.scope);
    let mut origin = PerceptionOriginCommitmentV1 {
        schema_version: PerceptionOriginCommitmentV1::SCHEMA_VERSION,
        source_authority: SourceAuthority::UserObserved,
        source_digest: fact.source_digest,
        model_digest: fact.extractor_digest,
        provider_digest: wire::domain_hash(
            PERCEPTION_PROVIDER_DOMAIN_V1,
            &[&provider_code, &fact.extractor_digest],
        ),
        scope: batch.scope.clone(),
        scope_digest,
        persona_scope,
        relation_present: batch.scope.relation_token.is_some(),
        relation_scope,
        // The semantic event identity is relation-local and must be
        // reproducible from durable Host input.  The committed inbound turn
        // satisfies both constraints; interaction batch/fact IDs retain their
        // separate database-global uniqueness contract.
        event_id: batch.causal.turn_id,
        turn_id: batch.causal.turn_id,
        observed_at_ms: fact.observed_at_utc_ms,
        canonical_base_revision: origin_journal_revision,
        incarnation_id: identity.incarnation_id,
        manifest_digest: identity.manifest_digest,
        origin_event_digest,
        origin_journal_revision,
        origin_digest: [0; 32],
    };
    origin.origin_digest = origin.digest_v1();
    if !(origin.validate_v1() || origin.validate_core_v1()) {
        return Err(StoreError::ContinuityFence("perception_origin_commitment"));
    }
    Ok(AttestedPerceptionOriginV1 {
        origin,
        current_journal_revision,
    })
}

fn cleanup_perception_challenges_v1(
    tx: &Transaction<'_>,
    authoritative_now_ms: u64,
) -> Result<(), StoreError> {
    // Capture the complete, bounded rowset before planning any mutation. The
    // permit authenticates both rowid and nonce so later deletes cannot widen
    // when a key is corrupt or the query plan changes.
    let permit = preflight_perception_challenges_v1(tx, PerceptionChallengeStorageLayoutV1::V3)?;
    let appraisal_tables_exist: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
                       WHERE type='table' AND name='semantic_appraisal_budget')
                AND EXISTS(SELECT 1 FROM sqlite_schema
                       WHERE type='table' AND name='semantic_appraisal_claim')",
        [],
        |row| row.get(0),
    )?;
    for token in permit.tokens {
        let challenge = token
            .challenge
            .as_ref()
            .ok_or(StoreError::ContinuityFence("perception_challenge_layout"))?;
        if challenge.expires_at_ms > authoritative_now_ms {
            continue;
        }
        if appraisal_tables_exist {
            let pending: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM semantic_appraisal_claim
                 WHERE request_nonce_digest=?1 AND settled_at_ms IS NULL)",
                params![blob(token.request_nonce_digest)],
                |row| row.get(0),
            )?;
            if pending {
                // Appraisal-backed challenges are owned exclusively by the
                // bounded authoritative begin/settle maintenance lane.  The
                // legacy challenge sweeper must neither settle nor delete
                // them with raw wall time.
                continue;
            }
        }
        delete_perception_challenge_token_v1(tx, &token)?;
    }
    Ok(())
}

fn delete_perception_challenge_token_v1(
    conn: &Connection,
    token: &PerceptionChallengeClosedTokenV1,
) -> Result<(), StoreError> {
    let deleted = conn.execute(
        "DELETE FROM perception_challenges WHERE rowid=?1 AND request_nonce_digest=?2",
        params![token.rowid, blob(token.request_nonce_digest)],
    )?;
    if deleted != 1 {
        return Err(StoreError::ContinuityFence("perception_challenge_consume"));
    }
    Ok(())
}

fn enforce_perception_quota_v1(
    tx: &Transaction<'_>,
    persona_scope: Digest,
) -> Result<(), StoreError> {
    // Capacity denial depends only on the already type/length/rowid-closed
    // phase-one rowset. At an exact boundary, reject before materializing and
    // replaying every challenge. Below the boundary, phase two still
    // authenticates the complete rowset before a new challenge may be minted.
    let budget =
        scan_perception_challenges_phase_one_v1(tx, PerceptionChallengeStorageLayoutV1::V3)?;
    let pending_persona = bounded_pending_challenges_for_persona_v1(tx, persona_scope)?;
    if pending_persona >= MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA {
        return Err(StoreError::StorageBudgetExceeded {
            resource: "perception_challenges.persona_pending",
            limit: MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA,
            actual: pending_persona.saturating_add(1),
        });
    }
    let pending_global = budget.rows;
    if pending_global >= MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL {
        return Err(StoreError::StorageBudgetExceeded {
            resource: "perception_challenges.global_pending",
            limit: MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL,
            actual: pending_global.saturating_add(1),
        });
    }
    scan_perception_challenges_phase_two_v1(tx, PerceptionChallengeStorageLayoutV1::V3, budget)?;
    Ok(())
}

fn bounded_pending_challenges_for_persona_v1(
    conn: &Connection,
    persona_scope: Digest,
) -> Result<u64, StoreError> {
    let mut statement = conn.prepare(
        "SELECT rowid FROM perception_challenges NOT INDEXED
         WHERE persona_scope=?1 ORDER BY rowid LIMIT ?2",
    )?;
    let mut rows = statement.query(params![
        blob(persona_scope),
        i64::try_from(MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA.saturating_add(1))
            .unwrap_or(65),
    ])?;
    let mut count = 0_u64;
    let mut previous = 0_i64;
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get(0)?;
        if rowid <= previous {
            return Err(StoreError::ContinuityFence(
                "perception_challenge_rowid_domain",
            ));
        }
        count = count.saturating_add(1);
        previous = rowid;
    }
    Ok(count)
}

fn semantic_appraisal_budget_receipt_v1(
    conn: &Connection,
    persona_scope: Digest,
    utc_day: u64,
) -> Result<SemanticAppraisalBudgetReceiptV1, StoreError> {
    let (limit, charged, reserved, blocked): (i64, i64, i64, i64) = conn.query_row(
        "SELECT daily_token_limit,charged_tokens,reserved_tokens,blocked
         FROM semantic_appraisal_budget WHERE persona_scope=?1 AND utc_day=?2",
        params![
            blob(persona_scope),
            i64::try_from(utc_day)
                .map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?
        ],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let daily_token_limit = u32::try_from(limit)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_limit"))?;
    let charged_tokens = u64::try_from(charged)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_charged"))?;
    let reserved_tokens = u64::try_from(reserved)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_reserved"))?;
    let spent = charged_tokens
        .checked_add(reserved_tokens)
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_budget_overflow",
        ))?;
    Ok(SemanticAppraisalBudgetReceiptV1 {
        utc_day,
        daily_token_limit,
        reserved_tokens: u32::try_from(reserved_tokens).unwrap_or(u32::MAX),
        charged_tokens,
        remaining_tokens: u64::from(daily_token_limit).saturating_sub(spent),
        blocked: blocked == 1,
    })
}

pub(crate) fn semantic_appraisal_authoritative_now_tx_v1(
    tx: &Transaction<'_>,
    wall_now_ms: u64,
) -> Result<u64, StoreError> {
    let stored_raw: i64 = tx
        .query_row(
            "SELECT last_authoritative_now_ms FROM semantic_appraisal_rollup
             WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_rollup_singleton",
        ))?;
    let stored = u64::try_from(stored_raw)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_authoritative_time"))?;
    let authoritative_now_ms = wall_now_ms.max(stored);
    let updated = tx.execute(
        "UPDATE semantic_appraisal_rollup SET last_authoritative_now_ms=?2
         WHERE singleton=1 AND last_authoritative_now_ms=?1",
        params![
            stored_raw,
            i64::try_from(authoritative_now_ms).map_err(|_| {
                StoreError::RevisionOutOfRange {
                    revision: authoritative_now_ms,
                }
            })?,
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_authoritative_time",
        ));
    }
    Ok(authoritative_now_ms)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableSemanticAppraisalReceiptV1 {
    schema_version: u16,
    outcome_code: String,
    usage_known: bool,
    usage_tokens: Option<u32>,
    proposal_identity_digest: Option<Digest>,
    scope_digest: Digest,
    canonical_revision: u64,
    semantic_revision: Option<u64>,
    charged_tokens: u64,
    reply_affect_digest: Digest,
}

#[derive(Clone, Debug)]
struct TerminalSemanticAppraisalV1 {
    receipt: DurableSemanticAppraisalReceiptV1,
    reply_affect: Option<ReplyAffectV1>,
}

fn semantic_appraisal_proposal_identity_v1(
    proposal: Option<&PerceptionProposalV1>,
) -> Option<Digest> {
    proposal.map(PerceptionProposalV1::estimator_digest_v1)
}

fn semantic_appraisal_settlement_identity_v1(
    outcome_code: &str,
    usage: &SemanticAppraisalProviderUsageV1,
    proposal_identity_digest: Option<Digest>,
) -> Digest {
    let known = [u8::from(usage.known)];
    let usage_present = [u8::from(usage.used_tokens.is_some())];
    let usage_tokens = usage.used_tokens.unwrap_or(0).to_le_bytes();
    let proposal_present = [u8::from(proposal_identity_digest.is_some())];
    let proposal = proposal_identity_digest.unwrap_or([0_u8; 32]);
    wire::domain_hash(
        SEMANTIC_APPRAISAL_SETTLEMENT_DOMAIN_V1,
        &[
            outcome_code.as_bytes(),
            &known,
            &usage_present,
            &usage_tokens,
            &proposal_present,
            &proposal,
        ],
    )
}

fn encode_reply_affect_v1(
    reply_affect: &Option<ReplyAffectV1>,
) -> Result<(Vec<u8>, Digest), StoreError> {
    if reply_affect
        .as_ref()
        .is_some_and(|value| !value.validate_v1())
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect",
        ));
    }
    let bytes = serde_json::to_vec(reply_affect)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_reply_affect_wire"))?;
    enforce_byte_budget(
        "semantic_appraisal.reply_affect_bytes",
        u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES,
    )?;
    let digest = wire::domain_hash(SEMANTIC_APPRAISAL_REPLY_AFFECT_DOMAIN_V1, &[&bytes]);
    Ok((bytes, digest))
}

fn encode_terminal_semantic_appraisal_v1(
    outcome_code: &str,
    usage: &SemanticAppraisalProviderUsageV1,
    proposal_identity_digest: Option<Digest>,
    scope: &ScopeRef,
    canonical_revision: u64,
    semantic_revision: Option<u64>,
    charged_tokens: u64,
    reply_affect: &Option<ReplyAffectV1>,
) -> Result<(Digest, Vec<u8>, Digest, Vec<u8>, Digest), StoreError> {
    let settlement_identity =
        semantic_appraisal_settlement_identity_v1(outcome_code, usage, proposal_identity_digest);
    let (reply_bytes, reply_digest) = encode_reply_affect_v1(reply_affect)?;
    let receipt = DurableSemanticAppraisalReceiptV1 {
        schema_version: 1,
        outcome_code: outcome_code.to_owned(),
        usage_known: usage.known,
        usage_tokens: usage.used_tokens,
        proposal_identity_digest,
        scope_digest: wire::scope_digest(scope),
        canonical_revision,
        semantic_revision,
        charged_tokens,
        reply_affect_digest: reply_digest,
    };
    let receipt_bytes = serde_json::to_vec(&receipt)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_terminal_receipt_wire"))?;
    enforce_byte_budget(
        "semantic_appraisal.terminal_receipt_bytes",
        u64::try_from(receipt_bytes.len()).unwrap_or(u64::MAX),
        MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES,
    )?;
    let receipt_digest = wire::domain_hash(
        SEMANTIC_APPRAISAL_TERMINAL_RECEIPT_DOMAIN_V1,
        &[&receipt_bytes],
    );
    Ok((
        settlement_identity,
        reply_bytes,
        reply_digest,
        receipt_bytes,
        receipt_digest,
    ))
}

fn terminal_semantic_appraisal_v1(
    conn: &Connection,
    request: &SemanticAppraisalSettleRequestV1,
    authoritative_now_ms: u64,
) -> Result<Option<TerminalSemanticAppraisalV1>, StoreError> {
    let settled_at_ms: Option<i64> = conn
        .query_row(
            "SELECT settled_at_ms FROM semantic_appraisal_claim
             WHERE request_nonce_digest=?1 AND settled_at_ms IS NOT NULL",
            params![blob(request.request_nonce_digest)],
            |row| row.get(0),
        )
        .optional()?;
    let Some(settled_at_ms) = settled_at_ms else {
        return Ok(None);
    };
    let settled_at_ms = u64::try_from(settled_at_ms)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_settled"))?;
    if !semantic_appraisal_terminal_retry_live_v1(settled_at_ms, authoritative_now_ms) {
        return Ok(None);
    }
    struct RawTerminal {
        persona_scope: Vec<u8>,
        origin_event_digest: Vec<u8>,
        charged_tokens: Option<i64>,
        canonical_revision: Option<i64>,
        semantic_revision: Option<i64>,
        outcome_code: Option<String>,
        usage_known: Option<i64>,
        usage_tokens: Option<i64>,
        proposal_identity_digest: Option<Vec<u8>>,
        settlement_identity_digest: Option<Vec<u8>>,
        reply_affect_bytes: Option<Vec<u8>>,
        reply_affect_digest: Option<Vec<u8>>,
        terminal_receipt_bytes: Option<Vec<u8>>,
        terminal_receipt_digest: Option<Vec<u8>>,
    }
    let raw: Option<RawTerminal> = conn
        .query_row(
            "SELECT persona_scope,origin_event_digest,charged_tokens,canonical_revision,
                    semantic_revision,outcome_code,usage_known,usage_tokens,
                    proposal_identity_digest,settlement_identity_digest,
                    CASE WHEN typeof(reply_affect_bytes)='blob' AND length(reply_affect_bytes)<=?2
                         THEN reply_affect_bytes END,
                    reply_affect_digest,
                    CASE WHEN typeof(terminal_receipt_bytes)='blob' AND length(terminal_receipt_bytes)<=?2
                         THEN terminal_receipt_bytes END,
                    terminal_receipt_digest
             FROM semantic_appraisal_claim
             WHERE request_nonce_digest=?1 AND settled_at_ms IS NOT NULL",
            params![
                blob(request.request_nonce_digest),
                MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES
            ],
            |row| {
                Ok(RawTerminal {
                    persona_scope: row.get(0)?,
                    origin_event_digest: row.get(1)?,
                    charged_tokens: row.get(2)?,
                    canonical_revision: row.get(3)?,
                    semantic_revision: row.get(4)?,
                    outcome_code: row.get(5)?,
                    usage_known: row.get(6)?,
                    usage_tokens: row.get(7)?,
                    proposal_identity_digest: row.get(8)?,
                    settlement_identity_digest: row.get(9)?,
                    reply_affect_bytes: row.get(10)?,
                    reply_affect_digest: row.get(11)?,
                    terminal_receipt_bytes: row.get(12)?,
                    terminal_receipt_digest: row.get(13)?,
                })
            },
        )
        .optional()?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let persona_scope = digest_from_vec(raw.persona_scope, "semantic_appraisal.persona")?;
    let expected_persona =
        wire::persona_scope_digest(&request.scope.bot_token, &request.scope.persona_token, None);
    let origin_event_digest =
        digest_from_vec(raw.origin_event_digest, "semantic_appraisal.origin_event")?;
    let origin_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM applied_events WHERE event_digest=?1 AND scope_digest=?2)",
        params![blob(origin_event_digest), blob(persona_scope)],
        |row| row.get(0),
    )?;
    if persona_scope != expected_persona || !origin_exists {
        return Err(StoreError::SemanticInvalid(
            "semantic_appraisal_claim_context",
        ));
    }
    let outcome_code = raw
        .outcome_code
        .filter(|value| !value.is_empty() && value.len() <= 32)
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_claim_outcome",
        ))?;
    // Exact V2 stores could persist this fully receipted terminal state. It is
    // valid durable history, but it was never a public V3 settlement outcome
    // and therefore cannot be replayed through the V3 request contract.
    if outcome_code == "budget_exhausted" {
        return Ok(None);
    }
    // Version-one terminal rows are deliberately not replayable: they never
    // persisted the Provider usage/proposal identity or the frozen projection.
    if raw.settlement_identity_digest.is_none() {
        return Ok(None);
    }
    let usage_known = match raw.usage_known {
        Some(0) => false,
        Some(1) => true,
        _ => {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_claim_usage",
            ))
        }
    };
    let usage_tokens = raw
        .usage_tokens
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_usage"))
        })
        .transpose()?;
    let stored_usage = SemanticAppraisalProviderUsageV1 {
        known: usage_known,
        used_tokens: usage_tokens,
    };
    if !stored_usage.validate_v1() {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_usage",
        ));
    }
    let stored_proposal = raw
        .proposal_identity_digest
        .map(|value| digest_from_vec(value, "semantic_appraisal.proposal"))
        .transpose()?;
    let stored_identity = digest_from_vec(
        raw.settlement_identity_digest
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_claim_identity",
            ))?,
        "semantic_appraisal.identity",
    )?;
    let computed_stored_identity =
        semantic_appraisal_settlement_identity_v1(&outcome_code, &stored_usage, stored_proposal);
    let request_proposal = semantic_appraisal_proposal_identity_v1(request.proposal.as_ref());
    let request_identity = semantic_appraisal_settlement_identity_v1(
        semantic_appraisal_outcome_code_v1(request.outcome),
        &request.provider_usage,
        request_proposal,
    );
    if stored_identity != computed_stored_identity
        || stored_identity != request_identity
        || outcome_code != semantic_appraisal_outcome_code_v1(request.outcome)
        || stored_usage != request.provider_usage
        || stored_proposal != request_proposal
    {
        return Err(StoreError::SemanticInvalid(
            "semantic_appraisal_settlement_identity",
        ));
    }
    let reply_bytes = raw.reply_affect_bytes.ok_or(StoreError::ContinuityFence(
        "semantic_appraisal_reply_affect_wire",
    ))?;
    let reply_digest = digest_from_vec(
        raw.reply_affect_digest.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect_digest",
        ))?,
        "semantic_appraisal.reply_affect",
    )?;
    if wire::domain_hash(SEMANTIC_APPRAISAL_REPLY_AFFECT_DOMAIN_V1, &[&reply_bytes]) != reply_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect_digest",
        ));
    }
    let reply_affect: Option<ReplyAffectV1> = serde_json::from_slice(&reply_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_reply_affect_wire"))?;
    if serde_json::to_vec(&reply_affect).ok().as_deref() != Some(reply_bytes.as_slice())
        || reply_affect
            .as_ref()
            .is_some_and(|value| !value.validate_v1())
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect_wire",
        ));
    }
    let receipt_bytes = raw
        .terminal_receipt_bytes
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_receipt_wire",
        ))?;
    let receipt_digest = digest_from_vec(
        raw.terminal_receipt_digest
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_terminal_receipt_digest",
            ))?,
        "semantic_appraisal.terminal_receipt",
    )?;
    if wire::domain_hash(
        SEMANTIC_APPRAISAL_TERMINAL_RECEIPT_DOMAIN_V1,
        &[&receipt_bytes],
    ) != receipt_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_receipt_digest",
        ));
    }
    let receipt: DurableSemanticAppraisalReceiptV1 = serde_json::from_slice(&receipt_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_terminal_receipt_wire"))?;
    let charged_tokens = u64::try_from(raw.charged_tokens.ok_or(StoreError::ContinuityFence(
        "semantic_appraisal_claim_charge",
    ))?)
    .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_charge"))?;
    let canonical_revision = u64::try_from(raw.canonical_revision.ok_or(
        StoreError::ContinuityFence("semantic_appraisal_claim_revision"),
    )?)
    .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_revision"))?;
    let semantic_revision = raw
        .semantic_revision
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_semantic"))
        })
        .transpose()?;
    let expected_receipt = DurableSemanticAppraisalReceiptV1 {
        schema_version: 1,
        outcome_code,
        usage_known,
        usage_tokens,
        proposal_identity_digest: stored_proposal,
        scope_digest: wire::scope_digest(&request.scope),
        canonical_revision,
        semantic_revision,
        charged_tokens,
        reply_affect_digest: reply_digest,
    };
    if receipt != expected_receipt
        || serde_json::to_vec(&receipt).ok().as_deref() != Some(receipt_bytes.as_slice())
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_receipt_identity",
        ));
    }
    Ok(Some(TerminalSemanticAppraisalV1 {
        receipt,
        reply_affect,
    }))
}

/// Append a Store-minted challenge and durable token reservation to the same
/// transaction that committed its sole inbound fact.
fn prospective_semantic_appraisal_capacity_v1(
    storage: SemanticAppraisalStorageBudgetV1,
    add_persona: bool,
    add_budget: bool,
    add_claim: bool,
    add_payload_bytes: u64,
) -> bool {
    let fits = |current: u64, add: u64, maximum: u64| {
        current
            .checked_add(add)
            .is_some_and(|actual| actual <= maximum)
    };
    fits(
        storage.personas,
        u64::from(add_persona),
        MAX_SEMANTIC_APPRAISAL_PERSONAS_GLOBAL,
    ) && fits(
        storage.budget_rows,
        u64::from(add_budget),
        MAX_SEMANTIC_APPRAISAL_BUDGET_ROWS_GLOBAL,
    ) && fits(
        storage.claim_rows,
        u64::from(add_claim),
        MAX_SEMANTIC_APPRAISAL_CLAIMS_GLOBAL,
    ) && fits(
        storage.aggregate_bytes,
        add_payload_bytes,
        MAX_SEMANTIC_APPRAISAL_AGGREGATE_BYTES_GLOBAL,
    )
}

fn semantic_appraisal_capacity_deferred_v1(
    budget: Option<SemanticAppraisalBudgetReceiptV1>,
    reply_affect: Option<ReplyAffectV1>,
) -> SemanticAppraisalClaimV1 {
    SemanticAppraisalClaimV1 {
        status: SemanticAppraisalBeginStatusV1::CapacityDeferred,
        challenge: None,
        capacity_reason: Some(SemanticAppraisalCapacityReasonV1::RetentionCapacityUnavailable),
        budget,
        reply_affect,
    }
}

fn semantic_appraisal_claim_capacity_decision_v1(
    storage: SemanticAppraisalStorageBudgetV1,
    add_persona: bool,
    add_budget: bool,
    budget: Option<SemanticAppraisalBudgetReceiptV1>,
    reply_affect: Option<ReplyAffectV1>,
) -> Option<SemanticAppraisalClaimV1> {
    (!prospective_semantic_appraisal_capacity_v1(
        storage,
        add_persona,
        add_budget,
        true,
        MAX_SEMANTIC_APPRAISAL_PENDING_PAYLOAD_BYTES,
    ))
    .then(|| semantic_appraisal_capacity_deferred_v1(budget, reply_affect))
}

/// Append a Store-minted challenge and durable token reservation to the same
/// transaction that committed its sole inbound fact. `None` is the private
/// post-commit retry-expired-or-unknown classification.
pub(crate) fn begin_semantic_appraisal_claim_tx_v1(
    tx: &Transaction<'_>,
    origin_event_digest: Digest,
    daily_token_limit: u32,
    reserved_tokens: u32,
    provider_digest: Digest,
    wall_now_ms: u64,
    replay: bool,
) -> Result<Option<SemanticAppraisalClaimV1>, StoreError> {
    preflight_semantic_appraisal_storage_v1(tx, SemanticAppraisalStorageLayoutV1::V3)?;
    let authoritative_now_ms = semantic_appraisal_authoritative_now_tx_v1(tx, wall_now_ms)?;
    let target_nonce: Option<Vec<u8>> = tx
        .query_row(
            "SELECT request_nonce_digest FROM semantic_appraisal_claim
             WHERE origin_event_digest=?1",
            params![blob(origin_event_digest)],
            |row| row.get(0),
        )
        .optional()?;
    let target_mutations = if let Some(target_nonce) = target_nonce {
        u64::from(maintain_semantic_appraisal_target_nonce_v1(
            tx,
            digest_from_vec(target_nonce, "semantic_appraisal.nonce")?,
            authoritative_now_ms,
        )?)
    } else {
        0
    };
    maintain_semantic_appraisal_retention_v1(tx, authoritative_now_ms, target_mutations)?;
    let storage =
        preflight_semantic_appraisal_storage_v1(tx, SemanticAppraisalStorageLayoutV1::V3)?;
    if target_mutations != 0 {
        return Ok(None);
    }
    let attested = committed_perception_origin_v1(tx, origin_event_digest, None)?;
    let origin = attested.origin;
    let utc_day = authoritative_now_ms / SEMANTIC_APPRAISAL_UTC_DAY_MS;
    let now = i64::try_from(authoritative_now_ms).map_err(|_| StoreError::RevisionOutOfRange {
        revision: authoritative_now_ms,
    })?;
    let day =
        i64::try_from(utc_day).map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?;
    let reply_affect = reply_affect_for_persona_tx_v1(tx, origin.persona_scope)?;
    type ExistingClaim = (Vec<u8>, Vec<u8>, i64, i64, i64, Option<i64>, Option<String>);
    let existing: Option<ExistingClaim> = tx
        .query_row(
            "SELECT request_nonce_digest,provider_digest,reserved_tokens,utc_day,
                    created_at_ms,settled_at_ms,outcome_code
             FROM semantic_appraisal_claim WHERE origin_event_digest=?1",
            params![blob(origin_event_digest)],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()?;
    if let Some((
        nonce,
        stored_provider,
        stored_reserved,
        stored_day,
        created_at_ms,
        settled,
        outcome,
    )) = existing
    {
        if settled.is_some() {
            if outcome.is_none() {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_claim_settlement",
                ));
            }
            return Ok(None);
        }
        if replay {
            return Ok(None);
        }
        let nonce = digest_from_vec(nonce, "semantic_appraisal.nonce")?;
        let created_at_ms = u64::try_from(created_at_ms)
            .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_created"))?;
        if semantic_appraisal_pending_expired_v1(created_at_ms, authoritative_now_ms) {
            let token = closed_perception_challenge_token_v1(tx, nonce)?.ok_or(
                StoreError::ContinuityFence("semantic_appraisal_internal_challenge_missing"),
            )?;
            settle_semantic_appraisal_claim_tx_v1(
                tx,
                &origin.scope,
                nonce,
                "expired",
                &SemanticAppraisalProviderUsageV1 {
                    known: false,
                    used_tokens: None,
                },
                None,
                attested.current_journal_revision,
                None,
                &reply_affect,
                authoritative_now_ms,
            )?;
            delete_perception_challenge_token_v1(tx, &token)?;
            preflight_semantic_appraisal_storage_v1(tx, SemanticAppraisalStorageLayoutV1::V3)?;
            return Ok(None);
        }
        if digest_from_vec(stored_provider, "semantic_appraisal.provider")? != provider_digest
            || u32::try_from(stored_reserved).ok() != Some(reserved_tokens)
        {
            return Err(StoreError::SemanticIdentityConflict);
        }
        let stored_day = u64::try_from(stored_day)
            .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_day"))?;
        let budget = semantic_appraisal_budget_receipt_v1(tx, origin.persona_scope, stored_day)?;
        let stored = read_perception_challenge_v1(tx, nonce)?.ok_or(
            StoreError::ContinuityFence("semantic_appraisal_challenge_missing"),
        )?;
        return Ok(Some(SemanticAppraisalClaimV1 {
            status: SemanticAppraisalBeginStatusV1::Claimed,
            challenge: Some(PerceptionChallengeV1 {
                schema_version: PerceptionChallengeV1::SCHEMA_VERSION,
                origin: stored.origin,
                created_at_ms: stored.created_at_ms,
                expires_at_ms: stored.expires_at_ms,
                request_nonce_digest: nonce,
            }),
            capacity_reason: None,
            budget: Some(budget),
            reply_affect,
        }));
    }
    if replay {
        return Ok(None);
    }

    let budget_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM semantic_appraisal_budget
         WHERE persona_scope=?1 AND utc_day=?2)",
        params![blob(origin.persona_scope), day],
        |row| row.get(0),
    )?;
    let persona_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM semantic_appraisal_budget WHERE persona_scope=?1)",
        params![blob(origin.persona_scope)],
        |row| row.get(0),
    )?;
    let add_budget = !budget_exists;
    let add_persona = add_budget && !persona_exists;
    if !prospective_semantic_appraisal_capacity_v1(storage, add_persona, add_budget, false, 0) {
        return Ok(Some(semantic_appraisal_capacity_deferred_v1(
            None,
            reply_affect,
        )));
    }
    let existing_budget = if budget_exists {
        Some(semantic_appraisal_budget_receipt_v1(
            tx,
            origin.persona_scope,
            utc_day,
        )?)
    } else {
        None
    };
    let provisional_budget = existing_budget
        .clone()
        .unwrap_or(SemanticAppraisalBudgetReceiptV1 {
            utc_day,
            daily_token_limit,
            reserved_tokens: 0,
            charged_tokens: 0,
            remaining_tokens: u64::from(daily_token_limit),
            blocked: false,
        });
    let would_spend = provisional_budget
        .charged_tokens
        .checked_add(u64::from(provisional_budget.reserved_tokens))
        .and_then(|value| value.checked_add(u64::from(reserved_tokens)))
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_budget_overflow",
        ))?;
    if daily_token_limit == 0
        || provisional_budget.daily_token_limit == 0
        || provisional_budget.blocked
        || would_spend > u64::from(provisional_budget.daily_token_limit)
    {
        if add_budget {
            tx.execute(
                "INSERT INTO semantic_appraisal_budget(
                   persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
                   blocked,updated_at_ms,compacted_claim_rows,compacted_charged_tokens,
                   compacted_chain_digest
                 ) VALUES(?1,?2,?3,0,0,0,?4,0,0,zeroblob(32))",
                params![
                    blob(origin.persona_scope),
                    day,
                    i64::from(daily_token_limit),
                    now,
                ],
            )?;
        }
        let budget = semantic_appraisal_budget_receipt_v1(tx, origin.persona_scope, utc_day)?;
        preflight_semantic_appraisal_storage_v1(tx, SemanticAppraisalStorageLayoutV1::V3)?;
        return Ok(Some(SemanticAppraisalClaimV1 {
            status: SemanticAppraisalBeginStatusV1::BudgetExhausted,
            challenge: None,
            capacity_reason: None,
            budget: Some(budget),
            reply_affect,
        }));
    }

    if let Some(deferred) = semantic_appraisal_claim_capacity_decision_v1(
        storage,
        add_persona,
        add_budget,
        existing_budget.clone(),
        reply_affect.clone(),
    ) {
        return Ok(Some(deferred));
    }
    let challenge_permit =
        preflight_perception_challenges_v1(tx, PerceptionChallengeStorageLayoutV1::V3)?;
    let pending_persona = bounded_pending_challenges_for_persona_v1(tx, origin.persona_scope)?;
    if pending_persona >= MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA
        || challenge_permit.budget.rows >= MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL
    {
        return Ok(Some(semantic_appraisal_capacity_deferred_v1(
            existing_budget,
            reply_affect,
        )));
    }
    let origin_bytes = serde_json::to_vec(&origin)
        .map_err(|_| StoreError::ContinuityFence("perception_origin_wire"))?;
    enforce_byte_budget(
        "perception_challenge.origin_bytes",
        u64::try_from(origin_bytes.len()).unwrap_or(u64::MAX),
        MAX_PERCEPTION_ORIGIN_BYTES,
    )?;
    if add_budget {
        tx.execute(
            "INSERT INTO semantic_appraisal_budget(
               persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
               blocked,updated_at_ms,compacted_claim_rows,compacted_charged_tokens,
               compacted_chain_digest
             ) VALUES(?1,?2,?3,0,0,0,?4,0,0,zeroblob(32))",
            params![
                blob(origin.persona_scope),
                day,
                i64::from(daily_token_limit),
                now,
            ],
        )?;
    }
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).map_err(|_| StoreError::PerceptionEntropyUnavailable)?;
    if !is_nonzero(&secret) {
        return Err(StoreError::PerceptionEntropyUnavailable);
    }
    let secret_commitment = perception_challenge_secret_commitment_v1(&secret);
    let expires_at_ms = authoritative_now_ms
        .checked_add(PERCEPTION_CHALLENGE_TTL_MS)
        .ok_or(StoreError::RevisionOutOfRange {
            revision: authoritative_now_ms,
        })?;
    let request_nonce_digest = perception_challenge_nonce_v1(
        &secret_commitment,
        &origin.origin_digest,
        authoritative_now_ms,
        expires_at_ms,
    );
    tx.execute(
        "INSERT INTO perception_challenges(
           request_nonce_digest,challenge_secret,origin_digest,origin_bytes,persona_scope,
           bot_token,persona_token,origin_event_digest,origin_journal_revision,base_revision,
           incarnation_id,manifest_digest,created_at_ms,expires_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
        params![
            blob(request_nonce_digest),
            blob(secret_commitment),
            blob(origin.origin_digest),
            origin_bytes,
            blob(origin.persona_scope),
            blob(origin.scope.bot_token),
            blob(origin.scope.persona_token),
            blob(origin.origin_event_digest),
            JournalRevision::new(origin.origin_journal_revision)
                .to_sqlite()?
                .get(),
            JournalRevision::new(origin.canonical_base_revision)
                .to_sqlite()?
                .get(),
            blob(origin.incarnation_id),
            blob(origin.manifest_digest),
            now,
            i64::try_from(expires_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                revision: expires_at_ms,
            })?,
        ],
    )?;
    tx.execute(
        "INSERT INTO semantic_appraisal_claim(
           request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
           provider_digest,reserved_tokens,created_at_ms
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
        params![
            blob(request_nonce_digest),
            blob(origin.persona_scope),
            day,
            blob(origin_event_digest),
            blob(origin.origin_digest),
            blob(provider_digest),
            i64::from(reserved_tokens),
            now,
        ],
    )?;
    let updated = tx.execute(
        "UPDATE semantic_appraisal_budget
         SET reserved_tokens=reserved_tokens+?3,updated_at_ms=?4
         WHERE persona_scope=?1 AND utc_day=?2 AND blocked=0
           AND charged_tokens+reserved_tokens+?3<=daily_token_limit",
        params![
            blob(origin.persona_scope),
            day,
            i64::from(reserved_tokens),
            now,
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_reservation",
        ));
    }
    let budget = semantic_appraisal_budget_receipt_v1(tx, origin.persona_scope, utc_day)?;
    preflight_semantic_appraisal_storage_v1(tx, SemanticAppraisalStorageLayoutV1::V3)?;
    Ok(Some(SemanticAppraisalClaimV1 {
        status: SemanticAppraisalBeginStatusV1::Claimed,
        challenge: Some(PerceptionChallengeV1 {
            schema_version: PerceptionChallengeV1::SCHEMA_VERSION,
            origin,
            created_at_ms: authoritative_now_ms,
            expires_at_ms,
            request_nonce_digest,
        }),
        capacity_reason: None,
        budget: Some(budget),
        reply_affect,
    }))
}

fn semantic_appraisal_outcome_code_v1(outcome: SemanticAppraisalOutcomeV1) -> &'static str {
    match outcome {
        SemanticAppraisalOutcomeV1::Success => "success",
        SemanticAppraisalOutcomeV1::ProviderError => "provider_error",
        SemanticAppraisalOutcomeV1::Timeout => "timeout",
        SemanticAppraisalOutcomeV1::Malformed => "malformed",
    }
}

fn valid_semantic_appraisal_terminal_outcome_code_v1(outcome_code: &str) -> bool {
    matches!(
        outcome_code,
        "success" | "provider_error" | "timeout" | "malformed" | "expired"
    )
}

fn valid_stored_semantic_appraisal_terminal_outcome_code_v1(outcome_code: &str) -> bool {
    valid_semantic_appraisal_terminal_outcome_code_v1(outcome_code)
        || outcome_code == "budget_exhausted"
}

fn settle_semantic_appraisal_claim_tx_v1(
    tx: &Transaction<'_>,
    scope: &ScopeRef,
    request_nonce_digest: Digest,
    outcome_code: &str,
    usage: &SemanticAppraisalProviderUsageV1,
    proposal: Option<&PerceptionProposalV1>,
    canonical_revision: u64,
    semantic_revision: Option<u64>,
    reply_affect: &Option<ReplyAffectV1>,
    authoritative_now_ms: u64,
) -> Result<u64, StoreError> {
    if !usage.validate_v1() || !valid_perception_scope(scope) {
        return Err(StoreError::SemanticInvalid("semantic_appraisal_settlement"));
    }
    type RawClaim = (Vec<u8>, Vec<u8>, i64, i64, i64, Option<i64>);
    let raw: RawClaim = tx
        .query_row(
            "SELECT persona_scope,origin_event_digest,utc_day,reserved_tokens,created_at_ms,settled_at_ms
             FROM semantic_appraisal_claim WHERE request_nonce_digest=?1",
            params![blob(request_nonce_digest)],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::SemanticInvalid("semantic_appraisal_claim_missing"))?;
    let persona_scope = digest_from_vec(raw.0, "semantic_appraisal.persona")?;
    let expected_persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let origin_event_digest = digest_from_vec(raw.1, "semantic_appraisal.origin_event")?;
    let internal_terminal = matches!(outcome_code, "abandoned" | "expired" | "binding_lost");
    let origin = match committed_perception_origin_v1(tx, origin_event_digest, Some(persona_scope))
    {
        Ok(attested) => attested.origin,
        Err(_) if internal_terminal => {
            let challenge = read_perception_challenge_v1(tx, request_nonce_digest)?.ok_or(
                StoreError::ContinuityFence("semantic_appraisal_internal_challenge_missing"),
            )?;
            if challenge.origin.origin_event_digest != origin_event_digest
                || challenge.origin.persona_scope != persona_scope
            {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_internal_challenge_identity",
                ));
            }
            challenge.origin
        }
        Err(error) => return Err(error),
    };
    if persona_scope != expected_persona || origin.scope != *scope || raw.5.is_some() {
        return Err(StoreError::SemanticInvalid(
            "semantic_appraisal_claim_consumed",
        ));
    }
    let utc_day = u64::try_from(raw.2)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_day"))?;
    let reserved_tokens = u64::try_from(raw.3)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_reserved"))?;
    let created_at_ms = u64::try_from(raw.4)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_created"))?;
    let charged_tokens = match (usage.known, usage.used_tokens) {
        (true, Some(actual)) => u64::from(actual),
        _ => reserved_tokens,
    };
    let blocked = charged_tokens > reserved_tokens;
    let settled_at_ms = authoritative_now_ms.max(created_at_ms);
    let proposal_identity_digest = semantic_appraisal_proposal_identity_v1(proposal);
    let (settlement_identity, reply_bytes, reply_digest, receipt_bytes, receipt_digest) =
        encode_terminal_semantic_appraisal_v1(
            outcome_code,
            usage,
            proposal_identity_digest,
            scope,
            canonical_revision,
            semantic_revision,
            charged_tokens,
            reply_affect,
        )?;
    let changed = tx.execute(
        "UPDATE semantic_appraisal_budget SET
           charged_tokens=charged_tokens+?3,
           reserved_tokens=reserved_tokens-?4,
           blocked=CASE WHEN blocked=1 OR ?5=1 THEN 1 ELSE 0 END,
           updated_at_ms=?6
         WHERE persona_scope=?1 AND utc_day=?2 AND reserved_tokens>=?4",
        params![
            blob(persona_scope),
            i64::try_from(utc_day)
                .map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?,
            i64::try_from(charged_tokens).map_err(|_| StoreError::RevisionOutOfRange {
                revision: charged_tokens
            })?,
            i64::try_from(reserved_tokens).map_err(|_| StoreError::RevisionOutOfRange {
                revision: reserved_tokens
            })?,
            i64::from(blocked),
            i64::try_from(settled_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                revision: settled_at_ms,
            })?,
        ],
    )?;
    if changed != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_settlement",
        ));
    }
    let settled = tx.execute(
        "UPDATE semantic_appraisal_claim SET
           settled_at_ms=?2,charged_tokens=?3,outcome_code=?4,
           canonical_revision=?5,semantic_revision=?6,
           usage_known=?7,usage_tokens=?8,proposal_identity_digest=?9,
           settlement_identity_digest=?10,reply_affect_bytes=?11,
           reply_affect_digest=?12,terminal_receipt_bytes=?13,
           terminal_receipt_digest=?14
         WHERE request_nonce_digest=?1 AND settled_at_ms IS NULL",
        params![
            blob(request_nonce_digest),
            i64::try_from(settled_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                revision: settled_at_ms,
            })?,
            i64::try_from(charged_tokens).map_err(|_| StoreError::RevisionOutOfRange {
                revision: charged_tokens
            })?,
            outcome_code,
            JournalRevision::new(canonical_revision).to_sqlite()?.get(),
            semantic_revision
                .map(|revision| JournalRevision::new(revision)
                    .to_sqlite()
                    .map(|value| value.get()))
                .transpose()?,
            i64::from(usage.known),
            usage.used_tokens.map(i64::from),
            proposal_identity_digest.map(blob),
            blob(settlement_identity),
            reply_bytes,
            blob(reply_digest),
            receipt_bytes,
            blob(receipt_digest),
        ],
    )?;
    if settled != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_settlement",
        ));
    }
    Ok(charged_tokens)
}

fn appraisal_compaction_push_u64_v1(output: &mut Vec<u8>, value: u64) {
    output.extend_from_slice(&value.to_le_bytes());
}

fn appraisal_compaction_push_bytes_v1(
    output: &mut Vec<u8>,
    value: &[u8],
) -> Result<(), StoreError> {
    let length = u64::try_from(value.len())
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_compaction_length"))?;
    appraisal_compaction_push_u64_v1(output, length);
    output.extend_from_slice(value);
    Ok(())
}

fn appraisal_compaction_push_optional_u64_v1(output: &mut Vec<u8>, value: Option<u64>) {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        appraisal_compaction_push_u64_v1(output, value);
    }
}

fn appraisal_compaction_push_optional_bytes_v1(
    output: &mut Vec<u8>,
    value: Option<&[u8]>,
) -> Result<(), StoreError> {
    output.push(u8::from(value.is_some()));
    if let Some(value) = value {
        appraisal_compaction_push_bytes_v1(output, value)?;
    }
    Ok(())
}

fn semantic_appraisal_compaction_chain_v1(
    domain: &[u8],
    prior_root: Digest,
    old_count: u64,
    leaf: Digest,
) -> Digest {
    let mut transition = Vec::with_capacity(72);
    transition.extend_from_slice(&prior_root);
    transition.extend_from_slice(&old_count.to_le_bytes());
    transition.extend_from_slice(&leaf);
    wire::domain_hash(domain, &[&transition])
}

#[derive(Debug)]
struct SemanticAppraisalTerminalCompactionRowV1 {
    rowid: i64,
    request_nonce_digest: Vec<u8>,
    persona_scope: Vec<u8>,
    utc_day: i64,
    origin_event_digest: Vec<u8>,
    origin_digest: Vec<u8>,
    provider_digest: Vec<u8>,
    reserved_tokens: i64,
    created_at_ms: i64,
    settled_at_ms: i64,
    charged_tokens: i64,
    outcome_code: String,
    canonical_revision: i64,
    semantic_revision: Option<i64>,
    usage_known: Option<i64>,
    usage_tokens: Option<i64>,
    proposal_identity_digest: Option<Vec<u8>>,
    settlement_identity_digest: Option<Vec<u8>>,
    reply_affect_bytes: Option<Vec<u8>>,
    reply_affect_digest: Option<Vec<u8>>,
    terminal_receipt_bytes: Option<Vec<u8>>,
    terminal_receipt_digest: Option<Vec<u8>>,
}

fn raw_semantic_appraisal_terminal_compaction_row_v1(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<SemanticAppraisalTerminalCompactionRowV1> {
    Ok(SemanticAppraisalTerminalCompactionRowV1 {
        rowid: row.get(0)?,
        request_nonce_digest: row.get(1)?,
        persona_scope: row.get(2)?,
        utc_day: row.get(3)?,
        origin_event_digest: row.get(4)?,
        origin_digest: row.get(5)?,
        provider_digest: row.get(6)?,
        reserved_tokens: row.get(7)?,
        created_at_ms: row.get(8)?,
        settled_at_ms: row.get(9)?,
        charged_tokens: row.get(10)?,
        outcome_code: row.get(11)?,
        canonical_revision: row.get(12)?,
        semantic_revision: row.get(13)?,
        usage_known: row.get(14)?,
        usage_tokens: row.get(15)?,
        proposal_identity_digest: row.get(16)?,
        settlement_identity_digest: row.get(17)?,
        reply_affect_bytes: row.get(18)?,
        reply_affect_digest: row.get(19)?,
        terminal_receipt_bytes: row.get(20)?,
        terminal_receipt_digest: row.get(21)?,
    })
}

fn read_semantic_appraisal_terminal_compaction_row_v1(
    conn: &Connection,
    rowid: i64,
    request_nonce_digest: Digest,
    settled_at_ms: u64,
) -> Result<SemanticAppraisalTerminalCompactionRowV1, StoreError> {
    conn.query_row(
        "SELECT rowid,request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                origin_digest,provider_digest,reserved_tokens,created_at_ms,settled_at_ms,
                charged_tokens,outcome_code,canonical_revision,semantic_revision,usage_known,
                usage_tokens,proposal_identity_digest,settlement_identity_digest,
                reply_affect_bytes,reply_affect_digest,terminal_receipt_bytes,
                terminal_receipt_digest
         FROM semantic_appraisal_claim
         WHERE rowid=?1 AND request_nonce_digest=?2 AND settled_at_ms=?3",
        params![
            rowid,
            blob(request_nonce_digest),
            i64::try_from(settled_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                revision: settled_at_ms,
            })?,
        ],
        raw_semantic_appraisal_terminal_compaction_row_v1,
    )
    .optional()?
    .ok_or(StoreError::ContinuityFence(
        "semantic_appraisal_terminal_compaction_identity",
    ))
}

fn read_semantic_appraisal_terminal_compaction_row_by_rowid_v1(
    conn: &Connection,
    rowid: i64,
) -> Result<SemanticAppraisalTerminalCompactionRowV1, StoreError> {
    conn.query_row(
        "SELECT rowid,request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                origin_digest,provider_digest,reserved_tokens,created_at_ms,settled_at_ms,
                charged_tokens,outcome_code,canonical_revision,semantic_revision,usage_known,
                usage_tokens,proposal_identity_digest,settlement_identity_digest,
                reply_affect_bytes,reply_affect_digest,terminal_receipt_bytes,
                terminal_receipt_digest
         FROM semantic_appraisal_claim WHERE rowid=?1",
        params![rowid],
        raw_semantic_appraisal_terminal_compaction_row_v1,
    )
    .optional()?
    .ok_or(StoreError::ContinuityFence(
        "semantic_appraisal_claim_rowset",
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SemanticAppraisalTerminalIntegrityV1 {
    Legacy,
    Modern,
}

fn verify_semantic_appraisal_terminal_integrity_v1(
    conn: &Connection,
    row: &SemanticAppraisalTerminalCompactionRowV1,
) -> Result<SemanticAppraisalTerminalIntegrityV1, StoreError> {
    let legacy = row.usage_known.is_none()
        && row.usage_tokens.is_none()
        && row.proposal_identity_digest.is_none()
        && row.settlement_identity_digest.is_none()
        && row.reply_affect_bytes.is_none()
        && row.reply_affect_digest.is_none()
        && row.terminal_receipt_bytes.is_none()
        && row.terminal_receipt_digest.is_none();
    if !valid_stored_semantic_appraisal_terminal_outcome_code_v1(&row.outcome_code) {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_outcome",
        ));
    }
    let persona_scope = digest_from_vec(row.persona_scope.clone(), "semantic_appraisal.persona")?;
    let origin_event_digest = digest_from_vec(
        row.origin_event_digest.clone(),
        "semantic_appraisal.origin_event",
    )?;
    let origin_digest = digest_from_vec(row.origin_digest.clone(), "semantic_appraisal.origin")?;
    let attested = committed_perception_origin_v1(conn, origin_event_digest, Some(persona_scope))?;
    if attested.origin.origin_digest != origin_digest {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_origin",
        ));
    }
    let budget_exhausted = row.outcome_code == "budget_exhausted";
    if budget_exhausted
        && (row.reserved_tokens != 0
            || row.charged_tokens != 0
            || row.settled_at_ms != row.created_at_ms
            || row.semantic_revision.is_some())
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_exhausted_shape",
        ));
    }
    if legacy {
        return Ok(SemanticAppraisalTerminalIntegrityV1::Legacy);
    }
    let usage_known = match row.usage_known {
        Some(0) => false,
        Some(1) => true,
        _ => {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_claim_usage",
            ))
        }
    };
    let usage_tokens = row
        .usage_tokens
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_usage_tokens"))
        })
        .transpose()?;
    let usage = SemanticAppraisalProviderUsageV1 {
        known: usage_known,
        used_tokens: usage_tokens,
    };
    if !usage.validate_v1() {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_usage",
        ));
    }
    let reserved_tokens = u64::try_from(row.reserved_tokens)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_reserved"))?;
    let charged_tokens = u64::try_from(row.charged_tokens)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_charged"))?;
    if charged_tokens != usage_tokens.map(u64::from).unwrap_or(reserved_tokens) {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_usage_charge",
        ));
    }
    let proposal_identity_digest = row
        .proposal_identity_digest
        .clone()
        .map(|value| digest_from_vec(value, "semantic_appraisal.proposal"))
        .transpose()?;
    if budget_exhausted
        && (usage_known || usage_tokens.is_some() || proposal_identity_digest.is_some())
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_exhausted_shape",
        ));
    }
    let stored_settlement_identity = digest_from_vec(
        row.settlement_identity_digest
            .clone()
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_settlement_type",
            ))?,
        "semantic_appraisal.settlement_identity",
    )?;
    if stored_settlement_identity
        != semantic_appraisal_settlement_identity_v1(
            &row.outcome_code,
            &usage,
            proposal_identity_digest,
        )
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_settlement_identity",
        ));
    }
    let reply_bytes = row
        .reply_affect_bytes
        .as_deref()
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect_wire",
        ))?;
    let reply_digest = digest_from_vec(
        row.reply_affect_digest
            .clone()
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_reply_affect_digest",
            ))?,
        "semantic_appraisal.reply_affect",
    )?;
    if wire::domain_hash(SEMANTIC_APPRAISAL_REPLY_AFFECT_DOMAIN_V1, &[reply_bytes]) != reply_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect_digest",
        ));
    }
    let reply_affect: Option<ReplyAffectV1> = serde_json::from_slice(reply_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_reply_affect_wire"))?;
    if serde_json::to_vec(&reply_affect).ok().as_deref() != Some(reply_bytes)
        || reply_affect
            .as_ref()
            .is_some_and(|value| !value.validate_v1())
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_reply_affect_wire",
        ));
    }
    let receipt_bytes =
        row.terminal_receipt_bytes
            .as_deref()
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_terminal_receipt_wire",
            ))?;
    let receipt_digest = digest_from_vec(
        row.terminal_receipt_digest
            .clone()
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_terminal_receipt_digest",
            ))?,
        "semantic_appraisal.terminal_receipt",
    )?;
    if wire::domain_hash(
        SEMANTIC_APPRAISAL_TERMINAL_RECEIPT_DOMAIN_V1,
        &[receipt_bytes],
    ) != receipt_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_receipt_digest",
        ));
    }
    let receipt: DurableSemanticAppraisalReceiptV1 = serde_json::from_slice(receipt_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_terminal_receipt_wire"))?;
    let canonical_revision = u64::try_from(row.canonical_revision)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_revision"))?;
    let semantic_revision = row
        .semantic_revision
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_semantic"))
        })
        .transpose()?;
    let expected_receipt = DurableSemanticAppraisalReceiptV1 {
        schema_version: 1,
        outcome_code: row.outcome_code.clone(),
        usage_known,
        usage_tokens,
        proposal_identity_digest,
        scope_digest: wire::scope_digest(&attested.origin.scope),
        canonical_revision,
        semantic_revision,
        charged_tokens,
        reply_affect_digest: reply_digest,
    };
    if receipt != expected_receipt
        || serde_json::to_vec(&receipt).ok().as_deref() != Some(receipt_bytes)
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_receipt_identity",
        ));
    }
    Ok(SemanticAppraisalTerminalIntegrityV1::Modern)
}

fn semantic_appraisal_terminal_compaction_leaf_v1(
    row: &SemanticAppraisalTerminalCompactionRowV1,
) -> Result<(Digest, u64, u64, Digest), StoreError> {
    let nonce = digest_from_vec(row.request_nonce_digest.clone(), "semantic_appraisal.nonce")?;
    let persona_scope = digest_from_vec(row.persona_scope.clone(), "semantic_appraisal.persona")?;
    let utc_day = u64::try_from(row.utc_day)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_day"))?;
    let reserved_tokens = u64::try_from(row.reserved_tokens)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_reserved"))?;
    let created_at_ms = u64::try_from(row.created_at_ms)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_created"))?;
    let settled_at_ms = u64::try_from(row.settled_at_ms)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_settled"))?;
    let charged_tokens = u64::try_from(row.charged_tokens)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_charged"))?;
    let canonical_revision = u64::try_from(row.canonical_revision)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_revision"))?;
    let semantic_revision = row
        .semantic_revision
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_semantic"))
        })
        .transpose()?;
    let usage_known = row
        .usage_known
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_usage"))
        })
        .transpose()?;
    let usage_tokens = row
        .usage_tokens
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_usage_tokens"))
        })
        .transpose()?;
    let mut receipt = Vec::new();
    appraisal_compaction_push_bytes_v1(&mut receipt, &nonce)?;
    appraisal_compaction_push_bytes_v1(&mut receipt, &persona_scope)?;
    appraisal_compaction_push_u64_v1(&mut receipt, utc_day);
    appraisal_compaction_push_bytes_v1(&mut receipt, &row.origin_event_digest)?;
    appraisal_compaction_push_bytes_v1(&mut receipt, &row.origin_digest)?;
    appraisal_compaction_push_bytes_v1(&mut receipt, &row.provider_digest)?;
    appraisal_compaction_push_u64_v1(&mut receipt, reserved_tokens);
    appraisal_compaction_push_u64_v1(&mut receipt, created_at_ms);
    appraisal_compaction_push_optional_u64_v1(&mut receipt, Some(settled_at_ms));
    appraisal_compaction_push_optional_u64_v1(&mut receipt, Some(charged_tokens));
    appraisal_compaction_push_optional_bytes_v1(&mut receipt, Some(row.outcome_code.as_bytes()))?;
    appraisal_compaction_push_optional_u64_v1(&mut receipt, Some(canonical_revision));
    appraisal_compaction_push_optional_u64_v1(&mut receipt, semantic_revision);
    appraisal_compaction_push_optional_u64_v1(&mut receipt, usage_known);
    appraisal_compaction_push_optional_u64_v1(&mut receipt, usage_tokens);
    for value in [
        row.proposal_identity_digest.as_deref(),
        row.settlement_identity_digest.as_deref(),
        row.reply_affect_bytes.as_deref(),
        row.reply_affect_digest.as_deref(),
        row.terminal_receipt_bytes.as_deref(),
        row.terminal_receipt_digest.as_deref(),
    ] {
        appraisal_compaction_push_optional_bytes_v1(&mut receipt, value)?;
    }
    let leaf = wire::domain_hash(
        SEMANTIC_APPRAISAL_CLAIM_COMPACTION_LEAF_DOMAIN_V1,
        &[&receipt],
    );
    Ok((persona_scope, utc_day, charged_tokens, leaf))
}

fn fold_semantic_appraisal_terminal_v1(
    tx: &Transaction<'_>,
    rowid: i64,
    request_nonce_digest: Digest,
    settled_at_ms: u64,
    authoritative_now_ms: u64,
) -> Result<(), StoreError> {
    let row = read_semantic_appraisal_terminal_compaction_row_v1(
        tx,
        rowid,
        request_nonce_digest,
        settled_at_ms,
    )?;
    verify_semantic_appraisal_terminal_integrity_v1(tx, &row)?;
    let (persona_scope, utc_day, charged_tokens, leaf) =
        semantic_appraisal_terminal_compaction_leaf_v1(&row)?;
    type RawBudget = (i64, i64, i64, i64, i64, i64, i64, Vec<u8>);
    let budget: RawBudget = tx.query_row(
        "SELECT daily_token_limit,charged_tokens,reserved_tokens,blocked,updated_at_ms,
                compacted_claim_rows,compacted_charged_tokens,compacted_chain_digest
         FROM semantic_appraisal_budget WHERE persona_scope=?1 AND utc_day=?2",
        params![
            blob(persona_scope),
            i64::try_from(utc_day)
                .map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?,
        ],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        },
    )?;
    let old_compacted_rows = u64::try_from(budget.5).map_err(|_| {
        StoreError::ContinuityFence("semantic_appraisal_budget_compacted_claim_rows")
    })?;
    let old_compacted_tokens = u64::try_from(budget.6).map_err(|_| {
        StoreError::ContinuityFence("semantic_appraisal_budget_compacted_charged_tokens")
    })?;
    let old_chain = digest_from_vec(budget.7.clone(), "semantic_appraisal.compacted_chain")?;
    let new_compacted_rows =
        old_compacted_rows
            .checked_add(1)
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_budget_compacted_claim_rows",
            ))?;
    let new_compacted_tokens =
        old_compacted_tokens
            .checked_add(charged_tokens)
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_budget_compacted_charged_tokens",
            ))?;
    let new_chain = semantic_appraisal_compaction_chain_v1(
        SEMANTIC_APPRAISAL_CLAIM_COMPACTION_CHAIN_DOMAIN_V1,
        old_chain,
        old_compacted_rows,
        leaf,
    );
    let updated = tx.execute(
        "UPDATE semantic_appraisal_budget SET
           compacted_claim_rows=?11,compacted_charged_tokens=?12,
           compacted_chain_digest=?13,updated_at_ms=?14
         WHERE persona_scope=?1 AND utc_day=?2 AND daily_token_limit=?3
           AND charged_tokens=?4 AND reserved_tokens=?5 AND blocked=?6
           AND updated_at_ms=?7 AND compacted_claim_rows=?8
           AND compacted_charged_tokens=?9 AND compacted_chain_digest=?10",
        params![
            blob(persona_scope),
            i64::try_from(utc_day)
                .map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?,
            budget.0,
            budget.1,
            budget.2,
            budget.3,
            budget.4,
            budget.5,
            budget.6,
            budget.7,
            i64::try_from(new_compacted_rows).map_err(|_| {
                StoreError::ContinuityFence("semantic_appraisal_budget_compacted_claim_rows")
            })?,
            i64::try_from(new_compacted_tokens).map_err(|_| {
                StoreError::ContinuityFence("semantic_appraisal_budget_compacted_charged_tokens")
            })?,
            blob(new_chain),
            i64::try_from(authoritative_now_ms).map_err(|_| {
                StoreError::RevisionOutOfRange {
                    revision: authoritative_now_ms,
                }
            })?,
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_compaction_budget_cas",
        ));
    }
    let deleted = tx.execute(
        "DELETE FROM semantic_appraisal_claim
         WHERE rowid=?1 AND request_nonce_digest=?2 AND settled_at_ms=?3",
        params![
            row.rowid,
            blob(request_nonce_digest),
            i64::try_from(settled_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                revision: settled_at_ms,
            })?,
        ],
    )?;
    if deleted != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_compaction_delete",
        ));
    }
    Ok(())
}

#[derive(Debug)]
struct SemanticAppraisalBudgetCompactionRowV1 {
    rowid: i64,
    persona_scope: Vec<u8>,
    utc_day: i64,
    daily_token_limit: i64,
    charged_tokens: i64,
    reserved_tokens: i64,
    blocked: i64,
    updated_at_ms: i64,
    compacted_claim_rows: i64,
    compacted_charged_tokens: i64,
    compacted_chain_digest: Vec<u8>,
}

fn read_semantic_appraisal_budget_compaction_row_v1(
    conn: &Connection,
    rowid: i64,
    persona_scope: Digest,
    utc_day: u64,
) -> Result<SemanticAppraisalBudgetCompactionRowV1, StoreError> {
    conn.query_row(
        "SELECT rowid,persona_scope,utc_day,daily_token_limit,charged_tokens,
                reserved_tokens,blocked,updated_at_ms,compacted_claim_rows,
                compacted_charged_tokens,compacted_chain_digest
         FROM semantic_appraisal_budget
         WHERE rowid=?1 AND persona_scope=?2 AND utc_day=?3",
        params![
            rowid,
            blob(persona_scope),
            i64::try_from(utc_day)
                .map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?,
        ],
        |row| {
            Ok(SemanticAppraisalBudgetCompactionRowV1 {
                rowid: row.get(0)?,
                persona_scope: row.get(1)?,
                utc_day: row.get(2)?,
                daily_token_limit: row.get(3)?,
                charged_tokens: row.get(4)?,
                reserved_tokens: row.get(5)?,
                blocked: row.get(6)?,
                updated_at_ms: row.get(7)?,
                compacted_claim_rows: row.get(8)?,
                compacted_charged_tokens: row.get(9)?,
                compacted_chain_digest: row.get(10)?,
            })
        },
    )
    .optional()?
    .ok_or(StoreError::ContinuityFence(
        "semantic_appraisal_budget_compaction_identity",
    ))
}

fn semantic_appraisal_budget_compaction_leaf_v1(
    row: &SemanticAppraisalBudgetCompactionRowV1,
) -> Result<(Digest, u64, u64, u64, Digest), StoreError> {
    let persona_scope = digest_from_vec(row.persona_scope.clone(), "semantic_appraisal.persona")?;
    let utc_day = u64::try_from(row.utc_day)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_day"))?;
    let daily_token_limit = u64::try_from(row.daily_token_limit)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_limit"))?;
    let charged_tokens = u64::try_from(row.charged_tokens)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_charged"))?;
    let reserved_tokens = u64::try_from(row.reserved_tokens)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_reserved"))?;
    let blocked = u64::try_from(row.blocked)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_blocked"))?;
    let updated_at_ms = u64::try_from(row.updated_at_ms)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_updated"))?;
    let compacted_claim_rows = u64::try_from(row.compacted_claim_rows).map_err(|_| {
        StoreError::ContinuityFence("semantic_appraisal_budget_compacted_claim_rows")
    })?;
    let compacted_charged_tokens = u64::try_from(row.compacted_charged_tokens).map_err(|_| {
        StoreError::ContinuityFence("semantic_appraisal_budget_compacted_charged_tokens")
    })?;
    let compacted_chain_digest = digest_from_vec(
        row.compacted_chain_digest.clone(),
        "semantic_appraisal.compacted_chain",
    )?;
    let mut receipt = Vec::new();
    appraisal_compaction_push_bytes_v1(&mut receipt, &persona_scope)?;
    for value in [
        utc_day,
        daily_token_limit,
        charged_tokens,
        reserved_tokens,
        blocked,
        updated_at_ms,
        compacted_claim_rows,
        compacted_charged_tokens,
    ] {
        appraisal_compaction_push_u64_v1(&mut receipt, value);
    }
    appraisal_compaction_push_bytes_v1(&mut receipt, &compacted_chain_digest)?;
    let leaf = wire::domain_hash(
        SEMANTIC_APPRAISAL_BUDGET_COMPACTION_LEAF_DOMAIN_V1,
        &[&receipt],
    );
    Ok((
        persona_scope,
        utc_day,
        compacted_claim_rows,
        compacted_charged_tokens,
        leaf,
    ))
}

fn fold_semantic_appraisal_budget_v1(
    tx: &Transaction<'_>,
    rowid: i64,
    persona_scope: Digest,
    utc_day: u64,
) -> Result<(), StoreError> {
    let row = read_semantic_appraisal_budget_compaction_row_v1(tx, rowid, persona_scope, utc_day)?;
    let (persona_scope, utc_day, claim_rows, charged_tokens, leaf) =
        semantic_appraisal_budget_compaction_leaf_v1(&row)?;
    type RawRollup = (i64, i64, i64, Vec<u8>, i64);
    let rollup: RawRollup = tx.query_row(
        "SELECT compacted_budget_rows,compacted_claim_rows,compacted_charged_tokens,
                compacted_chain_digest,last_authoritative_now_ms
         FROM semantic_appraisal_rollup WHERE singleton=1",
        [],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    let old_budget_rows = u64::try_from(rollup.0)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_rollup_budget_rows"))?;
    let old_claim_rows = u64::try_from(rollup.1)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_rollup_claim_rows"))?;
    let old_charged_tokens = u64::try_from(rollup.2)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_rollup_charged_tokens"))?;
    let old_chain = digest_from_vec(rollup.3.clone(), "semantic_appraisal.rollup_chain")?;
    let new_budget_rows = old_budget_rows
        .checked_add(1)
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_rollup_budget_rows",
        ))?;
    let new_claim_rows =
        old_claim_rows
            .checked_add(claim_rows)
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_rollup_claim_rows",
            ))?;
    let new_charged_tokens =
        old_charged_tokens
            .checked_add(charged_tokens)
            .ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_rollup_charged_tokens",
            ))?;
    let new_chain = semantic_appraisal_compaction_chain_v1(
        SEMANTIC_APPRAISAL_BUDGET_COMPACTION_CHAIN_DOMAIN_V1,
        old_chain,
        old_budget_rows,
        leaf,
    );
    let updated = tx.execute(
        "UPDATE semantic_appraisal_rollup SET
           compacted_budget_rows=?6,compacted_claim_rows=?7,
           compacted_charged_tokens=?8,compacted_chain_digest=?9
         WHERE singleton=1 AND compacted_budget_rows=?1 AND compacted_claim_rows=?2
           AND compacted_charged_tokens=?3 AND compacted_chain_digest=?4
           AND last_authoritative_now_ms=?5",
        params![
            rollup.0,
            rollup.1,
            rollup.2,
            rollup.3,
            rollup.4,
            i64::try_from(new_budget_rows).map_err(|_| {
                StoreError::ContinuityFence("semantic_appraisal_rollup_budget_rows")
            })?,
            i64::try_from(new_claim_rows).map_err(|_| {
                StoreError::ContinuityFence("semantic_appraisal_rollup_claim_rows")
            })?,
            i64::try_from(new_charged_tokens).map_err(|_| {
                StoreError::ContinuityFence("semantic_appraisal_rollup_charged_tokens")
            })?,
            blob(new_chain),
        ],
    )?;
    if updated != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_compaction_rollup_cas",
        ));
    }
    let deleted = tx.execute(
        "DELETE FROM semantic_appraisal_budget
         WHERE rowid=?1 AND persona_scope=?2 AND utc_day=?3
           AND daily_token_limit=?4 AND charged_tokens=?5 AND reserved_tokens=?6
           AND blocked=?7 AND updated_at_ms=?8 AND compacted_claim_rows=?9
           AND compacted_charged_tokens=?10 AND compacted_chain_digest=?11",
        params![
            row.rowid,
            blob(persona_scope),
            i64::try_from(utc_day)
                .map_err(|_| StoreError::RevisionOutOfRange { revision: utc_day })?,
            row.daily_token_limit,
            row.charged_tokens,
            row.reserved_tokens,
            row.blocked,
            row.updated_at_ms,
            row.compacted_claim_rows,
            row.compacted_charged_tokens,
            row.compacted_chain_digest,
        ],
    )?;
    if deleted != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_compaction_delete",
        ));
    }
    Ok(())
}

fn expire_semantic_appraisal_pending_nonce_v1(
    tx: &Transaction<'_>,
    request_nonce_digest: Digest,
    authoritative_now_ms: u64,
) -> Result<bool, StoreError> {
    let created_at_ms: Option<i64> = tx
        .query_row(
            "SELECT created_at_ms FROM semantic_appraisal_claim
             WHERE request_nonce_digest=?1 AND settled_at_ms IS NULL",
            params![blob(request_nonce_digest)],
            |row| row.get(0),
        )
        .optional()?;
    let Some(created_at_ms) = created_at_ms else {
        return Ok(false);
    };
    let created_at_ms = u64::try_from(created_at_ms)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_created"))?;
    if !semantic_appraisal_pending_expired_v1(created_at_ms, authoritative_now_ms) {
        return Ok(false);
    }
    let token = closed_perception_challenge_token_v1(tx, request_nonce_digest)?.ok_or(
        StoreError::ContinuityFence("semantic_appraisal_internal_challenge_missing"),
    )?;
    let challenge = token
        .challenge
        .as_ref()
        .ok_or(StoreError::ContinuityFence("perception_challenge_layout"))?;
    let current_raw: i64 = tx.query_row(
        "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
        params![blob(challenge.origin.persona_scope)],
        |row| row.get(0),
    )?;
    let canonical_revision = JournalRevision::try_from(current_raw)?.get();
    let reply_affect = reply_affect_for_persona_tx_v1(tx, challenge.origin.persona_scope)?;
    settle_semantic_appraisal_claim_tx_v1(
        tx,
        &challenge.origin.scope,
        request_nonce_digest,
        "expired",
        &SemanticAppraisalProviderUsageV1 {
            known: false,
            used_tokens: None,
        },
        None,
        canonical_revision,
        None,
        &reply_affect,
        authoritative_now_ms,
    )?;
    delete_perception_challenge_token_v1(tx, &token)?;
    Ok(true)
}

fn maintain_semantic_appraisal_target_nonce_v1(
    tx: &Transaction<'_>,
    request_nonce_digest: Digest,
    authoritative_now_ms: u64,
) -> Result<bool, StoreError> {
    let target: Option<(i64, Option<i64>)> = tx
        .query_row(
            "SELECT rowid,settled_at_ms FROM semantic_appraisal_claim
             WHERE request_nonce_digest=?1",
            params![blob(request_nonce_digest)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((rowid, settled_at_ms)) = target else {
        return Ok(false);
    };
    let Some(settled_at_ms) = settled_at_ms else {
        return expire_semantic_appraisal_pending_nonce_v1(
            tx,
            request_nonce_digest,
            authoritative_now_ms,
        );
    };
    let settled_at_ms = u64::try_from(settled_at_ms)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_settled"))?;
    if semantic_appraisal_terminal_retry_live_v1(settled_at_ms, authoritative_now_ms) {
        return Ok(false);
    }
    fold_semantic_appraisal_terminal_v1(
        tx,
        rowid,
        request_nonce_digest,
        settled_at_ms,
        authoritative_now_ms,
    )?;
    Ok(true)
}

fn maintain_semantic_appraisal_retention_v1(
    tx: &Transaction<'_>,
    authoritative_now_ms: u64,
    initial_mutations: u64,
) -> Result<u64, StoreError> {
    if initial_mutations > SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_retention_mutations",
        ));
    }
    let pending_cutoff = authoritative_now_ms.saturating_sub(SEMANTIC_APPRAISAL_PENDING_TTL_MS);
    let pending_cutoff_sql =
        i64::try_from(pending_cutoff).map_err(|_| StoreError::RevisionOutOfRange {
            revision: pending_cutoff,
        })?;
    let mut mutations = initial_mutations;
    let pending_remaining = SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE
        .checked_sub(mutations)
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_retention_mutations",
        ))?;

    let pending = if pending_remaining == 0 {
        Vec::new()
    } else {
        let mut statement = tx.prepare(
            "SELECT request_nonce_digest
             FROM semantic_appraisal_claim INDEXED BY semantic_appraisal_claim_retention_v3
             WHERE settled_at_ms IS NULL AND created_at_ms<=?1
             ORDER BY settled_at_ms,created_at_ms,request_nonce_digest LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![
                pending_cutoff_sql,
                i64::try_from(pending_remaining).map_err(|_| {
                    StoreError::ContinuityFence("semantic_appraisal_retention_mutations")
                })?,
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        rows.collect::<Result<Vec<_>, _>>()?
    };
    for nonce in pending {
        let nonce = digest_from_vec(nonce, "semantic_appraisal.nonce")?;
        if !expire_semantic_appraisal_pending_nonce_v1(tx, nonce, authoritative_now_ms)? {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_pending_retention_identity",
            ));
        }
        mutations = mutations.checked_add(1).ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_retention_mutations",
        ))?;
    }

    let terminal_remaining = SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE
        .checked_sub(mutations)
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_retention_mutations",
        ))?;
    if terminal_remaining > 0 {
        let terminal_cutoff =
            authoritative_now_ms.saturating_sub(SEMANTIC_APPRAISAL_TERMINAL_EXACT_RETRY_MS);
        let terminal_cutoff_sql =
            i64::try_from(terminal_cutoff).map_err(|_| StoreError::RevisionOutOfRange {
                revision: terminal_cutoff,
            })?;
        let terminals = {
            let mut statement = tx.prepare(
                "SELECT rowid,request_nonce_digest,settled_at_ms
                 FROM semantic_appraisal_claim INDEXED BY semantic_appraisal_claim_retention_v3
                 WHERE settled_at_ms IS NOT NULL AND settled_at_ms<=?1
                 ORDER BY settled_at_ms,created_at_ms,request_nonce_digest LIMIT ?2",
            )?;
            let rows = statement.query_map(
                params![
                    terminal_cutoff_sql,
                    i64::try_from(terminal_remaining).map_err(|_| {
                        StoreError::ContinuityFence("semantic_appraisal_retention_mutations")
                    })?,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (rowid, nonce, settled_at_ms) in terminals {
            let nonce = digest_from_vec(nonce, "semantic_appraisal.nonce")?;
            let settled_at_ms = u64::try_from(settled_at_ms)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_settled"))?;
            fold_semantic_appraisal_terminal_v1(
                tx,
                rowid,
                nonce,
                settled_at_ms,
                authoritative_now_ms,
            )?;
            mutations = mutations.checked_add(1).ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_retention_mutations",
            ))?;
        }
    }

    let budget_remaining = SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE
        .checked_sub(mutations)
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_retention_mutations",
        ))?;
    if budget_remaining > 0 {
        let current_day = authoritative_now_ms / SEMANTIC_APPRAISAL_UTC_DAY_MS;
        let budgets = {
            let mut statement =
                tx.prepare(SEMANTIC_APPRAISAL_BUDGET_RETENTION_CANDIDATES_V1_SQL)?;
            let rows = statement.query_map(
                params![
                    i64::try_from(current_day).map_err(|_| {
                        StoreError::RevisionOutOfRange {
                            revision: current_day,
                        }
                    })?,
                    i64::try_from(budget_remaining).map_err(|_| {
                        StoreError::ContinuityFence("semantic_appraisal_retention_mutations")
                    })?,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        for (rowid, persona_scope, utc_day) in budgets {
            let persona_scope = digest_from_vec(persona_scope, "semantic_appraisal.persona")?;
            let utc_day = u64::try_from(utc_day)
                .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_budget_day"))?;
            let eligible: Option<i64> = tx
                .query_row(
                    SEMANTIC_APPRAISAL_BUDGET_RETENTION_ELIGIBLE_V1_SQL,
                    params![
                        rowid,
                        blob(persona_scope),
                        i64::try_from(utc_day).map_err(|_| {
                            StoreError::RevisionOutOfRange { revision: utc_day }
                        })?,
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            let eligible = eligible.ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_budget_retention_identity",
            ))?;
            if eligible == 0 {
                continue;
            }
            if eligible != 1 {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_budget_retention_eligibility",
                ));
            }
            fold_semantic_appraisal_budget_v1(tx, rowid, persona_scope, utc_day)?;
            mutations = mutations.checked_add(1).ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_retention_mutations",
            ))?;
        }
    }
    if mutations > SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_retention_mutations",
        ));
    }
    Ok(mutations)
}

fn exact_perception_retry_v1(
    tx: &Connection,
    scope: &ScopeRef,
    proposal: &PerceptionProposalV1,
) -> Result<Option<CommittedSemanticV1>, StoreError> {
    type RawReceipt = (Vec<u8>, i64, Vec<u8>, Vec<u8>, Option<Vec<u8>>, i64);
    let row: Option<RawReceipt> = tx
        .query_row(
            "SELECT persona_scope,semantic_revision,perception_proposal_digest,
                    perception_origin_digest,
                    CASE WHEN typeof(perception_origin_bytes)='blob'
                                AND length(perception_origin_bytes)<=?2
                         THEN perception_origin_bytes END,
                    CASE WHEN typeof(perception_origin_bytes)='blob'
                         THEN length(perception_origin_bytes) ELSE -1 END
             FROM semantic_evidence_authority WHERE perception_nonce_digest=?1",
            params![
                blob(proposal.request_nonce_digest),
                MAX_PERCEPTION_ORIGIN_BYTES
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((persona, revision, proposal_digest, origin_digest, origin_bytes, origin_bytes_len)) =
        row
    else {
        return Ok(None);
    };
    let origin_bytes = bounded_typed_value(
        origin_bytes,
        origin_bytes_len,
        MAX_PERCEPTION_ORIGIN_BYTES,
        "perception_receipt.origin_bytes",
        "perception_receipt_origin",
    )?;
    let origin: PerceptionOriginCommitmentV1 = serde_json::from_slice(&origin_bytes)
        .map_err(|_| StoreError::ContinuityFence("perception_receipt_origin"))?;
    let persona = digest_from_vec(persona, "perception_receipt.persona")?;
    let canonical_origin = serde_json::to_vec(&origin)
        .map_err(|_| StoreError::ContinuityFence("perception_receipt_origin"))?;
    let authoritative =
        committed_perception_origin_v1(tx, origin.origin_event_digest, Some(persona))?.origin;
    let authoritative_bytes = serde_json::to_vec(&authoritative)
        .map_err(|_| StoreError::ContinuityFence("perception_receipt_origin"))?;
    if canonical_origin != origin_bytes
        || !(origin.validate_v1() || origin.validate_core_v1())
        || origin != authoritative
        || canonical_origin != authoritative_bytes
        || origin.scope != *scope
        || origin.origin_digest != proposal.origin_digest
        || digest_from_vec(origin_digest, "perception_receipt.origin")? != proposal.origin_digest
        || digest_from_vec(proposal_digest, "perception_receipt.proposal")?
            != proposal.estimator_digest_v1()
    {
        return Err(StoreError::SemanticIdentityConflict);
    }
    let revision = semantic_revision_from_sql(revision)?;
    let committed = read_semantic_commit(tx, persona, revision)?;
    if committed.estimator_digest != Some(proposal.estimator_digest_v1())
        || committed.event_id != origin.event_id
        || committed.relation_scope != origin.relation_present.then_some(origin.relation_scope)
    {
        return Err(StoreError::SemanticIdentityConflict);
    }
    Ok(Some(committed))
}

fn semantic_core_input_error(error: SemanticCoreError) -> StoreError {
    match error {
        SemanticCoreError::InvalidPerceptionProposal => {
            StoreError::SemanticInvalid("semantic_perception_proposal")
        }
        SemanticCoreError::SemanticRevisionOverflow => {
            StoreError::RevisionOutOfRange { revision: u64::MAX }
        }
        _ => StoreError::ContinuityFence("semantic_core_derivation"),
    }
}

fn semantic_core_history_error(_error: SemanticCoreError) -> StoreError {
    StoreError::ContinuityFence("semantic_dynamics_replay")
}

/// Store-owned evidence identity.  It commits the complete canonical event,
/// including persona/relation scope, event ID, vector, confidence and
/// estimator identity.
pub(crate) fn semantic_evidence_digest_v1(event: &CanonicalEvent) -> Result<Digest, StoreError> {
    validated_stimulus(event)?;
    let bytes = wire::encode_event_checked(event)
        .map_err(|_| StoreError::SemanticInvalid("canonical_event_wire"))?;
    Ok(wire::domain_hash(SEMANTIC_EVIDENCE_DOMAIN_V1, &[&bytes]))
}

pub(crate) fn active_identity_for_scope_tx(
    tx: &Connection,
    scope: &ScopeRef,
) -> Result<ActiveSemanticIdentityV1, StoreError> {
    let row: Option<(Vec<u8>, Vec<u8>)> = tx
        .query_row(
            "SELECT
                CASE WHEN typeof(binding.incarnation_id)='blob' AND length(binding.incarnation_id)=32
                     THEN binding.incarnation_id ELSE zeroblob(0) END,
                CASE WHEN typeof(incarnation.manifest_digest)='blob' AND length(incarnation.manifest_digest)=32
                     THEN incarnation.manifest_digest ELSE zeroblob(0) END
             FROM active_bindings AS binding
             JOIN incarnations AS incarnation ON incarnation.incarnation_id=binding.incarnation_id
             WHERE binding.bot_token=?1 AND binding.persona_token=?2",
            params![blob(scope.bot_token), blob(scope.persona_token)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((incarnation, manifest)) = row else {
        return Err(StoreError::GenesisNotFound);
    };
    let incarnation = digest_from_vec(incarnation, "active_binding.incarnation")?;
    let manifest = digest_from_vec(manifest, "active_binding.manifest")?;
    active_identity(tx, scope, incarnation, manifest)
}

pub(crate) fn attest_persona_journal_tx(
    tx: &Connection,
    persona_scope: Digest,
    chain_seed: Digest,
) -> Result<(u64, Digest), StoreError> {
    let current_raw: Option<i64> = tx
        .query_row(
            "SELECT logical_revision FROM journal
             WHERE scope_digest=?1 ORDER BY logical_revision DESC LIMIT 1",
            params![blob(persona_scope)],
            |row| row.get(0),
        )
        .optional()?;
    let Some(current_raw) = current_raw else {
        let orphan_applied: Option<i64> = tx
            .query_row(
                "SELECT revision FROM applied_events WHERE scope_digest=?1 LIMIT 1",
                params![blob(persona_scope)],
                |row| row.get(0),
            )
            .optional()?;
        if orphan_applied.is_some() {
            return Err(StoreError::ContinuityFence("persona_journal_head"));
        }
        return Ok((0, chain_seed));
    };
    let current = semantic_revision_from_sql(current_raw)?;
    let row = query_bounded_journal_row(tx, &persona_scope, JournalRevision::new(current))?
        .ok_or(StoreError::ContinuityFence("persona_journal_head"))?;
    if row.revision != current || row.base_revision.checked_add(1) != Some(current) {
        return Err(StoreError::ContinuityFence("persona_journal_head"));
    }
    let previous_chain = if current == 1 {
        chain_seed
    } else {
        let previous_raw: Option<(Option<Vec<u8>>, i64)> = tx
            .query_row(
                "SELECT CASE WHEN typeof(chain_digest)='blob' AND length(chain_digest)=32
                             THEN chain_digest END,
                        CASE WHEN typeof(chain_digest)='blob' THEN length(chain_digest) ELSE -1 END
                 FROM journal WHERE scope_digest=?1 AND logical_revision=?2",
                params![
                    blob(persona_scope),
                    JournalRevision::new(current - 1).to_sqlite()?.get()
                ],
                |previous| Ok((previous.get(0)?, previous.get(1)?)),
            )
            .optional()?;
        let (bytes, length) =
            previous_raw.ok_or(StoreError::ContinuityFence("persona_journal_predecessor"))?;
        super::stored_digest(bytes, length, "journal.predecessor_chain")?
    };
    let report = ae_continuum::verify_replay(previous_chain, std::slice::from_ref(&row));
    if !report.ok || report.checked != 1 || report.final_revision != current {
        return Err(StoreError::ContinuityFence("persona_journal_head"));
    }
    if row.event_kind != "operational_checkpoint_v1" {
        let receipt = row
            .decode_receipt()
            .map_err(|_| StoreError::ContinuityFence("persona_journal_head_receipt"))?;
        if receipt.next_revision != current || receipt.status != CommitStatus::Committed {
            return Err(StoreError::ContinuityFence("persona_journal_head_receipt"));
        }
    }
    let applied_revision: Option<i64> = tx
        .query_row(
            "SELECT revision FROM applied_events
             WHERE scope_digest=?1 AND event_digest=?2",
            params![blob(persona_scope), blob(row.event_digest)],
            |applied| applied.get(0),
        )
        .optional()?;
    if applied_revision
        .map(JournalRevision::try_from)
        .transpose()?
        .map(JournalRevision::get)
        != Some(current)
    {
        return Err(StoreError::ContinuityFence("persona_journal_head_applied"));
    }
    Ok((current, row.chain_digest))
}

fn semantic_scopes(stimulus: &UserStimulus) -> (Digest, Option<Digest>, Digest) {
    let persona_scope = wire::persona_scope_digest(
        &stimulus.scope.bot_token,
        &stimulus.scope.persona_token,
        None,
    );
    let relation_scope = stimulus.scope.relation_token.as_ref().map(|relation| {
        wire::persona_scope_digest(
            &stimulus.scope.bot_token,
            &stimulus.scope.persona_token,
            Some(relation),
        )
    });
    let relation_storage_scope = relation_scope.unwrap_or(persona_scope);
    (persona_scope, relation_scope, relation_storage_scope)
}

fn exact_semantic_retry_tx(
    tx: &Connection,
    event: &CanonicalEvent,
    persona_scope: Digest,
    relation_scope: Option<Digest>,
) -> Result<Option<CommittedSemanticV1>, StoreError> {
    let stimulus = validated_stimulus(event)?;
    let relation_present = relation_scope.is_some();
    let relation_storage_scope = relation_scope.unwrap_or(persona_scope);
    let stored: Option<(i64, Vec<u8>)> = tx
        .query_row(
            "SELECT semantic_revision,
                CASE WHEN typeof(event_digest)='blob' AND length(event_digest)=32
                     THEN event_digest ELSE zeroblob(0) END
             FROM semantic_evidence_authority
             WHERE persona_scope=?1 AND relation_present=?2 AND relation_scope=?3 AND event_id=?4",
            params![
                blob(persona_scope),
                i64::from(relation_present),
                blob(relation_storage_scope),
                blob(stimulus.event_id),
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((revision, stored_event_digest)) = stored else {
        return Ok(None);
    };
    let revision = semantic_revision_from_sql(revision)?;
    let stored_event_digest = digest_from_vec(stored_event_digest, "semantic_retry.event")?;
    let incoming_event_digest = wire::event_digest(event);
    if stored_event_digest != incoming_event_digest {
        return Err(StoreError::SemanticIdentityConflict);
    }
    let committed = read_semantic_commit(tx, persona_scope, revision)?;
    let canonical_event_bytes = wire::encode_event_checked(event)
        .map_err(|_| StoreError::SemanticInvalid("canonical_event_wire"))?;
    if committed.journal.event_bytes != canonical_event_bytes
        || committed.event_digest != incoming_event_digest
        || committed.event_id != stimulus.event_id
        || committed.relation_scope != relation_scope
    {
        return Err(StoreError::SemanticIdentityConflict);
    }
    let (head_revision, _, _, _) = semantic_head(tx, persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_cursor_missing"))?;
    if head_revision < revision {
        return Err(StoreError::ContinuityFence("semantic_cursor_order"));
    }
    Ok(Some(committed))
}

fn origin_digest(origin: &SemanticOriginV1) -> Digest {
    let revision = origin.source_revision.to_le_bytes();
    let legacy = [u8::from(origin.legacy_migrated)];
    wire::domain_hash(
        SEMANTIC_ORIGIN_DOMAIN_V1,
        &[
            &origin.persona_scope,
            &origin.source_scope_digest,
            &revision,
            &legacy,
            &origin.incarnation_id,
            &origin.manifest_digest,
            &origin.route_digest,
            &origin.formula_digest,
            &origin.state_digest,
            &origin.graph_digest,
        ],
    )
}

fn commit_digest(
    candidate: &PairedSemanticCommitV1,
    relation_present: bool,
    relation_storage_scope: &Digest,
    journal_revision: u64,
    semantic_revision: u64,
    state_before: &Digest,
    graph_before: &Digest,
    authority_digest: &Digest,
    telemetry_digest: &Digest,
    snapshot_wire_digest: &Digest,
    graph_wire_digest: &Digest,
    receipt_wire_digest: &Digest,
    telemetry_wire_digest: &Digest,
    origin_digest: &Digest,
) -> Digest {
    let relation = [u8::from(relation_present)];
    let journal_revision = journal_revision.to_le_bytes();
    let semantic_base = candidate.semantic_base_revision.to_le_bytes();
    let semantic_revision = semantic_revision.to_le_bytes();
    let canonical_nonce = canonical_semantic_nonce_v1(
        &candidate.journal.event_bytes,
        &candidate.persona_scope,
        relation_storage_scope,
        &candidate.incarnation_id,
        candidate.semantic_base_revision,
    );
    let estimator_binding = canonical_estimator_binding_v1(
        &candidate.journal.event_bytes,
        &candidate.estimator_digest,
        &canonical_nonce,
        &candidate.incarnation_id,
        candidate.semantic_base_revision,
    );
    wire::domain_hash(
        SEMANTIC_COMMITMENT_DOMAIN_V1,
        &[
            &candidate.persona_scope,
            &relation,
            relation_storage_scope,
            &journal_revision,
            &semantic_base,
            &semantic_revision,
            &candidate.event_id,
            &candidate.event_digest,
            &candidate.evidence_digest,
            &candidate.estimator_digest,
            &canonical_nonce,
            &estimator_binding,
            &candidate.incarnation_id,
            &candidate.manifest_digest,
            &candidate.route_digest,
            &candidate.formula_digest,
            state_before,
            &candidate.state_digest,
            graph_before,
            &candidate.graph_digest,
            authority_digest,
            telemetry_digest,
            snapshot_wire_digest,
            graph_wire_digest,
            receipt_wire_digest,
            telemetry_wire_digest,
            origin_digest,
        ],
    )
}

const SEMANTIC_SCHEMA_V3_SQL: &str = r#"
        CREATE TABLE IF NOT EXISTS semantic_origins (
            persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            source_scope_digest BLOB NOT NULL CHECK(typeof(source_scope_digest)='blob' AND length(source_scope_digest)=32),
            source_revision INTEGER NOT NULL CHECK(typeof(source_revision)='integer' AND source_revision>=0),
            legacy_migrated INTEGER NOT NULL CHECK(typeof(legacy_migrated)='integer' AND legacy_migrated IN (0,1)),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
            UNIQUE(persona_scope, origin_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_commits (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            journal_revision INTEGER NOT NULL CHECK(typeof(journal_revision)='integer' AND journal_revision>0),
            relation_present INTEGER NOT NULL CHECK(typeof(relation_present)='integer' AND relation_present IN (0,1)),
            relation_scope BLOB NOT NULL CHECK(typeof(relation_scope)='blob' AND length(relation_scope)=32),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            state_before BLOB NOT NULL CHECK(typeof(state_before)='blob' AND length(state_before)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            graph_before BLOB NOT NULL CHECK(typeof(graph_before)='blob' AND length(graph_before)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            authority_digest BLOB NOT NULL CHECK(typeof(authority_digest)='blob' AND length(authority_digest)=32),
            telemetry_digest BLOB NOT NULL CHECK(typeof(telemetry_digest)='blob' AND length(telemetry_digest)=32),
            snapshot_wire_digest BLOB NOT NULL CHECK(typeof(snapshot_wire_digest)='blob' AND length(snapshot_wire_digest)=32),
            graph_wire_digest BLOB NOT NULL CHECK(typeof(graph_wire_digest)='blob' AND length(graph_wire_digest)=32),
            receipt_wire_digest BLOB NOT NULL CHECK(typeof(receipt_wire_digest)='blob' AND length(receipt_wire_digest)=32),
            telemetry_wire_digest BLOB NOT NULL CHECK(typeof(telemetry_wire_digest)='blob' AND length(telemetry_wire_digest)=32),
            origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            PRIMARY KEY(persona_scope, semantic_revision),
            UNIQUE(persona_scope, commitment_digest),
            UNIQUE(persona_scope, journal_revision),
            UNIQUE(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest),
            UNIQUE(persona_scope, semantic_revision, relation_present, relation_scope, event_id, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest),
            CHECK(relation_present=1 OR relation_scope=persona_scope),
            FOREIGN KEY(persona_scope, origin_digest) REFERENCES semantic_origins(persona_scope, origin_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_cursor (
            persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            journal_revision INTEGER NOT NULL CHECK(typeof(journal_revision)='integer' AND journal_revision>0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            FOREIGN KEY(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
              REFERENCES semantic_commits(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_snapshots (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            snapshot_bytes BLOB NOT NULL CHECK(typeof(snapshot_bytes)='blob' AND length(snapshot_bytes)<=16777216),
            PRIMARY KEY(persona_scope, semantic_revision),
            FOREIGN KEY(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
              REFERENCES semantic_commits(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_graphs (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            graph_bytes BLOB NOT NULL CHECK(typeof(graph_bytes)='blob' AND length(graph_bytes)<=16777216),
            PRIMARY KEY(persona_scope, semantic_revision),
            FOREIGN KEY(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
              REFERENCES semantic_commits(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_receipts (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            receipt_bytes BLOB NOT NULL CHECK(typeof(receipt_bytes)='blob' AND length(receipt_bytes)<=65536),
            PRIMARY KEY(persona_scope, semantic_revision),
            FOREIGN KEY(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
              REFERENCES semantic_commits(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_telemetry (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            telemetry_bytes BLOB NOT NULL CHECK(typeof(telemetry_bytes)='blob' AND length(telemetry_bytes)<=65536),
            PRIMARY KEY(persona_scope, semantic_revision),
            FOREIGN KEY(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
              REFERENCES semantic_commits(persona_scope, semantic_revision, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_evidence_authority (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            relation_present INTEGER NOT NULL CHECK(typeof(relation_present)='integer' AND relation_present IN (0,1)),
            relation_scope BLOB NOT NULL CHECK(typeof(relation_scope)='blob' AND length(relation_scope)=32),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            route_digest BLOB NOT NULL CHECK(typeof(route_digest)='blob' AND length(route_digest)=32),
            formula_digest BLOB NOT NULL CHECK(typeof(formula_digest)='blob' AND length(formula_digest)=32),
            graph_digest BLOB NOT NULL CHECK(typeof(graph_digest)='blob' AND length(graph_digest)=32),
            state_digest BLOB NOT NULL CHECK(typeof(state_digest)='blob' AND length(state_digest)=32),
            evidence_digest BLOB NOT NULL CHECK(typeof(evidence_digest)='blob' AND length(evidence_digest)=32),
            estimator_digest BLOB NOT NULL CHECK(typeof(estimator_digest)='blob' AND length(estimator_digest)=32),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            commitment_digest BLOB NOT NULL CHECK(typeof(commitment_digest)='blob' AND length(commitment_digest)=32),
            evidence_bytes BLOB NOT NULL CHECK(typeof(evidence_bytes)='blob' AND length(evidence_bytes)<=262144),
            perception_nonce_digest BLOB,
            perception_proposal_digest BLOB,
            perception_origin_digest BLOB,
            perception_origin_bytes BLOB,
            PRIMARY KEY(persona_scope, relation_present, relation_scope, event_id),
            UNIQUE(persona_scope, semantic_revision),
            CHECK(relation_present=1 OR relation_scope=persona_scope),
            CHECK(
              (perception_nonce_digest IS NULL AND perception_proposal_digest IS NULL
                AND perception_origin_digest IS NULL AND perception_origin_bytes IS NULL)
              OR
              (typeof(perception_nonce_digest)='blob' AND length(perception_nonce_digest)=32
                AND typeof(perception_proposal_digest)='blob' AND length(perception_proposal_digest)=32
                AND typeof(perception_origin_digest)='blob' AND length(perception_origin_digest)=32
                AND typeof(perception_origin_bytes)='blob' AND length(perception_origin_bytes)<=4096)
            ),
            FOREIGN KEY(persona_scope, semantic_revision, relation_present, relation_scope, event_id, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
              REFERENCES semantic_commits(persona_scope, semantic_revision, relation_present, relation_scope, event_id, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest, state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest)
        );

        CREATE TABLE IF NOT EXISTS perception_challenges (
            request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
            challenge_secret BLOB NOT NULL CHECK(typeof(challenge_secret)='blob' AND length(challenge_secret)=32),
            origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
            origin_bytes BLOB NOT NULL CHECK(typeof(origin_bytes)='blob' AND length(origin_bytes)<=4096),
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            bot_token BLOB NOT NULL CHECK(typeof(bot_token)='blob' AND length(bot_token)=16),
            persona_token BLOB NOT NULL CHECK(typeof(persona_token)='blob' AND length(persona_token)=16),
            origin_event_digest BLOB NOT NULL CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
            origin_journal_revision INTEGER NOT NULL CHECK(typeof(origin_journal_revision)='integer' AND origin_journal_revision>0),
            base_revision INTEGER NOT NULL CHECK(typeof(base_revision)='integer' AND base_revision>=0),
            incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
            manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
            created_at_ms INTEGER NOT NULL CHECK(typeof(created_at_ms)='integer' AND created_at_ms>0),
            expires_at_ms INTEGER NOT NULL CHECK(typeof(expires_at_ms)='integer' AND expires_at_ms>created_at_ms),
            UNIQUE(origin_digest,base_revision,incarnation_id),
            FOREIGN KEY(persona_scope,origin_event_digest)
              REFERENCES applied_events(scope_digest,event_digest)
        );

        CREATE TABLE IF NOT EXISTS semantic_budget_checkpoint (
            persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            row_count INTEGER NOT NULL CHECK(typeof(row_count)='integer' AND row_count>0),
            aggregate_bytes INTEGER NOT NULL CHECK(typeof(aggregate_bytes)='integer' AND aggregate_bytes>=0),
            head_revision INTEGER NOT NULL CHECK(typeof(head_revision)='integer' AND head_revision>0),
            head_commitment_digest BLOB NOT NULL CHECK(typeof(head_commitment_digest)='blob' AND length(head_commitment_digest)=32),
            FOREIGN KEY(persona_scope,head_revision,head_commitment_digest)
              REFERENCES semantic_commits(persona_scope,semantic_revision,commitment_digest)
        );

        CREATE UNIQUE INDEX IF NOT EXISTS semantic_commit_journal_identity_v1
          ON semantic_commits(persona_scope, journal_revision);
        CREATE UNIQUE INDEX IF NOT EXISTS semantic_commit_relation_identity_v1
          ON semantic_commits(persona_scope, semantic_revision, relation_present, relation_scope,
             event_id, incarnation_id, manifest_digest, route_digest, formula_digest, graph_digest,
             state_digest, evidence_digest, estimator_digest, event_digest, commitment_digest);
        CREATE UNIQUE INDEX IF NOT EXISTS semantic_commit_checkpoint_identity_v1
          ON semantic_commits(persona_scope,semantic_revision,commitment_digest);
        CREATE INDEX IF NOT EXISTS semantic_evidence_relation_revision
          ON semantic_evidence_authority(persona_scope, relation_present, relation_scope, semantic_revision);
        CREATE INDEX IF NOT EXISTS perception_challenge_context_v1
          ON perception_challenges(origin_digest,base_revision,incarnation_id);
        CREATE INDEX IF NOT EXISTS perception_challenge_persona_pending_v1
          ON perception_challenges(persona_scope);
        CREATE INDEX IF NOT EXISTS perception_challenge_expiry_v1
          ON perception_challenges(expires_at_ms);
        CREATE UNIQUE INDEX IF NOT EXISTS semantic_evidence_perception_nonce_v1
          ON semantic_evidence_authority(perception_nonce_digest)
          WHERE perception_nonce_digest IS NOT NULL;
        "#;

const SEMANTIC_SCHEMA_VERSION_V2: u8 = 2;
const SEMANTIC_SCHEMA_VERSION_V3: u8 = 3;
const SEMANTIC_SCHEMA_VERSION_V4: u8 = 4;
const SEMANTIC_SCHEMA_VERSION_V5: u8 = 5;
const SEMANTIC_SCHEMA_CORE_TABLE_COUNT: u64 = 8;
const SEMANTIC_SCHEMA_V3_TABLE_COUNT: u64 = 10;
const SEMANTIC_SCHEMA_TABLE_COUNT: u64 = 11;
const MAX_SEMANTIC_SCHEMA_NAME_BYTES: u64 = 96;
const MAX_SEMANTIC_SCHEMA_SQL_BYTES: u64 = 64 * 1024;
const MAX_SEMANTIC_SCHEMA_OBJECTS: u64 = 64;
const MAX_SQLITE_SCHEMA_OBJECTS_GLOBAL: u64 = 4_096;
const SEMANTIC_COMMON_COMMIT_KEY: [&str; 12] = [
    "persona_scope",
    "semantic_revision",
    "incarnation_id",
    "manifest_digest",
    "route_digest",
    "formula_digest",
    "graph_digest",
    "state_digest",
    "evidence_digest",
    "estimator_digest",
    "event_digest",
    "commitment_digest",
];
const SEMANTIC_RELATION_COMMIT_KEY: [&str; 15] = [
    "persona_scope",
    "semantic_revision",
    "relation_present",
    "relation_scope",
    "event_id",
    "incarnation_id",
    "manifest_digest",
    "route_digest",
    "formula_digest",
    "graph_digest",
    "state_digest",
    "evidence_digest",
    "estimator_digest",
    "event_digest",
    "commitment_digest",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SemanticSchemaLayout {
    Weak898b,
    StrongV2,
}

#[derive(Debug, PartialEq, Eq)]
struct SemanticForeignKeyColumn {
    id: i64,
    sequence: i64,
    parent: String,
    from: String,
    to: String,
}

fn preflight_sqlite_schema_catalog_v1(tx: &Transaction<'_>) -> Result<(), StoreError> {
    // This is the first SQL operation performed by `migrate_schema`. It closes
    // the complete catalog before any filtered lookup, aggregate, DDL, or
    // variable-length catalog value can run. SQLite walks rowid order and may
    // visit only the first forbidden row, so an attacker-controlled tail cannot
    // amplify open-time work.
    let mut statement = tx.prepare(
        "SELECT rowid,
                CASE WHEN typeof(type)='text' THEN length(CAST(type AS BLOB)) ELSE -1 END,
                CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END,
                CASE WHEN typeof(tbl_name)='text' THEN length(CAST(tbl_name AS BLOB)) ELSE -1 END,
                CASE WHEN sql IS NULL THEN -2 WHEN typeof(sql)='text'
                     THEN length(CAST(sql AS BLOB)) ELSE -1 END
         FROM sqlite_schema NOT INDEXED
         ORDER BY rowid LIMIT 4097",
    )?;
    let mut rows = statement.query([])?;
    let mut count = 0_u64;
    let mut previous = None;
    while let Some(row) = rows.next()? {
        count = count.saturating_add(1);
        if count > MAX_SQLITE_SCHEMA_OBJECTS_GLOBAL {
            return Err(StoreError::StorageBudgetExceeded {
                resource: "sqlite_schema.objects",
                limit: MAX_SQLITE_SCHEMA_OBJECTS_GLOBAL,
                actual: MAX_SQLITE_SCHEMA_OBJECTS_GLOBAL.saturating_add(1),
            });
        }
        let rowid: i64 = row.get(0)?;
        if rowid <= 0 || previous.is_some_and(|last| rowid <= last) {
            return Err(StoreError::ContinuityFence("sqlite_schema_rowid_domain"));
        }
        let kind_len: i64 = row.get(1)?;
        let name_len: i64 = row.get(2)?;
        let table_len: i64 = row.get(3)?;
        let sql_len: i64 = row.get(4)?;
        if !(0..=16).contains(&kind_len)
            || !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_NAME_BYTES).unwrap_or(i64::MAX))
                .contains(&name_len)
            || !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_NAME_BYTES).unwrap_or(i64::MAX))
                .contains(&table_len)
        {
            return Err(StoreError::ContinuityFence("sqlite_schema_object_type"));
        }
        if sql_len != -2
            && !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_SQL_BYTES).unwrap_or(i64::MAX))
                .contains(&sql_len)
        {
            return Err(StoreError::ContinuityFence("sqlite_schema_sql_size"));
        }
        previous = Some(rowid);
    }
    Ok(())
}

fn semantic_schema_table_count(tx: &Transaction<'_>) -> Result<u64, StoreError> {
    let raw: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type='table' AND name IN (
           'semantic_origins','semantic_commits','semantic_cursor','semantic_snapshots',
           'semantic_graphs','semantic_receipts','semantic_telemetry',
           'semantic_evidence_authority','perception_challenges','semantic_budget_checkpoint',
           'semantic_time_authority')",
        [],
        |row| row.get(0),
    )?;
    sqlite_length(raw, "semantic_schema.tables")
}

fn semantic_schema_core_table_count(tx: &Transaction<'_>) -> Result<u64, StoreError> {
    let raw: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sqlite_schema
         WHERE type='table' AND name IN (
           'semantic_origins','semantic_commits','semantic_cursor','semantic_snapshots',
           'semantic_graphs','semantic_receipts','semantic_telemetry',
           'semantic_evidence_authority')",
        [],
        |row| row.get(0),
    )?;
    sqlite_length(raw, "semantic_schema.core_tables")
}

#[derive(Debug, PartialEq, Eq)]
struct SemanticColumnV1 {
    name: String,
    kind: String,
    not_null: i64,
    primary_key_order: i64,
}

fn semantic_table_columns(
    tx: &Transaction<'_>,
    table: &'static str,
) -> Result<Vec<SemanticColumnV1>, StoreError> {
    let sql = format!(
        "SELECT
           CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?1 THEN name END,
           CASE WHEN typeof(type)='text' AND length(CAST(type AS BLOB))<=?1 THEN type END,
           CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END,
           CASE WHEN typeof(type)='text' THEN length(CAST(type AS BLOB)) ELSE -1 END,
           \"notnull\",pk
         FROM pragma_table_info('{table}') ORDER BY cid LIMIT 65"
    );
    let mut statement = tx.prepare(&sql)?;
    let mut rows = statement.query(params![MAX_SEMANTIC_SCHEMA_NAME_BYTES])?;
    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        let actual = u64::try_from(columns.len())
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        enforce_byte_budget("semantic_schema.columns", actual, 64)?;
        let name_len: i64 = row.get(2)?;
        let kind_len: i64 = row.get(3)?;
        if !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_NAME_BYTES).unwrap_or(i64::MAX))
            .contains(&name_len)
            || !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_NAME_BYTES).unwrap_or(i64::MAX))
                .contains(&kind_len)
        {
            return Err(StoreError::ContinuityFence("semantic_schema_column_layout"));
        }
        columns.push(SemanticColumnV1 {
            name: row
                .get::<_, Option<String>>(0)?
                .ok_or(StoreError::ContinuityFence("semantic_schema_column_layout"))?,
            kind: row
                .get::<_, Option<String>>(1)?
                .ok_or(StoreError::ContinuityFence("semantic_schema_column_layout"))?,
            not_null: row.get(4)?,
            primary_key_order: row.get(5)?,
        });
    }
    Ok(columns)
}

fn require_semantic_columns(
    tx: &Transaction<'_>,
    table: &'static str,
    expected: &[(&str, &str, i64, i64)],
) -> Result<(), StoreError> {
    let actual = semantic_table_columns(tx, table)?;
    if actual.len() != expected.len()
        || actual.iter().zip(expected).any(|(actual, expected)| {
            actual.name != expected.0
                || actual.kind != expected.1
                || actual.not_null != expected.2
                || actual.primary_key_order != expected.3
        })
    {
        return Err(StoreError::ContinuityFence("semantic_schema_column_layout"));
    }
    Ok(())
}

const SEMANTIC_EVIDENCE_V2_COLUMNS: [(&str, &str, i64, i64); 16] = [
    ("persona_scope", "BLOB", 1, 1),
    ("semantic_revision", "INTEGER", 1, 0),
    ("relation_present", "INTEGER", 1, 2),
    ("relation_scope", "BLOB", 1, 3),
    ("event_id", "BLOB", 1, 4),
    ("incarnation_id", "BLOB", 1, 0),
    ("manifest_digest", "BLOB", 1, 0),
    ("route_digest", "BLOB", 1, 0),
    ("formula_digest", "BLOB", 1, 0),
    ("graph_digest", "BLOB", 1, 0),
    ("state_digest", "BLOB", 1, 0),
    ("evidence_digest", "BLOB", 1, 0),
    ("estimator_digest", "BLOB", 1, 0),
    ("event_digest", "BLOB", 1, 0),
    ("commitment_digest", "BLOB", 1, 0),
    ("evidence_bytes", "BLOB", 1, 0),
];

const SEMANTIC_EVIDENCE_V3_COLUMNS: [(&str, &str, i64, i64); 20] = [
    ("persona_scope", "BLOB", 1, 1),
    ("semantic_revision", "INTEGER", 1, 0),
    ("relation_present", "INTEGER", 1, 2),
    ("relation_scope", "BLOB", 1, 3),
    ("event_id", "BLOB", 1, 4),
    ("incarnation_id", "BLOB", 1, 0),
    ("manifest_digest", "BLOB", 1, 0),
    ("route_digest", "BLOB", 1, 0),
    ("formula_digest", "BLOB", 1, 0),
    ("graph_digest", "BLOB", 1, 0),
    ("state_digest", "BLOB", 1, 0),
    ("evidence_digest", "BLOB", 1, 0),
    ("estimator_digest", "BLOB", 1, 0),
    ("event_digest", "BLOB", 1, 0),
    ("commitment_digest", "BLOB", 1, 0),
    ("evidence_bytes", "BLOB", 1, 0),
    ("perception_nonce_digest", "BLOB", 0, 0),
    ("perception_proposal_digest", "BLOB", 0, 0),
    ("perception_origin_digest", "BLOB", 0, 0),
    ("perception_origin_bytes", "BLOB", 0, 0),
];

const PERCEPTION_CHALLENGE_V2_COLUMNS: [(&str, &str, i64, i64); 13] = [
    ("request_nonce_digest", "BLOB", 0, 1),
    ("challenge_secret", "BLOB", 1, 0),
    ("scope_digest", "BLOB", 1, 0),
    ("scope_bytes", "BLOB", 1, 0),
    ("persona_scope", "BLOB", 1, 0),
    ("event_id", "BLOB", 1, 0),
    ("turn_id", "BLOB", 1, 0),
    ("base_revision", "INTEGER", 1, 0),
    ("incarnation_id", "BLOB", 1, 0),
    ("status", "INTEGER", 1, 0),
    ("consumed_event_digest", "BLOB", 0, 0),
    ("consumed_estimator_digest", "BLOB", 0, 0),
    ("consumed_semantic_revision", "INTEGER", 0, 0),
];

const PERCEPTION_CHALLENGE_V3_COLUMNS: [(&str, &str, i64, i64); 14] = [
    ("request_nonce_digest", "BLOB", 0, 1),
    ("challenge_secret", "BLOB", 1, 0),
    ("origin_digest", "BLOB", 1, 0),
    ("origin_bytes", "BLOB", 1, 0),
    ("persona_scope", "BLOB", 1, 0),
    ("bot_token", "BLOB", 1, 0),
    ("persona_token", "BLOB", 1, 0),
    ("origin_event_digest", "BLOB", 1, 0),
    ("origin_journal_revision", "INTEGER", 1, 0),
    ("base_revision", "INTEGER", 1, 0),
    ("incarnation_id", "BLOB", 1, 0),
    ("manifest_digest", "BLOB", 1, 0),
    ("created_at_ms", "INTEGER", 1, 0),
    ("expires_at_ms", "INTEGER", 1, 0),
];

const SEMANTIC_BUDGET_COLUMNS: [(&str, &str, i64, i64); 5] = [
    ("persona_scope", "BLOB", 0, 1),
    ("row_count", "INTEGER", 1, 0),
    ("aggregate_bytes", "INTEGER", 1, 0),
    ("head_revision", "INTEGER", 1, 0),
    ("head_commitment_digest", "BLOB", 1, 0),
];

fn semantic_schema_version(tx: &Transaction<'_>) -> Result<Option<u8>, StoreError> {
    let stored: Option<(Option<Vec<u8>>, i64)> = tx
        .query_row(
            "SELECT
               CASE WHEN typeof(value)='blob' AND length(value)=1 THEN value END,
               CASE WHEN typeof(value)='blob' THEN length(value) ELSE -1 END
             FROM meta WHERE key='semantic_schema_version'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    stored
        .map(|(value, raw_len)| {
            let value = bounded_typed_value(
                value,
                raw_len,
                1,
                "semantic_schema.version",
                "semantic_schema_version_type",
            )?;
            value
                .first()
                .copied()
                .ok_or(StoreError::ContinuityFence("semantic_schema_version"))
        })
        .transpose()
}

fn semantic_foreign_key_columns(
    tx: &Transaction<'_>,
    table: &'static str,
) -> Result<Vec<SemanticForeignKeyColumn>, StoreError> {
    let count_sql = format!("SELECT COUNT(*) FROM pragma_foreign_key_list('{table}')");
    let raw_count: i64 = tx.query_row(&count_sql, [], |row| row.get(0))?;
    let count = sqlite_length(raw_count, "semantic_schema.foreign_keys")?;
    enforce_byte_budget("semantic_schema.foreign_keys", count, 32)?;

    let sql = format!(
        "SELECT id,seq,
           CASE WHEN typeof(\"table\")='text' AND length(CAST(\"table\" AS BLOB))<=?1 THEN \"table\" END,
           CASE WHEN typeof(\"table\")='text' THEN length(CAST(\"table\" AS BLOB)) ELSE -1 END,
           CASE WHEN typeof(\"from\")='text' AND length(CAST(\"from\" AS BLOB))<=?1 THEN \"from\" END,
           CASE WHEN typeof(\"from\")='text' THEN length(CAST(\"from\" AS BLOB)) ELSE -1 END,
           CASE WHEN typeof(\"to\")='text' AND length(CAST(\"to\" AS BLOB))<=?1 THEN \"to\" END,
           CASE WHEN typeof(\"to\")='text' THEN length(CAST(\"to\" AS BLOB)) ELSE -1 END
         FROM pragma_foreign_key_list('{table}') ORDER BY id,seq"
    );
    let mut statement = tx.prepare(&sql)?;
    let mut rows = statement.query(params![MAX_SEMANTIC_SCHEMA_NAME_BYTES])?;
    let mut columns = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    while let Some(row) = rows.next()? {
        let parent = bounded_typed_value(
            row.get::<_, Option<String>>(2)?,
            row.get(3)?,
            MAX_SEMANTIC_SCHEMA_NAME_BYTES,
            "semantic_schema.foreign_key.parent",
            "semantic_schema_name_type",
        )?;
        let from = bounded_typed_value(
            row.get::<_, Option<String>>(4)?,
            row.get(5)?,
            MAX_SEMANTIC_SCHEMA_NAME_BYTES,
            "semantic_schema.foreign_key.from",
            "semantic_schema_name_type",
        )?;
        let to = bounded_typed_value(
            row.get::<_, Option<String>>(6)?,
            row.get(7)?,
            MAX_SEMANTIC_SCHEMA_NAME_BYTES,
            "semantic_schema.foreign_key.to",
            "semantic_schema_name_type",
        )?;
        columns.push(SemanticForeignKeyColumn {
            id: row.get(0)?,
            sequence: row.get(1)?,
            parent,
            from,
            to,
        });
    }
    if u64::try_from(columns.len()).unwrap_or(u64::MAX) != count {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_set",
        ));
    }
    Ok(columns)
}

fn require_semantic_foreign_key(
    tx: &Transaction<'_>,
    table: &'static str,
    parent: &'static str,
    from: &[&str],
    to: &[&str],
) -> Result<(), StoreError> {
    if from.len() != to.len() {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_spec",
        ));
    }
    let columns = semantic_foreign_key_columns(tx, table)?;
    if columns.len() != from.len()
        || columns.iter().enumerate().any(|(index, column)| {
            column.id != 0
                || column.sequence != i64::try_from(index).unwrap_or(-1)
                || column.parent != parent
                || column.from != from[index]
                || column.to != to[index]
        })
    {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_layout",
        ));
    }
    Ok(())
}

fn require_named_index(
    tx: &Transaction<'_>,
    table: &'static str,
    index: &'static str,
    unique: bool,
    partial: bool,
    columns: &[&str],
) -> Result<(), StoreError> {
    let index_row: Option<(i64, i64)> = tx
        .query_row(
            &format!("SELECT \"unique\",partial FROM pragma_index_list('{table}') WHERE name=?1"),
            params![index],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if index_row != Some((i64::from(unique), i64::from(partial))) {
        return Err(StoreError::ContinuityFence("semantic_schema_index_layout"));
    }
    let count = sqlite_length(
        tx.query_row(
            &format!("SELECT COUNT(*) FROM pragma_index_info('{index}')"),
            [],
            |row| row.get(0),
        )?,
        "semantic_schema.index_columns",
    )?;
    if count != u64::try_from(columns.len()).unwrap_or(u64::MAX) {
        return Err(StoreError::ContinuityFence("semantic_schema_index_layout"));
    }
    let mut statement = tx.prepare(&format!(
        "SELECT name FROM pragma_index_info('{index}') ORDER BY seqno"
    ))?;
    let mut rows = statement.query([])?;
    for expected in columns {
        let actual: String = rows
            .next()?
            .ok_or(StoreError::ContinuityFence("semantic_schema_index_layout"))?
            .get(0)?;
        if actual != *expected {
            return Err(StoreError::ContinuityFence("semantic_schema_index_layout"));
        }
    }
    if rows.next()?.is_some() {
        return Err(StoreError::ContinuityFence("semantic_schema_index_layout"));
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
struct SemanticSchemaObjectV1 {
    kind: String,
    name: String,
    table: String,
    canonical_sql: Option<String>,
}

fn canonical_schema_sql_v1(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn semantic_schema_objects_v1(
    conn: &Connection,
) -> Result<Vec<SemanticSchemaObjectV1>, StoreError> {
    bounded_semantic_schema_objects_v1(conn, false)
}

const SEMANTIC_SCHEMA_CATALOG_PHASE_ONE_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(type)='text' THEN length(CAST(type AS BLOB)) ELSE -1 END,
            CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END,
            CASE WHEN typeof(tbl_name)='text' THEN length(CAST(tbl_name AS BLOB)) ELSE -1 END,
            CASE WHEN sql IS NULL THEN -2 WHEN typeof(sql)='text'
                 THEN length(CAST(sql AS BLOB)) ELSE -1 END
     FROM sqlite_schema NOT INDEXED
     WHERE rowid>?1 AND type IN ('table','index','trigger') AND tbl_name IN (
       'semantic_origins','semantic_commits','semantic_cursor','semantic_snapshots',
       'semantic_graphs','semantic_receipts','semantic_telemetry',
       'semantic_evidence_authority','perception_challenges','semantic_budget_checkpoint',
       'semantic_time_authority')
     ORDER BY rowid LIMIT ?2";
const SEMANTIC_APPRAISAL_CATALOG_PHASE_ONE_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(type)='text' THEN length(CAST(type AS BLOB)) ELSE -1 END,
            CASE WHEN typeof(name)='text' THEN length(CAST(name AS BLOB)) ELSE -1 END,
            CASE WHEN typeof(tbl_name)='text' THEN length(CAST(tbl_name AS BLOB)) ELSE -1 END,
            CASE WHEN sql IS NULL THEN -2 WHEN typeof(sql)='text'
                 THEN length(CAST(sql AS BLOB)) ELSE -1 END
     FROM sqlite_schema NOT INDEXED
     WHERE rowid>?1 AND type IN ('table','index','trigger')
       AND tbl_name IN ('semantic_appraisal_budget','semantic_appraisal_claim',
                        'semantic_appraisal_rollup')
     ORDER BY rowid LIMIT ?2";
const SEMANTIC_SCHEMA_CATALOG_PHASE_TWO_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(type)='text' AND length(CAST(type AS BLOB))<=16 THEN type END,
            CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?3 THEN name END,
            CASE WHEN typeof(tbl_name)='text' AND length(CAST(tbl_name AS BLOB))<=?3
                 THEN tbl_name END,
            CASE WHEN sql IS NULL THEN NULL WHEN typeof(sql)='text'
                      AND length(CAST(sql AS BLOB))<=?4 THEN sql END,
            CASE WHEN sql IS NULL THEN -2 WHEN typeof(sql)='text'
                 THEN length(CAST(sql AS BLOB)) ELSE -1 END
     FROM sqlite_schema NOT INDEXED
     WHERE rowid>?1 AND type IN ('table','index','trigger') AND tbl_name IN (
       'semantic_origins','semantic_commits','semantic_cursor','semantic_snapshots',
       'semantic_graphs','semantic_receipts','semantic_telemetry',
       'semantic_evidence_authority','perception_challenges','semantic_budget_checkpoint',
       'semantic_time_authority')
     ORDER BY rowid LIMIT ?2";
const SEMANTIC_APPRAISAL_CATALOG_PHASE_TWO_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(type)='text' AND length(CAST(type AS BLOB))<=16 THEN type END,
            CASE WHEN typeof(name)='text' AND length(CAST(name AS BLOB))<=?3 THEN name END,
            CASE WHEN typeof(tbl_name)='text' AND length(CAST(tbl_name AS BLOB))<=?3
                 THEN tbl_name END,
            CASE WHEN sql IS NULL THEN NULL WHEN typeof(sql)='text'
                      AND length(CAST(sql AS BLOB))<=?4 THEN sql END,
            CASE WHEN sql IS NULL THEN -2 WHEN typeof(sql)='text'
                 THEN length(CAST(sql AS BLOB)) ELSE -1 END
     FROM sqlite_schema NOT INDEXED
     WHERE rowid>?1 AND type IN ('table','index','trigger')
       AND tbl_name IN ('semantic_appraisal_budget','semantic_appraisal_claim',
                        'semantic_appraisal_rollup')
     ORDER BY rowid LIMIT ?2";

fn bounded_semantic_schema_objects_v1(
    conn: &Connection,
    appraisal_only: bool,
) -> Result<Vec<SemanticSchemaObjectV1>, StoreError> {
    let maximum = if appraisal_only {
        9_u64
    } else {
        MAX_SEMANTIC_SCHEMA_OBJECTS
    };
    let phase_one_sql = if appraisal_only {
        SEMANTIC_APPRAISAL_CATALOG_PHASE_ONE_SQL
    } else {
        SEMANTIC_SCHEMA_CATALOG_PHASE_ONE_SQL
    };
    let phase_two_sql = if appraisal_only {
        SEMANTIC_APPRAISAL_CATALOG_PHASE_TWO_SQL
    } else {
        SEMANTIC_SCHEMA_CATALOG_PHASE_TWO_SQL
    };
    let set_fence = if appraisal_only {
        "semantic_appraisal_schema_object_set"
    } else {
        "semantic_schema_object_set"
    };
    let type_fence = if appraisal_only {
        "semantic_appraisal_schema_object_type"
    } else {
        "semantic_schema_object_type"
    };
    let sql_fence = if appraisal_only {
        "semantic_appraisal_schema_sql_size"
    } else {
        "semantic_schema_sql_size"
    };

    let limit = i64::try_from(maximum.saturating_add(1))
        .map_err(|_| StoreError::ContinuityFence(set_fence))?;
    let mut phase_one = conn.prepare(phase_one_sql)?;
    let mut rows = phase_one.query(params![0_i64, limit])?;
    let mut count = 0_u64;
    let mut previous = 0_i64;
    while let Some(row) = rows.next()? {
        count = count.saturating_add(1);
        if count > maximum {
            return Err(StoreError::ContinuityFence(set_fence));
        }
        let rowid: i64 = row.get(0)?;
        let kind_len: i64 = row.get(1)?;
        let name_len: i64 = row.get(2)?;
        let table_len: i64 = row.get(3)?;
        let sql_len: i64 = row.get(4)?;
        if rowid <= previous
            || !(0..=16).contains(&kind_len)
            || !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_NAME_BYTES).unwrap_or(i64::MAX))
                .contains(&name_len)
            || !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_NAME_BYTES).unwrap_or(i64::MAX))
                .contains(&table_len)
        {
            return Err(StoreError::ContinuityFence(type_fence));
        }
        if sql_len != -2
            && !(0..=i64::try_from(MAX_SEMANTIC_SCHEMA_SQL_BYTES).unwrap_or(i64::MAX))
                .contains(&sql_len)
        {
            return Err(StoreError::ContinuityFence(sql_fence));
        }
        previous = rowid;
    }
    drop(rows);
    drop(phase_one);

    let mut phase_two = conn.prepare(phase_two_sql)?;
    let mut rows = phase_two.query(params![
        0_i64,
        i64::try_from(count).map_err(|_| StoreError::ContinuityFence(set_fence))?,
        MAX_SEMANTIC_SCHEMA_NAME_BYTES,
        MAX_SEMANTIC_SCHEMA_SQL_BYTES,
    ])?;
    let mut objects = Vec::with_capacity(usize::try_from(count).unwrap_or(0));
    let mut previous = 0_i64;
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get(0)?;
        if rowid <= previous {
            return Err(StoreError::ContinuityFence(set_fence));
        }
        let kind = row
            .get::<_, Option<String>>(1)?
            .ok_or(StoreError::ContinuityFence(type_fence))?;
        let name = row
            .get::<_, Option<String>>(2)?
            .ok_or(StoreError::ContinuityFence(type_fence))?;
        let table = row
            .get::<_, Option<String>>(3)?
            .ok_or(StoreError::ContinuityFence(type_fence))?;
        let raw_sql: Option<String> = row.get(4)?;
        let raw_sql_len: i64 = row.get(5)?;
        let canonical_sql = if raw_sql_len == -2 {
            if raw_sql.is_some() {
                return Err(StoreError::ContinuityFence(type_fence));
            }
            None
        } else {
            Some(canonical_schema_sql_v1(
                &raw_sql.ok_or(StoreError::ContinuityFence(sql_fence))?,
            ))
        };
        objects.push(SemanticSchemaObjectV1 {
            kind,
            name,
            table,
            canonical_sql,
        });
        previous = rowid;
    }
    if u64::try_from(objects.len()).unwrap_or(u64::MAX) != count {
        return Err(StoreError::ContinuityFence(set_fence));
    }
    objects.sort_by(|left, right| {
        (&left.kind, &left.name, &left.table).cmp(&(&right.kind, &right.name, &right.table))
    });
    Ok(objects)
}

fn require_exact_semantic_schema_v3(conn: &Connection) -> Result<(), StoreError> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(SEMANTIC_SCHEMA_V3_SQL)?;
    if semantic_schema_objects_v1(conn)? != semantic_schema_objects_v1(&reference)? {
        return Err(StoreError::ContinuityFence("semantic_schema_sql_identity"));
    }
    Ok(())
}

fn verify_semantic_schema_v3(tx: &Transaction<'_>) -> Result<(), StoreError> {
    if semantic_schema_table_count(tx)? != SEMANTIC_SCHEMA_V3_TABLE_COUNT {
        return Err(StoreError::ContinuityFence("semantic_schema_table_set"));
    }
    if detect_semantic_schema_layout(tx)? != SemanticSchemaLayout::StrongV2 {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_layout",
        ));
    }
    require_semantic_columns(
        tx,
        "semantic_evidence_authority",
        &SEMANTIC_EVIDENCE_V3_COLUMNS,
    )?;
    require_semantic_columns(
        tx,
        "perception_challenges",
        &PERCEPTION_CHALLENGE_V3_COLUMNS,
    )?;
    require_semantic_columns(tx, "semantic_budget_checkpoint", &SEMANTIC_BUDGET_COLUMNS)?;
    require_semantic_foreign_key(
        tx,
        "perception_challenges",
        "applied_events",
        &["persona_scope", "origin_event_digest"],
        &["scope_digest", "event_digest"],
    )?;
    require_semantic_foreign_key(
        tx,
        "semantic_budget_checkpoint",
        "semantic_commits",
        &["persona_scope", "head_revision", "head_commitment_digest"],
        &["persona_scope", "semantic_revision", "commitment_digest"],
    )?;
    require_named_index(
        tx,
        "perception_challenges",
        "perception_challenge_context_v1",
        false,
        false,
        &["origin_digest", "base_revision", "incarnation_id"],
    )?;
    require_named_index(
        tx,
        "perception_challenges",
        "perception_challenge_persona_pending_v1",
        false,
        false,
        &["persona_scope"],
    )?;
    require_named_index(
        tx,
        "perception_challenges",
        "perception_challenge_expiry_v1",
        false,
        false,
        &["expires_at_ms"],
    )?;
    require_named_index(
        tx,
        "semantic_evidence_authority",
        "semantic_evidence_perception_nonce_v1",
        true,
        true,
        &["perception_nonce_digest"],
    )?;
    // Columns and foreign-key pragmas do not expose CHECK clauses, UNIQUE
    // constraints or a partial-index predicate. Compare every semantic table,
    // named index and SQLite autoindex with a schema built from the canonical
    // V3 DDL so a same-column but weakened clone can never be accepted.
    require_exact_semantic_schema_v3(tx)?;
    Ok(())
}

const ZERO_DIGEST_SQL: &str = "X'0000000000000000000000000000000000000000000000000000000000000000'";

fn apply_semantic_schema_v4(conn: &Connection) -> Result<(), StoreError> {
    let has_kind: i64 = conn.query_row(
        "SELECT COUNT(*) FROM pragma_table_info('semantic_commits') WHERE name='transition_kind'",
        [],
        |row| row.get(0),
    )?;
    if has_kind == 0 {
        conn.execute_batch(&format!(
            "ALTER TABLE semantic_commits ADD COLUMN transition_kind TEXT NOT NULL DEFAULT 'perception'
               CHECK(typeof(transition_kind)='text' AND transition_kind IN ('perception','time'));
             ALTER TABLE semantic_cursor ADD COLUMN transition_kind TEXT NOT NULL DEFAULT 'perception'
               CHECK(typeof(transition_kind)='text' AND transition_kind IN ('perception','time'));
             ALTER TABLE semantic_cursor ADD COLUMN time_anchor_semantic_revision INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_anchor_semantic_revision)='integer' AND time_anchor_semantic_revision>=0);
             ALTER TABLE semantic_cursor ADD COLUMN time_anchor_state_digest BLOB NOT NULL DEFAULT {ZERO_DIGEST_SQL}
               CHECK(typeof(time_anchor_state_digest)='blob' AND length(time_anchor_state_digest)=32);
             ALTER TABLE semantic_cursor ADD COLUMN time_awake_ticks INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_awake_ticks)='integer' AND time_awake_ticks>=0);
             ALTER TABLE semantic_cursor ADD COLUMN time_drowsy_ticks INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_drowsy_ticks)='integer' AND time_drowsy_ticks>=0);
             ALTER TABLE semantic_cursor ADD COLUMN time_asleep_ticks INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_asleep_ticks)='integer' AND time_asleep_ticks>=0);
             ALTER TABLE semantic_cursor ADD COLUMN time_awake_remainder_ms INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_awake_remainder_ms)='integer' AND time_awake_remainder_ms>=0 AND time_awake_remainder_ms<600000);
             ALTER TABLE semantic_cursor ADD COLUMN time_drowsy_remainder_ms INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_drowsy_remainder_ms)='integer' AND time_drowsy_remainder_ms>=0 AND time_drowsy_remainder_ms<600000);
             ALTER TABLE semantic_cursor ADD COLUMN time_asleep_remainder_ms INTEGER NOT NULL DEFAULT 0
               CHECK(typeof(time_asleep_remainder_ms)='integer' AND time_asleep_remainder_ms>=0 AND time_asleep_remainder_ms<600000);
             UPDATE semantic_cursor SET
               time_anchor_semantic_revision=semantic_revision,
               time_anchor_state_digest=state_digest;"
        ))?;
    } else if has_kind != 1 {
        return Err(StoreError::ContinuityFence("semantic_schema_column_layout"));
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS semantic_time_authority (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            journal_revision INTEGER NOT NULL UNIQUE CHECK(typeof(journal_revision)='integer' AND journal_revision>0),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            authority_digest BLOB NOT NULL CHECK(typeof(authority_digest)='blob' AND length(authority_digest)=32),
            authority_bytes BLOB NOT NULL CHECK(typeof(authority_bytes)='blob' AND length(authority_bytes)<=16384),
            PRIMARY KEY(persona_scope,semantic_revision),
            UNIQUE(persona_scope,event_id),
            UNIQUE(persona_scope,event_digest),
            FOREIGN KEY(persona_scope,semantic_revision)
              REFERENCES semantic_commits(persona_scope,semantic_revision)
        );
        CREATE INDEX IF NOT EXISTS semantic_time_journal_v1
          ON semantic_time_authority(persona_scope,journal_revision);",
    )?;
    Ok(())
}

fn require_exact_semantic_schema_v4(conn: &Connection) -> Result<(), StoreError> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(SEMANTIC_SCHEMA_V3_SQL)?;
    apply_semantic_schema_v4(&reference)?;
    if semantic_schema_objects_v1(conn)? != semantic_schema_objects_v1(&reference)? {
        return Err(StoreError::ContinuityFence("semantic_schema_sql_identity"));
    }
    Ok(())
}

fn verify_semantic_schema_v4(tx: &Transaction<'_>) -> Result<(), StoreError> {
    if semantic_schema_table_count(tx)? != SEMANTIC_SCHEMA_TABLE_COUNT
        || detect_semantic_schema_layout(tx)? != SemanticSchemaLayout::StrongV2
    {
        return Err(StoreError::ContinuityFence("semantic_schema_table_set"));
    }
    require_semantic_columns(
        tx,
        "semantic_evidence_authority",
        &SEMANTIC_EVIDENCE_V3_COLUMNS,
    )?;
    require_semantic_columns(
        tx,
        "perception_challenges",
        &PERCEPTION_CHALLENGE_V3_COLUMNS,
    )?;
    require_semantic_columns(tx, "semantic_budget_checkpoint", &SEMANTIC_BUDGET_COLUMNS)?;
    require_named_index(
        tx,
        "semantic_time_authority",
        "semantic_time_journal_v1",
        false,
        false,
        &["persona_scope", "journal_revision"],
    )?;
    require_exact_semantic_schema_v4(tx)
}

/// V5 is the first releasable time-lane layout. V4 accidentally made a
/// persona-local journal revision globally unique and therefore prevented two
/// personas from both settling logical revision 1.
fn apply_semantic_schema_v5(conn: &Connection) -> Result<(), StoreError> {
    let staging: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE name='__ae_semantic_time_authority_v4'",
        [],
        |row| row.get(0),
    )?;
    if staging != 0 {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_staging_collision",
        ));
    }
    conn.execute_batch(
        "PRAGMA defer_foreign_keys=ON;
         DROP INDEX IF EXISTS semantic_time_journal_v1;
         ALTER TABLE semantic_time_authority RENAME TO __ae_semantic_time_authority_v4;
         CREATE TABLE semantic_time_authority (
            persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
            semantic_revision INTEGER NOT NULL CHECK(typeof(semantic_revision)='integer' AND semantic_revision>0),
            journal_revision INTEGER NOT NULL CHECK(typeof(journal_revision)='integer' AND journal_revision>0),
            event_id BLOB NOT NULL CHECK(typeof(event_id)='blob' AND length(event_id)=16),
            event_digest BLOB NOT NULL CHECK(typeof(event_digest)='blob' AND length(event_digest)=32),
            authority_digest BLOB NOT NULL CHECK(typeof(authority_digest)='blob' AND length(authority_digest)=32),
            authority_bytes BLOB NOT NULL CHECK(typeof(authority_bytes)='blob' AND length(authority_bytes)<=16384),
            PRIMARY KEY(persona_scope,semantic_revision),
            UNIQUE(persona_scope,journal_revision),
            UNIQUE(persona_scope,event_id),
            UNIQUE(persona_scope,event_digest),
            FOREIGN KEY(persona_scope,semantic_revision)
              REFERENCES semantic_commits(persona_scope,semantic_revision)
         );
         INSERT INTO semantic_time_authority(
            persona_scope,semantic_revision,journal_revision,event_id,event_digest,
            authority_digest,authority_bytes)
          SELECT persona_scope,semantic_revision,journal_revision,event_id,event_digest,
                 authority_digest,authority_bytes
          FROM __ae_semantic_time_authority_v4;
         DROP TABLE __ae_semantic_time_authority_v4;
         CREATE INDEX semantic_time_journal_v1
           ON semantic_time_authority(persona_scope,journal_revision);",
    )?;
    Ok(())
}

fn require_exact_semantic_schema_v5(conn: &Connection) -> Result<(), StoreError> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(SEMANTIC_SCHEMA_V3_SQL)?;
    apply_semantic_schema_v4(&reference)?;
    apply_semantic_schema_v5(&reference)?;
    if semantic_schema_objects_v1(conn)? != semantic_schema_objects_v1(&reference)? {
        return Err(StoreError::ContinuityFence("semantic_schema_sql_identity"));
    }
    Ok(())
}

fn verify_semantic_schema_v5(tx: &Transaction<'_>) -> Result<(), StoreError> {
    if semantic_schema_table_count(tx)? != SEMANTIC_SCHEMA_TABLE_COUNT
        || detect_semantic_schema_layout(tx)? != SemanticSchemaLayout::StrongV2
    {
        return Err(StoreError::ContinuityFence("semantic_schema_table_set"));
    }
    require_semantic_columns(
        tx,
        "semantic_evidence_authority",
        &SEMANTIC_EVIDENCE_V3_COLUMNS,
    )?;
    require_semantic_columns(
        tx,
        "perception_challenges",
        &PERCEPTION_CHALLENGE_V3_COLUMNS,
    )?;
    require_semantic_columns(tx, "semantic_budget_checkpoint", &SEMANTIC_BUDGET_COLUMNS)?;
    require_named_index(
        tx,
        "semantic_time_authority",
        "semantic_time_journal_v1",
        false,
        false,
        &["persona_scope", "journal_revision"],
    )?;
    require_exact_semantic_schema_v5(tx)
}

fn verify_semantic_schema_v2_supplement(
    tx: &Transaction<'_>,
    supplemental_count: u64,
) -> Result<(), StoreError> {
    require_semantic_columns(
        tx,
        "semantic_evidence_authority",
        &SEMANTIC_EVIDENCE_V2_COLUMNS,
    )?;
    if supplemental_count == 0 {
        return Ok(());
    }
    if supplemental_count != 2 {
        return Err(StoreError::ContinuityFence("semantic_schema_table_set"));
    }
    require_semantic_columns(
        tx,
        "perception_challenges",
        &PERCEPTION_CHALLENGE_V2_COLUMNS,
    )?;
    require_named_index(
        tx,
        "perception_challenges",
        "perception_challenge_context_v1",
        false,
        false,
        &[
            "scope_digest",
            "event_id",
            "turn_id",
            "base_revision",
            "incarnation_id",
        ],
    )?;
    require_semantic_columns(tx, "semantic_budget_checkpoint", &SEMANTIC_BUDGET_COLUMNS)?;
    require_semantic_foreign_key(
        tx,
        "semantic_budget_checkpoint",
        "semantic_commits",
        &["persona_scope", "head_revision", "head_commitment_digest"],
        &["persona_scope", "semantic_revision", "commitment_digest"],
    )?;
    Ok(())
}

fn verify_semantic_schema_v2_namespace_v1(
    tx: &Transaction<'_>,
    supplemental_count: u64,
) -> Result<(), StoreError> {
    const CORE: [(&str, &str, &str, bool); 26] = [
        ("table", "semantic_origins", "semantic_origins", true),
        (
            "index",
            "sqlite_autoindex_semantic_origins_1",
            "semantic_origins",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_origins_2",
            "semantic_origins",
            false,
        ),
        ("table", "semantic_commits", "semantic_commits", true),
        (
            "index",
            "sqlite_autoindex_semantic_commits_1",
            "semantic_commits",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_commits_2",
            "semantic_commits",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_commits_3",
            "semantic_commits",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_commits_4",
            "semantic_commits",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_commits_5",
            "semantic_commits",
            false,
        ),
        (
            "index",
            "semantic_commit_journal_identity_v1",
            "semantic_commits",
            true,
        ),
        (
            "index",
            "semantic_commit_relation_identity_v1",
            "semantic_commits",
            true,
        ),
        (
            "index",
            "semantic_commit_checkpoint_identity_v1",
            "semantic_commits",
            true,
        ),
        ("table", "semantic_cursor", "semantic_cursor", true),
        (
            "index",
            "sqlite_autoindex_semantic_cursor_1",
            "semantic_cursor",
            false,
        ),
        ("table", "semantic_snapshots", "semantic_snapshots", true),
        (
            "index",
            "sqlite_autoindex_semantic_snapshots_1",
            "semantic_snapshots",
            false,
        ),
        ("table", "semantic_graphs", "semantic_graphs", true),
        (
            "index",
            "sqlite_autoindex_semantic_graphs_1",
            "semantic_graphs",
            false,
        ),
        ("table", "semantic_receipts", "semantic_receipts", true),
        (
            "index",
            "sqlite_autoindex_semantic_receipts_1",
            "semantic_receipts",
            false,
        ),
        ("table", "semantic_telemetry", "semantic_telemetry", true),
        (
            "index",
            "sqlite_autoindex_semantic_telemetry_1",
            "semantic_telemetry",
            false,
        ),
        (
            "table",
            "semantic_evidence_authority",
            "semantic_evidence_authority",
            true,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_evidence_authority_1",
            "semantic_evidence_authority",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_evidence_authority_2",
            "semantic_evidence_authority",
            false,
        ),
        (
            "index",
            "semantic_evidence_relation_revision",
            "semantic_evidence_authority",
            true,
        ),
    ];
    const SUPPLEMENT: [(&str, &str, &str, bool); 6] = [
        (
            "table",
            "perception_challenges",
            "perception_challenges",
            true,
        ),
        (
            "index",
            "sqlite_autoindex_perception_challenges_1",
            "perception_challenges",
            false,
        ),
        (
            "index",
            "sqlite_autoindex_perception_challenges_2",
            "perception_challenges",
            false,
        ),
        (
            "index",
            "perception_challenge_context_v1",
            "perception_challenges",
            true,
        ),
        (
            "table",
            "semantic_budget_checkpoint",
            "semantic_budget_checkpoint",
            true,
        ),
        (
            "index",
            "sqlite_autoindex_semantic_budget_checkpoint_1",
            "semantic_budget_checkpoint",
            false,
        ),
    ];

    if supplemental_count != 0 && supplemental_count != 2 {
        return Err(StoreError::ContinuityFence("semantic_schema_table_set"));
    }
    let objects = semantic_schema_objects_v1(tx)?;
    let expected_count = CORE.len()
        + if supplemental_count == 2 {
            SUPPLEMENT.len()
        } else {
            0
        };
    if objects.len() != expected_count {
        return Err(StoreError::ContinuityFence("semantic_schema_object_set"));
    }
    for (kind, name, table, has_sql) in CORE.iter().chain(
        (supplemental_count == 2)
            .then_some(SUPPLEMENT.as_slice())
            .unwrap_or_default()
            .iter(),
    ) {
        let exact = objects.iter().any(|object| {
            object.kind == *kind
                && object.name == *name
                && object.table == *table
                && object.canonical_sql.is_some() == *has_sql
        });
        if !exact {
            return Err(StoreError::ContinuityFence("semantic_schema_object_set"));
        }
    }

    // The bounded catalog above closes every object attached to an authority
    // table. This marker additionally rejects a shadow object elsewhere in the
    // reserved namespace without materializing an attacker-sized identifier.
    let shadow: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM sqlite_schema NOT INDEXED
             WHERE type IN ('table','index','trigger')
               AND (name GLOB 'semantic_*' OR tbl_name GLOB 'semantic_*'
                    OR name GLOB 'perception_*' OR tbl_name GLOB 'perception_*')
               AND tbl_name NOT IN (
                 'semantic_origins','semantic_commits','semantic_cursor','semantic_snapshots',
                 'semantic_graphs','semantic_receipts','semantic_telemetry',
                 'semantic_evidence_authority','perception_challenges',
                 'semantic_budget_checkpoint','semantic_appraisal_budget',
                 'semantic_appraisal_claim','semantic_appraisal_rollup',
                 'semantic_migration_authority_v1'
               )
         )",
        [],
        |row| row.get(0),
    )?;
    if shadow {
        return Err(StoreError::ContinuityFence("semantic_schema_object_set"));
    }
    Ok(())
}

fn detect_semantic_schema_layout(tx: &Transaction<'_>) -> Result<SemanticSchemaLayout, StoreError> {
    require_semantic_foreign_key(
        tx,
        "semantic_commits",
        "semantic_origins",
        &["persona_scope", "origin_digest"],
        &["persona_scope", "origin_digest"],
    )?;
    for table in [
        "semantic_cursor",
        "semantic_snapshots",
        "semantic_graphs",
        "semantic_receipts",
        "semantic_telemetry",
    ] {
        require_semantic_foreign_key(
            tx,
            table,
            "semantic_commits",
            &SEMANTIC_COMMON_COMMIT_KEY,
            &SEMANTIC_COMMON_COMMIT_KEY,
        )?;
    }
    let evidence = semantic_foreign_key_columns(tx, "semantic_evidence_authority")?;
    let relation_matches = evidence.len() == SEMANTIC_RELATION_COMMIT_KEY.len()
        && evidence.iter().enumerate().all(|(index, column)| {
            column.id == 0
                && column.sequence == i64::try_from(index).unwrap_or(-1)
                && column.parent == "semantic_commits"
                && column.from == SEMANTIC_RELATION_COMMIT_KEY[index]
                && column.to == SEMANTIC_RELATION_COMMIT_KEY[index]
        });
    if relation_matches {
        return Ok(SemanticSchemaLayout::StrongV2);
    }
    let weak_matches = evidence.len() == SEMANTIC_COMMON_COMMIT_KEY.len()
        && evidence.iter().enumerate().all(|(index, column)| {
            column.id == 0
                && column.sequence == i64::try_from(index).unwrap_or(-1)
                && column.parent == "semantic_commits"
                && column.from == SEMANTIC_COMMON_COMMIT_KEY[index]
                && column.to == SEMANTIC_COMMON_COMMIT_KEY[index]
        });
    if weak_matches {
        Ok(SemanticSchemaLayout::Weak898b)
    } else {
        Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_layout",
        ))
    }
}

fn rebuild_weak_semantic_schema_v3(tx: &Transaction<'_>) -> Result<(), StoreError> {
    // The legacy tables are authenticated above by exact schema checks. Scan
    // their bounded row markers and payload lengths before the first DDL write;
    // a corrupt/oversized weak database must remain byte-for-byte untouched.
    preflight_weak_semantic_history_v1(tx, false)?;
    let staging_count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM sqlite_schema WHERE name IN (
           '__ae_semantic_origins_v1','__ae_semantic_commits_v1','__ae_semantic_cursor_v1',
           '__ae_semantic_snapshots_v1','__ae_semantic_graphs_v1','__ae_semantic_receipts_v1',
           '__ae_semantic_telemetry_v1','__ae_semantic_evidence_authority_v1')",
        [],
        |row| row.get(0),
    )?;
    if sqlite_length(staging_count, "semantic_schema.staging_tables")? != 0 {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_staging_collision",
        ));
    }
    tx.execute_batch(
        "PRAGMA defer_foreign_keys=ON;
         DROP INDEX IF EXISTS semantic_commit_journal_identity_v1;
         DROP INDEX IF EXISTS semantic_commit_relation_identity_v1;
         DROP INDEX IF EXISTS semantic_commit_checkpoint_identity_v1;
         DROP INDEX IF EXISTS semantic_evidence_relation_revision;
         DROP INDEX IF EXISTS perception_challenge_context_v1;
         DROP INDEX IF EXISTS perception_challenge_persona_pending_v1;
         DROP INDEX IF EXISTS perception_challenge_expiry_v1;
         DROP INDEX IF EXISTS semantic_evidence_perception_nonce_v1;
         DROP TABLE IF EXISTS perception_challenges;
         DROP TABLE IF EXISTS semantic_budget_checkpoint;
         ALTER TABLE semantic_evidence_authority RENAME TO __ae_semantic_evidence_authority_v1;
         ALTER TABLE semantic_cursor RENAME TO __ae_semantic_cursor_v1;
         ALTER TABLE semantic_snapshots RENAME TO __ae_semantic_snapshots_v1;
         ALTER TABLE semantic_graphs RENAME TO __ae_semantic_graphs_v1;
         ALTER TABLE semantic_receipts RENAME TO __ae_semantic_receipts_v1;
         ALTER TABLE semantic_telemetry RENAME TO __ae_semantic_telemetry_v1;
         ALTER TABLE semantic_commits RENAME TO __ae_semantic_commits_v1;
         ALTER TABLE semantic_origins RENAME TO __ae_semantic_origins_v1;",
    )?;
    tx.execute_batch(SEMANTIC_SCHEMA_V3_SQL)?;
    // Re-check the renamed sources through the closed staging layout before
    // any INSERT..SELECT can copy them into newly authoritative tables.
    preflight_weak_semantic_history_v1(tx, true)?;
    tx.execute_batch(
        "INSERT INTO semantic_origins
           SELECT * FROM __ae_semantic_origins_v1;
         INSERT INTO semantic_commits
           SELECT * FROM __ae_semantic_commits_v1;
         INSERT INTO semantic_cursor
           SELECT * FROM __ae_semantic_cursor_v1;
         INSERT INTO semantic_snapshots
           SELECT * FROM __ae_semantic_snapshots_v1;
         INSERT INTO semantic_graphs
           SELECT * FROM __ae_semantic_graphs_v1;
         INSERT INTO semantic_receipts
           SELECT * FROM __ae_semantic_receipts_v1;
         INSERT INTO semantic_telemetry
           SELECT * FROM __ae_semantic_telemetry_v1;
         INSERT INTO semantic_evidence_authority(
           persona_scope,semantic_revision,relation_present,relation_scope,event_id,
           incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
           state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
           evidence_bytes
         ) SELECT
           persona_scope,semantic_revision,relation_present,relation_scope,event_id,
           incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
           state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
           evidence_bytes
           FROM __ae_semantic_evidence_authority_v1;
         DROP TABLE __ae_semantic_evidence_authority_v1;
         DROP TABLE __ae_semantic_cursor_v1;
         DROP TABLE __ae_semantic_snapshots_v1;
         DROP TABLE __ae_semantic_graphs_v1;
         DROP TABLE __ae_semantic_receipts_v1;
         DROP TABLE __ae_semantic_telemetry_v1;
         DROP TABLE __ae_semantic_commits_v1;
         DROP TABLE __ae_semantic_origins_v1;",
    )?;
    if detect_semantic_schema_layout(tx)? != SemanticSchemaLayout::StrongV2 {
        return Err(StoreError::ContinuityFence("semantic_schema_migration"));
    }
    let violations: i64 =
        tx.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if sqlite_length(violations, "semantic_schema.foreign_key_violations")? != 0 {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_check",
        ));
    }
    Ok(())
}

fn migrate_strong_semantic_v2_to_v3(
    tx: &Transaction<'_>,
    supplemental_count: u64,
) -> Result<(), StoreError> {
    let staging: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
         WHERE name='__ae_semantic_evidence_authority_v2')",
        [],
        |row| row.get(0),
    )?;
    if staging {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_staging_collision",
        ));
    }
    verify_semantic_schema_v2_supplement(tx, supplemental_count)?;
    verify_semantic_schema_v2_namespace_v1(tx, supplemental_count)?;
    require_positive_table_rowids_v1(
        tx,
        "semantic_evidence_authority",
        "semantic_schema_migration_rowid_domain",
    )?;
    // The exact V2 tables remain live while every retained history row and
    // every discard-only challenge is admitted. No DROP/ALTER is permitted
    // before this closed snapshot exists.
    if supplemental_count == 2 {
        preflight_semantic_v3_history_v1(tx)?;
    } else {
        preflight_weak_semantic_history_v1(tx, false)?;
    }
    let challenge_permit = if supplemental_count == 2 {
        Some(preflight_perception_challenges_v1(
            tx,
            PerceptionChallengeStorageLayoutV1::V2,
        )?)
    } else {
        None
    };
    // V2 challenges were minted from caller-selected identity and therefore
    // cannot be promoted into V3 authority. They are ephemeral authorizations,
    // not semantic truth. Consume only the bounded rowid+nonce permit before
    // dropping the now-empty legacy table.
    if let Some(permit) = challenge_permit.as_ref() {
        if permit.layout != PerceptionChallengeStorageLayoutV1::V2 {
            return Err(StoreError::ContinuityFence("perception_challenge_layout"));
        }
        for token in &permit.tokens {
            delete_perception_challenge_token_v1(tx, token)?;
        }
    }
    tx.execute_batch(
        "DROP INDEX IF EXISTS semantic_evidence_relation_revision;
         DROP INDEX IF EXISTS perception_challenge_context_v1;
         DROP TABLE IF EXISTS perception_challenges;
         ALTER TABLE semantic_evidence_authority
           RENAME TO __ae_semantic_evidence_authority_v2;",
    )?;
    tx.execute_batch(SEMANTIC_SCHEMA_V3_SQL)?;
    require_semantic_columns(
        tx,
        "__ae_semantic_evidence_authority_v2",
        &SEMANTIC_EVIDENCE_V2_COLUMNS,
    )?;
    preflight_semantic_history_layout_v1(
        tx,
        &STRONG_V2_EVIDENCE_STAGING_SCOPE_TABLES_V1,
        &STRONG_V2_EVIDENCE_STAGING_SCAN_TABLES_V1,
        SEMANTIC_HISTORY_SCAN_LIMITS_V1,
    )?;
    require_positive_table_rowids_v1(
        tx,
        "__ae_semantic_evidence_authority_v2",
        "semantic_schema_migration_rowid_domain",
    )?;
    require_positive_table_rowids_v1(
        tx,
        "semantic_evidence_authority",
        "semantic_schema_migration_rowid_domain",
    )?;
    if tx
        .query_row(
            "SELECT 1 FROM semantic_evidence_authority NOT INDEXED LIMIT 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .is_some()
    {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_migration_target",
        ));
    }
    migrate_strong_v2_evidence_rows_v1(tx)?;
    tx.execute_batch("DROP TABLE __ae_semantic_evidence_authority_v2;")?;
    Ok(())
}

fn require_positive_table_rowids_v1(
    conn: &Connection,
    table: &'static str,
    fence: &'static str,
) -> Result<(), StoreError> {
    let sql = match table {
        "__ae_semantic_evidence_authority_v2" => {
            "SELECT 1 FROM __ae_semantic_evidence_authority_v2 NOT INDEXED
             WHERE rowid<=0 ORDER BY rowid LIMIT 1"
        }
        "semantic_evidence_authority" => {
            "SELECT 1 FROM semantic_evidence_authority NOT INDEXED
             WHERE rowid<=0 ORDER BY rowid LIMIT 1"
        }
        _ => return Err(StoreError::ContinuityFence("semantic_schema_table_set")),
    };
    if conn
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .optional()?
        .is_some()
    {
        return Err(StoreError::ContinuityFence(fence));
    }
    Ok(())
}

fn bounded_table_rowids_v1(
    conn: &Connection,
    table: &'static str,
    after: i64,
) -> Result<Vec<i64>, StoreError> {
    if after == 0 {
        require_positive_table_rowids_v1(conn, table, "semantic_schema_migration_rowid_domain")?;
    }
    let sql = match table {
        "__ae_semantic_evidence_authority_v2" => {
            "SELECT rowid FROM __ae_semantic_evidence_authority_v2 NOT INDEXED
             WHERE rowid>?1 ORDER BY rowid LIMIT ?2"
        }
        "semantic_evidence_authority" => {
            "SELECT rowid FROM semantic_evidence_authority NOT INDEXED
             WHERE rowid>?1 ORDER BY rowid LIMIT ?2"
        }
        _ => return Err(StoreError::ContinuityFence("semantic_schema_table_set")),
    };
    let mut statement = conn.prepare(sql)?;
    let mut rows = statement.query(params![
        after,
        i64::try_from(PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1).unwrap_or(128)
    ])?;
    let mut rowids = Vec::new();
    while let Some(row) = rows.next()? {
        let rowid: i64 = row.get(0)?;
        if rowid <= after || rowids.last().is_some_and(|previous| rowid <= *previous) {
            return Err(StoreError::ContinuityFence(
                "semantic_schema_migration_rowset",
            ));
        }
        rowids.push(rowid);
    }
    Ok(rowids)
}

fn bounded_table_row_count_v1(conn: &Connection, table: &'static str) -> Result<u64, StoreError> {
    let mut cursor = 0_i64;
    let mut count = 0_u64;
    loop {
        let rowids = bounded_table_rowids_v1(conn, table, cursor)?;
        if rowids.is_empty() {
            return Ok(count);
        }
        count = count
            .checked_add(u64::try_from(rowids.len()).unwrap_or(u64::MAX))
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic_schema.migration_rows",
                limit: MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "semantic_schema.migration_rows",
            count,
            MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
        )?;
        cursor = *rowids.last().ok_or(StoreError::ContinuityFence(
            "semantic_schema_migration_rowset",
        ))?;
        if rowids.len() < usize::try_from(PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1).unwrap_or(128) {
            return Ok(count);
        }
    }
}

fn migrate_strong_v2_evidence_rows_v1(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let expected = bounded_table_row_count_v1(tx, "__ae_semantic_evidence_authority_v2")?;
    let mut cursor = 0_i64;
    let mut copied = 0_u64;
    loop {
        let rowids = bounded_table_rowids_v1(tx, "__ae_semantic_evidence_authority_v2", cursor)?;
        if rowids.is_empty() {
            break;
        }
        for rowid in &rowids {
            let inserted = tx.execute(
                "INSERT INTO semantic_evidence_authority(
                   persona_scope,semantic_revision,relation_present,relation_scope,event_id,
                   incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
                   state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
                   evidence_bytes
                 ) SELECT
                   persona_scope,semantic_revision,relation_present,relation_scope,event_id,
                   incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
                   state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
                   evidence_bytes
                 FROM __ae_semantic_evidence_authority_v2 WHERE rowid=?1",
                params![rowid],
            )?;
            if inserted != 1 {
                return Err(StoreError::ContinuityFence(
                    "semantic_schema_migration_copy",
                ));
            }
            copied = copied.checked_add(1).ok_or(StoreError::ContinuityFence(
                "semantic_schema_migration_copy",
            ))?;
        }
        cursor = *rowids.last().ok_or(StoreError::ContinuityFence(
            "semantic_schema_migration_rowset",
        ))?;
        if rowids.len() < usize::try_from(PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1).unwrap_or(128) {
            break;
        }
    }
    let target = bounded_table_row_count_v1(tx, "semantic_evidence_authority")?;
    if copied != expected || target != expected {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_migration_parity",
        ));
    }
    Ok(())
}

struct CompactTimeMigrationV5 {
    persona_scope: Digest,
    semantic_revision: u64,
    old_commitment: Digest,
    new_commitment: Digest,
    snapshot_bytes: Vec<u8>,
    snapshot_wire_digest: Digest,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct SemanticTimePersonaWalkV5 {
    persona_count: u64,
    total_rows: u64,
    max_persona_rows: u64,
}

fn next_semantic_time_persona_v5(
    conn: &Connection,
    after: Option<&Digest>,
) -> Result<Option<Digest>, StoreError> {
    let read =
        |row: &rusqlite::Row<'_>| Ok((row.get::<_, Option<Vec<u8>>>(0)?, row.get::<_, i64>(1)?));
    let raw: Option<(Option<Vec<u8>>, i64)> = if let Some(after) = after {
        conn.query_row(
            "SELECT CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                         THEN persona_scope END,
                    CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END
             FROM semantic_commits
             WHERE transition_kind='time' AND persona_scope>?1
             ORDER BY persona_scope,semantic_revision
             LIMIT 1",
            params![blob(*after)],
            read,
        )
        .optional()?
    } else {
        conn.query_row(
            "SELECT CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                         THEN persona_scope END,
                    CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END
             FROM semantic_commits
             WHERE transition_kind='time'
             ORDER BY persona_scope,semantic_revision
             LIMIT 1",
            [],
            read,
        )
        .optional()?
    };
    raw.map(|(value, raw_len)| {
        bounded_typed_value(
            value,
            raw_len,
            32,
            "semantic_v5.persona_scope",
            "semantic_v5_persona_type",
        )
        .and_then(|value| digest_from_vec(value, "semantic_v5.persona_scope"))
    })
    .transpose()
}

fn for_each_semantic_time_persona_v5<F>(
    conn: &Connection,
    mut visit: F,
) -> Result<SemanticTimePersonaWalkV5, StoreError>
where
    F: FnMut(&Connection, Digest) -> Result<u64, StoreError>,
{
    let mut stats = SemanticTimePersonaWalkV5::default();
    let mut previous = None;
    while let Some(persona_scope) = next_semantic_time_persona_v5(conn, previous.as_ref())? {
        if previous.is_some_and(|value| persona_scope <= value) {
            return Err(StoreError::ContinuityFence("semantic_v5_persona_keyset"));
        }
        let next_persona_count =
            stats
                .persona_count
                .checked_add(1)
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "semantic_v5.time_personas.global",
                    limit: MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
                    actual: u64::MAX,
                })?;
        // Reject the first forbidden persona before the visitor can scan or
        // rewrite any of its rows.
        enforce_byte_budget(
            "semantic_v5.time_personas.global",
            next_persona_count,
            MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
        )?;
        let persona_rows = visit(conn, persona_scope)?;
        if persona_rows == 0 {
            return Err(StoreError::ContinuityFence(
                "semantic_v5_time_persona_row_set",
            ));
        }
        enforce_byte_budget(
            "semantic_v5.time_rows.persona",
            persona_rows,
            MAX_SEMANTIC_ROWS_PER_PERSONA,
        )?;
        let total_rows = stats.total_rows.checked_add(persona_rows).ok_or(
            StoreError::StorageBudgetExceeded {
                resource: "semantic_v5.time_rows.global",
                limit: MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
                actual: u64::MAX,
            },
        )?;
        enforce_byte_budget(
            "semantic_v5.time_rows.global",
            total_rows,
            MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
        )?;
        stats.persona_count = next_persona_count;
        stats.total_rows = total_rows;
        stats.max_persona_rows = stats.max_persona_rows.max(persona_rows);
        previous = Some(persona_scope);
    }
    Ok(stats)
}

fn verify_semantic_time_rows_per_persona_v5(
    conn: &Connection,
    limit: u64,
) -> Result<(), StoreError> {
    for_each_semantic_time_persona_v5(conn, |conn, persona_scope| {
        semantic_time_row_count_for_persona_with_limit_v5(conn, &persona_scope, limit)
    })?;
    Ok(())
}

fn semantic_time_row_count_for_persona_v5(
    conn: &Connection,
    persona_scope: &Digest,
) -> Result<u64, StoreError> {
    semantic_time_row_count_for_persona_with_limit_v5(
        conn,
        persona_scope,
        MAX_SEMANTIC_ROWS_PER_PERSONA,
    )
}

fn semantic_time_row_count_for_persona_with_limit_v5(
    conn: &Connection,
    persona_scope: &Digest,
    limit: u64,
) -> Result<u64, StoreError> {
    let first_forbidden = limit
        .checked_add(1)
        .ok_or(StoreError::StorageBudgetExceeded {
            resource: "semantic_v5.time_rows.persona",
            limit,
            actual: u64::MAX,
        })?;
    let sql_limit =
        i64::try_from(first_forbidden).map_err(|_| StoreError::StorageBudgetExceeded {
            resource: "semantic_v5.time_rows.persona",
            limit,
            actual: first_forbidden,
        })?;
    let mut statement = conn.prepare(
        "SELECT semantic_revision FROM semantic_commits
         WHERE transition_kind='time' AND persona_scope=?1
         ORDER BY semantic_revision
         LIMIT ?2",
    )?;
    let mut rows = statement.query(params![blob(*persona_scope), sql_limit])?;
    let mut count = 0_u64;
    while rows.next()?.is_some() {
        count = count
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic_v5.time_rows.persona",
                limit,
                actual: u64::MAX,
            })?;
        enforce_byte_budget("semantic_v5.time_rows.persona", count, limit)?;
    }
    Ok(count)
}

/// Authenticate every prerelease V4 full time snapshot for one persona before
/// replacing any row in that persona with its compact V5 projection.
fn compact_semantic_time_persona_v5(
    conn: &Connection,
    expected_persona_scope: &Digest,
) -> Result<u64, StoreError> {
    let expected_count = semantic_time_row_count_for_persona_v5(conn, expected_persona_scope)?;
    type Raw = (
        Vec<u8>,
        i64,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
    );
    let mut statement = conn.prepare(
        "SELECT c.persona_scope,c.semantic_revision,c.journal_revision,c.event_id,c.event_digest,
                c.incarnation_id,c.manifest_digest,c.route_digest,c.formula_digest,
                c.state_before,c.state_digest,c.graph_digest,c.authority_digest,c.origin_digest,
                c.commitment_digest,c.snapshot_wire_digest,c.graph_wire_digest,
                CASE WHEN typeof(t.authority_bytes)='blob' AND length(t.authority_bytes)<=16384 THEN t.authority_bytes END,
                CASE WHEN typeof(t.authority_bytes)='blob' THEN length(t.authority_bytes) ELSE -1 END,
                CASE WHEN typeof(s.snapshot_bytes)='blob' AND length(s.snapshot_bytes)<=16777216 THEN s.snapshot_bytes END,
                CASE WHEN typeof(s.snapshot_bytes)='blob' THEN length(s.snapshot_bytes) ELSE -1 END,
                CASE WHEN typeof(g.graph_bytes)='blob' AND length(g.graph_bytes)<=16777216 THEN g.graph_bytes END,
                CASE WHEN typeof(g.graph_bytes)='blob' THEN length(g.graph_bytes) ELSE -1 END
         FROM semantic_commits AS c
         JOIN semantic_time_authority AS t USING(persona_scope,semantic_revision)
         JOIN semantic_snapshots AS s USING(persona_scope,semantic_revision)
         JOIN semantic_graphs AS g USING(persona_scope,semantic_revision)
         WHERE c.transition_kind='time' AND c.persona_scope=?1
         ORDER BY c.persona_scope,c.semantic_revision",
    )?;
    let mut rows = statement.query(params![blob(*expected_persona_scope)])?;
    let capacity = usize::try_from(expected_count)
        .map_err(|_| StoreError::ContinuityFence("semantic_v5_time_row_count"))?;
    let mut migrations = Vec::with_capacity(capacity);
    while let Some(row) = rows.next()? {
        let raw: Raw = (
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
            row.get(8)?,
            row.get(9)?,
            row.get(10)?,
            row.get(11)?,
            row.get(12)?,
            row.get(13)?,
            row.get(14)?,
            row.get(15)?,
            row.get(16)?,
            row.get(17)?,
            row.get(18)?,
            row.get(19)?,
            row.get(20)?,
            row.get(21)?,
            row.get(22)?,
        );
        let persona_scope = digest_from_vec(raw.0, "semantic_v5.persona")?;
        if persona_scope != *expected_persona_scope {
            return Err(StoreError::ContinuityFence("semantic_v5_persona_keyset"));
        }
        let semantic_revision = semantic_revision_from_sql(raw.1)?;
        let journal_revision = semantic_revision_from_sql(raw.2)?;
        let event_id = id_from_vec(raw.3, "semantic_v5.event_id")?;
        let event_digest = digest_from_vec(raw.4, "semantic_v5.event")?;
        let incarnation_id = digest_from_vec(raw.5, "semantic_v5.incarnation")?;
        let manifest_digest = digest_from_vec(raw.6, "semantic_v5.manifest")?;
        let route_digest = digest_from_vec(raw.7, "semantic_v5.route")?;
        let formula_digest = digest_from_vec(raw.8, "semantic_v5.formula")?;
        let state_before = digest_from_vec(raw.9, "semantic_v5.state_before")?;
        let state_after = digest_from_vec(raw.10, "semantic_v5.state_after")?;
        let graph = digest_from_vec(raw.11, "semantic_v5.graph")?;
        let event_authority = digest_from_vec(raw.12, "semantic_v5.event_authority")?;
        let origin_digest = digest_from_vec(raw.13, "semantic_v5.origin")?;
        let old_commitment = digest_from_vec(raw.14, "semantic_v5.commitment")?;
        let old_snapshot_wire = digest_from_vec(raw.15, "semantic_v5.snapshot_wire")?;
        let old_graph_wire = digest_from_vec(raw.16, "semantic_v5.graph_wire")?;
        let authority_bytes = bounded_typed_value(
            raw.17,
            raw.18,
            16_384,
            "semantic_v5.authority_bytes",
            "semantic_v5_authority_type",
        )?;
        let old_snapshot = bounded_typed_value(
            raw.19,
            raw.20,
            MAX_SNAPSHOT_STATE_BYTES,
            "semantic_v5.snapshot_bytes",
            "semantic_v5_snapshot_type",
        )?;
        let old_graph = bounded_typed_value(
            raw.21,
            raw.22,
            MAX_SEMANTIC_GRAPH_BYTES,
            "semantic_v5.graph_bytes",
            "semantic_v5_graph_type",
        )?;
        let authority: SemanticTimeAuthorityV1 = serde_json::from_slice(&authority_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_v5_authority_wire"))?;
        let canonical_authority = serde_json::to_vec(&authority)
            .map_err(|_| StoreError::ContinuityFence("semantic_v5_authority_wire"))?;
        let time_authority_digest = wire::domain_hash(
            b"astr-embodiment/semantic-time-authority-v1",
            &[&authority_bytes],
        );
        let decoded =
            decode_legacy_time_snapshot_v1(&old_snapshot).map_err(semantic_core_history_error)?;
        if !authority.validate_v1()
            || canonical_authority != authority_bytes
            || authority.persona_scope != persona_scope
            || authority.semantic_revision != semantic_revision
            || authority.journal_revision != journal_revision
            || authority.event_id != event_id
            || authority.event_digest != event_digest
            || authority.incarnation_id != incarnation_id
            || authority.manifest_digest != manifest_digest
            || authority.route_digest != route_digest
            || authority.semantic_formula_digest != formula_digest
            || authority.state_before != state_before
            || authority.state_after != state_after
            || authority.graph_digest != graph
            || decoded.semantic_formula_digest != formula_digest
            || decoded.time_formula_digest != authority.time_formula_digest
            || state_digest(&decoded.field, &formula_digest) != state_after
            || graph_digest(&decoded.graph) != graph
            || encode_canonical_graph_v1(&decoded.graph).map_err(semantic_core_history_error)?
                != old_graph
            || wire::domain_hash(SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1, &[&old_snapshot])
                != old_snapshot_wire
            || wire::domain_hash(SEMANTIC_GRAPH_WIRE_DOMAIN_V1, &[&old_graph]) != old_graph_wire
            || legacy_time_commitment_digest_v4(
                &persona_scope,
                semantic_revision,
                journal_revision,
                &event_id,
                &event_digest,
                &incarnation_id,
                &manifest_digest,
                &route_digest,
                &formula_digest,
                &state_before,
                &state_after,
                &graph,
                &event_authority,
                &old_snapshot_wire,
                &old_graph_wire,
                &origin_digest,
                &time_authority_digest,
            ) != old_commitment
        {
            return Err(StoreError::ContinuityFence("semantic_v5_time_preimage"));
        }
        let new_commitment = time_commitment_digest_v1(
            &persona_scope,
            semantic_revision,
            journal_revision,
            &event_id,
            &event_digest,
            &incarnation_id,
            &manifest_digest,
            &route_digest,
            &formula_digest,
            &state_before,
            &state_after,
            &graph,
            &event_authority,
            &origin_digest,
            &time_authority_digest,
        );
        let snapshot_bytes = encode_time_snapshot_v1(&DecodedTimeSnapshotV1 {
            semantic_formula_digest: formula_digest,
            time_formula_digest: authority.time_formula_digest,
            epoch_before: authority.epoch_before.clone(),
            epoch_after: authority.epoch_after.clone(),
            requested_elapsed_ms: authority.raw_elapsed_ms,
            applied_elapsed_ms: authority.applied_elapsed_ms,
            capped_gap: authority.capped_gap,
            pre_sleep_phase: authority.pre_sleep_phase,
            state_before,
            state_after,
            graph_digest: graph,
            authority_digest: event_authority,
            commitment_digest: new_commitment,
        })
        .map_err(semantic_core_history_error)?;
        let snapshot_wire_digest =
            wire::domain_hash(SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1, &[&snapshot_bytes]);
        migrations.push(CompactTimeMigrationV5 {
            persona_scope,
            semantic_revision,
            old_commitment,
            new_commitment,
            snapshot_bytes,
            snapshot_wire_digest,
        });
    }
    drop(rows);
    drop(statement);
    if u64::try_from(migrations.len()).unwrap_or(u64::MAX) != expected_count {
        return Err(StoreError::ContinuityFence(
            "semantic_v5_time_persona_row_set",
        ));
    }
    for migration in migrations {
        let revision = JournalRevision::new(migration.semantic_revision)
            .to_sqlite()?
            .get();
        let changed = conn.execute(
            "UPDATE semantic_commits SET commitment_digest=?4,snapshot_wire_digest=?5,graph_wire_digest=?6
             WHERE persona_scope=?1 AND semantic_revision=?2 AND transition_kind='time'
               AND commitment_digest=?3",
            params![
                blob(migration.persona_scope),
                revision,
                blob(migration.old_commitment),
                blob(migration.new_commitment),
                blob(migration.snapshot_wire_digest),
                blob([0_u8; 32]),
            ],
        )?;
        if changed != 1 {
            return Err(StoreError::ContinuityFence("semantic_v5_commit_cas"));
        }
        let snapshot_changed = conn.execute(
            "UPDATE semantic_snapshots SET commitment_digest=?3,snapshot_bytes=?4
             WHERE persona_scope=?1 AND semantic_revision=?2 AND commitment_digest=?5",
            params![
                blob(migration.persona_scope),
                revision,
                blob(migration.new_commitment),
                migration.snapshot_bytes,
                blob(migration.old_commitment),
            ],
        )?;
        let graph_deleted = conn.execute(
            "DELETE FROM semantic_graphs WHERE persona_scope=?1 AND semantic_revision=?2",
            params![blob(migration.persona_scope), revision],
        )?;
        if snapshot_changed != 1 || graph_deleted != 1 {
            return Err(StoreError::ContinuityFence("semantic_v5_sidecar_cas"));
        }
        conn.execute(
            "UPDATE semantic_cursor SET commitment_digest=?3
             WHERE persona_scope=?1 AND semantic_revision=?2 AND commitment_digest=?4",
            params![
                blob(migration.persona_scope),
                revision,
                blob(migration.new_commitment),
                blob(migration.old_commitment),
            ],
        )?;
        conn.execute(
            "UPDATE semantic_budget_checkpoint SET head_commitment_digest=?3
             WHERE persona_scope=?1 AND head_revision=?2 AND head_commitment_digest=?4",
            params![
                blob(migration.persona_scope),
                revision,
                blob(migration.new_commitment),
                blob(migration.old_commitment),
            ],
        )?;
    }
    Ok(expected_count)
}

/// Walk persona scopes by keyset so migration memory is bounded by one
/// persona. The caller's Immediate transaction still makes every persona's
/// changes commit or roll back as a single database migration.
fn compact_semantic_time_rows_v5(conn: &Connection) -> Result<(), StoreError> {
    for_each_semantic_time_persona_v5(conn, |conn, persona_scope| {
        compact_semantic_time_persona_v5(conn, &persona_scope)
    })?;
    // V4 stored time snapshots in two sidecars. Recalculate each affected
    // checkpoint with the same bounded length-only scanner used by open/audit;
    // never run one correlated aggregate over the entire database.
    let mut last_scope = None;
    let mut global_budget = SemanticHistoryScanBudgetV1::default();
    while let Some(persona_scope) =
        next_semantic_persona_scope_in_table_v1(conn, "semantic_budget_checkpoint", last_scope)?
    {
        global_budget.admit_persona()?;
        let scanned = scan_semantic_history_scope_v1(conn, persona_scope, &mut global_budget)?;
        conn.execute(
            "UPDATE semantic_budget_checkpoint SET aggregate_bytes=?2 WHERE persona_scope=?1",
            params![
                blob(persona_scope),
                JournalRevision::new(scanned.aggregate_bytes)
                    .to_sqlite()?
                    .get(),
            ],
        )?;
        last_scope = Some(persona_scope);
    }
    Ok(())
}

fn semantic_appraisal_schema_objects_v1(
    conn: &Connection,
) -> Result<Vec<SemanticSchemaObjectV1>, StoreError> {
    bounded_semantic_schema_objects_v1(conn, true)
}

fn verify_semantic_appraisal_schema_against_v1(
    conn: &Connection,
    schema_sql: &str,
) -> Result<(), StoreError> {
    let reference = Connection::open_in_memory()?;
    reference.execute_batch(schema_sql)?;
    if semantic_appraisal_schema_objects_v1(conn)?
        != semantic_appraisal_schema_objects_v1(&reference)?
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_schema_sql_identity",
        ));
    }
    Ok(())
}

fn verify_semantic_appraisal_schema_v1(conn: &Connection) -> Result<(), StoreError> {
    verify_semantic_appraisal_schema_against_v1(conn, SEMANTIC_APPRAISAL_SCHEMA_V3_SQL)
}

fn semantic_appraisal_schema_version_v1(conn: &Connection) -> Result<Option<u8>, StoreError> {
    let raw: Option<Vec<u8>> = conn
        .query_row(
            "SELECT CASE WHEN typeof(value)='blob' AND length(value)=1 THEN value END
             FROM meta WHERE key='semantic_appraisal_schema_version'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    raw.map(|value| {
        value.first().copied().ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_schema_version",
        ))
    })
    .transpose()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SemanticAppraisalStorageLayoutV1 {
    V1,
    V2,
    V3,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SemanticAppraisalStorageBudgetV1 {
    personas: u64,
    budget_rows: u64,
    claim_rows: u64,
    aggregate_bytes: u64,
}

impl SemanticAppraisalStorageBudgetV1 {
    fn admit_persona(&mut self) -> Result<(), StoreError> {
        let actual = self
            .personas
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic_appraisal.personas",
                limit: MAX_SEMANTIC_APPRAISAL_PERSONAS_GLOBAL,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "semantic_appraisal.personas",
            actual,
            MAX_SEMANTIC_APPRAISAL_PERSONAS_GLOBAL,
        )?;
        self.personas = actual;
        Ok(())
    }

    fn admit_budget_row(&mut self) -> Result<(), StoreError> {
        let actual = self
            .budget_rows
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic_appraisal.budget_rows",
                limit: MAX_SEMANTIC_APPRAISAL_BUDGET_ROWS_GLOBAL,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "semantic_appraisal.budget_rows",
            actual,
            MAX_SEMANTIC_APPRAISAL_BUDGET_ROWS_GLOBAL,
        )?;
        self.budget_rows = actual;
        Ok(())
    }

    fn admit_claim_row(&mut self) -> Result<(), StoreError> {
        let actual = self
            .claim_rows
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic_appraisal.claim_rows",
                limit: MAX_SEMANTIC_APPRAISAL_CLAIMS_GLOBAL,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "semantic_appraisal.claim_rows",
            actual,
            MAX_SEMANTIC_APPRAISAL_CLAIMS_GLOBAL,
        )?;
        self.claim_rows = actual;
        Ok(())
    }

    fn admit_payload_bytes(&mut self, bytes: u64) -> Result<(), StoreError> {
        let actual =
            self.aggregate_bytes
                .checked_add(bytes)
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "semantic_appraisal.aggregate_bytes",
                    limit: MAX_SEMANTIC_APPRAISAL_AGGREGATE_BYTES_GLOBAL,
                    actual: u64::MAX,
                })?;
        enforce_byte_budget(
            "semantic_appraisal.aggregate_bytes",
            actual,
            MAX_SEMANTIC_APPRAISAL_AGGREGATE_BYTES_GLOBAL,
        )?;
        self.aggregate_bytes = actual;
        Ok(())
    }
}

const SEMANTIC_APPRAISAL_SCAN_BATCH_ROWS_V1: u64 = 128;

const SEMANTIC_APPRAISAL_BUDGET_PHASE_ONE_SCAN_V1_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' AND utc_day>=0 THEN utc_day END
     FROM semantic_appraisal_budget NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const SEMANTIC_APPRAISAL_CLAIM_PHASE_ONE_SCAN_V1_SQL: &str =
    "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(request_nonce_digest)='blob'
                 THEN length(request_nonce_digest) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' AND utc_day>=0 THEN utc_day END,
            CASE WHEN settled_at_ms IS NULL THEN 0
                 WHEN typeof(settled_at_ms)='integer' THEN 1 ELSE -1 END,
            CASE WHEN outcome_code IS NULL THEN -2
                 WHEN typeof(outcome_code)='text' THEN length(CAST(outcome_code AS BLOB)) ELSE -1 END,
            CASE WHEN reply_affect_bytes IS NULL THEN -2
                 WHEN typeof(reply_affect_bytes)='blob' THEN length(reply_affect_bytes) ELSE -1 END,
            CASE WHEN terminal_receipt_bytes IS NULL THEN -2
                 WHEN typeof(terminal_receipt_bytes)='blob'
                 THEN length(terminal_receipt_bytes) ELSE -1 END
     FROM semantic_appraisal_claim NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const SEMANTIC_APPRAISAL_CLAIM_PHASE_ONE_SCAN_LEGACY_V1_SQL: &str =
    "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(request_nonce_digest)='blob'
                 THEN length(request_nonce_digest) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' AND utc_day>=0 THEN utc_day END,
            CASE WHEN settled_at_ms IS NULL THEN 0
                 WHEN typeof(settled_at_ms)='integer' THEN 1 ELSE -1 END,
            CASE WHEN outcome_code IS NULL THEN -2
                 WHEN typeof(outcome_code)='text' THEN length(CAST(outcome_code AS BLOB)) ELSE -1 END,
            -2,-2
     FROM semantic_appraisal_claim NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const SEMANTIC_APPRAISAL_BUDGET_PHASE_TWO_SCAN_V1_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' THEN utc_day END,
            CASE WHEN typeof(daily_token_limit)='integer' THEN daily_token_limit END,
            CASE WHEN typeof(charged_tokens)='integer' THEN charged_tokens END,
            CASE WHEN typeof(reserved_tokens)='integer' THEN reserved_tokens END,
            CASE WHEN typeof(blocked)='integer' THEN blocked END,
            CASE WHEN typeof(updated_at_ms)='integer' THEN updated_at_ms END
     FROM semantic_appraisal_budget NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const SEMANTIC_APPRAISAL_BUDGET_PHASE_TWO_SCAN_V3_SQL: &str = "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' THEN utc_day END,
            CASE WHEN typeof(daily_token_limit)='integer' THEN daily_token_limit END,
            CASE WHEN typeof(charged_tokens)='integer' THEN charged_tokens END,
            CASE WHEN typeof(reserved_tokens)='integer' THEN reserved_tokens END,
            CASE WHEN typeof(blocked)='integer' THEN blocked END,
            CASE WHEN typeof(updated_at_ms)='integer' THEN updated_at_ms END,
            CASE WHEN typeof(compacted_claim_rows)='integer' THEN compacted_claim_rows END,
            CASE WHEN typeof(compacted_charged_tokens)='integer'
                 THEN compacted_charged_tokens END,
            CASE WHEN typeof(compacted_chain_digest)='blob'
                 THEN length(compacted_chain_digest) ELSE -1 END,
            CASE WHEN typeof(compacted_chain_digest)='blob'
                      AND length(compacted_chain_digest)=32
                 THEN compacted_chain_digest END
     FROM semantic_appraisal_budget NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

#[derive(Clone, Copy, Debug)]
enum SemanticAppraisalTableV1 {
    Budget,
    Claim,
}

fn reject_nonpositive_semantic_appraisal_rowid_v1(
    conn: &Connection,
    table: SemanticAppraisalTableV1,
) -> Result<(), StoreError> {
    let sql = match table {
        SemanticAppraisalTableV1::Budget => {
            "SELECT 1 FROM semantic_appraisal_budget NOT INDEXED
             WHERE rowid<=0 ORDER BY rowid LIMIT 1"
        }
        SemanticAppraisalTableV1::Claim => {
            "SELECT 1 FROM semantic_appraisal_claim NOT INDEXED
             WHERE rowid<=0 ORDER BY rowid LIMIT 1"
        }
    };
    if conn
        .query_row(sql, [], |row| row.get::<_, i64>(0))
        .optional()?
        .is_some()
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_rowid_domain",
        ));
    }
    Ok(())
}

fn appraisal_nonnegative_integer_v1(
    value: Option<i64>,
    fence: &'static str,
) -> Result<u64, StoreError> {
    value
        .and_then(|value| u64::try_from(value).ok())
        .ok_or(StoreError::ContinuityFence(fence))
}

fn appraisal_positive_integer_v1(
    value: Option<i64>,
    fence: &'static str,
) -> Result<u64, StoreError> {
    let value = appraisal_nonnegative_integer_v1(value, fence)?;
    if value == 0 {
        return Err(StoreError::ContinuityFence(fence));
    }
    Ok(value)
}

fn appraisal_required_digest_length_v1(
    length: i64,
    field: &'static str,
    type_fence: &'static str,
) -> Result<(), StoreError> {
    if length < 0 {
        return Err(StoreError::ContinuityFence(type_fence));
    }
    let actual = u64::try_from(length).unwrap_or(u64::MAX);
    if actual != 32 {
        return Err(StoreError::InvalidStoredDigest { field, actual });
    }
    Ok(())
}

fn appraisal_optional_digest_length_v1(
    length: i64,
    field: &'static str,
    type_fence: &'static str,
) -> Result<bool, StoreError> {
    if length == -2 {
        return Ok(false);
    }
    appraisal_required_digest_length_v1(length, field, type_fence)?;
    Ok(true)
}

fn appraisal_required_payload_length_v1(
    length: i64,
    limit: u64,
    resource: &'static str,
    type_fence: &'static str,
) -> Result<u64, StoreError> {
    if length < 0 {
        return Err(StoreError::ContinuityFence(type_fence));
    }
    let length = u64::try_from(length).unwrap_or(u64::MAX);
    enforce_byte_budget(resource, length, limit)?;
    Ok(length)
}

fn semantic_appraisal_scan_limit_v1(remaining: u64) -> Result<i64, StoreError> {
    let limit = remaining
        .checked_add(1)
        .unwrap_or(u64::MAX)
        .min(SEMANTIC_APPRAISAL_SCAN_BATCH_ROWS_V1);
    i64::try_from(limit).map_err(|_| StoreError::ContinuityFence("semantic_appraisal_scan_limit"))
}

fn admit_semantic_appraisal_persona_v1(
    persona_scope: Digest,
    personas: &mut BTreeSet<Digest>,
    storage: &mut SemanticAppraisalStorageBudgetV1,
) -> Result<(), StoreError> {
    if !personas.contains(&persona_scope) {
        storage.admit_persona()?;
        personas.insert(persona_scope);
    }
    Ok(())
}

fn admit_optional_semantic_appraisal_payload_v1(
    length: i64,
    per_value_limit: u64,
    resource: &'static str,
    type_fence: &'static str,
    storage: &mut SemanticAppraisalStorageBudgetV1,
) -> Result<bool, StoreError> {
    if length == -2 {
        return Ok(false);
    }
    if length < 0 {
        return Err(StoreError::ContinuityFence(type_fence));
    }
    let length = u64::try_from(length).unwrap_or(u64::MAX);
    storage.admit_payload_bytes(length)?;
    enforce_byte_budget(resource, length, per_value_limit)?;
    Ok(true)
}

fn scan_semantic_appraisal_phase_one_v1(
    conn: &Connection,
    layout: SemanticAppraisalStorageLayoutV1,
) -> Result<SemanticAppraisalStorageBudgetV1, StoreError> {
    reject_nonpositive_semantic_appraisal_rowid_v1(conn, SemanticAppraisalTableV1::Budget)?;
    reject_nonpositive_semantic_appraisal_rowid_v1(conn, SemanticAppraisalTableV1::Claim)?;

    let mut storage = SemanticAppraisalStorageBudgetV1::default();
    let mut personas = BTreeSet::new();
    let mut pending_by_persona = BTreeMap::<Digest, u64>::new();

    let mut cursor = 0_i64;
    loop {
        let remaining =
            MAX_SEMANTIC_APPRAISAL_BUDGET_ROWS_GLOBAL.saturating_sub(storage.budget_rows);
        let limit = semantic_appraisal_scan_limit_v1(remaining)?;
        let mut statement = conn.prepare(SEMANTIC_APPRAISAL_BUDGET_PHASE_ONE_SCAN_V1_SQL)?;
        let mut rows = statement.query(params![cursor, limit])?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let persona_scope = stored_typed_digest(
                row.get(1)?,
                row.get(2)?,
                "semantic_appraisal.persona",
                "semantic_appraisal_persona_type",
            )?;
            admit_semantic_appraisal_persona_v1(persona_scope, &mut personas, &mut storage)?;
            storage.admit_budget_row()?;
            appraisal_nonnegative_integer_v1(row.get(3)?, "semantic_appraisal_budget_day")?;
            visited += 1;
            last_rowid = Some(rowid);
        }
        drop(rows);
        drop(statement);
        if visited < limit {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_budget_rowset",
        ))?;
    }

    let claim_sql = match layout {
        SemanticAppraisalStorageLayoutV1::V1 => {
            SEMANTIC_APPRAISAL_CLAIM_PHASE_ONE_SCAN_LEGACY_V1_SQL
        }
        SemanticAppraisalStorageLayoutV1::V2 | SemanticAppraisalStorageLayoutV1::V3 => {
            SEMANTIC_APPRAISAL_CLAIM_PHASE_ONE_SCAN_V1_SQL
        }
    };
    cursor = 0;
    loop {
        let remaining = MAX_SEMANTIC_APPRAISAL_CLAIMS_GLOBAL.saturating_sub(storage.claim_rows);
        let limit = semantic_appraisal_scan_limit_v1(remaining)?;
        let mut statement = conn.prepare(claim_sql)?;
        let mut rows = statement.query(params![cursor, limit])?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            let persona_scope = stored_typed_digest(
                row.get(1)?,
                row.get(2)?,
                "semantic_appraisal.persona",
                "semantic_appraisal_persona_type",
            )?;
            admit_semantic_appraisal_persona_v1(persona_scope, &mut personas, &mut storage)?;
            storage.admit_claim_row()?;
            appraisal_required_digest_length_v1(
                row.get(3)?,
                "semantic_appraisal.nonce",
                "semantic_appraisal_nonce_type",
            )?;
            appraisal_nonnegative_integer_v1(row.get(4)?, "semantic_appraisal_claim_day")?;
            let settlement_marker: i64 = row.get(5)?;
            let has_outcome = admit_optional_semantic_appraisal_payload_v1(
                row.get(6)?,
                MAX_SEMANTIC_APPRAISAL_OUTCOME_BYTES,
                "semantic_appraisal.outcome_code",
                "semantic_appraisal_outcome_type",
                &mut storage,
            )?;
            let _ = admit_optional_semantic_appraisal_payload_v1(
                row.get(7)?,
                MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES,
                "semantic_appraisal.reply_affect_bytes",
                "semantic_appraisal_reply_affect_type",
                &mut storage,
            )?;
            let _ = admit_optional_semantic_appraisal_payload_v1(
                row.get(8)?,
                MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES,
                "semantic_appraisal.terminal_receipt_bytes",
                "semantic_appraisal_terminal_receipt_type",
                &mut storage,
            )?;
            match settlement_marker {
                0 => {
                    let pending = pending_by_persona.entry(persona_scope).or_default();
                    let actual =
                        pending
                            .checked_add(1)
                            .ok_or(StoreError::StorageBudgetExceeded {
                                resource: "semantic_appraisal.pending_rows_per_persona",
                                limit: MAX_SEMANTIC_APPRAISAL_REPAIR_ROWS_PER_PERSONA,
                                actual: u64::MAX,
                            })?;
                    enforce_byte_budget(
                        "semantic_appraisal.pending_rows_per_persona",
                        actual,
                        MAX_SEMANTIC_APPRAISAL_REPAIR_ROWS_PER_PERSONA,
                    )?;
                    *pending = actual;
                    storage.admit_payload_bytes(MAX_SEMANTIC_APPRAISAL_PENDING_PAYLOAD_BYTES)?;
                }
                1 if has_outcome => {}
                1 => {
                    return Err(StoreError::ContinuityFence(
                        "semantic_appraisal_terminal_shape",
                    ))
                }
                _ => {
                    return Err(StoreError::ContinuityFence(
                        "semantic_appraisal_claim_settled_type",
                    ))
                }
            }
            visited += 1;
            last_rowid = Some(rowid);
        }
        drop(rows);
        drop(statement);
        if visited < limit {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_claim_rowset",
        ))?;
    }
    Ok(storage)
}

#[derive(Clone, Copy, Debug)]
struct SemanticAppraisalBudgetClosureV1 {
    daily_token_limit: u64,
    charged_tokens: u64,
    reserved_tokens: u64,
    blocked: bool,
    compacted_claim_rows: u64,
    compacted_charged_tokens: u64,
    compacted_chain_digest: Digest,
    pending_reserved_tokens: u64,
    settled_charged_tokens: u64,
    requires_blocked: bool,
}

fn scan_semantic_appraisal_budgets_v1(
    conn: &Connection,
    layout: SemanticAppraisalStorageLayoutV1,
    expected_rows: u64,
) -> Result<BTreeMap<(Digest, u64), SemanticAppraisalBudgetClosureV1>, StoreError> {
    let mut budgets = BTreeMap::new();
    let mut cursor = 0_i64;
    let mut scanned = 0_u64;
    loop {
        let remaining = expected_rows.saturating_sub(scanned);
        let limit = semantic_appraisal_scan_limit_v1(remaining)?;
        let sql = if layout == SemanticAppraisalStorageLayoutV1::V3 {
            SEMANTIC_APPRAISAL_BUDGET_PHASE_TWO_SCAN_V3_SQL
        } else {
            SEMANTIC_APPRAISAL_BUDGET_PHASE_TWO_SCAN_V1_SQL
        };
        let mut statement = conn.prepare(sql)?;
        let mut rows = statement.query(params![cursor, limit])?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            let rowid: i64 = row.get(0)?;
            scanned = scanned.checked_add(1).ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_budget_rowset",
            ))?;
            if scanned > expected_rows {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_budget_rowset",
                ));
            }
            let persona_scope = stored_typed_digest(
                row.get(1)?,
                row.get(2)?,
                "semantic_appraisal.persona",
                "semantic_appraisal_persona_type",
            )?;
            let utc_day =
                appraisal_nonnegative_integer_v1(row.get(3)?, "semantic_appraisal_budget_day")?;
            let daily_token_limit =
                appraisal_nonnegative_integer_v1(row.get(4)?, "semantic_appraisal_budget_limit")?;
            if daily_token_limit > 1_000_000 {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_budget_limit",
                ));
            }
            let charged_tokens =
                appraisal_nonnegative_integer_v1(row.get(5)?, "semantic_appraisal_budget_charged")?;
            let reserved_tokens = appraisal_nonnegative_integer_v1(
                row.get(6)?,
                "semantic_appraisal_budget_reserved",
            )?;
            let blocked = match row.get::<_, Option<i64>>(7)? {
                Some(0) => false,
                Some(1) => true,
                _ => {
                    return Err(StoreError::ContinuityFence(
                        "semantic_appraisal_budget_blocked",
                    ))
                }
            };
            let updated_at_ms =
                appraisal_positive_integer_v1(row.get(8)?, "semantic_appraisal_budget_updated")?;
            if utc_day > updated_at_ms / SEMANTIC_APPRAISAL_UTC_DAY_MS {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_budget_day_closure",
                ));
            }
            let (compacted_claim_rows, compacted_charged_tokens, compacted_chain_digest) =
                if layout == SemanticAppraisalStorageLayoutV1::V3 {
                    let compacted_claim_rows = appraisal_nonnegative_integer_v1(
                        row.get(9)?,
                        "semantic_appraisal_budget_compacted_claim_rows",
                    )?;
                    let compacted_charged_tokens = appraisal_nonnegative_integer_v1(
                        row.get(10)?,
                        "semantic_appraisal_budget_compacted_charged_tokens",
                    )?;
                    appraisal_required_digest_length_v1(
                        row.get(11)?,
                        "semantic_appraisal.compacted_chain",
                        "semantic_appraisal_budget_compacted_chain_type",
                    )?;
                    let compacted_chain_digest = digest_from_vec(
                        row.get::<_, Option<Vec<u8>>>(12)?
                            .ok_or(StoreError::ContinuityFence(
                                "semantic_appraisal_budget_compacted_chain_type",
                            ))?,
                        "semantic_appraisal.compacted_chain",
                    )?;
                    if compacted_charged_tokens > charged_tokens
                        || (compacted_claim_rows == 0
                            && (compacted_charged_tokens != 0 || compacted_chain_digest != [0; 32]))
                        || (compacted_claim_rows > 0 && compacted_chain_digest == [0; 32])
                    {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_budget_compaction_closure",
                        ));
                    }
                    (
                        compacted_claim_rows,
                        compacted_charged_tokens,
                        compacted_chain_digest,
                    )
                } else {
                    (0, 0, [0; 32])
                };
            if budgets
                .insert(
                    (persona_scope, utc_day),
                    SemanticAppraisalBudgetClosureV1 {
                        daily_token_limit,
                        charged_tokens,
                        reserved_tokens,
                        blocked,
                        compacted_claim_rows,
                        compacted_charged_tokens,
                        compacted_chain_digest,
                        pending_reserved_tokens: 0,
                        settled_charged_tokens: 0,
                        requires_blocked: false,
                    },
                )
                .is_some()
            {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_budget_identity",
                ));
            }
            visited += 1;
            last_rowid = Some(rowid);
        }
        drop(rows);
        drop(statement);
        if visited < limit {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_budget_rowset",
        ))?;
    }
    if scanned != expected_rows {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_budget_rowset",
        ));
    }
    Ok(budgets)
}

#[derive(Debug)]
struct RawSemanticAppraisalClaimV1 {
    rowid: i64,
    persona_scope: Option<Vec<u8>>,
    persona_scope_len: i64,
    nonce_len: i64,
    utc_day: Option<i64>,
    origin_event_len: i64,
    origin_len: i64,
    provider_len: i64,
    reserved_tokens: Option<i64>,
    created_at_ms: Option<i64>,
    settled_at_ms: Option<i64>,
    settled_marker: i64,
    charged_tokens: Option<i64>,
    charged_marker: i64,
    outcome_len: i64,
    canonical_revision: Option<i64>,
    canonical_marker: i64,
    semantic_revision: Option<i64>,
    semantic_marker: i64,
    usage_known: Option<i64>,
    usage_known_marker: i64,
    usage_tokens: Option<i64>,
    usage_tokens_marker: i64,
    proposal_len: i64,
    settlement_len: i64,
    reply_affect_bytes_len: i64,
    reply_affect_digest_len: i64,
    terminal_receipt_bytes_len: i64,
    terminal_receipt_digest_len: i64,
}

fn raw_semantic_appraisal_claim_v1(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<RawSemanticAppraisalClaimV1> {
    Ok(RawSemanticAppraisalClaimV1 {
        rowid: row.get(0)?,
        persona_scope: row.get(1)?,
        persona_scope_len: row.get(2)?,
        nonce_len: row.get(3)?,
        utc_day: row.get(4)?,
        origin_event_len: row.get(5)?,
        origin_len: row.get(6)?,
        provider_len: row.get(7)?,
        reserved_tokens: row.get(8)?,
        created_at_ms: row.get(9)?,
        settled_at_ms: row.get(10)?,
        settled_marker: row.get(11)?,
        charged_tokens: row.get(12)?,
        charged_marker: row.get(13)?,
        outcome_len: row.get(14)?,
        canonical_revision: row.get(15)?,
        canonical_marker: row.get(16)?,
        semantic_revision: row.get(17)?,
        semantic_marker: row.get(18)?,
        usage_known: row.get(19)?,
        usage_known_marker: row.get(20)?,
        usage_tokens: row.get(21)?,
        usage_tokens_marker: row.get(22)?,
        proposal_len: row.get(23)?,
        settlement_len: row.get(24)?,
        reply_affect_bytes_len: row.get(25)?,
        reply_affect_digest_len: row.get(26)?,
        terminal_receipt_bytes_len: row.get(27)?,
        terminal_receipt_digest_len: row.get(28)?,
    })
}

const SEMANTIC_APPRAISAL_CLAIM_SCAN_V1_SQL: &str =
    "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(request_nonce_digest)='blob'
                 THEN length(request_nonce_digest) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' THEN utc_day END,
            CASE WHEN typeof(origin_event_digest)='blob'
                 THEN length(origin_event_digest) ELSE -1 END,
            CASE WHEN typeof(origin_digest)='blob' THEN length(origin_digest) ELSE -1 END,
            CASE WHEN typeof(provider_digest)='blob' THEN length(provider_digest) ELSE -1 END,
            CASE WHEN typeof(reserved_tokens)='integer' THEN reserved_tokens END,
            CASE WHEN typeof(created_at_ms)='integer' THEN created_at_ms END,
            CASE WHEN settled_at_ms IS NULL THEN NULL
                 WHEN typeof(settled_at_ms)='integer' THEN settled_at_ms END,
            CASE WHEN settled_at_ms IS NULL THEN 0
                 WHEN typeof(settled_at_ms)='integer' THEN 1 ELSE -1 END,
            CASE WHEN charged_tokens IS NULL THEN NULL
                 WHEN typeof(charged_tokens)='integer' THEN charged_tokens END,
            CASE WHEN charged_tokens IS NULL THEN -2
                 WHEN typeof(charged_tokens)='integer' THEN 1 ELSE -1 END,
            CASE WHEN outcome_code IS NULL THEN -2
                 WHEN typeof(outcome_code)='text' THEN length(CAST(outcome_code AS BLOB)) ELSE -1 END,
            CASE WHEN canonical_revision IS NULL THEN NULL
                 WHEN typeof(canonical_revision)='integer' THEN canonical_revision END,
            CASE WHEN canonical_revision IS NULL THEN -2
                 WHEN typeof(canonical_revision)='integer' THEN 1 ELSE -1 END,
            CASE WHEN semantic_revision IS NULL THEN NULL
                 WHEN typeof(semantic_revision)='integer' THEN semantic_revision END,
            CASE WHEN semantic_revision IS NULL THEN -2
                 WHEN typeof(semantic_revision)='integer' THEN 1 ELSE -1 END,
            NULL,-2,NULL,-2,-2,-2,-2,-2,-2,-2
     FROM semantic_appraisal_claim NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

const SEMANTIC_APPRAISAL_CLAIM_SCAN_V2_SQL: &str =
    "SELECT rowid,
            CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                 THEN persona_scope END,
            CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END,
            CASE WHEN typeof(request_nonce_digest)='blob'
                 THEN length(request_nonce_digest) ELSE -1 END,
            CASE WHEN typeof(utc_day)='integer' THEN utc_day END,
            CASE WHEN typeof(origin_event_digest)='blob'
                 THEN length(origin_event_digest) ELSE -1 END,
            CASE WHEN typeof(origin_digest)='blob' THEN length(origin_digest) ELSE -1 END,
            CASE WHEN typeof(provider_digest)='blob' THEN length(provider_digest) ELSE -1 END,
            CASE WHEN typeof(reserved_tokens)='integer' THEN reserved_tokens END,
            CASE WHEN typeof(created_at_ms)='integer' THEN created_at_ms END,
            CASE WHEN settled_at_ms IS NULL THEN NULL
                 WHEN typeof(settled_at_ms)='integer' THEN settled_at_ms END,
            CASE WHEN settled_at_ms IS NULL THEN 0
                 WHEN typeof(settled_at_ms)='integer' THEN 1 ELSE -1 END,
            CASE WHEN charged_tokens IS NULL THEN NULL
                 WHEN typeof(charged_tokens)='integer' THEN charged_tokens END,
            CASE WHEN charged_tokens IS NULL THEN -2
                 WHEN typeof(charged_tokens)='integer' THEN 1 ELSE -1 END,
            CASE WHEN outcome_code IS NULL THEN -2
                 WHEN typeof(outcome_code)='text' THEN length(CAST(outcome_code AS BLOB)) ELSE -1 END,
            CASE WHEN canonical_revision IS NULL THEN NULL
                 WHEN typeof(canonical_revision)='integer' THEN canonical_revision END,
            CASE WHEN canonical_revision IS NULL THEN -2
                 WHEN typeof(canonical_revision)='integer' THEN 1 ELSE -1 END,
            CASE WHEN semantic_revision IS NULL THEN NULL
                 WHEN typeof(semantic_revision)='integer' THEN semantic_revision END,
            CASE WHEN semantic_revision IS NULL THEN -2
                 WHEN typeof(semantic_revision)='integer' THEN 1 ELSE -1 END,
            CASE WHEN usage_known IS NULL THEN NULL
                 WHEN typeof(usage_known)='integer' THEN usage_known END,
            CASE WHEN usage_known IS NULL THEN -2
                 WHEN typeof(usage_known)='integer' THEN 1 ELSE -1 END,
            CASE WHEN usage_tokens IS NULL THEN NULL
                 WHEN typeof(usage_tokens)='integer' THEN usage_tokens END,
            CASE WHEN usage_tokens IS NULL THEN -2
                 WHEN typeof(usage_tokens)='integer' THEN 1 ELSE -1 END,
            CASE WHEN proposal_identity_digest IS NULL THEN -2
                 WHEN typeof(proposal_identity_digest)='blob'
                 THEN length(proposal_identity_digest) ELSE -1 END,
            CASE WHEN settlement_identity_digest IS NULL THEN -2
                 WHEN typeof(settlement_identity_digest)='blob'
                 THEN length(settlement_identity_digest) ELSE -1 END,
            CASE WHEN reply_affect_bytes IS NULL THEN -2
                 WHEN typeof(reply_affect_bytes)='blob'
                 THEN length(reply_affect_bytes) ELSE -1 END,
            CASE WHEN reply_affect_digest IS NULL THEN -2
                 WHEN typeof(reply_affect_digest)='blob'
                 THEN length(reply_affect_digest) ELSE -1 END,
            CASE WHEN terminal_receipt_bytes IS NULL THEN -2
                 WHEN typeof(terminal_receipt_bytes)='blob'
                 THEN length(terminal_receipt_bytes) ELSE -1 END,
            CASE WHEN terminal_receipt_digest IS NULL THEN -2
                 WHEN typeof(terminal_receipt_digest)='blob'
                 THEN length(terminal_receipt_digest) ELSE -1 END
     FROM semantic_appraisal_claim NOT INDEXED
     WHERE rowid>?1 ORDER BY rowid LIMIT ?2";

fn checked_appraisal_usage_add_v1(current: u64, value: u64) -> Result<u64, StoreError> {
    current
        .checked_add(value)
        .ok_or(StoreError::StorageBudgetExceeded {
            resource: "semantic_appraisal.usage_tokens",
            limit: u64::MAX,
            actual: u64::MAX,
        })
}

fn verify_semantic_appraisal_pending_challenge_v1(
    conn: &Connection,
    rowid: i64,
    persona_scope: Digest,
    created_at_ms: u64,
) -> Result<(), StoreError> {
    let (nonce, origin_event, origin): (Vec<u8>, Vec<u8>, Vec<u8>) = conn
        .query_row(
            "SELECT request_nonce_digest,origin_event_digest,origin_digest
             FROM semantic_appraisal_claim
             WHERE rowid=?1 AND settled_at_ms IS NULL",
            params![rowid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_pending_identity",
        ))?;
    let nonce = digest_from_vec(nonce, "semantic_appraisal.nonce")?;
    let origin_event = digest_from_vec(origin_event, "semantic_appraisal.origin_event")?;
    let origin = digest_from_vec(origin, "semantic_appraisal.origin")?;
    let challenge = read_perception_challenge_v1(conn, nonce)?.ok_or(
        StoreError::ContinuityFence("semantic_appraisal_pending_challenge"),
    )?;
    if challenge.origin.persona_scope != persona_scope
        || challenge.origin.origin_event_digest != origin_event
        || challenge.origin.origin_digest != origin
        || challenge.created_at_ms != created_at_ms
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_pending_challenge",
        ));
    }
    let active_identity = active_identity_for_scope_tx(conn, &challenge.origin.scope).map_err(
        |error| match error {
            StoreError::GenesisNotFound => {
                StoreError::ContinuityFence("perception_challenge_binding")
            }
            other => other,
        },
    )?;
    if active_identity.incarnation_id != challenge.origin.incarnation_id
        || active_identity.manifest_digest != challenge.origin.manifest_digest
    {
        return Err(StoreError::ContinuityFence("perception_challenge_binding"));
    }
    let attested = committed_perception_origin_v1(conn, origin_event, Some(persona_scope))?;
    if attested.origin.origin_digest != origin {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_pending_origin",
        ));
    }
    Ok(())
}

fn verify_semantic_appraisal_terminal_origin_v1(
    conn: &Connection,
    rowid: i64,
    persona_scope: Digest,
) -> Result<(), StoreError> {
    let (origin_event, origin, outcome_code): (Vec<u8>, Vec<u8>, String) = conn
        .query_row(
            "SELECT origin_event_digest,origin_digest,outcome_code
             FROM semantic_appraisal_claim
             WHERE rowid=?1 AND settled_at_ms IS NOT NULL",
            params![rowid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?
        .ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_identity",
        ))?;
    let origin_event = digest_from_vec(origin_event, "semantic_appraisal.origin_event")?;
    let origin = digest_from_vec(origin, "semantic_appraisal.origin")?;
    if !valid_stored_semantic_appraisal_terminal_outcome_code_v1(&outcome_code) {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_outcome",
        ));
    }
    let attested = committed_perception_origin_v1(conn, origin_event, Some(persona_scope))?;
    if attested.origin.origin_digest != origin {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_terminal_origin",
        ));
    }
    Ok(())
}

fn scan_semantic_appraisal_claims_v1(
    conn: &Connection,
    layout: SemanticAppraisalStorageLayoutV1,
    expected_rows: u64,
    budgets: &mut BTreeMap<(Digest, u64), SemanticAppraisalBudgetClosureV1>,
) -> Result<(), StoreError> {
    let sql = match layout {
        SemanticAppraisalStorageLayoutV1::V1 => SEMANTIC_APPRAISAL_CLAIM_SCAN_V1_SQL,
        SemanticAppraisalStorageLayoutV1::V2 | SemanticAppraisalStorageLayoutV1::V3 => {
            SEMANTIC_APPRAISAL_CLAIM_SCAN_V2_SQL
        }
    };
    let mut cursor = 0_i64;
    let mut scanned = 0_u64;
    loop {
        let remaining = expected_rows.saturating_sub(scanned);
        let limit = semantic_appraisal_scan_limit_v1(remaining)?;
        let mut statement = conn.prepare(sql)?;
        let mut rows = statement.query(params![cursor, limit])?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            let raw = raw_semantic_appraisal_claim_v1(row)?;
            scanned = scanned.checked_add(1).ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_claim_rowset",
            ))?;
            if scanned > expected_rows {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_claim_rowset",
                ));
            }
            let persona_scope = stored_typed_digest(
                raw.persona_scope,
                raw.persona_scope_len,
                "semantic_appraisal.persona",
                "semantic_appraisal_persona_type",
            )?;
            appraisal_required_digest_length_v1(
                raw.nonce_len,
                "semantic_appraisal.nonce",
                "semantic_appraisal_nonce_type",
            )?;
            appraisal_required_digest_length_v1(
                raw.origin_event_len,
                "semantic_appraisal.origin_event",
                "semantic_appraisal_origin_event_type",
            )?;
            appraisal_required_digest_length_v1(
                raw.origin_len,
                "semantic_appraisal.origin",
                "semantic_appraisal_origin_type",
            )?;
            appraisal_required_digest_length_v1(
                raw.provider_len,
                "semantic_appraisal.provider",
                "semantic_appraisal_provider_type",
            )?;
            let utc_day =
                appraisal_nonnegative_integer_v1(raw.utc_day, "semantic_appraisal_claim_day")?;
            let reserved_tokens = appraisal_nonnegative_integer_v1(
                raw.reserved_tokens,
                "semantic_appraisal_claim_reserved",
            )?;
            if reserved_tokens > 1_000_000 {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_claim_reserved",
                ));
            }
            let created_at_ms = appraisal_positive_integer_v1(
                raw.created_at_ms,
                "semantic_appraisal_claim_created",
            )?;
            if utc_day != created_at_ms / SEMANTIC_APPRAISAL_UTC_DAY_MS {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_claim_day_closure",
                ));
            }
            let budget =
                budgets
                    .get_mut(&(persona_scope, utc_day))
                    .ok_or(StoreError::ContinuityFence(
                        "semantic_appraisal_claim_budget_closure",
                    ))?;
            match raw.settled_marker {
                0 => {
                    if raw.settled_at_ms.is_some()
                        || raw.charged_marker != -2
                        || raw.charged_tokens.is_some()
                        || raw.outcome_len != -2
                        || raw.canonical_marker != -2
                        || raw.canonical_revision.is_some()
                        || raw.semantic_marker != -2
                        || raw.semantic_revision.is_some()
                    {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_pending_shape",
                        ));
                    }
                    if reserved_tokens < 768 {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_pending_reservation",
                        ));
                    }
                    if layout != SemanticAppraisalStorageLayoutV1::V1
                        && (raw.usage_known_marker != -2
                            || raw.usage_known.is_some()
                            || raw.usage_tokens_marker != -2
                            || raw.usage_tokens.is_some()
                            || raw.proposal_len != -2
                            || raw.settlement_len != -2
                            || raw.reply_affect_bytes_len != -2
                            || raw.reply_affect_digest_len != -2
                            || raw.terminal_receipt_bytes_len != -2
                            || raw.terminal_receipt_digest_len != -2)
                    {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_pending_receipt_shape",
                        ));
                    }
                    budget.pending_reserved_tokens = checked_appraisal_usage_add_v1(
                        budget.pending_reserved_tokens,
                        reserved_tokens,
                    )?;
                }
                1 => {
                    let settled_at_ms = appraisal_positive_integer_v1(
                        raw.settled_at_ms,
                        "semantic_appraisal_claim_settled",
                    )?;
                    if settled_at_ms < created_at_ms || raw.charged_marker != 1 {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_terminal_shape",
                        ));
                    }
                    let charged_tokens = appraisal_nonnegative_integer_v1(
                        raw.charged_tokens,
                        "semantic_appraisal_claim_charged",
                    )?;
                    if raw.outcome_len < 0 {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_outcome_type",
                        ));
                    }
                    let outcome_len = u64::try_from(raw.outcome_len).unwrap_or(u64::MAX);
                    if outcome_len == 0 || outcome_len > MAX_SEMANTIC_APPRAISAL_OUTCOME_BYTES {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_outcome_length",
                        ));
                    }
                    if raw.canonical_marker != 1
                        || appraisal_nonnegative_integer_v1(
                            raw.canonical_revision,
                            "semantic_appraisal_claim_revision",
                        )
                        .is_err()
                    {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_claim_revision",
                        ));
                    }
                    match raw.semantic_marker {
                        -2 if raw.semantic_revision.is_none() => {}
                        1 => {
                            appraisal_positive_integer_v1(
                                raw.semantic_revision,
                                "semantic_appraisal_claim_semantic",
                            )?;
                        }
                        _ => {
                            return Err(StoreError::ContinuityFence(
                                "semantic_appraisal_claim_semantic",
                            ))
                        }
                    }

                    if layout != SemanticAppraisalStorageLayoutV1::V1 {
                        let legacy_terminal = raw.usage_known_marker == -2
                            && raw.usage_known.is_none()
                            && raw.usage_tokens_marker == -2
                            && raw.usage_tokens.is_none()
                            && raw.proposal_len == -2
                            && raw.settlement_len == -2
                            && raw.reply_affect_bytes_len == -2
                            && raw.reply_affect_digest_len == -2
                            && raw.terminal_receipt_bytes_len == -2
                            && raw.terminal_receipt_digest_len == -2;
                        if !legacy_terminal {
                            if raw.usage_known_marker != 1 {
                                return Err(StoreError::ContinuityFence(
                                    "semantic_appraisal_usage_type",
                                ));
                            }
                            let usage_known = match raw.usage_known {
                                Some(0) => false,
                                Some(1) => true,
                                _ => {
                                    return Err(StoreError::ContinuityFence(
                                        "semantic_appraisal_usage_type",
                                    ))
                                }
                            };
                            let usage_tokens = match (usage_known, raw.usage_tokens_marker) {
                                (false, -2) if raw.usage_tokens.is_none() => None,
                                (true, 1) => Some(appraisal_nonnegative_integer_v1(
                                    raw.usage_tokens,
                                    "semantic_appraisal_usage_tokens",
                                )?),
                                _ => {
                                    return Err(StoreError::ContinuityFence(
                                        "semantic_appraisal_usage_tokens",
                                    ))
                                }
                            };
                            if usage_tokens.is_some_and(|tokens| tokens > 1_000_000) {
                                return Err(StoreError::ContinuityFence(
                                    "semantic_appraisal_usage_tokens",
                                ));
                            }
                            appraisal_optional_digest_length_v1(
                                raw.proposal_len,
                                "semantic_appraisal.proposal_identity",
                                "semantic_appraisal_proposal_type",
                            )?;
                            appraisal_required_digest_length_v1(
                                raw.settlement_len,
                                "semantic_appraisal.settlement_identity",
                                "semantic_appraisal_settlement_type",
                            )?;
                            let reply_bytes = appraisal_required_payload_length_v1(
                                raw.reply_affect_bytes_len,
                                MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES,
                                "semantic_appraisal.reply_affect_bytes",
                                "semantic_appraisal_reply_affect_type",
                            )?;
                            appraisal_required_digest_length_v1(
                                raw.reply_affect_digest_len,
                                "semantic_appraisal.reply_affect_digest",
                                "semantic_appraisal_reply_affect_digest_type",
                            )?;
                            let receipt_bytes = appraisal_required_payload_length_v1(
                                raw.terminal_receipt_bytes_len,
                                MAX_SEMANTIC_APPRAISAL_RECEIPT_BYTES,
                                "semantic_appraisal.terminal_receipt_bytes",
                                "semantic_appraisal_terminal_receipt_type",
                            )?;
                            appraisal_required_digest_length_v1(
                                raw.terminal_receipt_digest_len,
                                "semantic_appraisal.terminal_receipt_digest",
                                "semantic_appraisal_terminal_receipt_digest_type",
                            )?;
                            let _ = (reply_bytes, receipt_bytes);
                            let expected_charge = usage_tokens.unwrap_or(reserved_tokens);
                            if charged_tokens != expected_charge {
                                return Err(StoreError::ContinuityFence(
                                    "semantic_appraisal_usage_charge",
                                ));
                            }
                            if usage_tokens.is_some_and(|tokens| tokens > reserved_tokens) {
                                budget.requires_blocked = true;
                            }
                        }
                    }
                    budget.settled_charged_tokens = checked_appraisal_usage_add_v1(
                        budget.settled_charged_tokens,
                        charged_tokens,
                    )?;
                }
                _ => {
                    return Err(StoreError::ContinuityFence(
                        "semantic_appraisal_claim_settled_type",
                    ))
                }
            }
            visited += 1;
            last_rowid = Some(raw.rowid);
        }
        drop(rows);
        drop(statement);
        if visited < limit {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_claim_rowset",
        ))?;
    }
    if scanned != expected_rows {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_rowset",
        ));
    }
    Ok(())
}

fn verify_semantic_appraisal_claim_closures_v1(
    conn: &Connection,
    layout: SemanticAppraisalStorageLayoutV1,
    expected_rows: u64,
) -> Result<(), StoreError> {
    let sql = match layout {
        SemanticAppraisalStorageLayoutV1::V1 => SEMANTIC_APPRAISAL_CLAIM_SCAN_V1_SQL,
        SemanticAppraisalStorageLayoutV1::V2 | SemanticAppraisalStorageLayoutV1::V3 => {
            SEMANTIC_APPRAISAL_CLAIM_SCAN_V2_SQL
        }
    };
    let mut cursor = 0_i64;
    let mut scanned = 0_u64;
    loop {
        let remaining = expected_rows.saturating_sub(scanned);
        let limit = semantic_appraisal_scan_limit_v1(remaining)?;
        let mut statement = conn.prepare(sql)?;
        let mut rows = statement.query(params![cursor, limit])?;
        let mut visited = 0_i64;
        let mut last_rowid = None;
        while let Some(row) = rows.next()? {
            let raw = raw_semantic_appraisal_claim_v1(row)?;
            scanned = scanned.checked_add(1).ok_or(StoreError::ContinuityFence(
                "semantic_appraisal_claim_rowset",
            ))?;
            if scanned > expected_rows {
                return Err(StoreError::ContinuityFence(
                    "semantic_appraisal_claim_rowset",
                ));
            }
            let persona_scope = stored_typed_digest(
                raw.persona_scope,
                raw.persona_scope_len,
                "semantic_appraisal.persona",
                "semantic_appraisal_persona_type",
            )?;
            match raw.settled_marker {
                0 => {
                    let created_at_ms = appraisal_positive_integer_v1(
                        raw.created_at_ms,
                        "semantic_appraisal_claim_created",
                    )?;
                    verify_semantic_appraisal_pending_challenge_v1(
                        conn,
                        raw.rowid,
                        persona_scope,
                        created_at_ms,
                    )?;
                }
                1 if layout == SemanticAppraisalStorageLayoutV1::V1 => {
                    verify_semantic_appraisal_terminal_origin_v1(conn, raw.rowid, persona_scope)?;
                }
                1 => {
                    let terminal = read_semantic_appraisal_terminal_compaction_row_by_rowid_v1(
                        conn, raw.rowid,
                    )?;
                    let integrity =
                        verify_semantic_appraisal_terminal_integrity_v1(conn, &terminal)?;
                    let legacy_terminal = raw.usage_known_marker == -2
                        && raw.usage_known.is_none()
                        && raw.usage_tokens_marker == -2
                        && raw.usage_tokens.is_none()
                        && raw.proposal_len == -2
                        && raw.settlement_len == -2
                        && raw.reply_affect_bytes_len == -2
                        && raw.reply_affect_digest_len == -2
                        && raw.terminal_receipt_bytes_len == -2
                        && raw.terminal_receipt_digest_len == -2;
                    if legacy_terminal
                        != (integrity == SemanticAppraisalTerminalIntegrityV1::Legacy)
                    {
                        return Err(StoreError::ContinuityFence(
                            "semantic_appraisal_terminal_receipt_shape",
                        ));
                    }
                }
                _ => {
                    return Err(StoreError::ContinuityFence(
                        "semantic_appraisal_claim_settled_type",
                    ))
                }
            }
            visited += 1;
            last_rowid = Some(raw.rowid);
        }
        drop(rows);
        drop(statement);
        if visited < limit {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_claim_rowset",
        ))?;
    }
    if scanned != expected_rows {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_claim_rowset",
        ));
    }
    Ok(())
}

fn verify_semantic_appraisal_rollup_v1(conn: &Connection) -> Result<u64, StoreError> {
    type RawRollup = (
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        i64,
        Option<Vec<u8>>,
        Option<i64>,
    );
    let mut statement = conn.prepare(
        "SELECT rowid,
               CASE WHEN typeof(singleton)='integer' THEN singleton END,
               CASE WHEN typeof(compacted_budget_rows)='integer'
                    THEN compacted_budget_rows END,
               CASE WHEN typeof(compacted_claim_rows)='integer'
                    THEN compacted_claim_rows END,
               CASE WHEN typeof(compacted_charged_tokens)='integer'
                    THEN compacted_charged_tokens END,
               CASE WHEN typeof(compacted_chain_digest)='blob'
                    THEN length(compacted_chain_digest) ELSE -1 END,
               CASE WHEN typeof(compacted_chain_digest)='blob'
                         AND length(compacted_chain_digest)=32
                    THEN compacted_chain_digest END,
               CASE WHEN typeof(last_authoritative_now_ms)='integer'
                    THEN last_authoritative_now_ms END
         FROM semantic_appraisal_rollup NOT INDEXED ORDER BY rowid LIMIT 2",
    )?;
    let mut rows = statement.query([])?;
    let raw: RawRollup = {
        let row = rows.next()?.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_rollup_singleton",
        ))?;
        (
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        )
    };
    if rows.next()?.is_some() || raw.0 != 1 || raw.1 != Some(1) {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_rollup_singleton",
        ));
    }
    let compacted_budget_rows =
        appraisal_nonnegative_integer_v1(raw.2, "semantic_appraisal_rollup_budget_rows")?;
    let compacted_claim_rows =
        appraisal_nonnegative_integer_v1(raw.3, "semantic_appraisal_rollup_claim_rows")?;
    let compacted_charged_tokens =
        appraisal_nonnegative_integer_v1(raw.4, "semantic_appraisal_rollup_charged_tokens")?;
    appraisal_required_digest_length_v1(
        raw.5,
        "semantic_appraisal.rollup_chain",
        "semantic_appraisal_rollup_chain_type",
    )?;
    let compacted_chain_digest = digest_from_vec(
        raw.6.ok_or(StoreError::ContinuityFence(
            "semantic_appraisal_rollup_chain_type",
        ))?,
        "semantic_appraisal.rollup_chain",
    )?;
    let last_authoritative_now_ms =
        appraisal_nonnegative_integer_v1(raw.7, "semantic_appraisal_authoritative_time")?;
    if (compacted_budget_rows == 0
        && (compacted_claim_rows != 0
            || compacted_charged_tokens != 0
            || compacted_chain_digest != [0; 32]))
        || (compacted_budget_rows > 0 && compacted_chain_digest == [0; 32])
    {
        return Err(StoreError::ContinuityFence(
            "semantic_appraisal_rollup_compaction_closure",
        ));
    }
    Ok(last_authoritative_now_ms)
}

fn preflight_semantic_appraisal_storage_v1(
    conn: &Connection,
    layout: SemanticAppraisalStorageLayoutV1,
) -> Result<SemanticAppraisalStorageBudgetV1, StoreError> {
    // Phase one admits every persona and row by bounded keyset before phase
    // two touches any payload length. Row+1 therefore always wins before a
    // large or adversarial retained receipt can trigger heavier work.
    let storage = scan_semantic_appraisal_phase_one_v1(conn, layout)?;
    let mut budgets = scan_semantic_appraisal_budgets_v1(conn, layout, storage.budget_rows)?;
    scan_semantic_appraisal_claims_v1(conn, layout, storage.claim_rows, &mut budgets)?;
    for budget in budgets.values() {
        let retained_and_compacted_charged = checked_appraisal_usage_add_v1(
            budget.compacted_charged_tokens,
            budget.settled_charged_tokens,
        )?;
        if budget.reserved_tokens != budget.pending_reserved_tokens
            || budget.charged_tokens != retained_and_compacted_charged
        {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_budget_claim_closure",
            ));
        }
        let spent = checked_appraisal_usage_add_v1(budget.charged_tokens, budget.reserved_tokens)?;
        if (spent > budget.daily_token_limit || budget.requires_blocked) && !budget.blocked {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_budget_blocked_closure",
            ));
        }
    }
    // Deep challenge/origin/receipt authentication deliberately follows all
    // bounded row, type, length, accounting, and aggregate resource gates.
    verify_semantic_appraisal_claim_closures_v1(conn, layout, storage.claim_rows)?;
    if layout == SemanticAppraisalStorageLayoutV1::V3 {
        let last_authoritative_now_ms = verify_semantic_appraisal_rollup_v1(conn)?;
        let maximum_live_time_raw: i64 = conn.query_row(
            "SELECT COALESCE(MAX(value),0) FROM (
               SELECT updated_at_ms AS value FROM semantic_appraisal_budget
               UNION ALL
               SELECT created_at_ms AS value FROM semantic_appraisal_claim
               UNION ALL
               SELECT settled_at_ms AS value FROM semantic_appraisal_claim
                WHERE settled_at_ms IS NOT NULL
             )",
            [],
            |row| row.get(0),
        )?;
        let maximum_live_time = u64::try_from(maximum_live_time_raw).map_err(|_| {
            StoreError::ContinuityFence("semantic_appraisal_authoritative_time_closure")
        })?;
        if last_authoritative_now_ms < maximum_live_time {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_authoritative_time_closure",
            ));
        }
    }
    Ok(storage)
}

fn migrate_semantic_appraisal_schema_v1(tx: &Transaction<'_>) -> Result<(), StoreError> {
    let (budget_table, claim_table, rollup_table): (bool, bool, bool) = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema
                       WHERE type='table' AND name='semantic_appraisal_budget'),
                EXISTS(SELECT 1 FROM sqlite_schema
                       WHERE type='table' AND name='semantic_appraisal_claim'),
                EXISTS(SELECT 1 FROM sqlite_schema
                       WHERE type='table' AND name='semantic_appraisal_rollup')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let table_count = i64::from(budget_table) + i64::from(claim_table) + i64::from(rollup_table);
    match (semantic_appraisal_schema_version_v1(tx)?, table_count) {
        (None, 0) => {
            // Old challenges have no durable reservation identity and must not
            // survive as a budget bypass after this boundary is introduced.
            let challenge_permit =
                preflight_perception_challenges_v1(tx, PerceptionChallengeStorageLayoutV1::V3)?;
            for token in &challenge_permit.tokens {
                delete_perception_challenge_token_v1(tx, token)?;
            }
            tx.execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V3_SQL)?;
            tx.execute(
                "INSERT INTO meta(key,value) VALUES('semantic_appraisal_schema_version',?1)",
                params![vec![SEMANTIC_APPRAISAL_SCHEMA_VERSION_V3]],
            )?;
        }
        (
            Some(
                version @ (SEMANTIC_APPRAISAL_SCHEMA_VERSION_V1
                | SEMANTIC_APPRAISAL_SCHEMA_VERSION_V2),
            ),
            2,
        ) => {
            let (legacy_schema, legacy_layout) = if version == SEMANTIC_APPRAISAL_SCHEMA_VERSION_V1
            {
                (
                    SEMANTIC_APPRAISAL_SCHEMA_V1_SQL,
                    SemanticAppraisalStorageLayoutV1::V1,
                )
            } else {
                (
                    SEMANTIC_APPRAISAL_SCHEMA_V2_SQL,
                    SemanticAppraisalStorageLayoutV1::V2,
                )
            };
            verify_semantic_appraisal_schema_against_v1(tx, legacy_schema)?;
            // Exact legacy data is admitted while the live transaction still
            // has the old tables. No DROP/ALTER/CREATE/INSERT SELECT may run
            // before all retention and accounting bounds close.
            preflight_semantic_appraisal_storage_v1(tx, legacy_layout)?;
            let challenge_permit =
                preflight_perception_challenges_v1(tx, PerceptionChallengeStorageLayoutV1::V3)?;
            let time_seed_raw: i64 = tx.query_row(
                "SELECT COALESCE(MAX(value),0) FROM (
                   SELECT updated_at_ms AS value FROM semantic_appraisal_budget
                   UNION ALL
                   SELECT created_at_ms AS value FROM semantic_appraisal_claim
                   UNION ALL
                   SELECT settled_at_ms AS value FROM semantic_appraisal_claim
                    WHERE settled_at_ms IS NOT NULL
                 )",
                [],
                |row| row.get(0),
            )?;
            let time_seed = u64::try_from(time_seed_raw).map_err(|_| {
                StoreError::ContinuityFence("semantic_appraisal_authoritative_time")
            })?;
            tx.execute_batch(
                "DROP INDEX semantic_appraisal_claim_pending_v1;
                 ALTER TABLE semantic_appraisal_claim RENAME TO __ae_semantic_appraisal_claim_migration;
                 ALTER TABLE semantic_appraisal_budget RENAME TO __ae_semantic_appraisal_budget_migration;",
            )?;
            tx.execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V3_SQL)?;
            tx.execute_batch(
                "INSERT INTO semantic_appraisal_budget(
                    persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
                    blocked,updated_at_ms,compacted_claim_rows,compacted_charged_tokens,
                    compacted_chain_digest
                 ) SELECT persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
                          blocked,updated_at_ms,0,0,zeroblob(32)
                   FROM __ae_semantic_appraisal_budget_migration;",
            )?;
            if version == SEMANTIC_APPRAISAL_SCHEMA_VERSION_V1 {
                tx.execute_batch(
                    "INSERT INTO semantic_appraisal_claim(
                        request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
                        provider_digest,reserved_tokens,created_at_ms,settled_at_ms,charged_tokens,
                        outcome_code,canonical_revision,semantic_revision
                     ) SELECT request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
                              provider_digest,reserved_tokens,created_at_ms,settled_at_ms,charged_tokens,
                              outcome_code,canonical_revision,semantic_revision
                       FROM __ae_semantic_appraisal_claim_migration;",
                )?;
            } else {
                tx.execute_batch(
                    "INSERT INTO semantic_appraisal_claim(
                        request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
                        provider_digest,reserved_tokens,created_at_ms,settled_at_ms,charged_tokens,
                        outcome_code,canonical_revision,semantic_revision,usage_known,usage_tokens,
                        proposal_identity_digest,settlement_identity_digest,reply_affect_bytes,
                        reply_affect_digest,terminal_receipt_bytes,terminal_receipt_digest
                     ) SELECT request_nonce_digest,persona_scope,utc_day,origin_event_digest,origin_digest,
                              provider_digest,reserved_tokens,created_at_ms,settled_at_ms,charged_tokens,
                              outcome_code,canonical_revision,semantic_revision,usage_known,usage_tokens,
                              proposal_identity_digest,settlement_identity_digest,reply_affect_bytes,
                              reply_affect_digest,terminal_receipt_bytes,terminal_receipt_digest
                       FROM __ae_semantic_appraisal_claim_migration;",
                )?;
            }
            tx.execute_batch(
                "DROP TABLE __ae_semantic_appraisal_claim_migration;
                 DROP TABLE __ae_semantic_appraisal_budget_migration;",
            )?;
            tx.execute(
                "UPDATE semantic_appraisal_rollup SET last_authoritative_now_ms=?1
                 WHERE singleton=1 AND last_authoritative_now_ms=0",
                params![i64::try_from(time_seed).map_err(|_| {
                    StoreError::RevisionOutOfRange {
                        revision: time_seed,
                    }
                })?],
            )?;
            // Pre-v2 direct challenges persisted the raw internal nonce
            // material. They have no budget claim and cannot cross the v2
            // commitment-at-rest boundary.
            if version == SEMANTIC_APPRAISAL_SCHEMA_VERSION_V1 {
                for token in &challenge_permit.tokens {
                    let retained: bool = tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM semantic_appraisal_claim
                         WHERE request_nonce_digest=?1)",
                        params![blob(token.request_nonce_digest)],
                        |row| row.get(0),
                    )?;
                    if !retained {
                        delete_perception_challenge_token_v1(tx, token)?;
                    }
                }
            }
            tx.execute(
                "UPDATE meta SET value=?1 WHERE key='semantic_appraisal_schema_version'",
                params![vec![SEMANTIC_APPRAISAL_SCHEMA_VERSION_V3]],
            )?;
        }
        (Some(SEMANTIC_APPRAISAL_SCHEMA_VERSION_V3), 3) => {}
        _ => {
            return Err(StoreError::ContinuityFence(
                "semantic_appraisal_schema_version",
            ));
        }
    }
    verify_semantic_appraisal_schema_v1(tx)?;
    preflight_semantic_appraisal_storage_v1(tx, SemanticAppraisalStorageLayoutV1::V3)?;
    Ok(())
}

pub(crate) fn migrate_schema(tx: &Transaction<'_>) -> Result<(), StoreError> {
    preflight_sqlite_schema_catalog_v1(tx)?;
    let existing_tables = semantic_schema_table_count(tx)?;
    let core_tables = semantic_schema_core_table_count(tx)?;
    if core_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT {
        require_positive_table_rowids_v1(
            tx,
            "semantic_evidence_authority",
            "semantic_schema_migration_rowid_domain",
        )?;
    }
    let supplemental_tables = existing_tables
        .checked_sub(core_tables)
        .ok_or(StoreError::ContinuityFence("semantic_schema_table_set"))?;
    let stored_version = semantic_schema_version(tx)?;
    let already_v4 = matches!(stored_version, Some(SEMANTIC_SCHEMA_VERSION_V4));
    let already_v5 = matches!(stored_version, Some(SEMANTIC_SCHEMA_VERSION_V5));
    match stored_version {
        None if existing_tables == 0 && core_tables == 0 => {
            tx.execute_batch(SEMANTIC_SCHEMA_V3_SQL)?;
        }
        None if core_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT
            && existing_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT =>
        {
            require_semantic_columns(
                tx,
                "semantic_evidence_authority",
                &SEMANTIC_EVIDENCE_V2_COLUMNS,
            )?;
            let layout = detect_semantic_schema_layout(tx)?;
            if layout == SemanticSchemaLayout::Weak898b {
                rebuild_weak_semantic_schema_v3(tx)?;
            } else {
                migrate_strong_semantic_v2_to_v3(tx, 0)?;
            }
        }
        Some(SEMANTIC_SCHEMA_VERSION_V2)
            if core_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT
                && (supplemental_tables == 0 || supplemental_tables == 2)
                && existing_tables == core_tables + supplemental_tables =>
        {
            let layout = detect_semantic_schema_layout(tx)?;
            if layout != SemanticSchemaLayout::StrongV2 {
                return Err(StoreError::ContinuityFence("semantic_schema_version"));
            }
            migrate_strong_semantic_v2_to_v3(tx, supplemental_tables)?;
        }
        Some(SEMANTIC_SCHEMA_VERSION_V3)
            if existing_tables == SEMANTIC_SCHEMA_V3_TABLE_COUNT
                && core_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT =>
        {
            // V3 is never repaired in place. Every object is checked below.
        }
        Some(SEMANTIC_SCHEMA_VERSION_V4)
            if existing_tables == SEMANTIC_SCHEMA_TABLE_COUNT
                && core_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT => {}
        Some(SEMANTIC_SCHEMA_VERSION_V5)
            if existing_tables == SEMANTIC_SCHEMA_TABLE_COUNT
                && core_tables == SEMANTIC_SCHEMA_CORE_TABLE_COUNT => {}
        _ => {
            return Err(StoreError::ContinuityFence("semantic_schema_version"));
        }
    }
    if already_v5 {
        verify_semantic_schema_v5(tx)?;
    } else {
        if already_v4 {
            verify_semantic_schema_v4(tx)?;
        } else {
            verify_semantic_schema_v3(tx)?;
            // V3 admission must finish while the live transaction still has
            // the exact V3 schema. In particular, no V4 ALTER, cursor UPDATE,
            // or time-authority CREATE may precede the first forbidden row.
            preflight_semantic_v3_history_v1(tx)?;
            // Adding the lane discriminator does not rewrite any legacy
            // perception payload. V3 reaches the same authenticated V4
            // boundary before the V5 time-lane compaction is applied.
            apply_semantic_schema_v4(tx)?;
            verify_semantic_schema_v4(tx)?;
        }
        // This must precede V5's ALTER/INSERT table rewrite and the per-persona
        // compaction. It is compatible with authenticated V4 regardless of
        // whether this open began at V3, weak-898b, or V4.
        preflight_semantic_v4_history_v1(tx)?;
        apply_semantic_schema_v5(tx)?;
        compact_semantic_time_rows_v5(tx)?;
        verify_semantic_schema_v5(tx)?;
    }
    verify_or_backfill_semantic_budget_checkpoints(tx, true)?;
    migrate_semantic_appraisal_schema_v1(tx)?;
    verify_all_perception_challenges(tx)?;
    let foreign_key_violations: i64 =
        tx.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |row| {
            row.get(0)
        })?;
    if foreign_key_violations != 0 {
        return Err(StoreError::ContinuityFence(
            "semantic_schema_foreign_key_check",
        ));
    }
    tx.execute(
        "INSERT INTO meta(key,value) VALUES('semantic_schema_version',?1)
         ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![vec![SEMANTIC_SCHEMA_VERSION_V5]],
    )?;
    Ok(())
}

fn decode_origin_row(
    persona_scope: Digest,
    row: (
        Vec<u8>,
        i64,
        i64,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
        Vec<u8>,
    ),
) -> Result<SemanticOriginV1, StoreError> {
    let (
        source_scope,
        source_revision,
        legacy_migrated,
        incarnation,
        manifest,
        route,
        formula,
        state,
        graph,
        stored_origin,
    ) = row;
    let source_revision = JournalRevision::try_from(source_revision)?.get();
    let legacy_migrated = match legacy_migrated {
        0 => false,
        1 => true,
        _ => return Err(StoreError::ContinuityFence("semantic_origin_flag")),
    };
    let origin = SemanticOriginV1 {
        persona_scope,
        source_scope_digest: digest_from_vec(source_scope, "semantic_origin.source_scope")?,
        source_revision,
        legacy_migrated,
        incarnation_id: digest_from_vec(incarnation, "semantic_origin.incarnation")?,
        manifest_digest: digest_from_vec(manifest, "semantic_origin.manifest")?,
        route_digest: digest_from_vec(route, "semantic_origin.route")?,
        formula_digest: digest_from_vec(formula, "semantic_origin.formula")?,
        state_digest: digest_from_vec(state, "semantic_origin.state")?,
        graph_digest: digest_from_vec(graph, "semantic_origin.graph")?,
        origin_digest: digest_from_vec(stored_origin, "semantic_origin.digest")?,
    };
    if origin_digest(&origin) != origin.origin_digest {
        return Err(StoreError::ContinuityFence("semantic_origin_closure"));
    }
    Ok(origin)
}

pub(crate) fn stored_origin(
    conn: &Connection,
    persona_scope: Digest,
) -> Result<Option<SemanticOriginV1>, StoreError> {
    let row = conn
        .query_row(
            "SELECT
                CASE WHEN typeof(source_scope_digest)='blob' AND length(source_scope_digest)=32 THEN source_scope_digest ELSE zeroblob(0) END,
                source_revision,
                legacy_migrated,
                CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32 THEN incarnation_id ELSE zeroblob(0) END,
                CASE WHEN typeof(manifest_digest)='blob' AND length(manifest_digest)=32 THEN manifest_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(route_digest)='blob' AND length(route_digest)=32 THEN route_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(formula_digest)='blob' AND length(formula_digest)=32 THEN formula_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(state_digest)='blob' AND length(state_digest)=32 THEN state_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(graph_digest)='blob' AND length(graph_digest)=32 THEN graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(origin_digest)='blob' AND length(origin_digest)=32 THEN origin_digest ELSE zeroblob(0) END
             FROM semantic_origins WHERE persona_scope=?1",
            params![blob(persona_scope)],
            |row| {
                Ok((
                    row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?,
                    row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?,
                ))
            },
        )
        .optional()?;
    row.map(|row| decode_origin_row(persona_scope, row))
        .transpose()
}

fn stored_origin_tx(
    tx: &Transaction<'_>,
    persona_scope: Digest,
) -> Result<Option<SemanticOriginV1>, StoreError> {
    stored_origin(tx, persona_scope)
}

fn derive_origin_tx(
    tx: &Transaction<'_>,
    scope: &ScopeRef,
    persona_scope: Digest,
    incarnation_id: Digest,
    manifest_digest: Digest,
    route_digest: Digest,
    formula_digest: Digest,
    identity: &ActiveSemanticIdentityV1,
) -> Result<SemanticOriginV1, StoreError> {
    let expected_route = phase0_semantic_route_digest_v1();
    let expected_formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    if route_digest != expected_route || formula_digest != expected_formula {
        return Err(StoreError::SemanticInvalid("frozen_semantic_formula"));
    }

    let binding = wire::domain_hash(
        LEGACY_SEMANTIC_LANE_NAMESPACE_DOMAIN_V1,
        &[&persona_scope, &incarnation_id, &identity.formula_digest],
    );
    let mut legacy_relation = [0_u8; 16];
    legacy_relation.copy_from_slice(&binding[..16]);
    let legacy_scope = wire::persona_scope_digest(
        &scope.bot_token,
        &scope.persona_token,
        Some(&legacy_relation),
    );
    let legacy_head_raw: i64 = tx.query_row(
        "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
        params![blob(legacy_scope)],
        |row| row.get(0),
    )?;
    let legacy_head = JournalRevision::try_from(legacy_head_raw)?.get();

    if let Some(origin) = stored_origin_tx(tx, persona_scope)? {
        if origin.incarnation_id != incarnation_id
            || origin.manifest_digest != manifest_digest
            || origin.route_digest != expected_route
            || origin.formula_digest != expected_formula
        {
            return Err(StoreError::ContinuityFence("semantic_origin_identity"));
        }
        if origin.legacy_migrated {
            if origin.source_scope_digest != legacy_scope || legacy_head != origin.source_revision {
                return Err(StoreError::SemanticSuffixAuthorityUnavailable {
                    boundary_revision: origin.source_revision,
                    current_revision: legacy_head,
                });
            }
        } else if origin.source_scope_digest != persona_scope
            || origin.source_revision != 0
            || legacy_head != 0
        {
            return Err(StoreError::ContinuityFence(
                "unattested_legacy_semantic_history",
            ));
        }
        return Ok(origin);
    }

    let upgrade_revision: Option<i64> = tx
        .query_row(
            "SELECT next_revision FROM legacy_semantic_formula_upgrades
             WHERE scope_digest=?1 AND from_formula_digest=?2 AND to_formula_digest=?3",
            params![
                blob(legacy_scope),
                blob(identity.formula_digest),
                blob(expected_formula)
            ],
            |row| row.get(0),
        )
        .optional()?;
    let mut origin = if let Some(upgrade_revision) = upgrade_revision {
        verify_committed_semantic_migration_rows(tx)?;
        let upgrade_revision = semantic_revision_from_sql(upgrade_revision)?;
        if legacy_head != upgrade_revision {
            return Err(StoreError::SemanticSuffixAuthorityUnavailable {
                boundary_revision: upgrade_revision,
                current_revision: legacy_head,
            });
        }
        let journal =
            query_bounded_journal_row(tx, &legacy_scope, JournalRevision::new(upgrade_revision))?
                .ok_or(StoreError::ContinuityFence("semantic_origin_journal"))?;
        let receipt = wire::decode_transition_receipt(&journal.receipt_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_origin_receipt"))?;
        if wire::encode_transition_receipt(&receipt) != journal.receipt_bytes
            || receipt.formula_digest != expected_formula
            || receipt.next_revision != upgrade_revision
            || receipt.scope_digest != legacy_scope
        {
            return Err(StoreError::ContinuityFence("semantic_origin_receipt"));
        }
        SemanticOriginV1 {
            persona_scope,
            source_scope_digest: legacy_scope,
            source_revision: upgrade_revision,
            legacy_migrated: true,
            incarnation_id,
            manifest_digest,
            route_digest: expected_route,
            formula_digest: expected_formula,
            state_digest: receipt.state_after,
            graph_digest: receipt.graph_after,
            origin_digest: [0; 32],
        }
    } else {
        if legacy_head != 0 {
            return Err(StoreError::ContinuityFence(
                "unattested_legacy_semantic_history",
            ));
        }
        SemanticOriginV1 {
            persona_scope,
            source_scope_digest: persona_scope,
            source_revision: 0,
            legacy_migrated: false,
            incarnation_id,
            manifest_digest,
            route_digest: expected_route,
            formula_digest: expected_formula,
            state_digest: state_digest(&identity.baseline_field, &expected_formula),
            graph_digest: graph_digest(&identity.baseline_graph),
            origin_digest: [0; 32],
        }
    };
    origin.origin_digest = origin_digest(&origin);
    Ok(origin)
}

fn decoded_semantic_state(
    bytes: &[u8],
    formula_digest: Digest,
    expected_state: Digest,
    expected_graph: Digest,
) -> Result<(NeuralField, SparseGraph), StoreError> {
    let decoded = decode_core_semantic_snapshot_v3(bytes).map_err(semantic_core_history_error)?;
    if decoded.telemetry.formula_digest != formula_digest
        || decoded.telemetry.state_after != expected_state
        || decoded.telemetry.graph_after != expected_graph
        || state_digest(&decoded.field, &formula_digest) != expected_state
        || graph_digest(&decoded.graph) != expected_graph
    {
        return Err(StoreError::ContinuityFence("semantic_state_hydration"));
    }
    Ok((decoded.field, decoded.graph))
}

fn semantic_origin_state(
    conn: &Connection,
    origin: &SemanticOriginV1,
    identity: &ActiveSemanticIdentityV1,
) -> Result<(NeuralField, SparseGraph), StoreError> {
    if origin.legacy_migrated {
        let snapshot = query_bounded_snapshot_row(
            conn,
            &origin.source_scope_digest,
            JournalRevision::new(origin.source_revision),
        )?
        .ok_or(StoreError::ContinuityFence("semantic_origin_snapshot"))?;
        if snapshot.state_digest != origin.state_digest {
            return Err(StoreError::ContinuityFence("semantic_origin_snapshot"));
        }
        return decoded_semantic_state(
            &snapshot.state_bytes,
            origin.formula_digest,
            origin.state_digest,
            origin.graph_digest,
        );
    }

    if origin.source_revision != 0
        || origin.state_digest != state_digest(&identity.baseline_field, &origin.formula_digest)
        || origin.graph_digest != graph_digest(&identity.baseline_graph)
    {
        return Err(StoreError::ContinuityFence(
            "semantic_origin_genesis_binding",
        ));
    }
    Ok((
        identity.baseline_field.clone(),
        identity.baseline_graph.clone(),
    ))
}

/// Load the only full state allowed to anchor compact time projections.  The
/// anchor is either immutable Genesis/migration state or an authenticated
/// perception revision; a time revision can never become a recursive anchor.
pub(crate) fn semantic_time_anchor_state_v1(
    conn: &Connection,
    persona_scope: Digest,
    epoch: &MatrixTimeEpochV1,
    origin: &SemanticOriginV1,
    identity: &ActiveSemanticIdentityV1,
) -> Result<(NeuralField, SparseGraph), StoreError> {
    if epoch.anchor_semantic_revision == origin.source_revision {
        let (field, graph) = semantic_origin_state(conn, origin, identity)?;
        if state_digest(&field, &origin.formula_digest) != epoch.anchor_state_digest {
            return Err(StoreError::ContinuityFence("semantic_time_anchor"));
        }
        return Ok((field, graph));
    }
    if epoch.anchor_semantic_revision <= origin.source_revision {
        return Err(StoreError::ContinuityFence("semantic_time_anchor_revision"));
    }
    let committed = read_semantic_commit(conn, persona_scope, epoch.anchor_semantic_revision)?;
    if committed.transition_kind != SemanticTransitionKindV1::Perception
        || committed.state_digest != epoch.anchor_state_digest
    {
        return Err(StoreError::ContinuityFence("semantic_time_anchor_kind"));
    }
    decoded_semantic_state(
        &committed.snapshot_bytes,
        committed.formula_digest,
        committed.state_digest,
        committed.graph_digest,
    )
}

pub(crate) fn semantic_state_for_derivation_tx(
    tx: &Connection,
    persona_scope: Digest,
    origin: &SemanticOriginV1,
    identity: &ActiveSemanticIdentityV1,
) -> Result<(u64, NeuralField, SparseGraph), StoreError> {
    if let Some((revision, state, graph, _)) = semantic_head(tx, persona_scope)? {
        let committed = read_semantic_commit(tx, persona_scope, revision)?;
        if committed.state_digest != state || committed.graph_digest != graph {
            return Err(StoreError::ContinuityFence(
                "semantic_cursor_history_binding",
            ));
        }
        let (field, graph_value) = match committed.transition_kind {
            SemanticTransitionKindV1::Perception => decoded_semantic_state(
                &committed.snapshot_bytes,
                committed.formula_digest,
                committed.state_digest,
                committed.graph_digest,
            )?,
            SemanticTransitionKindV1::Time => {
                let (_, epoch) =
                    semantic_cursor_epoch_v1(tx, persona_scope, revision, committed.state_digest)?;
                let (anchor, graph_value) =
                    semantic_time_anchor_state_v1(tx, persona_scope, &epoch, origin, identity)?;
                let reconstructed = advance_matrix_time_v1(MatrixTimeInputV1 {
                    anchor_field: &anchor,
                    genesis_baseline: &identity.baseline_field,
                    epoch: &epoch,
                    elapsed_ms: 0,
                    phase: MatrixSleepPhaseV1::Awake,
                    semantic_formula_digest: committed.formula_digest,
                })
                .map_err(semantic_core_history_error)?;
                if reconstructed.epoch != epoch
                    || state_digest(&reconstructed.field, &committed.formula_digest)
                        != committed.state_digest
                    || graph_digest(&graph_value) != committed.graph_digest
                {
                    return Err(StoreError::ContinuityFence("semantic_time_cursor_state"));
                }
                (reconstructed.field, graph_value)
            }
        };
        return Ok((revision, field, graph_value));
    }

    let (field, graph) = semantic_origin_state(tx, origin, identity)?;
    Ok((origin.source_revision, field, graph))
}

fn observer_identity_for_persona_tx_v1(
    tx: &Connection,
    persona_scope: Digest,
) -> Result<(ScopeRef, ActiveSemanticIdentityV1, u64), StoreError> {
    if crate::core_boundary_v9::preflight(tx)? == crate::core_boundary_v9::OpenRoute::V9 {
        // V9 has no active autonomy_scope_binding. Resolve the persona against
        // the actual Genesis bindings; all candidate identities are bounded.
        let mut statement=tx.prepare("SELECT bot_token,persona_token,revision FROM active_bindings ORDER BY bot_token,persona_token LIMIT 4097")?;
        let mut rows=statement.query([])?;let mut found=None;let mut count=0;
        while let Some(row)=rows.next()? {
            count+=1;if count>4096{return Err(StoreError::ContinuityFence("semantic_observer_inventory_bound"));}
            let bot=id_from_vec(row.get(0)?,"active_binding.bot")?;let persona=id_from_vec(row.get(1)?,"active_binding.persona")?;
            if wire::persona_scope_digest(&bot,&persona,None)==persona_scope {
                if found.is_some(){return Err(StoreError::ContinuityFence("semantic_observer_identity_collision"));}
                let revision=semantic_revision_from_sql(row.get(2)?)?;
                if revision==0{return Err(StoreError::ContinuityFence("semantic_observer_personality_revision"));}
                found=Some((ScopeRef{bot_token:bot,persona_token:persona,relation_token:None,session_token:[0;16]},revision));
            }
        }
        let (scope,revision)=found.ok_or(StoreError::GenesisNotFound)?;
        let identity=active_identity_for_scope_tx(tx,&scope)?;
        return Ok((scope,identity,revision));
    }
    type RawScope = (Option<String>, i64);
    let raw: RawScope = tx
        .query_row(
            "SELECT
               CASE WHEN typeof(scope_json)='text'
                          AND length(CAST(scope_json AS BLOB))<=?2
                    THEN scope_json END,
               CASE WHEN typeof(scope_json)='text'
                    THEN length(CAST(scope_json AS BLOB)) ELSE -1 END
             FROM autonomy_scope_binding
             WHERE work_scope=?1 AND persona_scope=?1 AND relation_scope IS NULL",
            params![
                blob(persona_scope),
                i64::try_from(MAX_SEMANTIC_OBSERVER_SCOPE_BYTES)
                    .map_err(|_| StoreError::ContinuityFence("semantic_observer_scope_budget"))?
            ],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
        .ok_or(StoreError::ContinuityFence(
            "semantic_observer_scope_missing",
        ))?;
    let body = bounded_typed_value(
        raw.0,
        raw.1,
        MAX_SEMANTIC_OBSERVER_SCOPE_BYTES,
        "autonomy_scope_binding.scope_json",
        "semantic_observer_scope_type",
    )?;
    let scope: ScopeRef = serde_json::from_str(&body)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_scope_wire"))?;
    let canonical = serde_json::to_string(&scope)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_scope_wire"))?;
    if canonical != body
        || scope.relation_token.is_some()
        || wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None) != persona_scope
    {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_scope_binding",
        ));
    }
    let raw_personality_revision: i64 = tx
        .query_row(
            "SELECT CASE WHEN typeof(revision)='integer' THEN revision ELSE -1 END
             FROM active_bindings WHERE bot_token=?1 AND persona_token=?2",
            params![blob(scope.bot_token), blob(scope.persona_token)],
            |row| row.get(0),
        )
        .optional()?
        .ok_or(StoreError::ContinuityFence(
            "semantic_observer_active_binding_missing",
        ))?;
    let personality_revision = semantic_revision_from_sql(raw_personality_revision)?;
    if personality_revision == 0 {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_personality_revision",
        ));
    }
    let identity = active_identity_for_scope_tx(tx, &scope)?;
    Ok((scope, identity, personality_revision))
}

fn reconstruct_time_epoch_field_v1(
    anchor: &NeuralField,
    baseline: &NeuralField,
    epoch: &MatrixTimeEpochV1,
    formula_digest: Digest,
) -> Result<NeuralField, StoreError> {
    let reconstructed = advance_matrix_time_v1(MatrixTimeInputV1 {
        anchor_field: anchor,
        genesis_baseline: baseline,
        epoch,
        elapsed_ms: 0,
        phase: MatrixSleepPhaseV1::Awake,
        semantic_formula_digest: formula_digest,
    })
    .map_err(semantic_core_history_error)?;
    if reconstructed.epoch != *epoch {
        return Err(StoreError::ContinuityFence("semantic_observer_time_epoch"));
    }
    Ok(reconstructed.field)
}

fn perception_attested_confidence_v1(
    conn: &Connection,
    committed: &CommittedSemanticV1,
    stimulus: &UserStimulus,
) -> Result<u32, StoreError> {
    type RawAttestation = (
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
    );
    let raw: RawAttestation = conn
        .query_row(
            "SELECT
               CASE
                 WHEN perception_nonce_digest IS NULL
                  AND perception_proposal_digest IS NULL
                  AND perception_origin_digest IS NULL
                  AND perception_origin_bytes IS NULL THEN 0
                 WHEN perception_nonce_digest IS NOT NULL
                  AND perception_proposal_digest IS NOT NULL
                  AND perception_origin_digest IS NOT NULL
                  AND perception_origin_bytes IS NOT NULL THEN 1
                 ELSE -1
               END,
               CASE WHEN typeof(perception_nonce_digest)='blob'
                          AND length(perception_nonce_digest)=32
                    THEN perception_nonce_digest END,
               CASE WHEN typeof(perception_nonce_digest)='blob'
                    THEN length(perception_nonce_digest) ELSE -1 END,
               CASE WHEN typeof(perception_proposal_digest)='blob'
                          AND length(perception_proposal_digest)=32
                    THEN perception_proposal_digest END,
               CASE WHEN typeof(perception_proposal_digest)='blob'
                    THEN length(perception_proposal_digest) ELSE -1 END,
               CASE WHEN typeof(perception_origin_digest)='blob'
                          AND length(perception_origin_digest)=32
                    THEN perception_origin_digest END,
               CASE WHEN typeof(perception_origin_digest)='blob'
                    THEN length(perception_origin_digest) ELSE -1 END,
               CASE WHEN typeof(perception_origin_bytes)='blob'
                          AND length(perception_origin_bytes)<=?3
                    THEN perception_origin_bytes END,
               CASE WHEN typeof(perception_origin_bytes)='blob'
                    THEN length(perception_origin_bytes) ELSE -1 END
             FROM semantic_evidence_authority
             WHERE persona_scope=?1 AND semantic_revision=?2",
            params![
                blob(committed.persona_scope),
                JournalRevision::new(committed.semantic_revision)
                    .to_sqlite()?
                    .get(),
                i64::try_from(MAX_PERCEPTION_ORIGIN_BYTES)
                    .map_err(|_| StoreError::ContinuityFence("semantic_observer_attestation"))?,
            ],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?
        .ok_or(StoreError::ContinuityFence(
            "semantic_observer_attestation_missing",
        ))?;
    if raw.0 == 0 {
        return Ok(0);
    }
    if raw.0 != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_attestation_partial",
        ));
    }
    let nonce = digest_from_vec(
        bounded_typed_value(
            raw.1,
            raw.2,
            32,
            "semantic_evidence_authority.perception_nonce_digest",
            "semantic_observer_attestation_nonce",
        )?,
        "semantic_observer_attestation_nonce",
    )?;
    let proposal_digest = digest_from_vec(
        bounded_typed_value(
            raw.3,
            raw.4,
            32,
            "semantic_evidence_authority.perception_proposal_digest",
            "semantic_observer_attestation_proposal",
        )?,
        "semantic_observer_attestation_proposal",
    )?;
    let origin_digest = digest_from_vec(
        bounded_typed_value(
            raw.5,
            raw.6,
            32,
            "semantic_evidence_authority.perception_origin_digest",
            "semantic_observer_attestation_origin",
        )?,
        "semantic_observer_attestation_origin",
    )?;
    let origin_bytes = bounded_typed_value(
        raw.7,
        raw.8,
        MAX_PERCEPTION_ORIGIN_BYTES,
        "semantic_evidence_authority.perception_origin_bytes",
        "semantic_observer_attestation_origin_wire",
    )?;
    let origin: PerceptionOriginCommitmentV1 = serde_json::from_slice(&origin_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_attestation_origin_wire"))?;
    let canonical_origin = serde_json::to_vec(&origin)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_attestation_origin_wire"))?;
    let authoritative = committed_perception_origin_v1(
        conn,
        origin.origin_event_digest,
        Some(committed.persona_scope),
    )?
    .origin;
    let authoritative_bytes = serde_json::to_vec(&authoritative)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_attestation_origin_wire"))?;
    let proposal = PerceptionProposalV1 {
        schema_version: PerceptionProposalV1::SCHEMA_VERSION,
        origin_digest,
        dimensions: stimulus.evidence.dimensions.clone(),
        estimator_confidence: stimulus.evidence.estimator_confidence,
        protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
        request_nonce_digest: nonce,
    };
    if proposal.validate_v1().is_err()
        || !(origin.validate_v1() || origin.validate_core_v1())
        || canonical_origin != origin_bytes
        || origin != authoritative
        || canonical_origin != authoritative_bytes
        || origin.origin_digest != origin_digest
        || proposal.estimator_digest_v1() != proposal_digest
        || committed.estimator_digest != Some(proposal_digest)
        || committed.event_id != origin.event_id
        || committed.relation_scope != origin.relation_present.then_some(origin.relation_scope)
        || stimulus.scope != origin.scope
        || stimulus.causal.turn_id != origin.turn_id
        || stimulus.causal.base_revision != committed.journal.base_revision
        || stimulus.causal.base_revision < origin.canonical_base_revision
        || stimulus.observed_at_ms != origin.observed_at_ms
        || committed.incarnation_id != origin.incarnation_id
        || committed.manifest_digest != origin.manifest_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_attestation_binding",
        ));
    }
    u32::try_from(proposal.estimator_confidence.raw())
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_confidence"))
}

fn perception_observer_metadata_v1(
    conn: &Connection,
    committed: &CommittedSemanticV1,
) -> Result<(u32, i64, ae_semantic_core::TransitionReceiptV2), StoreError> {
    if committed.transition_kind != SemanticTransitionKindV1::Perception {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_perception_kind",
        ));
    }
    let event = wire::decode_event(&committed.journal.event_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_perception_event"))?;
    let stimulus = validated_stimulus(&event)
        .map_err(|_| StoreError::ContinuityFence("semantic_observer_perception_event"))?;
    let confidence_fxp6 = perception_attested_confidence_v1(conn, committed, stimulus)?;
    let receipt = decode_transition_receipt_v2(committed.receipt_bytes.as_deref().ok_or(
        StoreError::ContinuityFence("semantic_observer_perception_receipt"),
    )?)
    .map_err(semantic_core_history_error)?;
    if !receipt.validate()
        || receipt.next_revision != committed.semantic_revision
        || receipt.state_after != committed.state_digest
        || receipt.graph_after != committed.graph_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_perception_receipt",
        ));
    }
    Ok((
        confidence_fxp6,
        receipt.residuals.renormalization.raw(),
        receipt,
    ))
}

fn field_for_attested_commit_v1(
    tx: &Connection,
    persona_scope: Digest,
    committed: &CommittedSemanticV1,
    origin: &SemanticOriginV1,
    identity: &ActiveSemanticIdentityV1,
) -> Result<NeuralField, StoreError> {
    match committed.transition_kind {
        SemanticTransitionKindV1::Perception => decoded_semantic_state(
            &committed.snapshot_bytes,
            committed.formula_digest,
            committed.state_digest,
            committed.graph_digest,
        )
        .map(|(field, _)| field),
        SemanticTransitionKindV1::Time => {
            let decoded = decode_time_snapshot_v1(&committed.snapshot_bytes)
                .map_err(semantic_core_history_error)?;
            let (anchor, graph) = semantic_time_anchor_state_v1(
                tx,
                persona_scope,
                &decoded.epoch_after,
                origin,
                identity,
            )?;
            if graph_digest(&graph) != committed.graph_digest {
                return Err(StoreError::ContinuityFence("semantic_observer_time_graph"));
            }
            let field = reconstruct_time_epoch_field_v1(
                &anchor,
                &identity.baseline_field,
                &decoded.epoch_after,
                committed.formula_digest,
            )?;
            if state_digest(&field, &committed.formula_digest) != committed.state_digest {
                return Err(StoreError::ContinuityFence("semantic_observer_time_state"));
            }
            Ok(field)
        }
    }
}

fn semantic_time_observer_anchor_state_v1(
    tx: &Connection,
    persona_scope: Digest,
    epoch: &MatrixTimeEpochV1,
    origin: &SemanticOriginV1,
    identity: &ActiveSemanticIdentityV1,
) -> Result<(NeuralField, SparseGraph, u32, i64), StoreError> {
    if epoch.anchor_semantic_revision == origin.source_revision {
        let (field, graph) = semantic_origin_state(tx, origin, identity)?;
        if state_digest(&field, &origin.formula_digest) != epoch.anchor_state_digest {
            return Err(StoreError::ContinuityFence("semantic_observer_time_anchor"));
        }
        return Ok((field, graph, 0, 0));
    }
    if epoch.anchor_semantic_revision <= origin.source_revision {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_time_anchor_revision",
        ));
    }
    let committed = read_semantic_commit(tx, persona_scope, epoch.anchor_semantic_revision)?;
    if committed.transition_kind != SemanticTransitionKindV1::Perception
        || committed.state_digest != epoch.anchor_state_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_time_anchor_kind",
        ));
    }
    let (field, graph) = decoded_semantic_state(
        &committed.snapshot_bytes,
        committed.formula_digest,
        committed.state_digest,
        committed.graph_digest,
    )?;
    let (confidence, residual, _) = perception_observer_metadata_v1(tx, &committed)?;
    Ok((field, graph, confidence, residual))
}

/// Build the complete affect observer input from at most the current semantic
/// commit, its immediate predecessor and one immutable perception/Genesis
/// anchor. The caller owns the read transaction and query-only guard.
pub(crate) fn attested_affect_projection_input_tx_v1(
    tx: &Connection,
    persona_scope: Digest,
) -> Result<Option<AttestedAffectProjectionInputV1>, StoreError> {
    let Some((semantic_revision, head_state, head_graph, head_commitment)) =
        semantic_head(tx, persona_scope)?
    else {
        return Ok(None);
    };
    let origin = stored_origin(tx, persona_scope)?.ok_or(StoreError::ContinuityFence(
        "semantic_observer_origin_missing",
    ))?;
    let (_scope, identity, personality_revision) =
        observer_identity_for_persona_tx_v1(tx, persona_scope)?;
    let formula_digest = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let current = read_semantic_commit(tx, persona_scope, semantic_revision)?;
    if current.commitment_digest != head_commitment
        || current.state_digest != head_state
        || current.graph_digest != head_graph
        || current.formula_digest != formula_digest
        || current.origin != origin
    {
        return Err(StoreError::ContinuityFence("semantic_observer_head"));
    }

    let (previous_field, current_field, graph, confidence_fxp6, semantic_residual) = match current
        .transition_kind
    {
        SemanticTransitionKindV1::Perception => {
            let (current_field, graph) = decoded_semantic_state(
                &current.snapshot_bytes,
                formula_digest,
                current.state_digest,
                current.graph_digest,
            )?;
            let (confidence, residual, receipt) = perception_observer_metadata_v1(tx, &current)?;
            let previous_field = if let Some((field, _)) = crate::embodiment_clock::semantic_proof_input(tx, &ae_contracts::PersonaScopeRef { bot_token: _scope.bot_token, persona_token: _scope.persona_token }, &current)? {
                field
            } else if receipt.base_revision == origin.source_revision {
                let (field, _) = semantic_origin_state(tx, &origin, &identity)?;
                if state_digest(&field, &formula_digest) != receipt.state_before
                    || origin.graph_digest != current.graph_before
                {
                    return Err(StoreError::ContinuityFence(
                        "semantic_observer_perception_predecessor",
                    ));
                }
                field
            } else {
                if receipt.base_revision.checked_add(1) != Some(semantic_revision) {
                    return Err(StoreError::ContinuityFence(
                        "semantic_observer_perception_predecessor",
                    ));
                }
                let predecessor = read_semantic_commit(tx, persona_scope, receipt.base_revision)?;
                if predecessor.graph_digest != current.graph_before {
                    return Err(StoreError::ContinuityFence(
                        "semantic_observer_perception_predecessor",
                    ));
                }
                field_for_attested_commit_v1(tx, persona_scope, &predecessor, &origin, &identity)?
            };
            if state_digest(&previous_field, &formula_digest) != receipt.state_before {
                return Err(StoreError::ContinuityFence(
                    "semantic_observer_perception_predecessor",
                ));
            }
            (previous_field, current_field, graph, confidence, residual)
        }
        SemanticTransitionKindV1::Time => {
            let decoded = decode_time_snapshot_v1(&current.snapshot_bytes)
                .map_err(semantic_core_history_error)?;
            let predecessor_revision =
                semantic_revision
                    .checked_sub(1)
                    .ok_or(StoreError::ContinuityFence(
                        "semantic_observer_time_predecessor",
                    ))?;
            if predecessor_revision < origin.source_revision {
                return Err(StoreError::ContinuityFence(
                    "semantic_observer_time_predecessor",
                ));
            }
            if predecessor_revision == origin.source_revision {
                if origin.state_digest != decoded.state_before
                    || origin.graph_digest != current.graph_digest
                {
                    return Err(StoreError::ContinuityFence(
                        "semantic_observer_time_predecessor",
                    ));
                }
            } else {
                let predecessor = read_semantic_commit(tx, persona_scope, predecessor_revision)?;
                if predecessor.state_digest != decoded.state_before
                    || predecessor.graph_digest != current.graph_digest
                {
                    return Err(StoreError::ContinuityFence(
                        "semantic_observer_time_predecessor",
                    ));
                }
            }
            let (anchor, graph, confidence, residual) = semantic_time_observer_anchor_state_v1(
                tx,
                persona_scope,
                &decoded.epoch_after,
                &origin,
                &identity,
            )?;
            let previous_field = reconstruct_time_epoch_field_v1(
                &anchor,
                &identity.baseline_field,
                &decoded.epoch_before,
                formula_digest,
            )?;
            let current_field = reconstruct_time_epoch_field_v1(
                &anchor,
                &identity.baseline_field,
                &decoded.epoch_after,
                formula_digest,
            )?;
            if state_digest(&previous_field, &formula_digest) != decoded.state_before
                || state_digest(&current_field, &formula_digest) != decoded.state_after
                || decoded.state_after != current.state_digest
                || graph_digest(&graph) != current.graph_digest
            {
                return Err(StoreError::ContinuityFence(
                    "semantic_observer_time_projection",
                ));
            }
            (previous_field, current_field, graph, confidence, residual)
        }
    };
    if !previous_field.validate()
        || !current_field.validate()
        || !graph.validate()
        || state_digest(&current_field, &formula_digest) != head_state
        || graph_digest(&graph) != head_graph
    {
        return Err(StoreError::ContinuityFence(
            "semantic_observer_projection_state",
        ));
    }
    Ok(Some(AttestedAffectProjectionInputV1 {
        semantic_revision,
        personality_revision,
        formula_digest,
        state_digest: head_state,
        graph_digest: head_graph,
        previous_field,
        current_field,
        graph,
        confidence_fxp6,
        semantic_dynamics_renormalization_residual_fxp6: semantic_residual,
    }))
}

pub(crate) struct SemanticWakeReceiptAuthorityV1 {
    pub(crate) formula_digest: Digest,
    pub(crate) graph_digest: Digest,
    pub(crate) active_nodes: u32,
    pub(crate) active_edges: u32,
    pub(crate) initial_snapshot_digest: Digest,
}

/// Resolve every semantic receipt field inside the same Store transaction
/// that will append the wake. Runtime hot state is deliberately not an input.
pub(crate) fn semantic_wake_receipt_authority_tx_v1(
    tx: &Connection,
    scope: &ScopeRef,
) -> Result<SemanticWakeReceiptAuthorityV1, StoreError> {
    let persona_scope = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let identity = active_identity_for_scope_tx(tx, scope)?;
    let formula_digest = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let (field, graph) = if let Some(origin) = stored_origin(tx, persona_scope)? {
        let (_, field, graph) =
            semantic_state_for_derivation_tx(tx, persona_scope, &origin, &identity)?;
        (field, graph)
    } else {
        if semantic_head(tx, persona_scope)?.is_some() {
            return Err(StoreError::ContinuityFence("semantic_origin_missing"));
        }
        (
            identity.baseline_field.clone(),
            identity.baseline_graph.clone(),
        )
    };
    if !field.validate() || !graph.validate() {
        return Err(StoreError::ContinuityFence("semantic_wake_receipt_state"));
    }
    Ok(SemanticWakeReceiptAuthorityV1 {
        formula_digest,
        graph_digest: graph_digest(&graph),
        active_nodes: field.active_node_count(),
        active_edges: u32::try_from(graph.edges.len())
            .map_err(|_| StoreError::ContinuityFence("semantic_wake_receipt_graph"))?,
        initial_snapshot_digest: identity.initial_snapshot_digest,
    })
}

fn checked_semantic_aggregate_bytes<const N: usize>(
    existing: u64,
    incoming: [u64; N],
) -> Result<u64, StoreError> {
    let total = incoming.into_iter().try_fold(existing, |total, bytes| {
        total
            .checked_add(bytes)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic.bytes",
                limit: MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
                actual: u64::MAX,
            })
    })?;
    enforce_byte_budget(
        "semantic.bytes",
        total,
        MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
    )?;
    Ok(total)
}

#[derive(Clone, Copy, Debug)]
struct SemanticBudgetAdvanceV1 {
    had_checkpoint: bool,
    row_count: u64,
    aggregate_bytes: u64,
}

fn preflight_semantic_budget_tx(
    tx: &Transaction<'_>,
    persona_scope: Digest,
    origin_source_revision: u64,
    semantic_base_revision: u64,
    incoming: [u64; 5],
) -> Result<SemanticBudgetAdvanceV1, StoreError> {
    let stored = read_semantic_budget_checkpoint_v1(tx, persona_scope)?;
    let (had_checkpoint, current_count, current_bytes) =
        if let Some((raw_count, raw_bytes, raw_head, head_commitment)) = stored {
            let count = sqlite_length(raw_count, "semantic_budget.rows")?;
            let aggregate = sqlite_length(raw_bytes, "semantic_budget.aggregate_bytes")?;
            let head_revision = semantic_revision_from_sql(raw_head)?;
            let expected_count = semantic_base_revision
                .checked_sub(origin_source_revision)
                .ok_or(StoreError::ContinuityFence("semantic_budget_revision"))?;
            let actual_head = semantic_head_tx(tx, persona_scope)?
                .ok_or(StoreError::ContinuityFence("semantic_budget_head"))?;
            if count != expected_count
                || head_revision != semantic_base_revision
                || actual_head.0 != head_revision
                || actual_head.3 != head_commitment
            {
                return Err(StoreError::ContinuityFence("semantic_budget_checkpoint"));
            }
            (true, count, aggregate)
        } else {
            if semantic_base_revision != origin_source_revision
                || semantic_head_tx(tx, persona_scope)?.is_some()
            {
                return Err(StoreError::ContinuityFence("semantic_budget_missing"));
            }
            (false, 0, 0)
        };
    let row_count = current_count
        .checked_add(1)
        .ok_or(StoreError::StorageBudgetExceeded {
            resource: "semantic.rows",
            limit: MAX_SEMANTIC_ROWS_PER_PERSONA,
            actual: u64::MAX,
        })?;
    enforce_byte_budget("semantic.rows", row_count, MAX_SEMANTIC_ROWS_PER_PERSONA)?;
    let aggregate_bytes = checked_semantic_aggregate_bytes(current_bytes, incoming)?;
    Ok(SemanticBudgetAdvanceV1 {
        had_checkpoint,
        row_count,
        aggregate_bytes,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemanticRevisionBounds {
    count: u64,
    min: u64,
    max: u64,
}

fn semantic_revision_bounds(
    conn: &Connection,
    table: &'static str,
    persona_scope: Digest,
) -> Result<SemanticRevisionBounds, StoreError> {
    // `table` is selected exclusively from the closed constant list below.
    let sql = format!(
        "SELECT COUNT(*), COALESCE(MIN(semantic_revision),0), \
         COALESCE(MAX(semantic_revision),0) FROM {table} WHERE persona_scope=?1"
    );
    let raw: (i64, i64, i64) = conn.query_row(&sql, params![blob(persona_scope)], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    let count = sqlite_length(raw.0, "semantic.history.rows")?;
    enforce_byte_budget(
        "semantic.history.rows",
        count,
        MAX_SEMANTIC_ROWS_PER_PERSONA,
    )?;
    let min = JournalRevision::try_from(raw.1)?.get();
    let max = JournalRevision::try_from(raw.2)?.get();
    if count == 0 && (min != 0 || max != 0) {
        return Err(StoreError::ContinuityFence("semantic_revision_set"));
    }
    Ok(SemanticRevisionBounds { count, min, max })
}

const SEMANTIC_HISTORY_SCOPE_TABLES_V1: [&str; 10] = [
    "semantic_origins",
    "semantic_commits",
    "semantic_cursor",
    "semantic_snapshots",
    "semantic_graphs",
    "semantic_receipts",
    "semantic_telemetry",
    "semantic_evidence_authority",
    "semantic_time_authority",
    "semantic_budget_checkpoint",
];

// Exact V3 has every V4 persona-scoped table except the time authority lane.
// Keep this list independent from the weak-898b layout: V3 already owns the
// authenticated budget checkpoint and it must participate in persona admission
// before V4 performs its first ALTER/UPDATE/CREATE.
const SEMANTIC_V3_HISTORY_SCOPE_TABLES_V1: [&str; 9] = [
    "semantic_origins",
    "semantic_commits",
    "semantic_cursor",
    "semantic_snapshots",
    "semantic_graphs",
    "semantic_receipts",
    "semantic_telemetry",
    "semantic_evidence_authority",
    "semantic_budget_checkpoint",
];

const WEAK_SEMANTIC_HISTORY_SCOPE_TABLES_V1: [&str; 8] = [
    "semantic_origins",
    "semantic_commits",
    "semantic_cursor",
    "semantic_snapshots",
    "semantic_graphs",
    "semantic_receipts",
    "semantic_telemetry",
    "semantic_evidence_authority",
];

const WEAK_SEMANTIC_HISTORY_STAGING_SCOPE_TABLES_V1: [&str; 8] = [
    "__ae_semantic_origins_v1",
    "__ae_semantic_commits_v1",
    "__ae_semantic_cursor_v1",
    "__ae_semantic_snapshots_v1",
    "__ae_semantic_graphs_v1",
    "__ae_semantic_receipts_v1",
    "__ae_semantic_telemetry_v1",
    "__ae_semantic_evidence_authority_v1",
];

const STRONG_V2_EVIDENCE_STAGING_SCOPE_TABLES_V1: [&str; 1] =
    ["__ae_semantic_evidence_authority_v2"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemanticHistoryScanTableV1 {
    table: &'static str,
    payload_column: Option<&'static str>,
    is_commit_table: bool,
}

const SEMANTIC_HISTORY_SCAN_TABLES_V1: [SemanticHistoryScanTableV1; 7] = [
    SemanticHistoryScanTableV1 {
        table: "semantic_commits",
        payload_column: None,
        is_commit_table: true,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_snapshots",
        payload_column: Some("snapshot_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_graphs",
        payload_column: Some("graph_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_receipts",
        payload_column: Some("receipt_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_telemetry",
        payload_column: Some("telemetry_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_evidence_authority",
        payload_column: Some("evidence_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_time_authority",
        payload_column: Some("authority_bytes"),
        is_commit_table: false,
    },
];

const SEMANTIC_V3_HISTORY_SCAN_TABLES_V1: [SemanticHistoryScanTableV1; 6] = [
    SemanticHistoryScanTableV1 {
        table: "semantic_commits",
        payload_column: None,
        is_commit_table: true,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_snapshots",
        payload_column: Some("snapshot_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_graphs",
        payload_column: Some("graph_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_receipts",
        payload_column: Some("receipt_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_telemetry",
        payload_column: Some("telemetry_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_evidence_authority",
        payload_column: Some("evidence_bytes"),
        is_commit_table: false,
    },
];

const WEAK_SEMANTIC_HISTORY_SCAN_TABLES_V1: [SemanticHistoryScanTableV1; 6] = [
    SemanticHistoryScanTableV1 {
        table: "semantic_commits",
        payload_column: None,
        is_commit_table: true,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_snapshots",
        payload_column: Some("snapshot_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_graphs",
        payload_column: Some("graph_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_receipts",
        payload_column: Some("receipt_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_telemetry",
        payload_column: Some("telemetry_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "semantic_evidence_authority",
        payload_column: Some("evidence_bytes"),
        is_commit_table: false,
    },
];

const WEAK_SEMANTIC_HISTORY_STAGING_SCAN_TABLES_V1: [SemanticHistoryScanTableV1; 6] = [
    SemanticHistoryScanTableV1 {
        table: "__ae_semantic_commits_v1",
        payload_column: None,
        is_commit_table: true,
    },
    SemanticHistoryScanTableV1 {
        table: "__ae_semantic_snapshots_v1",
        payload_column: Some("snapshot_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "__ae_semantic_graphs_v1",
        payload_column: Some("graph_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "__ae_semantic_receipts_v1",
        payload_column: Some("receipt_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "__ae_semantic_telemetry_v1",
        payload_column: Some("telemetry_bytes"),
        is_commit_table: false,
    },
    SemanticHistoryScanTableV1 {
        table: "__ae_semantic_evidence_authority_v1",
        payload_column: Some("evidence_bytes"),
        is_commit_table: false,
    },
];

const STRONG_V2_EVIDENCE_STAGING_SCAN_TABLES_V1: [SemanticHistoryScanTableV1; 1] =
    [SemanticHistoryScanTableV1 {
        table: "__ae_semantic_evidence_authority_v2",
        payload_column: Some("evidence_bytes"),
        is_commit_table: false,
    }];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemanticHistoryScanLimitsV1 {
    personas_global: u64,
    table_rows_per_persona: u64,
    scan_rows_per_persona: u64,
    rows_global: u64,
    bytes_per_persona: u64,
    bytes_global: u64,
}

const SEMANTIC_HISTORY_SCAN_LIMITS_V1: SemanticHistoryScanLimitsV1 = SemanticHistoryScanLimitsV1 {
    personas_global: MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
    table_rows_per_persona: MAX_SEMANTIC_ROWS_PER_PERSONA,
    scan_rows_per_persona: MAX_SEMANTIC_HISTORY_SCAN_ROWS_PER_PERSONA,
    rows_global: MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
    bytes_per_persona: MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
    bytes_global: MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SemanticHistoryScanBudgetV1 {
    personas: u64,
    rows: u64,
    bytes: u64,
    limits: SemanticHistoryScanLimitsV1,
}

impl Default for SemanticHistoryScanBudgetV1 {
    fn default() -> Self {
        Self {
            personas: 0,
            rows: 0,
            bytes: 0,
            limits: SEMANTIC_HISTORY_SCAN_LIMITS_V1,
        }
    }
}

impl SemanticHistoryScanBudgetV1 {
    fn with_limits(limits: SemanticHistoryScanLimitsV1) -> Self {
        Self {
            personas: 0,
            rows: 0,
            bytes: 0,
            limits,
        }
    }

    fn admit_persona(&mut self) -> Result<(), StoreError> {
        let personas = self
            .personas
            .checked_add(1)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.global_personas",
                limit: self.limits.personas_global,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "semantic.history.global_personas",
            personas,
            self.limits.personas_global,
        )?;
        self.personas = personas;
        Ok(())
    }

    fn remaining_rows(&self) -> u64 {
        self.limits.rows_global.saturating_sub(self.rows)
    }

    fn admit_history(&mut self, rows: u64, bytes: u64) -> Result<(), StoreError> {
        let next_rows = self
            .rows
            .checked_add(rows)
            .ok_or(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.global_scan_rows",
                limit: self.limits.rows_global,
                actual: u64::MAX,
            })?;
        enforce_byte_budget(
            "semantic.history.global_scan_rows",
            next_rows,
            self.limits.rows_global,
        )?;
        let next_bytes =
            self.bytes
                .checked_add(bytes)
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "semantic.history.global_scan_bytes",
                    limit: self.limits.bytes_global,
                    actual: u64::MAX,
                })?;
        enforce_byte_budget(
            "semantic.history.global_scan_bytes",
            next_bytes,
            self.limits.bytes_global,
        )?;
        self.rows = next_rows;
        self.bytes = next_bytes;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SemanticHistoryScopeScanV1 {
    commit_rows: u64,
    physical_rows: u64,
    aggregate_bytes: u64,
}

fn next_semantic_persona_scope_in_table_v1(
    conn: &Connection,
    table: &'static str,
    last_scope: Option<Digest>,
) -> Result<Option<Digest>, StoreError> {
    let known_table = SEMANTIC_HISTORY_SCOPE_TABLES_V1.contains(&table)
        || WEAK_SEMANTIC_HISTORY_SCOPE_TABLES_V1.contains(&table)
        || WEAK_SEMANTIC_HISTORY_STAGING_SCOPE_TABLES_V1.contains(&table)
        || STRONG_V2_EVIDENCE_STAGING_SCOPE_TABLES_V1.contains(&table);
    if !known_table {
        return Err(StoreError::ContinuityFence(
            "semantic_history_table_identifier",
        ));
    }
    let projection = format!(
        "SELECT
           CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                THEN persona_scope END,
           CASE WHEN typeof(persona_scope)='blob' THEN length(persona_scope) ELSE -1 END
         FROM {table}"
    );
    let raw: Option<(Option<Vec<u8>>, i64)> = if let Some(last_scope) = last_scope {
        conn.query_row(
            &format!("{projection} WHERE persona_scope>?1 ORDER BY persona_scope LIMIT 1"),
            params![blob(last_scope)],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    } else {
        conn.query_row(
            &format!("{projection} ORDER BY persona_scope LIMIT 1"),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?
    };
    let Some((bounded_scope, raw_len)) = raw else {
        return Ok(None);
    };
    if raw_len < 0 {
        return Err(StoreError::ContinuityFence("semantic_persona_scope_type"));
    }
    let bounded_scope = bounded_scope.ok_or(StoreError::InvalidStoredDigest {
        field: "semantic_history.persona_scope",
        actual: u64::try_from(raw_len).unwrap_or(u64::MAX),
    })?;
    digest_from_vec(bounded_scope, "semantic_history.persona_scope").map(Some)
}

fn next_semantic_persona_scope_v1(
    conn: &Connection,
    last_scope: Option<Digest>,
) -> Result<Option<Digest>, StoreError> {
    next_semantic_persona_scope_in_tables_v1(conn, &SEMANTIC_HISTORY_SCOPE_TABLES_V1, last_scope)
}

fn next_semantic_persona_scope_in_tables_v1(
    conn: &Connection,
    tables: &[&'static str],
    last_scope: Option<Digest>,
) -> Result<Option<Digest>, StoreError> {
    let mut next = None;
    for &table in tables {
        if let Some(candidate) = next_semantic_persona_scope_in_table_v1(conn, table, last_scope)? {
            if next.map_or(true, |current| candidate < current) {
                next = Some(candidate);
            }
        }
    }
    Ok(next)
}

fn scan_semantic_history_scope_v1(
    conn: &Connection,
    persona_scope: Digest,
    global: &mut SemanticHistoryScanBudgetV1,
) -> Result<SemanticHistoryScopeScanV1, StoreError> {
    scan_semantic_history_scope_in_tables_v1(
        conn,
        persona_scope,
        global,
        &SEMANTIC_HISTORY_SCAN_TABLES_V1,
    )
}

fn scan_semantic_history_scope_in_tables_v1(
    conn: &Connection,
    persona_scope: Digest,
    global: &mut SemanticHistoryScanBudgetV1,
    tables: &[SemanticHistoryScanTableV1],
) -> Result<SemanticHistoryScopeScanV1, StoreError> {
    let mut scanned = SemanticHistoryScopeScanV1::default();
    for table_spec in tables {
        let known_table = SEMANTIC_HISTORY_SCAN_TABLES_V1.contains(table_spec)
            || SEMANTIC_V3_HISTORY_SCAN_TABLES_V1.contains(table_spec)
            || WEAK_SEMANTIC_HISTORY_SCAN_TABLES_V1.contains(table_spec)
            || WEAK_SEMANTIC_HISTORY_STAGING_SCAN_TABLES_V1.contains(table_spec)
            || STRONG_V2_EVIDENCE_STAGING_SCAN_TABLES_V1.contains(table_spec);
        if !known_table {
            return Err(StoreError::ContinuityFence(
                "semantic_history_column_identifier",
            ));
        }
        let table = table_spec.table;
        let payload_column = table_spec.payload_column;
        // Table/column identifiers come only from the closed constants above.
        // LIMIT is always the first forbidden row for the narrowest remaining
        // budget, so SQLite cannot walk an unbounded corrupt table before Rust
        // rejects it.
        let length_projection = payload_column.map_or_else(
            || "0".to_owned(),
            |column| format!("CASE WHEN typeof({column})='blob' THEN length({column}) ELSE -1 END"),
        );
        let local_remaining = global
            .limits
            .scan_rows_per_persona
            .saturating_sub(scanned.physical_rows);
        let limit = global
            .limits
            .table_rows_per_persona
            .saturating_add(1)
            .min(local_remaining.saturating_add(1))
            .min(global.remaining_rows().saturating_add(1));
        let limit_sql = i64::try_from(limit).map_err(|_| StoreError::StorageBudgetExceeded {
            resource: "semantic.history.scan_limit",
            limit: global.limits.rows_global,
            actual: limit,
        })?;
        let sql = format!(
            "SELECT {length_projection}
             FROM {table}
             WHERE persona_scope=?1
             ORDER BY semantic_revision
             LIMIT ?2"
        );
        let mut statement = conn.prepare(&sql)?;
        let mut rows = statement.query(params![blob(persona_scope), limit_sql])?;
        let mut table_rows = 0_u64;
        while let Some(row) = rows.next()? {
            table_rows = table_rows
                .checked_add(1)
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "semantic.history.table_rows",
                    limit: global.limits.table_rows_per_persona,
                    actual: u64::MAX,
                })?;
            enforce_byte_budget(
                "semantic.history.table_rows",
                table_rows,
                global.limits.table_rows_per_persona,
            )?;
            let payload_bytes = sqlite_length(row.get(0)?, "semantic.history.payload_bytes")?;
            let physical_rows =
                scanned
                    .physical_rows
                    .checked_add(1)
                    .ok_or(StoreError::StorageBudgetExceeded {
                        resource: "semantic.history.persona_scan_rows",
                        limit: global.limits.scan_rows_per_persona,
                        actual: u64::MAX,
                    })?;
            enforce_byte_budget(
                "semantic.history.persona_scan_rows",
                physical_rows,
                global.limits.scan_rows_per_persona,
            )?;
            let aggregate_bytes = scanned.aggregate_bytes.checked_add(payload_bytes).ok_or(
                StoreError::StorageBudgetExceeded {
                    resource: "semantic.bytes",
                    limit: global.limits.bytes_per_persona,
                    actual: u64::MAX,
                },
            )?;
            enforce_byte_budget(
                "semantic.bytes",
                aggregate_bytes,
                global.limits.bytes_per_persona,
            )?;
            global.admit_history(1, payload_bytes)?;
            scanned.physical_rows = physical_rows;
            scanned.aggregate_bytes = aggregate_bytes;
        }
        if table_spec.is_commit_table {
            scanned.commit_rows = table_rows;
        }
    }
    Ok(scanned)
}

fn preflight_semantic_history_layout_v1(
    conn: &Connection,
    scope_tables: &[&'static str],
    scan_tables: &[SemanticHistoryScanTableV1],
    limits: SemanticHistoryScanLimitsV1,
) -> Result<SemanticHistoryScanBudgetV1, StoreError> {
    let mut last_scope = None;
    let mut budget = SemanticHistoryScanBudgetV1::with_limits(limits);
    while let Some(persona_scope) =
        next_semantic_persona_scope_in_tables_v1(conn, scope_tables, last_scope)?
    {
        // Admission is deliberately before any row/length scan for this scope.
        budget.admit_persona()?;
        scan_semantic_history_scope_in_tables_v1(conn, persona_scope, &mut budget, scan_tables)?;
        last_scope = Some(persona_scope);
    }
    Ok(budget)
}

fn preflight_semantic_v4_history_v1(conn: &Connection) -> Result<(), StoreError> {
    preflight_semantic_history_layout_v1(
        conn,
        &SEMANTIC_HISTORY_SCOPE_TABLES_V1,
        &SEMANTIC_HISTORY_SCAN_TABLES_V1,
        SEMANTIC_HISTORY_SCAN_LIMITS_V1,
    )?;
    Ok(())
}

fn preflight_semantic_v3_history_with_limits_v1(
    conn: &Connection,
    limits: SemanticHistoryScanLimitsV1,
) -> Result<(), StoreError> {
    preflight_semantic_history_layout_v1(
        conn,
        &SEMANTIC_V3_HISTORY_SCOPE_TABLES_V1,
        &SEMANTIC_V3_HISTORY_SCAN_TABLES_V1,
        limits,
    )?;
    Ok(())
}

fn preflight_semantic_v3_history_v1(conn: &Connection) -> Result<(), StoreError> {
    preflight_semantic_v3_history_with_limits_v1(conn, SEMANTIC_HISTORY_SCAN_LIMITS_V1)
}

fn preflight_weak_semantic_history_v1(conn: &Connection, staging: bool) -> Result<(), StoreError> {
    let (scope_tables, scan_tables): (&[&'static str], &[SemanticHistoryScanTableV1]) = if staging {
        (
            &WEAK_SEMANTIC_HISTORY_STAGING_SCOPE_TABLES_V1,
            &WEAK_SEMANTIC_HISTORY_STAGING_SCAN_TABLES_V1,
        )
    } else {
        (
            &WEAK_SEMANTIC_HISTORY_SCOPE_TABLES_V1,
            &WEAK_SEMANTIC_HISTORY_SCAN_TABLES_V1,
        )
    };
    preflight_semantic_history_layout_v1(
        conn,
        scope_tables,
        scan_tables,
        SEMANTIC_HISTORY_SCAN_LIMITS_V1,
    )?;
    Ok(())
}

fn verify_semantic_history_closure(
    conn: &Connection,
    persona_scope: Digest,
) -> Result<(), StoreError> {
    const REVISION_TABLES: [&str; 2] = ["semantic_commits", "semantic_snapshots"];
    let mut bounds = Vec::with_capacity(REVISION_TABLES.len());
    for table in REVISION_TABLES {
        bounds.push(semantic_revision_bounds(conn, table, persona_scope)?);
    }

    let (cursor_rows_raw, origin_rows_raw): (i64, i64) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM semantic_cursor WHERE persona_scope=?1),
           (SELECT COUNT(*) FROM semantic_origins WHERE persona_scope=?1)",
        params![blob(persona_scope)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let cursor_rows = sqlite_length(cursor_rows_raw, "semantic_cursor.rows")?;
    let origin_rows = sqlite_length(origin_rows_raw, "semantic_origins.rows")?;
    let commit_bounds = bounds[0];
    if commit_bounds.count == 0 {
        let orphan_sidecars: i64 = conn.query_row(
            "SELECT
               (SELECT COUNT(*) FROM semantic_receipts WHERE persona_scope=?1)+
               (SELECT COUNT(*) FROM semantic_telemetry WHERE persona_scope=?1)+
               (SELECT COUNT(*) FROM semantic_evidence_authority WHERE persona_scope=?1)+
               (SELECT COUNT(*) FROM semantic_time_authority WHERE persona_scope=?1)",
            params![blob(persona_scope)],
            |row| row.get(0),
        )?;
        if bounds.iter().any(|entry| entry.count != 0)
            || orphan_sidecars != 0
            || cursor_rows != 0
            || origin_rows != 0
        {
            return Err(StoreError::ContinuityFence("semantic_root_closure"));
        }
        return Ok(());
    }
    if cursor_rows == 0 {
        return Err(StoreError::ContinuityFence("semantic_cursor_missing"));
    }
    if cursor_rows != 1 || origin_rows != 1 {
        return Err(StoreError::ContinuityFence("semantic_root_closure"));
    }

    let origin = stored_origin(conn, persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_origin_missing"))?;
    let (cursor_revision, _, _, _) = semantic_head(conn, persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_cursor_missing"))?;
    if cursor_revision != commit_bounds.max {
        return Err(StoreError::ContinuityFence(
            "semantic_cursor_not_canonical_head",
        ));
    }
    let expected_min =
        origin
            .source_revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOutOfRange {
                revision: origin.source_revision,
            })?;
    let expected_count = cursor_revision
        .checked_sub(origin.source_revision)
        .ok_or(StoreError::ContinuityFence("semantic_revision_set"))?;
    if expected_count == 0
        || bounds.iter().any(|entry| {
            entry.count != expected_count
                || entry.min != expected_min
                || entry.max != cursor_revision
        })
    {
        return Err(StoreError::ContinuityFence("semantic_revision_set"));
    }
    let lane_counts: (i64, i64, i64, i64, i64, i64) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM semantic_commits WHERE persona_scope=?1 AND transition_kind='perception'),
           (SELECT COUNT(*) FROM semantic_commits WHERE persona_scope=?1 AND transition_kind='time'),
           (SELECT COUNT(*) FROM semantic_receipts WHERE persona_scope=?1),
           (SELECT COUNT(*) FROM semantic_telemetry WHERE persona_scope=?1),
           (SELECT COUNT(*) FROM semantic_evidence_authority WHERE persona_scope=?1),
           (SELECT COUNT(*) FROM semantic_time_authority WHERE persona_scope=?1)",
        params![blob(persona_scope)],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    if lane_counts.0 < 0
        || lane_counts.1 < 0
        || lane_counts.0 + lane_counts.1 != i64::try_from(expected_count).unwrap_or(-1)
        || lane_counts.2 != lane_counts.0
        || lane_counts.3 != lane_counts.0
        || lane_counts.4 != lane_counts.0
        || lane_counts.5 != lane_counts.1
    {
        return Err(StoreError::ContinuityFence("semantic_lane_sidecar_set"));
    }
    let lane_xor_mismatch: i64 = conn.query_row(
        "SELECT COUNT(*) FROM semantic_commits AS c WHERE c.persona_scope=?1 AND NOT (
           (c.transition_kind='perception'
             AND (SELECT COUNT(*) FROM semantic_snapshots WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1
             AND (SELECT COUNT(*) FROM semantic_graphs WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1
             AND (SELECT COUNT(*) FROM semantic_receipts WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1
             AND (SELECT COUNT(*) FROM semantic_telemetry WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1
             AND (SELECT COUNT(*) FROM semantic_evidence_authority WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1
             AND (SELECT COUNT(*) FROM semantic_time_authority WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=0)
           OR
           (c.transition_kind='time'
             AND (SELECT COUNT(*) FROM semantic_snapshots WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1
             AND (SELECT COUNT(*) FROM semantic_graphs WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=0
             AND (SELECT COUNT(*) FROM semantic_receipts WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=0
             AND (SELECT COUNT(*) FROM semantic_telemetry WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=0
             AND (SELECT COUNT(*) FROM semantic_evidence_authority WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=0
             AND (SELECT COUNT(*) FROM semantic_time_authority WHERE persona_scope=c.persona_scope AND semantic_revision=c.semantic_revision)=1)
         )",
        params![blob(persona_scope)],
        |row| row.get(0),
    )?;
    if sqlite_length(lane_xor_mismatch, "semantic_lane_xor.rows")? != 0 {
        return Err(StoreError::ContinuityFence("semantic_lane_sidecar_xor"));
    }
    // Relation-private evidence carries the exact same identity tuple as its
    // owning commit. This catches raw-database drift even when FK checking was
    // disabled by the corrupting connection.
    let relation_mismatch_raw: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM semantic_evidence_authority AS evidence
         LEFT JOIN semantic_commits AS commit_row ON
           commit_row.persona_scope=evidence.persona_scope AND
           commit_row.semantic_revision=evidence.semantic_revision
         WHERE evidence.persona_scope=?1 AND (
           commit_row.persona_scope IS NULL OR
           evidence.relation_present<>commit_row.relation_present OR
           evidence.relation_scope<>commit_row.relation_scope OR
           evidence.event_id<>commit_row.event_id OR
           evidence.incarnation_id<>commit_row.incarnation_id OR
           evidence.manifest_digest<>commit_row.manifest_digest OR
           evidence.route_digest<>commit_row.route_digest OR
           evidence.formula_digest<>commit_row.formula_digest OR
           evidence.graph_digest<>commit_row.graph_digest OR
           evidence.state_digest<>commit_row.state_digest OR
           evidence.evidence_digest<>commit_row.evidence_digest OR
           evidence.estimator_digest<>commit_row.estimator_digest OR
           evidence.event_digest<>commit_row.event_digest OR
           evidence.commitment_digest<>commit_row.commitment_digest)",
        params![blob(persona_scope)],
        |row| row.get(0),
    )?;
    if sqlite_length(relation_mismatch_raw, "semantic_relation_identity.rows")? != 0 {
        return Err(StoreError::ContinuityFence(
            "semantic_relation_identity_set",
        ));
    }

    let journal_mismatch_raw: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM semantic_commits AS commit_row
         LEFT JOIN journal AS journal_row ON
           journal_row.scope_digest=commit_row.persona_scope AND
           journal_row.logical_revision=commit_row.journal_revision AND
           journal_row.event_digest=commit_row.event_digest
         LEFT JOIN applied_events AS applied ON
           applied.scope_digest=commit_row.persona_scope AND
           applied.revision=commit_row.journal_revision AND
           applied.event_digest=commit_row.event_digest
         WHERE commit_row.persona_scope=?1 AND
           (journal_row.logical_revision IS NULL OR applied.revision IS NULL)",
        params![blob(persona_scope)],
        |row| row.get(0),
    )?;
    if sqlite_length(journal_mismatch_raw, "semantic_journal_identity.rows")? != 0 {
        return Err(StoreError::ContinuityFence("semantic_journal_identity_set"));
    }

    // Counts, extrema and FKs cannot prove payload integrity. Reconstruct and
    // re-attest every revision, then bind each predecessor to the immutable
    // origin or to the preceding semantic revision.
    let mut expected_state_before = origin.state_digest;
    let mut expected_graph_before = origin.graph_digest;
    let mut expected_revision = expected_min;
    let mut latest_commitment = None;
    let mut replay_identity: Option<ActiveSemanticIdentityV1> = None;
    let mut replay_field: Option<NeuralField> = None;
    let mut replay_graph: Option<SparseGraph> = None;
    let mut replay_anchor_field: Option<NeuralField> = None;
    let mut replay_epoch: Option<MatrixTimeEpochV1> = None;
    let mut replay_base = origin.source_revision;
    while expected_revision <= cursor_revision {
        #[cfg(test)]
        note_full_replay_row_v1();
        let stored = stored_commit(conn, persona_scope, expected_revision)?;
        let committed = read_semantic_commit(conn, persona_scope, expected_revision)?;

        let event = wire::decode_event(&committed.journal.event_bytes)
            .map_err(|_| StoreError::ContinuityFence("semantic_dynamics_replay"))?;
        let event_scope = match &event {
            CanonicalEvent::UserStimulus(stimulus) => &stimulus.scope,
            CanonicalEvent::TimeAdvance(time) => &time.scope,
            _ => return Err(StoreError::ContinuityFence("semantic_dynamics_replay")),
        };
        if replay_identity.is_none() {
            let identity = active_identity(
                conn,
                event_scope,
                committed.incarnation_id,
                committed.manifest_digest,
            )?;
            let (field, graph) = semantic_origin_state(conn, &origin, &identity)?;
            replay_anchor_field = Some(field.clone());
            replay_epoch = Some(MatrixTimeEpochV1 {
                schema_version: MatrixTimeEpochV1::SCHEMA_VERSION,
                anchor_semantic_revision: origin.source_revision,
                anchor_state_digest: origin.state_digest,
                awake_ticks: 0,
                drowsy_ticks: 0,
                asleep_ticks: 0,
                awake_remainder_ms: 0,
                drowsy_remainder_ms: 0,
                asleep_remainder_ms: 0,
            });
            replay_identity = Some(identity);
            replay_field = Some(field);
            replay_graph = Some(graph);
        }
        let identity = replay_identity
            .as_ref()
            .ok_or(StoreError::ContinuityFence("semantic_dynamics_replay"))?;
        if committed.incarnation_id != identity.incarnation_id
            || committed.manifest_digest != identity.manifest_digest
            || committed.formula_digest != origin.formula_digest
            || committed.route_digest != origin.route_digest
            || replay_base.checked_add(1) != Some(expected_revision)
        {
            return Err(StoreError::ContinuityFence("semantic_dynamics_replay"));
        }
        if committed.transition_kind == SemanticTransitionKindV1::Perception {
            if let Some((projected, graph)) = crate::embodiment_clock::semantic_proof_input(conn, &ae_contracts::PersonaScopeRef { bot_token: event_scope.bot_token, persona_token: event_scope.persona_token }, &committed)? {
                expected_state_before = state_digest(&projected, &committed.formula_digest);
                expected_graph_before = graph_digest(&graph);
                replay_field = Some(projected);
                replay_graph = Some(graph);
            }
        }
        if digest_from_vec(stored.state_before, "semantic_history.state_before")? != expected_state_before
            || digest_from_vec(stored.graph_before, "semantic_history.graph_before")? != expected_graph_before {
            return Err(StoreError::ContinuityFence("semantic_predecessor_chain"));
        }
        let field = replay_field
            .as_ref()
            .ok_or(StoreError::ContinuityFence("semantic_dynamics_replay"))?;
        let graph = replay_graph
            .as_ref()
            .ok_or(StoreError::ContinuityFence("semantic_dynamics_replay"))?;
        match committed.transition_kind {
            SemanticTransitionKindV1::Perception => {
                let stimulus = validated_stimulus(&event)
                    .map_err(|_| StoreError::ContinuityFence("semantic_dynamics_replay"))?;
                let relation_storage_scope = committed.relation_scope.unwrap_or(persona_scope);
                let canonical_nonce = canonical_semantic_nonce_v1(
                    &committed.journal.event_bytes,
                    &persona_scope,
                    &relation_storage_scope,
                    &committed.incarnation_id,
                    replay_base,
                );
                let proposal = PerceptionProposalV1 {
                    schema_version: PerceptionProposalV1::SCHEMA_VERSION,
                    origin_digest: committed.event_digest,
                    dimensions: stimulus.evidence.dimensions.clone(),
                    estimator_confidence: stimulus.evidence.estimator_confidence,
                    protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
                    request_nonce_digest: canonical_nonce,
                };
                let replayed = derive_user_stimulus_transition_v1(UserStimulusTransitionInputV1 {
                    field,
                    baseline: &identity.baseline_field,
                    graph,
                    manifest_digest: identity.manifest_digest,
                    development_seed_digest: identity.development_seed_digest,
                    proposal: &proposal,
                    formula_digest: committed.formula_digest,
                    scope_digest: persona_scope,
                    event_digest: committed.event_digest,
                    source_digest: committed
                        .evidence_digest
                        .ok_or(StoreError::ContinuityFence("semantic_perception_payload"))?,
                    authority_digest: ae_authority::authority_projection_digest(&event),
                    semantic_base_revision: replay_base,
                })
                .map_err(semantic_core_history_error)?;
                if replayed.route_digest != committed.route_digest
                    || replayed.state_before_digest != expected_state_before
                    || replayed.graph_before_digest != expected_graph_before
                    || replayed.state_after_digest != committed.state_digest
                    || replayed.graph_after_digest != committed.graph_digest
                    || replayed.snapshot_bytes != committed.snapshot_bytes
                    || Some(replayed.semantic_receipt_bytes) != committed.receipt_bytes
                    || Some(replayed.telemetry_bytes) != committed.telemetry_bytes
                {
                    return Err(StoreError::ContinuityFence("semantic_dynamics_replay"));
                }
                replay_anchor_field = Some(replayed.next_field.clone());
                replay_epoch = Some(MatrixTimeEpochV1 {
                    schema_version: MatrixTimeEpochV1::SCHEMA_VERSION,
                    anchor_semantic_revision: expected_revision,
                    anchor_state_digest: replayed.state_after_digest,
                    awake_ticks: 0,
                    drowsy_ticks: 0,
                    asleep_ticks: 0,
                    awake_remainder_ms: 0,
                    drowsy_remainder_ms: 0,
                    asleep_remainder_ms: 0,
                });
                replay_field = Some(replayed.next_field);
                replay_graph = Some(replayed.next_graph);
            }
            SemanticTransitionKindV1::Time => {
                let authority =
                    committed
                        .time_authority
                        .as_ref()
                        .ok_or(StoreError::ContinuityFence(
                            "semantic_time_authority_missing",
                        ))?;
                let epoch = replay_epoch
                    .as_ref()
                    .ok_or(StoreError::ContinuityFence("semantic_time_epoch_missing"))?;
                let anchor = replay_anchor_field
                    .as_ref()
                    .ok_or(StoreError::ContinuityFence("semantic_time_anchor_missing"))?;
                if &authority.epoch_before != epoch
                    || authority.semantic_base_revision != replay_base
                    || authority.state_before != expected_state_before
                    || authority.graph_digest != expected_graph_before
                {
                    return Err(StoreError::ContinuityFence("semantic_time_replay"));
                }
                let replayed = advance_matrix_time_v1(MatrixTimeInputV1 {
                    anchor_field: anchor,
                    genesis_baseline: &identity.baseline_field,
                    epoch,
                    elapsed_ms: authority.raw_elapsed_ms,
                    phase: authority.pre_sleep_phase,
                    semantic_formula_digest: committed.formula_digest,
                })
                .map_err(semantic_core_history_error)?;
                let snapshot = encode_time_snapshot_v1(&DecodedTimeSnapshotV1 {
                    semantic_formula_digest: committed.formula_digest,
                    time_formula_digest: authority.time_formula_digest,
                    epoch_before: authority.epoch_before.clone(),
                    epoch_after: authority.epoch_after.clone(),
                    requested_elapsed_ms: authority.raw_elapsed_ms,
                    applied_elapsed_ms: authority.applied_elapsed_ms,
                    capped_gap: authority.capped_gap,
                    pre_sleep_phase: authority.pre_sleep_phase,
                    state_before: authority.state_before,
                    state_after: authority.state_after,
                    graph_digest: authority.graph_digest,
                    authority_digest: ae_authority::authority_projection_digest(&event),
                    commitment_digest: committed.commitment_digest,
                })
                .map_err(semantic_core_history_error)?;
                if replayed.epoch != authority.epoch_after
                    || replayed.applied_elapsed_ms != authority.applied_elapsed_ms
                    || replayed.capped_gap != authority.capped_gap
                    || state_digest(&replayed.field, &committed.formula_digest)
                        != committed.state_digest
                    || committed.graph_digest != expected_graph_before
                    || snapshot != committed.snapshot_bytes
                    || committed.evidence_digest.is_some()
                    || committed.estimator_digest.is_some()
                    || committed.receipt_bytes.is_some()
                    || committed.telemetry_bytes.is_some()
                {
                    return Err(StoreError::ContinuityFence("semantic_time_replay"));
                }
                replay_epoch = Some(replayed.epoch);
                replay_field = Some(replayed.field);
            }
        }
        replay_base = expected_revision;
        expected_state_before = committed.state_digest;
        expected_graph_before = committed.graph_digest;
        latest_commitment = Some(committed.commitment_digest);
        expected_revision =
            expected_revision
                .checked_add(1)
                .ok_or(StoreError::RevisionOutOfRange {
                    revision: expected_revision,
                })?;
    }
    let (_, cursor_state, cursor_graph, cursor_commitment) = semantic_head(conn, persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_cursor_missing"))?;
    if cursor_state != expected_state_before
        || cursor_graph != expected_graph_before
        || latest_commitment != Some(cursor_commitment)
    {
        return Err(StoreError::ContinuityFence(
            "semantic_cursor_history_binding",
        ));
    }
    Ok(())
}

fn read_semantic_budget_checkpoint_v1(
    conn: &Connection,
    persona_scope: Digest,
) -> Result<Option<(i64, i64, i64, Digest)>, StoreError> {
    let stored: Option<(i64, i64, i64, Option<Vec<u8>>, i64)> = conn
        .query_row(
            "SELECT row_count,aggregate_bytes,head_revision,
                    CASE WHEN typeof(head_commitment_digest)='blob'
                               AND length(head_commitment_digest)=32
                         THEN head_commitment_digest END,
                    CASE WHEN typeof(head_commitment_digest)='blob'
                         THEN length(head_commitment_digest) ELSE -1 END
             FROM semantic_budget_checkpoint WHERE persona_scope=?1",
            params![blob(persona_scope)],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()?;
    stored
        .map(
            |(row_count, aggregate_bytes, head_revision, commitment, commitment_len)| {
                let commitment = stored_typed_digest(
                    commitment,
                    commitment_len,
                    "semantic_budget.commitment",
                    "semantic_budget_commitment_type",
                )?;
                Ok((row_count, aggregate_bytes, head_revision, commitment))
            },
        )
        .transpose()
}

fn verify_or_backfill_semantic_budget_checkpoints(
    conn: &Connection,
    allow_backfill: bool,
) -> Result<(), StoreError> {
    let mut last_scope = None;
    let mut global_budget = SemanticHistoryScanBudgetV1::default();
    while let Some(persona_scope) = next_semantic_persona_scope_v1(conn, last_scope)? {
        // This is deliberately the first admission for a discovered scope.
        // The first forbidden persona fails before any per-scope COUNT, length
        // walk, SUM-equivalent work, or semantic replay.
        global_budget.admit_persona()?;
        let raw_metadata: Option<(i64, i64, Option<Vec<u8>>, i64)> = conn
            .query_row(
                "SELECT origin.source_revision,cursor.semantic_revision,
                        CASE WHEN typeof(cursor.commitment_digest)='blob'
                                   AND length(cursor.commitment_digest)=32
                             THEN cursor.commitment_digest END,
                        CASE WHEN typeof(cursor.commitment_digest)='blob'
                             THEN length(cursor.commitment_digest) ELSE -1 END
                 FROM semantic_origins AS origin
                 JOIN semantic_cursor AS cursor USING(persona_scope)
                 WHERE origin.persona_scope=?1",
                params![blob(persona_scope)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .optional()?;
        let Some((raw_origin, raw_head, raw_commitment, raw_commitment_len)) = raw_metadata else {
            let checkpoint_exists: i64 = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM semantic_budget_checkpoint WHERE persona_scope=?1)",
                params![blob(persona_scope)],
                |row| row.get(0),
            )?;
            return Err(StoreError::ContinuityFence(if checkpoint_exists == 1 {
                "semantic_budget_orphan"
            } else {
                "semantic_root_closure"
            }));
        };
        let origin_revision = JournalRevision::try_from(raw_origin)?.get();
        let head_revision = semantic_revision_from_sql(raw_head)?;
        let row_count = head_revision
            .checked_sub(origin_revision)
            .ok_or(StoreError::ContinuityFence("semantic_budget_revision"))?;
        if row_count == 0 {
            return Err(StoreError::ContinuityFence("semantic_budget_revision"));
        }
        if raw_commitment_len < 0 {
            return Err(StoreError::ContinuityFence(
                "semantic_budget_commitment_type",
            ));
        }
        let commitment = digest_from_vec(
            raw_commitment.ok_or(StoreError::InvalidStoredDigest {
                field: "semantic_budget.commitment",
                actual: u64::try_from(raw_commitment_len).unwrap_or(u64::MAX),
            })?,
            "semantic_budget.commitment",
        )?;
        enforce_byte_budget(
            "semantic_budget.rows",
            row_count,
            MAX_SEMANTIC_ROWS_PER_PERSONA,
        )?;
        // Scan only integer lengths and one row marker at a time. Both local
        // and global gates are enforced before closure can read or replay any
        // variable-length payload.
        let scanned = scan_semantic_history_scope_v1(conn, persona_scope, &mut global_budget)?;
        if scanned.commit_rows != row_count {
            return Err(StoreError::ContinuityFence("semantic_budget_revision"));
        }
        let aggregate_bytes = scanned.aggregate_bytes;
        enforce_read_budget(
            "semantic_budget.rows",
            "semantic_budget.aggregate_bytes",
            row_count,
            aggregate_bytes,
            MAX_SEMANTIC_ROWS_PER_PERSONA,
            MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
        )?;
        // Preserve the public integrity-audit contract: once the bounded row
        // and byte preflights have admitted this persona, report a missing or
        // cross-lane sidecar as the causal closure violation rather than as a
        // derivative checkpoint mismatch. This remains a read-only walk.
        verify_semantic_history_closure(conn, persona_scope)?;
        if let Some((stored_count, stored_bytes, stored_head, stored_commitment)) =
            read_semantic_budget_checkpoint_v1(conn, persona_scope)?
        {
            if sqlite_length(stored_count, "semantic_budget.rows")? != row_count
                || sqlite_length(stored_bytes, "semantic_budget.aggregate_bytes")?
                    != aggregate_bytes
                || semantic_revision_from_sql(stored_head)? != head_revision
                || stored_commitment != commitment
            {
                return Err(StoreError::ContinuityFence("semantic_budget_checkpoint"));
            }
        } else {
            if !allow_backfill {
                return Err(StoreError::ContinuityFence("semantic_budget_missing"));
            }
            conn.execute(
                "INSERT INTO semantic_budget_checkpoint(
                    persona_scope,row_count,aggregate_bytes,head_revision,head_commitment_digest
                 ) VALUES(?1,?2,?3,?4,?5)",
                params![
                    blob(persona_scope),
                    JournalRevision::new(row_count).to_sqlite()?.get(),
                    JournalRevision::new(aggregate_bytes).to_sqlite()?.get(),
                    JournalRevision::new(head_revision).to_sqlite()?.get(),
                    blob(commitment),
                ],
            )?;
        }
        last_scope = Some(persona_scope);
    }
    Ok(())
}

fn verify_all_perception_challenges(conn: &Connection) -> Result<(), StoreError> {
    preflight_perception_challenges_v1(conn, PerceptionChallengeStorageLayoutV1::V3)?;
    require_positive_table_rowids_v1(
        conn,
        "semantic_evidence_authority",
        "perception_receipt_rowid_domain",
    )?;

    // Consumed authorizations are immutable history. Walk them by the table's
    // INTEGER rowid in bounded pages; no aggregate, sort temp table, or raw
    // fixed-width blob may precede its SQL type/length guard.
    let mut receipt_count = 0_u64;
    let mut receipt_bytes = 0_u64;
    let mut cursor = 0_i64;
    loop {
        let mut receipts = conn.prepare(
            "SELECT rowid,
                    CASE WHEN typeof(persona_scope)='blob' AND length(persona_scope)=32
                         THEN persona_scope END,
                    CASE WHEN typeof(semantic_revision)='integer' THEN semantic_revision END,
                    CASE WHEN typeof(perception_nonce_digest)='blob'
                              AND length(perception_nonce_digest)=32
                         THEN perception_nonce_digest END,
                    CASE WHEN typeof(perception_proposal_digest)='blob'
                              AND length(perception_proposal_digest)=32
                         THEN perception_proposal_digest END,
                    CASE WHEN typeof(perception_origin_digest)='blob'
                              AND length(perception_origin_digest)=32
                         THEN perception_origin_digest END,
                    CASE WHEN typeof(perception_origin_bytes)='blob'
                              AND length(perception_origin_bytes)<=?3
                         THEN perception_origin_bytes END,
                    CASE WHEN typeof(perception_origin_bytes)='blob'
                         THEN length(perception_origin_bytes) ELSE -1 END
             FROM semantic_evidence_authority NOT INDEXED
             WHERE rowid>?1 AND perception_nonce_digest IS NOT NULL
             ORDER BY rowid LIMIT ?2",
        )?;
        let mut receipt_rows = receipts.query(params![
            cursor,
            i64::try_from(PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1).unwrap_or(128),
            MAX_PERCEPTION_ORIGIN_BYTES,
        ])?;
        let mut visited = 0_u64;
        let mut last_rowid = None;
        while let Some(row) = receipt_rows.next()? {
            receipt_count =
                receipt_count
                    .checked_add(1)
                    .ok_or(StoreError::StorageBudgetExceeded {
                        resource: "perception_receipt.rows",
                        limit: MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
                        actual: u64::MAX,
                    })?;
            enforce_byte_budget(
                "perception_receipt.rows",
                receipt_count,
                MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
            )?;
            let rowid: i64 = row.get(0)?;
            if rowid <= cursor {
                return Err(StoreError::ContinuityFence("perception_receipt_rows"));
            }
            let persona = required_materialized_digest_v1(
                row.get(1)?,
                "perception_receipt.persona",
                "perception_receipt_persona_type",
            )?;
            let revision = semantic_revision_from_sql(row.get::<_, Option<i64>>(2)?.ok_or(
                StoreError::ContinuityFence("perception_receipt_revision_type"),
            )?)?;
            let nonce = required_materialized_digest_v1(
                row.get(3)?,
                "perception_receipt.nonce",
                "perception_receipt_nonce_type",
            )?;
            let proposal = required_materialized_digest_v1(
                row.get(4)?,
                "perception_receipt.proposal",
                "perception_receipt_proposal_type",
            )?;
            let origin_digest = required_materialized_digest_v1(
                row.get(5)?,
                "perception_receipt.origin",
                "perception_receipt_origin_type",
            )?;
            let origin_bytes = bounded_typed_value(
                row.get::<_, Option<Vec<u8>>>(6)?,
                row.get(7)?,
                MAX_PERCEPTION_ORIGIN_BYTES,
                "perception_receipt.origin_bytes",
                "perception_receipt_origin",
            )?;
            receipt_bytes = receipt_bytes
                .checked_add(u64::try_from(origin_bytes.len()).unwrap_or(u64::MAX))
                .ok_or(StoreError::StorageBudgetExceeded {
                    resource: "perception_receipt.aggregate_bytes",
                    limit: MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL,
                    actual: u64::MAX,
                })?;
            enforce_byte_budget(
                "perception_receipt.aggregate_bytes",
                receipt_bytes,
                MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL,
            )?;
            let origin: PerceptionOriginCommitmentV1 = serde_json::from_slice(&origin_bytes)
                .map_err(|_| StoreError::ContinuityFence("perception_receipt_origin"))?;
            let canonical_origin = serde_json::to_vec(&origin)
                .map_err(|_| StoreError::ContinuityFence("perception_receipt_origin"))?;
            let authoritative =
                committed_perception_origin_v1(conn, origin.origin_event_digest, Some(persona))?
                    .origin;
            let authoritative_bytes = serde_json::to_vec(&authoritative)
                .map_err(|_| StoreError::ContinuityFence("perception_receipt_origin"))?;
            let committed = read_semantic_commit(conn, persona, revision)?;
            let event = wire::decode_event(&committed.journal.event_bytes)
                .map_err(|_| StoreError::ContinuityFence("perception_receipt_event"))?;
            let stimulus = validated_stimulus(&event)
                .map_err(|_| StoreError::ContinuityFence("perception_receipt_event"))?;
            if !is_nonzero(&nonce)
                || !(origin.validate_v1() || origin.validate_core_v1())
                || canonical_origin != origin_bytes
                || origin != authoritative
                || canonical_origin != authoritative_bytes
                || origin.origin_digest != origin_digest
                || committed.estimator_digest != Some(proposal)
                || committed.event_id != origin.event_id
                || stimulus.scope != origin.scope
                || stimulus.causal.turn_id != origin.turn_id
                || stimulus.causal.base_revision != committed.journal.base_revision
                || stimulus.causal.base_revision < origin.canonical_base_revision
                || stimulus.observed_at_ms != origin.observed_at_ms
                || committed.incarnation_id != origin.incarnation_id
                || committed.manifest_digest != origin.manifest_digest
            {
                return Err(StoreError::ContinuityFence("perception_receipt_binding"));
            }
            visited += 1;
            last_rowid = Some(rowid);
        }
        drop(receipt_rows);
        drop(receipts);
        if visited < PERCEPTION_CHALLENGE_SCAN_BATCH_ROWS_V1 {
            break;
        }
        cursor = last_rowid.ok_or(StoreError::ContinuityFence("perception_receipt_rows"))?;
    }
    Ok(())
}

fn verify_semantic_cursor_epoch_binding_v1(
    conn: &Connection,
    persona_scope: Digest,
    revision: u64,
    state: Digest,
    graph: Digest,
) -> Result<(), StoreError> {
    let (kind, kind_len): (Option<String>, i64) = conn.query_row(
        "SELECT CASE WHEN typeof(transition_kind)='text' AND length(CAST(transition_kind AS BLOB))<=10
                     THEN transition_kind END,
                CASE WHEN typeof(transition_kind)='text' THEN length(CAST(transition_kind AS BLOB)) ELSE -1 END
         FROM semantic_cursor WHERE persona_scope=?1",
        params![blob(persona_scope)],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let kind = bounded_typed_value(
        kind,
        kind_len,
        10,
        "semantic_cursor.transition_kind",
        "semantic_cursor_transition_kind_type",
    )?;
    let kind = SemanticTransitionKindV1::from_str(&kind)
        .ok_or(StoreError::ContinuityFence("semantic_transition_kind"))?;
    let (_, epoch) = semantic_cursor_epoch_v1(conn, persona_scope, revision, state)?;
    let time_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM semantic_time_authority
         WHERE persona_scope=?1 AND semantic_revision=?2",
        params![
            blob(persona_scope),
            JournalRevision::new(revision).to_sqlite()?.get()
        ],
        |row| row.get(0),
    )?;
    match kind {
        SemanticTransitionKindV1::Perception => {
            if time_rows != 0
                || epoch.anchor_semantic_revision != revision
                || epoch.anchor_state_digest != state
                || epoch.awake_ticks != 0
                || epoch.drowsy_ticks != 0
                || epoch.asleep_ticks != 0
                || epoch.awake_remainder_ms != 0
                || epoch.drowsy_remainder_ms != 0
                || epoch.asleep_remainder_ms != 0
            {
                return Err(StoreError::ContinuityFence(
                    "semantic_perception_epoch_reset",
                ));
            }
        }
        SemanticTransitionKindV1::Time => {
            if time_rows != 1 {
                return Err(StoreError::ContinuityFence(
                    "semantic_time_authority_missing",
                ));
            }
            type RawAuthority = (Option<Vec<u8>>, i64, Option<Vec<u8>>);
            let raw: RawAuthority = conn.query_row(
                "SELECT
                   CASE WHEN typeof(authority_bytes)='blob' AND length(authority_bytes)<=?3 THEN authority_bytes END,
                   CASE WHEN typeof(authority_bytes)='blob' THEN length(authority_bytes) ELSE -1 END,
                   CASE WHEN typeof(authority_digest)='blob' AND length(authority_digest)=32
                        THEN authority_digest END
                  FROM semantic_time_authority WHERE persona_scope=?1 AND semantic_revision=?2",
                params![
                    blob(persona_scope),
                    JournalRevision::new(revision).to_sqlite()?.get(),
                    16_384_i64,
                ],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
            let bytes = bounded_typed_value(
                raw.0,
                raw.1,
                16_384,
                "semantic_time.authority_bytes",
                "semantic_time_authority_type",
            )?;
            let authority: SemanticTimeAuthorityV1 = serde_json::from_slice(&bytes)
                .map_err(|_| StoreError::ContinuityFence("semantic_time_authority_wire"))?;
            let canonical = serde_json::to_vec(&authority)
                .map_err(|_| StoreError::ContinuityFence("semantic_time_authority_wire"))?;
            if canonical != bytes
                || !authority.validate_v1()
                || digest_from_vec(
                    raw.2.ok_or(StoreError::ContinuityFence(
                        "semantic_time_authority_digest_type",
                    ))?,
                    "semantic_time.authority",
                )? != wire::domain_hash(b"astr-embodiment/semantic-time-authority-v1", &[&bytes])
                || authority.persona_scope != persona_scope
                || authority.semantic_revision != revision
                || authority.epoch_after != epoch
                || authority.state_after != state
                || authority.graph_digest != graph
                || epoch.anchor_semantic_revision >= revision
            {
                return Err(StoreError::ContinuityFence(
                    "semantic_time_cursor_authority",
                ));
            }
            let origin = stored_origin(conn, persona_scope)?
                .ok_or(StoreError::ContinuityFence("semantic_origin_missing"))?;
            if epoch.anchor_semantic_revision == origin.source_revision {
                if epoch.anchor_state_digest != origin.state_digest || graph != origin.graph_digest
                {
                    return Err(StoreError::ContinuityFence("semantic_time_anchor"));
                }
            } else {
                let anchor_closes: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM semantic_commits
                     WHERE persona_scope=?1 AND semantic_revision=?2
                       AND transition_kind='perception' AND state_digest=?3 AND graph_digest=?4",
                    params![
                        blob(persona_scope),
                        JournalRevision::new(epoch.anchor_semantic_revision)
                            .to_sqlite()?
                            .get(),
                        blob(epoch.anchor_state_digest),
                        blob(graph),
                    ],
                    |row| row.get(0),
                )?;
                if anchor_closes != 1 {
                    return Err(StoreError::ContinuityFence("semantic_time_anchor"));
                }
            }
        }
    }
    Ok(())
}

fn semantic_head(
    conn: &Connection,
    persona_scope: Digest,
) -> Result<Option<(u64, Digest, Digest, Digest)>, StoreError> {
    let row: Option<(i64, Vec<u8>, Vec<u8>, Vec<u8>, i64, i64)> = conn
        .query_row(
            "SELECT cursor.semantic_revision,
                CASE WHEN typeof(cursor.state_digest)='blob' AND length(cursor.state_digest)=32 THEN cursor.state_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(cursor.graph_digest)='blob' AND length(cursor.graph_digest)=32 THEN cursor.graph_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(cursor.commitment_digest)='blob' AND length(cursor.commitment_digest)=32 THEN cursor.commitment_digest ELSE zeroblob(0) END,
                commit_row.persona_scope IS NOT NULL,
                checkpoint.persona_scope IS NOT NULL AND
                  checkpoint.head_revision=cursor.semantic_revision AND
                  checkpoint.head_commitment_digest=cursor.commitment_digest
             FROM semantic_cursor AS cursor
             LEFT JOIN semantic_commits AS commit_row ON
                commit_row.persona_scope=cursor.persona_scope AND
                commit_row.semantic_revision=cursor.semantic_revision AND
                commit_row.transition_kind=cursor.transition_kind AND
                commit_row.journal_revision=cursor.journal_revision AND
                commit_row.incarnation_id=cursor.incarnation_id AND
                commit_row.manifest_digest=cursor.manifest_digest AND
                commit_row.route_digest=cursor.route_digest AND
                commit_row.formula_digest=cursor.formula_digest AND
                commit_row.graph_digest=cursor.graph_digest AND
                commit_row.state_digest=cursor.state_digest AND
                commit_row.evidence_digest=cursor.evidence_digest AND
                commit_row.estimator_digest=cursor.estimator_digest AND
                commit_row.event_digest=cursor.event_digest AND
                commit_row.commitment_digest=cursor.commitment_digest
             LEFT JOIN semantic_budget_checkpoint AS checkpoint ON
                checkpoint.persona_scope=cursor.persona_scope
             WHERE cursor.persona_scope=?1",
            params![blob(persona_scope)],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((revision, state, graph, commitment, commit_closes, checkpoint_closes)) = row else {
        let stranded: i64 = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM semantic_budget_checkpoint WHERE persona_scope=?1)
                 OR EXISTS(SELECT 1 FROM semantic_origins WHERE persona_scope=?1)",
            params![blob(persona_scope)],
            |row| row.get(0),
        )?;
        if stranded != 0 {
            return Err(StoreError::ContinuityFence("semantic_cursor_missing"));
        }
        return Ok(None);
    };
    if commit_closes != 1 {
        return Err(StoreError::ContinuityFence("semantic_cursor_binding"));
    }
    if checkpoint_closes != 1 {
        return Err(StoreError::ContinuityFence(
            "semantic_cursor_not_canonical_head",
        ));
    }
    let revision = semantic_revision_from_sql(revision)?;
    let state = digest_from_vec(state, "semantic_cursor.state")?;
    let graph = digest_from_vec(graph, "semantic_cursor.graph")?;
    let commitment = digest_from_vec(commitment, "semantic_cursor.commitment")?;
    verify_semantic_cursor_epoch_binding_v1(conn, persona_scope, revision, state, graph)?;
    Ok(Some((revision, state, graph, commitment)))
}

fn semantic_head_tx(
    tx: &Transaction<'_>,
    persona_scope: Digest,
) -> Result<Option<(u64, Digest, Digest, Digest)>, StoreError> {
    semantic_head(tx, persona_scope)
}

fn derive_candidate_tx(
    tx: &Transaction<'_>,
    candidate: &PairedSemanticCommitV1,
) -> Result<(PairedSemanticCommitV1, DerivedSemanticCommitV1), StoreError> {
    enforce_byte_budget(
        "semantic.snapshot_bytes",
        u64::try_from(candidate.snapshot_bytes.len()).unwrap_or(u64::MAX),
        MAX_SNAPSHOT_STATE_BYTES,
    )?;
    enforce_byte_budget(
        "semantic.receipt_bytes",
        u64::try_from(candidate.receipt_bytes.len()).unwrap_or(u64::MAX),
        MAX_SEMANTIC_RECEIPT_BYTES,
    )?;
    enforce_byte_budget(
        "semantic.telemetry_bytes",
        u64::try_from(candidate.telemetry_bytes.len()).unwrap_or(u64::MAX),
        MAX_SEMANTIC_TELEMETRY_BYTES,
    )?;
    enforce_byte_budget(
        "semantic.event_bytes",
        u64::try_from(candidate.journal.event_bytes.len()).unwrap_or(u64::MAX),
        MAX_JOURNAL_EVENT_BYTES,
    )?;
    for (field, digest) in [
        ("incarnation_id", candidate.incarnation_id),
        ("manifest_digest", candidate.manifest_digest),
    ] {
        if !is_nonzero(&digest) {
            return Err(StoreError::SemanticInvalid(field));
        }
    }

    let event = wire::decode_event(&candidate.journal.event_bytes)
        .map_err(|_| StoreError::SemanticInvalid("canonical_event_wire"))?;
    if wire::encode_event_checked(&event)
        .map_err(|_| StoreError::SemanticInvalid("canonical_event_wire"))?
        != candidate.journal.event_bytes
        || !candidate.journal.delta_bytes.is_empty()
    {
        return Err(StoreError::SemanticInvalid("canonical_event_envelope"));
    }
    let stimulus = validated_stimulus(&event)?;
    let persona_scope = wire::persona_scope_digest(
        &stimulus.scope.bot_token,
        &stimulus.scope.persona_token,
        None,
    );
    let relation_scope = stimulus.scope.relation_token.as_ref().map(|relation| {
        wire::persona_scope_digest(
            &stimulus.scope.bot_token,
            &stimulus.scope.persona_token,
            Some(relation),
        )
    });
    let relation_present = relation_scope.is_some();
    let relation_storage_scope = relation_scope.unwrap_or(persona_scope);
    let event_digest = wire::event_digest(&event);
    let evidence_digest = semantic_evidence_digest_v1(&event)?;
    let authority_digest = ae_authority::authority_projection_digest(&event);

    let identity = active_identity(
        tx,
        &stimulus.scope,
        candidate.incarnation_id,
        candidate.manifest_digest,
    )?;
    let expected_route = phase0_semantic_route_digest_v1();
    let expected_formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    if stimulus.causal.base_revision == 0
        && candidate.journal.chain_seed != identity.initial_snapshot_digest
    {
        return Err(StoreError::SemanticInvalid("genesis_chain_seed"));
    }

    let claims = derive_canonical_semantic_sidecar_v1(
        &candidate.snapshot_bytes,
        &candidate.receipt_bytes,
        &candidate.telemetry_bytes,
    )?;
    let semantic_revision =
        candidate
            .semantic_base_revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOutOfRange {
                revision: candidate.semantic_base_revision,
            })?;
    JournalRevision::new(semantic_revision).to_sqlite()?;
    let nonzero_evidence = u8::try_from(
        evidence_values(&stimulus.evidence.dimensions)
            .iter()
            .filter(|value| **value != Fixed::ZERO)
            .count(),
    )
    .map_err(|_| StoreError::SemanticInvalid("evidence_dimension_count"))?;
    if claims.formula_digest != expected_formula
        || claims.scope_digest != persona_scope
        || claims.event_digest != event_digest
        || claims.authority_digest != authority_digest
        || claims.source_digest != evidence_digest
        || claims.base_revision != candidate.semantic_base_revision
        || claims.next_revision != semantic_revision
        || claims.nonzero_evidence_dimension_count != nonzero_evidence
    {
        return Err(StoreError::SemanticInvalid("semantic_sidecar_identity"));
    }

    // The low-level candidate is never production authority.  Rebuild every
    // event/scope/revision field from the canonical event and strict sidecars
    // so test-hook callers cannot introduce a second identity namespace.
    let journal_next =
        stimulus
            .causal
            .base_revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOutOfRange {
                revision: stimulus.causal.base_revision,
            })?;
    JournalRevision::new(journal_next).to_sqlite()?;
    let mut normalized = candidate.clone();
    normalized.persona_scope = persona_scope;
    normalized.relation_scope = relation_scope;
    normalized.event_id = stimulus.event_id;
    normalized.event_digest = event_digest;
    normalized.evidence_digest = evidence_digest;
    normalized.estimator_digest = stimulus.evidence.estimator_digest;
    normalized.route_digest = expected_route;
    normalized.formula_digest = expected_formula;
    normalized.state_digest = claims.state_after;
    normalized.graph_digest = claims.graph_after;
    normalized.journal.event_kind = wire::event_kind_name(&event).to_owned();
    normalized.journal.delta_bytes.clear();
    normalized.journal.receipt.schema_version = 1;
    normalized.journal.receipt.formula_digest = expected_formula;
    normalized.journal.receipt.scope_digest = persona_scope;
    normalized.journal.receipt.event_digest = event_digest;
    normalized.journal.receipt.authority_digest = authority_digest;
    normalized.journal.receipt.base_revision = stimulus.causal.base_revision;
    normalized.journal.receipt.next_revision = journal_next;
    normalized.journal.receipt.state_before = claims.state_before;
    normalized.journal.receipt.state_after = claims.state_after;
    normalized.journal.receipt.graph_after = claims.graph_after;
    normalized.journal.receipt.action_contract = None;
    normalized.journal.receipt.active_nodes = claims.active_nodes;
    normalized.journal.receipt.active_edges = claims.active_edges;
    normalized.journal.receipt.residuals = claims.residuals.clone();
    normalized.journal.receipt.status = CommitStatus::Committed;

    let origin = derive_origin_tx(
        tx,
        &stimulus.scope,
        persona_scope,
        normalized.incarnation_id,
        normalized.manifest_digest,
        expected_route,
        expected_formula,
        &identity,
    )?;
    // The candidate's predecessor is authenticated by its paired receipt and
    // telemetry here.  Cursor comparison happens only after the relation-keyed
    // exact-retry lookup, so an exact retry wins before its now-stale CAS.
    let state_before = claims.state_before;
    let graph_before = claims.graph_before;

    let blocks = decode_canonical_aesem3_blocks(&candidate.snapshot_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_aesem3_wire"))?;
    let graph_bytes = blocks.graph.to_vec();
    enforce_byte_budget(
        "semantic.graph_bytes",
        u64::try_from(graph_bytes.len()).unwrap_or(u64::MAX),
        MAX_SEMANTIC_GRAPH_BYTES,
    )?;
    let snapshot_wire_digest = wire::domain_hash(
        SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1,
        &[&candidate.snapshot_bytes],
    );
    let graph_wire_digest = wire::domain_hash(SEMANTIC_GRAPH_WIRE_DOMAIN_V1, &[&graph_bytes]);
    let receipt_wire_digest =
        wire::domain_hash(SEMANTIC_RECEIPT_WIRE_DOMAIN_V1, &[&candidate.receipt_bytes]);
    let telemetry_wire_digest = wire::domain_hash(
        SEMANTIC_TELEMETRY_WIRE_DOMAIN_V1,
        &[&candidate.telemetry_bytes],
    );
    let commitment_digest = commit_digest(
        &normalized,
        relation_present,
        &relation_storage_scope,
        normalized.journal.receipt.next_revision,
        semantic_revision,
        &state_before,
        &graph_before,
        &authority_digest,
        &claims.telemetry_digest,
        &snapshot_wire_digest,
        &graph_wire_digest,
        &receipt_wire_digest,
        &telemetry_wire_digest,
        &origin.origin_digest,
    );
    Ok((
        normalized,
        DerivedSemanticCommitV1 {
            relation_present,
            relation_storage_scope,
            semantic_revision,
            state_before,
            graph_before,
            authority_digest,
            telemetry_digest: claims.telemetry_digest,
            snapshot_wire_digest,
            graph_wire_digest,
            receipt_wire_digest,
            telemetry_wire_digest,
            graph_bytes,
            commitment_digest,
            origin,
        },
    ))
}

fn stored_commit(
    conn: &Connection,
    persona_scope: Digest,
    revision: u64,
) -> Result<StoredCommitColumns, StoreError> {
    let revision_sql = JournalRevision::new(revision).to_sqlite()?.get();
    conn.query_row(
        "SELECT
            CASE WHEN typeof(semantic_revision)='integer' THEN semantic_revision ELSE -1 END,
            CASE WHEN typeof(journal_revision)='integer' THEN journal_revision ELSE -1 END,
            CASE WHEN typeof(relation_present)='integer' THEN relation_present ELSE -1 END,
            CASE WHEN typeof(relation_scope)='blob' AND length(relation_scope)=32 THEN relation_scope ELSE zeroblob(0) END,
            CASE WHEN typeof(event_id)='blob' AND length(event_id)=16 THEN event_id ELSE zeroblob(0) END,
            CASE WHEN typeof(event_digest)='blob' AND length(event_digest)=32 THEN event_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(evidence_digest)='blob' AND length(evidence_digest)=32 THEN evidence_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(estimator_digest)='blob' AND length(estimator_digest)=32 THEN estimator_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(incarnation_id)='blob' AND length(incarnation_id)=32 THEN incarnation_id ELSE zeroblob(0) END,
            CASE WHEN typeof(manifest_digest)='blob' AND length(manifest_digest)=32 THEN manifest_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(route_digest)='blob' AND length(route_digest)=32 THEN route_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(formula_digest)='blob' AND length(formula_digest)=32 THEN formula_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(state_before)='blob' AND length(state_before)=32 THEN state_before ELSE zeroblob(0) END,
            CASE WHEN typeof(state_digest)='blob' AND length(state_digest)=32 THEN state_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(graph_before)='blob' AND length(graph_before)=32 THEN graph_before ELSE zeroblob(0) END,
            CASE WHEN typeof(graph_digest)='blob' AND length(graph_digest)=32 THEN graph_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(authority_digest)='blob' AND length(authority_digest)=32 THEN authority_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(telemetry_digest)='blob' AND length(telemetry_digest)=32 THEN telemetry_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(snapshot_wire_digest)='blob' AND length(snapshot_wire_digest)=32 THEN snapshot_wire_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(graph_wire_digest)='blob' AND length(graph_wire_digest)=32 THEN graph_wire_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(receipt_wire_digest)='blob' AND length(receipt_wire_digest)=32 THEN receipt_wire_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(telemetry_wire_digest)='blob' AND length(telemetry_wire_digest)=32 THEN telemetry_wire_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(origin_digest)='blob' AND length(origin_digest)=32 THEN origin_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(commitment_digest)='blob' AND length(commitment_digest)=32 THEN commitment_digest ELSE zeroblob(0) END,
            CASE WHEN typeof(transition_kind)='text' AND transition_kind IN ('perception','time') THEN transition_kind ELSE '' END
         FROM semantic_commits WHERE persona_scope=?1 AND semantic_revision=?2",
        params![blob(persona_scope), revision_sql],
        |row| {
            Ok(StoredCommitColumns {
                semantic_revision: row.get(0)?,
                journal_revision: row.get(1)?,
                relation_present: row.get(2)?,
                relation_scope: row.get(3)?,
                event_id: row.get(4)?,
                event_digest: row.get(5)?,
                evidence_digest: row.get(6)?,
                estimator_digest: row.get(7)?,
                incarnation_id: row.get(8)?,
                manifest_digest: row.get(9)?,
                route_digest: row.get(10)?,
                formula_digest: row.get(11)?,
                state_before: row.get(12)?,
                state_digest: row.get(13)?,
                graph_before: row.get(14)?,
                graph_digest: row.get(15)?,
                authority_digest: row.get(16)?,
                telemetry_digest: row.get(17)?,
                snapshot_wire_digest: row.get(18)?,
                graph_wire_digest: row.get(19)?,
                receipt_wire_digest: row.get(20)?,
                telemetry_wire_digest: row.get(21)?,
                origin_digest: row.get(22)?,
                commitment_digest: row.get(23)?,
                transition_kind: row.get(24)?,
            })
        },
    )
    .optional()?
    .ok_or(StoreError::SemanticNotFound)
}

fn bounded_payload(
    conn: &Connection,
    persona_scope: Digest,
    revision: u64,
    sql: &str,
    resource: &'static str,
    limit: u64,
) -> Result<Vec<u8>, StoreError> {
    let revision_sql = JournalRevision::new(revision).to_sqlite()?.get();
    let limit_sql = i64::try_from(limit).map_err(|_| StoreError::StorageBudgetExceeded {
        resource,
        limit,
        actual: u64::MAX,
    })?;
    let row: Option<(Option<Vec<u8>>, i64)> = conn
        .query_row(
            sql,
            params![blob(persona_scope), revision_sql, limit_sql],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((bounded, raw_len)) = row else {
        return Err(StoreError::ContinuityFence("semantic_sidecar_missing"));
    };
    let raw_len = sqlite_length(raw_len, resource)?;
    enforce_byte_budget(resource, raw_len, limit)?;
    let bytes = bounded.ok_or(StoreError::ContinuityFence("semantic_sidecar_type"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != raw_len {
        return Err(StoreError::ContinuityFence("semantic_sidecar_length"));
    }
    Ok(bytes)
}

fn time_commitment_digest_v1(
    persona_scope: &Digest,
    semantic_revision: u64,
    journal_revision: u64,
    event_id: &Id128,
    event_digest: &Digest,
    incarnation_id: &Digest,
    manifest_digest: &Digest,
    route_digest: &Digest,
    formula_digest: &Digest,
    state_before: &Digest,
    state_after: &Digest,
    graph_digest: &Digest,
    event_authority_digest: &Digest,
    origin_digest: &Digest,
    time_authority_digest: &Digest,
) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/semantic-time-commit-v1",
        &[
            persona_scope,
            &semantic_revision.to_le_bytes(),
            &journal_revision.to_le_bytes(),
            event_id,
            event_digest,
            incarnation_id,
            manifest_digest,
            route_digest,
            formula_digest,
            state_before,
            state_after,
            graph_digest,
            event_authority_digest,
            origin_digest,
            time_authority_digest,
        ],
    )
}

/// Commitment used by prerelease semantic schema V4, where every time row
/// duplicated a full field and graph. It remains solely as a migration
/// verifier: V5 writes cannot call it or produce the legacy representation.
fn legacy_time_commitment_digest_v4(
    persona_scope: &Digest,
    semantic_revision: u64,
    journal_revision: u64,
    event_id: &Id128,
    event_digest: &Digest,
    incarnation_id: &Digest,
    manifest_digest: &Digest,
    route_digest: &Digest,
    formula_digest: &Digest,
    state_before: &Digest,
    state_after: &Digest,
    graph_digest: &Digest,
    event_authority_digest: &Digest,
    snapshot_wire_digest: &Digest,
    graph_wire_digest: &Digest,
    origin_digest: &Digest,
    time_authority_digest: &Digest,
) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/semantic-time-commit-v1",
        &[
            persona_scope,
            &semantic_revision.to_le_bytes(),
            &journal_revision.to_le_bytes(),
            event_id,
            event_digest,
            incarnation_id,
            manifest_digest,
            route_digest,
            formula_digest,
            state_before,
            state_after,
            graph_digest,
            event_authority_digest,
            snapshot_wire_digest,
            graph_wire_digest,
            origin_digest,
            time_authority_digest,
        ],
    )
}

fn read_time_semantic_commit(
    conn: &Connection,
    persona_scope: Digest,
    revision: u64,
    stored: StoredCommitColumns,
) -> Result<CommittedSemanticV1, StoreError> {
    let semantic_revision = semantic_revision_from_sql(stored.semantic_revision)?;
    let journal_revision = semantic_revision_from_sql(stored.journal_revision)?;
    if semantic_revision != revision || stored.relation_present != 0 {
        return Err(StoreError::ContinuityFence("semantic_time_identity"));
    }
    let relation_storage = digest_from_vec(stored.relation_scope, "semantic_time.relation")?;
    let event_id = id_from_vec(stored.event_id, "semantic_time.event_id")?;
    let event_digest = digest_from_vec(stored.event_digest, "semantic_time.event")?;
    let evidence_digest = digest_from_vec(stored.evidence_digest, "semantic_time.evidence")?;
    let estimator_digest = digest_from_vec(stored.estimator_digest, "semantic_time.estimator")?;
    let incarnation_id = digest_from_vec(stored.incarnation_id, "semantic_time.incarnation")?;
    let manifest_digest = digest_from_vec(stored.manifest_digest, "semantic_time.manifest")?;
    let route_digest = digest_from_vec(stored.route_digest, "semantic_time.route")?;
    let formula_digest = digest_from_vec(stored.formula_digest, "semantic_time.formula")?;
    let state_before = digest_from_vec(stored.state_before, "semantic_time.state_before")?;
    let state_after = digest_from_vec(stored.state_digest, "semantic_time.state")?;
    let graph_before = digest_from_vec(stored.graph_before, "semantic_time.graph_before")?;
    let graph_after = digest_from_vec(stored.graph_digest, "semantic_time.graph")?;
    let event_authority =
        digest_from_vec(stored.authority_digest, "semantic_time.event_authority")?;
    let telemetry_digest = digest_from_vec(stored.telemetry_digest, "semantic_time.telemetry")?;
    let snapshot_wire_digest =
        digest_from_vec(stored.snapshot_wire_digest, "semantic_time.snapshot_wire")?;
    let graph_wire_digest = digest_from_vec(stored.graph_wire_digest, "semantic_time.graph_wire")?;
    let receipt_wire_digest =
        digest_from_vec(stored.receipt_wire_digest, "semantic_time.receipt_wire")?;
    let telemetry_wire_digest =
        digest_from_vec(stored.telemetry_wire_digest, "semantic_time.telemetry_wire")?;
    let stored_origin_digest = digest_from_vec(stored.origin_digest, "semantic_time.origin")?;
    let commitment_digest = digest_from_vec(stored.commitment_digest, "semantic_time.commitment")?;
    if relation_storage != persona_scope
        || evidence_digest != [0; 32]
        || estimator_digest != [0; 32]
        || telemetry_digest != [0; 32]
        || receipt_wire_digest != [0; 32]
        || telemetry_wire_digest != [0; 32]
        || graph_wire_digest != [0; 32]
        || graph_before != graph_after
    {
        return Err(StoreError::ContinuityFence("semantic_time_lane_xor"));
    }
    let sidecar_counts: (i64, i64, i64, i64, i64, i64) = conn.query_row(
        "SELECT
           (SELECT COUNT(*) FROM semantic_snapshots WHERE persona_scope=?1 AND semantic_revision=?2),
           (SELECT COUNT(*) FROM semantic_graphs WHERE persona_scope=?1 AND semantic_revision=?2),
           (SELECT COUNT(*) FROM semantic_receipts WHERE persona_scope=?1 AND semantic_revision=?2),
           (SELECT COUNT(*) FROM semantic_telemetry WHERE persona_scope=?1 AND semantic_revision=?2),
           (SELECT COUNT(*) FROM semantic_evidence_authority WHERE persona_scope=?1 AND semantic_revision=?2),
           (SELECT COUNT(*) FROM semantic_time_authority WHERE persona_scope=?1 AND semantic_revision=?2)",
        params![blob(persona_scope), JournalRevision::new(revision).to_sqlite()?.get()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    if sidecar_counts != (1, 0, 0, 0, 0, 1) {
        return Err(StoreError::ContinuityFence("semantic_time_sidecar_closure"));
    }

    let snapshot_bytes = bounded_payload(
        conn,
        persona_scope,
        revision,
        "SELECT CASE WHEN typeof(snapshot_bytes)='blob' AND length(snapshot_bytes)<=?3 THEN snapshot_bytes END,
                CASE WHEN typeof(snapshot_bytes)='blob' THEN length(snapshot_bytes) ELSE -1 END
         FROM semantic_snapshots WHERE persona_scope=?1 AND semantic_revision=?2",
        "semantic_time.snapshot_bytes",
        MAX_SNAPSHOT_STATE_BYTES,
    )?;
    type StoredTimeAuthority = (
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        i64,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
    );
    let authority_row: StoredTimeAuthority = conn.query_row(
        "SELECT
           CASE WHEN typeof(authority_bytes)='blob' AND length(authority_bytes)<=?3 THEN authority_bytes END,
           CASE WHEN typeof(authority_bytes)='blob' THEN length(authority_bytes) ELSE -1 END,
           CASE WHEN typeof(authority_digest)='blob' AND length(authority_digest)=32
                THEN authority_digest END,
           journal_revision,
           CASE WHEN typeof(event_id)='blob' AND length(event_id)=16 THEN event_id END,
           CASE WHEN typeof(event_digest)='blob' AND length(event_digest)=32 THEN event_digest END
         FROM semantic_time_authority WHERE persona_scope=?1 AND semantic_revision=?2",
        params![
            blob(persona_scope),
            JournalRevision::new(revision).to_sqlite()?.get(),
            16_384_i64,
        ],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        },
    )?;
    let authority_bytes = bounded_typed_value(
        authority_row.0,
        authority_row.1,
        16_384,
        "semantic_time.authority_bytes",
        "semantic_time_authority_type",
    )?;
    let authority: SemanticTimeAuthorityV1 = serde_json::from_slice(&authority_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_time_authority_wire"))?;
    let canonical_authority = serde_json::to_vec(&authority)
        .map_err(|_| StoreError::ContinuityFence("semantic_time_authority_wire"))?;
    let time_authority_digest = digest_from_vec(
        authority_row.2.ok_or(StoreError::ContinuityFence(
            "semantic_time_authority_digest_type",
        ))?,
        "semantic_time.authority",
    )?;
    if canonical_authority != authority_bytes
        || !authority.validate_v1()
        || wire::domain_hash(
            b"astr-embodiment/semantic-time-authority-v1",
            &[&authority_bytes],
        ) != time_authority_digest
        || semantic_revision_from_sql(authority_row.3)? != journal_revision
        || id_from_vec(
            authority_row.4.ok_or(StoreError::ContinuityFence(
                "semantic_time_authority_event_id_type",
            ))?,
            "semantic_time.authority_event_id",
        )? != event_id
        || digest_from_vec(
            authority_row.5.ok_or(StoreError::ContinuityFence(
                "semantic_time_authority_event_digest_type",
            ))?,
            "semantic_time.authority_event",
        )? != event_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_time_authority_binding",
        ));
    }
    let decoded = decode_time_snapshot_v1(&snapshot_bytes).map_err(semantic_core_history_error)?;
    if decoded.semantic_formula_digest != formula_digest
        || decoded.time_formula_digest != authority.time_formula_digest
        || decoded.epoch_before != authority.epoch_before
        || decoded.epoch_after != authority.epoch_after
        || decoded.requested_elapsed_ms != authority.raw_elapsed_ms
        || decoded.applied_elapsed_ms != authority.applied_elapsed_ms
        || decoded.capped_gap != authority.capped_gap
        || decoded.pre_sleep_phase != authority.pre_sleep_phase
        || decoded.state_before != state_before
        || decoded.state_after != state_after
        || decoded.graph_digest != graph_after
        || decoded.authority_digest != event_authority
        || decoded.commitment_digest != commitment_digest
        || wire::domain_hash(SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1, &[&snapshot_bytes])
            != snapshot_wire_digest
    {
        return Err(StoreError::ContinuityFence(
            "semantic_time_snapshot_binding",
        ));
    }
    let journal =
        query_bounded_journal_row(conn, &persona_scope, JournalRevision::new(journal_revision))?
            .ok_or(StoreError::ContinuityFence("semantic_time_journal_missing"))?;
    let event = wire::decode_event(&journal.event_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_time_event"))?;
    let time = match &event {
        CanonicalEvent::TimeAdvance(value) => value,
        _ => return Err(StoreError::ContinuityFence("semantic_time_event")),
    };
    if wire::event_digest(&event) != event_digest
        || time.event_id != event_id
        || wire::persona_scope_digest(&time.scope.bot_token, &time.scope.persona_token, None)
            != persona_scope
        || ae_authority::authority_projection_digest(&event) != event_authority
        || authority.event_digest != event_digest
        || authority.event_id != event_id
        || authority.persona_scope != persona_scope
        || authority.semantic_revision != semantic_revision
        || authority.journal_revision != journal_revision
        || authority.state_before != state_before
        || authority.state_after != state_after
        || authority.graph_digest != graph_after
        || authority.incarnation_id != incarnation_id
        || authority.manifest_digest != manifest_digest
        || authority.route_digest != route_digest
        || authority.semantic_formula_digest != formula_digest
    {
        return Err(StoreError::ContinuityFence("semantic_time_event_binding"));
    }
    let origin = stored_origin(conn, persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_origin_missing"))?;
    if origin.origin_digest != stored_origin_digest
        || origin.incarnation_id != incarnation_id
        || origin.manifest_digest != manifest_digest
        || origin.route_digest != route_digest
        || origin.formula_digest != formula_digest
    {
        return Err(StoreError::ContinuityFence("semantic_time_origin_binding"));
    }
    let recomputed = time_commitment_digest_v1(
        &persona_scope,
        semantic_revision,
        journal_revision,
        &event_id,
        &event_digest,
        &incarnation_id,
        &manifest_digest,
        &route_digest,
        &formula_digest,
        &state_before,
        &state_after,
        &graph_after,
        &event_authority,
        &stored_origin_digest,
        &time_authority_digest,
    );
    if recomputed != commitment_digest {
        return Err(StoreError::ContinuityFence("semantic_time_commitment"));
    }
    Ok(CommittedSemanticV1 {
        journal_revision,
        semantic_revision,
        persona_scope,
        relation_scope: None,
        event_id,
        event_digest,
        transition_kind: SemanticTransitionKindV1::Time,
        evidence_digest: None,
        estimator_digest: None,
        incarnation_id,
        manifest_digest,
        route_digest,
        formula_digest,
        state_digest: state_after,
        graph_before,
        graph_digest: graph_after,
        snapshot_bytes,
        receipt_bytes: None,
        telemetry_bytes: None,
        time_authority: Some(authority),
        commitment_digest,
        origin,
        journal,
    })
}

pub(crate) fn read_semantic_commit(
    conn: &Connection,
    persona_scope: Digest,
    revision: u64,
) -> Result<CommittedSemanticV1, StoreError> {
    let stored = stored_commit(conn, persona_scope, revision)?;
    let transition_kind = SemanticTransitionKindV1::from_str(&stored.transition_kind)
        .ok_or(StoreError::ContinuityFence("semantic_transition_kind"))?;
    if transition_kind == SemanticTransitionKindV1::Time {
        return read_time_semantic_commit(conn, persona_scope, revision, stored);
    }
    let semantic_revision = semantic_revision_from_sql(stored.semantic_revision)?;
    let journal_revision = semantic_revision_from_sql(stored.journal_revision)?;
    if semantic_revision != revision {
        return Err(StoreError::ContinuityFence("semantic_revision_identity"));
    }
    let relation_present = match stored.relation_present {
        0 => false,
        1 => true,
        _ => return Err(StoreError::ContinuityFence("semantic_relation_flag")),
    };
    let relation_storage_scope =
        digest_from_vec(stored.relation_scope, "semantic_commit.relation_scope")?;
    if !relation_present && relation_storage_scope != persona_scope {
        return Err(StoreError::ContinuityFence("semantic_relation_root"));
    }
    let relation_scope = relation_present.then_some(relation_storage_scope);
    let event_id = id_from_vec(stored.event_id, "semantic_commit.event_id")?;
    let event_digest = digest_from_vec(stored.event_digest, "semantic_commit.event")?;
    let evidence_digest = digest_from_vec(stored.evidence_digest, "semantic_commit.evidence")?;
    let estimator_digest = digest_from_vec(stored.estimator_digest, "semantic_commit.estimator")?;
    let incarnation_id = digest_from_vec(stored.incarnation_id, "semantic_commit.incarnation")?;
    let manifest_digest = digest_from_vec(stored.manifest_digest, "semantic_commit.manifest")?;
    let route_digest = digest_from_vec(stored.route_digest, "semantic_commit.route")?;
    let formula_digest = digest_from_vec(stored.formula_digest, "semantic_commit.formula")?;
    let state_before = digest_from_vec(stored.state_before, "semantic_commit.state_before")?;
    let state_digest_value = digest_from_vec(stored.state_digest, "semantic_commit.state")?;
    let graph_before = digest_from_vec(stored.graph_before, "semantic_commit.graph_before")?;
    let graph_digest_value = digest_from_vec(stored.graph_digest, "semantic_commit.graph")?;
    let authority_digest = digest_from_vec(stored.authority_digest, "semantic_commit.authority")?;
    let telemetry_digest = digest_from_vec(stored.telemetry_digest, "semantic_commit.telemetry")?;
    let snapshot_wire_digest =
        digest_from_vec(stored.snapshot_wire_digest, "semantic_commit.snapshot_wire")?;
    let graph_wire_digest =
        digest_from_vec(stored.graph_wire_digest, "semantic_commit.graph_wire")?;
    let receipt_wire_digest =
        digest_from_vec(stored.receipt_wire_digest, "semantic_commit.receipt_wire")?;
    let telemetry_wire_digest = digest_from_vec(
        stored.telemetry_wire_digest,
        "semantic_commit.telemetry_wire",
    )?;
    let stored_origin_digest = digest_from_vec(stored.origin_digest, "semantic_commit.origin")?;
    let commitment_digest =
        digest_from_vec(stored.commitment_digest, "semantic_commit.commitment")?;

    let revision_sql = JournalRevision::new(revision).to_sqlite()?.get();
    let sidecar_counts: (i64, i64, i64, i64, i64, i64) = conn.query_row(
        "SELECT
            (SELECT COUNT(*) FROM semantic_snapshots AS sidecar
             JOIN semantic_commits USING(persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest)
             WHERE sidecar.persona_scope=?1 AND sidecar.semantic_revision=?2),
            (SELECT COUNT(*) FROM semantic_graphs AS sidecar
             JOIN semantic_commits USING(persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest)
             WHERE sidecar.persona_scope=?1 AND sidecar.semantic_revision=?2),
            (SELECT COUNT(*) FROM semantic_receipts AS sidecar
             JOIN semantic_commits USING(persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest)
             WHERE sidecar.persona_scope=?1 AND sidecar.semantic_revision=?2),
            (SELECT COUNT(*) FROM semantic_telemetry AS sidecar
             JOIN semantic_commits USING(persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest)
             WHERE sidecar.persona_scope=?1 AND sidecar.semantic_revision=?2),
            (SELECT COUNT(*) FROM semantic_evidence_authority AS sidecar
             JOIN semantic_commits USING(persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest)
             WHERE sidecar.persona_scope=?1 AND sidecar.semantic_revision=?2),
            (SELECT COUNT(*) FROM semantic_time_authority
             WHERE persona_scope=?1 AND semantic_revision=?2)",
        params![blob(persona_scope), revision_sql],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
    )?;
    if [
        sidecar_counts.0,
        sidecar_counts.1,
        sidecar_counts.2,
        sidecar_counts.3,
        sidecar_counts.4,
        sidecar_counts.5,
    ] != [1, 1, 1, 1, 1, 0]
    {
        return Err(StoreError::ContinuityFence("semantic_sidecar_closure"));
    }

    let snapshot_bytes = bounded_payload(
        conn,
        persona_scope,
        revision,
        "SELECT CASE WHEN typeof(snapshot_bytes)='blob' AND length(snapshot_bytes)<=?3 THEN snapshot_bytes END,
                CASE WHEN typeof(snapshot_bytes)='blob' THEN length(snapshot_bytes) ELSE -1 END
         FROM semantic_snapshots WHERE persona_scope=?1 AND semantic_revision=?2",
        "semantic_snapshots.snapshot_bytes",
        MAX_SNAPSHOT_STATE_BYTES,
    )?;
    let graph_bytes = bounded_payload(
        conn,
        persona_scope,
        revision,
        "SELECT CASE WHEN typeof(graph_bytes)='blob' AND length(graph_bytes)<=?3 THEN graph_bytes END,
                CASE WHEN typeof(graph_bytes)='blob' THEN length(graph_bytes) ELSE -1 END
         FROM semantic_graphs WHERE persona_scope=?1 AND semantic_revision=?2",
        "semantic_graphs.graph_bytes",
        MAX_SEMANTIC_GRAPH_BYTES,
    )?;
    let receipt_bytes = bounded_payload(
        conn,
        persona_scope,
        revision,
        "SELECT CASE WHEN typeof(receipt_bytes)='blob' AND length(receipt_bytes)<=?3 THEN receipt_bytes END,
                CASE WHEN typeof(receipt_bytes)='blob' THEN length(receipt_bytes) ELSE -1 END
         FROM semantic_receipts WHERE persona_scope=?1 AND semantic_revision=?2",
        "semantic_receipts.receipt_bytes",
        MAX_SEMANTIC_RECEIPT_BYTES,
    )?;
    let telemetry_bytes = bounded_payload(
        conn,
        persona_scope,
        revision,
        "SELECT CASE WHEN typeof(telemetry_bytes)='blob' AND length(telemetry_bytes)<=?3 THEN telemetry_bytes END,
                CASE WHEN typeof(telemetry_bytes)='blob' THEN length(telemetry_bytes) ELSE -1 END
         FROM semantic_telemetry WHERE persona_scope=?1 AND semantic_revision=?2",
        "semantic_telemetry.telemetry_bytes",
        MAX_SEMANTIC_TELEMETRY_BYTES,
    )?;
    let evidence_bytes = bounded_payload(
        conn,
        persona_scope,
        revision,
        "SELECT CASE WHEN typeof(evidence_bytes)='blob' AND length(evidence_bytes)<=?3 THEN evidence_bytes END,
                CASE WHEN typeof(evidence_bytes)='blob' THEN length(evidence_bytes) ELSE -1 END
         FROM semantic_evidence_authority WHERE persona_scope=?1 AND semantic_revision=?2",
        "semantic_evidence_authority.evidence_bytes",
        MAX_JOURNAL_EVENT_BYTES,
    )?;
    if wire::domain_hash(SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1, &[&snapshot_bytes])
        != snapshot_wire_digest
        || wire::domain_hash(SEMANTIC_GRAPH_WIRE_DOMAIN_V1, &[&graph_bytes]) != graph_wire_digest
        || wire::domain_hash(SEMANTIC_RECEIPT_WIRE_DOMAIN_V1, &[&receipt_bytes])
            != receipt_wire_digest
        || wire::domain_hash(SEMANTIC_TELEMETRY_WIRE_DOMAIN_V1, &[&telemetry_bytes])
            != telemetry_wire_digest
    {
        return Err(StoreError::ContinuityFence("semantic_sidecar_digest"));
    }

    let event = wire::decode_event(&evidence_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_evidence_wire"))?;
    if wire::encode_event_checked(&event)
        .map_err(|_| StoreError::ContinuityFence("semantic_evidence_wire"))?
        != evidence_bytes
        || wire::event_digest(&event) != event_digest
        || semantic_evidence_digest_v1(&event)? != evidence_digest
        || ae_authority::authority_projection_digest(&event) != authority_digest
    {
        return Err(StoreError::ContinuityFence("semantic_evidence_closure"));
    }
    let stimulus = validated_stimulus(&event)?;
    let derived_persona = wire::persona_scope_digest(
        &stimulus.scope.bot_token,
        &stimulus.scope.persona_token,
        None,
    );
    let derived_relation = stimulus.scope.relation_token.as_ref().map(|relation| {
        wire::persona_scope_digest(
            &stimulus.scope.bot_token,
            &stimulus.scope.persona_token,
            Some(relation),
        )
    });
    if derived_persona != persona_scope
        || derived_relation != relation_scope
        || stimulus.event_id != event_id
        || stimulus.evidence.estimator_digest != estimator_digest
    {
        return Err(StoreError::ContinuityFence("semantic_evidence_identity"));
    }

    let identity = active_identity(conn, &stimulus.scope, incarnation_id, manifest_digest)?;
    let expected_route = phase0_semantic_route_digest_v1();
    let expected_formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    if route_digest != expected_route || formula_digest != expected_formula {
        return Err(StoreError::ContinuityFence("semantic_frozen_formula"));
    }

    let journal =
        query_bounded_journal_row(conn, &persona_scope, JournalRevision::new(journal_revision))?
            .ok_or(StoreError::ContinuityFence("semantic_journal_missing"))?;
    if journal.revision != journal_revision
        || journal.base_revision.checked_add(1) != Some(journal_revision)
        || journal.event_kind != wire::event_kind_name(&event)
        || !journal.delta_bytes.is_empty()
        || journal.event_bytes != evidence_bytes
        || journal.event_digest != event_digest
    {
        return Err(StoreError::ContinuityFence("semantic_journal_binding"));
    }
    let journal_receipt = wire::decode_transition_receipt(&journal.receipt_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_journal_receipt"))?;
    if wire::encode_transition_receipt(&journal_receipt) != journal.receipt_bytes
        || journal_receipt.scope_digest != persona_scope
        || journal_receipt.event_digest != event_digest
        || journal_receipt.authority_digest != authority_digest
        || journal_receipt.formula_digest != formula_digest
        || journal_receipt.base_revision != journal.base_revision
        || journal_receipt.next_revision != journal.revision
        || journal_receipt.state_before != state_before
        || journal_receipt.state_after != state_digest_value
        || journal_receipt.graph_after != graph_digest_value
        || journal_receipt.action_contract.is_some()
        || journal_receipt.status != CommitStatus::Committed
    {
        return Err(StoreError::ContinuityFence("semantic_journal_receipt"));
    }
    let chain_seed = if journal_revision == 1 {
        identity.initial_snapshot_digest
    } else {
        query_bounded_journal_row(
            conn,
            &persona_scope,
            JournalRevision::new(journal_revision - 1),
        )?
        .ok_or(StoreError::ContinuityFence("semantic_journal_predecessor"))?
        .chain_digest
    };
    if ae_continuum::chain_link(&chain_seed, &journal.event_bytes, &journal.receipt_bytes)
        != journal.chain_digest
    {
        return Err(StoreError::ContinuityFence("semantic_journal_chain"));
    }
    let claims =
        derive_canonical_semantic_sidecar_v1(&snapshot_bytes, &receipt_bytes, &telemetry_bytes)?;
    let blocks = decode_canonical_aesem3_blocks(&snapshot_bytes)
        .map_err(|_| StoreError::ContinuityFence("semantic_aesem3_wire"))?;
    if blocks.graph != graph_bytes
        || claims.formula_digest != formula_digest
        || claims.scope_digest != persona_scope
        || claims.event_digest != event_digest
        || claims.authority_digest != authority_digest
        || claims.source_digest != evidence_digest
        || claims.telemetry_digest != telemetry_digest
        || claims.base_revision.checked_add(1) != Some(semantic_revision)
        || claims.state_before != state_before
        || claims.state_after != state_digest_value
        || claims.graph_before != graph_before
        || claims.graph_after != graph_digest_value
        || journal_receipt.active_nodes != claims.active_nodes
        || journal_receipt.active_edges != claims.active_edges
        || journal_receipt.residuals != claims.residuals
    {
        return Err(StoreError::ContinuityFence("semantic_sidecar_binding"));
    }

    let origin = stored_origin(conn, persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_origin_missing"))?;
    if origin.origin_digest != stored_origin_digest
        || origin.incarnation_id != incarnation_id
        || origin.manifest_digest != manifest_digest
        || origin.route_digest != route_digest
        || origin.formula_digest != formula_digest
    {
        return Err(StoreError::ContinuityFence("semantic_origin_binding"));
    }
    if !origin.legacy_migrated
        && (origin.source_scope_digest != persona_scope
            || origin.source_revision != 0
            || origin.state_digest != state_digest(&identity.baseline_field, &expected_formula)
            || origin.graph_digest != graph_digest(&identity.baseline_graph))
    {
        return Err(StoreError::ContinuityFence(
            "semantic_origin_genesis_binding",
        ));
    }
    let candidate = PairedSemanticCommitV1 {
        journal: CommitEnvelope {
            event_kind: journal.event_kind.clone(),
            event_bytes: journal.event_bytes.clone(),
            receipt: journal_receipt,
            chain_seed: [0; 32],
            delta_bytes: journal.delta_bytes.clone(),
        },
        persona_scope,
        relation_scope,
        semantic_base_revision: claims.base_revision,
        event_id,
        event_digest,
        evidence_digest,
        estimator_digest,
        incarnation_id,
        manifest_digest,
        route_digest,
        formula_digest,
        state_digest: state_digest_value,
        snapshot_bytes: snapshot_bytes.clone(),
        graph_digest: graph_digest_value,
        receipt_bytes: receipt_bytes.clone(),
        telemetry_bytes: telemetry_bytes.clone(),
    };
    let recomputed = commit_digest(
        &candidate,
        relation_present,
        &relation_storage_scope,
        journal_revision,
        semantic_revision,
        &state_before,
        &graph_before,
        &authority_digest,
        &telemetry_digest,
        &snapshot_wire_digest,
        &graph_wire_digest,
        &receipt_wire_digest,
        &telemetry_wire_digest,
        &origin.origin_digest,
    );
    if recomputed != commitment_digest {
        return Err(StoreError::ContinuityFence("semantic_commitment"));
    }

    Ok(CommittedSemanticV1 {
        journal_revision,
        semantic_revision,
        persona_scope,
        relation_scope,
        event_id,
        event_digest,
        transition_kind,
        evidence_digest: Some(evidence_digest),
        estimator_digest: Some(estimator_digest),
        incarnation_id,
        manifest_digest,
        route_digest,
        formula_digest,
        state_digest: state_digest_value,
        graph_before,
        graph_digest: graph_digest_value,
        snapshot_bytes,
        receipt_bytes: Some(receipt_bytes),
        telemetry_bytes: Some(telemetry_bytes),
        time_authority: None,
        commitment_digest,
        origin,
        journal,
    })
}

fn validate_semantic_cas_tx(
    tx: &Transaction<'_>,
    candidate: &PairedSemanticCommitV1,
    derived: &DerivedSemanticCommitV1,
) -> Result<bool, StoreError> {
    let event = wire::decode_event(&candidate.journal.event_bytes).map_err(|_| StoreError::SemanticInvalid("canonical_event_wire"))?;
    let scope = &validated_stimulus(&event)?.scope;
    let clock_input = crate::embodiment_clock::semantic_input(tx, &ae_contracts::PersonaScopeRef { bot_token: scope.bot_token, persona_token: scope.persona_token })?;
    let projected_state = clock_input.as_ref().map(|(f, _)| state_digest(f, &candidate.formula_digest));
    match semantic_head_tx(tx, candidate.persona_scope)? {
        Some((current, state, graph, commitment)) => {
            if current != candidate.semantic_base_revision {
                return Err(StoreError::StaleRevision {
                    expected: candidate.semantic_base_revision,
                    actual: current,
                });
            }
            // `semantic_state_for_derivation_tx` already authenticated and
            // decoded the one predecessor row.  The cursor FK-closure above is
            // sufficient for the CAS fence; reading every predecessor sidecar
            // a second time here would double the bounded (up to 16 MiB) head
            // verification inside the writer transaction.
            if !is_nonzero(&commitment)
                || projected_state.unwrap_or(state) != derived.state_before
                || graph != derived.graph_before
                || derived.origin.incarnation_id != candidate.incarnation_id
                || derived.origin.manifest_digest != candidate.manifest_digest
                || derived.origin.route_digest != candidate.route_digest
                || derived.origin.formula_digest != candidate.formula_digest
            {
                return Err(StoreError::ContinuityFence("semantic_cursor_closure"));
            }
            Ok(true)
        }
        None => {
            let actual = derived.origin.source_revision;
            if candidate.semantic_base_revision != actual {
                return Err(StoreError::StaleRevision {
                    expected: candidate.semantic_base_revision,
                    actual,
                });
            }
            if derived.state_before != projected_state.unwrap_or(derived.origin.state_digest)
                || derived.graph_before != derived.origin.graph_digest
            {
                return Err(StoreError::ContinuityFence("semantic_origin_predecessor"));
            }
            Ok(false)
        }
    }
}

fn insert_origin_tx(tx: &Transaction<'_>, origin: &SemanticOriginV1) -> Result<(), StoreError> {
    tx.execute(
        "INSERT INTO semantic_origins (
            persona_scope,source_scope_digest,source_revision,legacy_migrated,
            incarnation_id,manifest_digest,route_digest,formula_digest,state_digest,
            graph_digest,origin_digest
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
         ON CONFLICT(persona_scope) DO NOTHING",
        params![
            blob(origin.persona_scope),
            blob(origin.source_scope_digest),
            JournalRevision::new(origin.source_revision)
                .to_sqlite()?
                .get(),
            i64::from(origin.legacy_migrated),
            blob(origin.incarnation_id),
            blob(origin.manifest_digest),
            blob(origin.route_digest),
            blob(origin.formula_digest),
            blob(origin.state_digest),
            blob(origin.graph_digest),
            blob(origin.origin_digest),
        ],
    )?;
    let stored = stored_origin_tx(tx, origin.persona_scope)?
        .ok_or(StoreError::ContinuityFence("semantic_origin_insert"))?;
    if &stored != origin {
        return Err(StoreError::ContinuityFence("semantic_origin_conflict"));
    }
    Ok(())
}

fn insert_semantic_rows_tx(
    tx: &Transaction<'_>,
    candidate: &PairedSemanticCommitV1,
    derived: &DerivedSemanticCommitV1,
    journal_revision: u64,
) -> Result<(), StoreError> {
    let semantic_revision = JournalRevision::new(derived.semantic_revision)
        .to_sqlite()?
        .get();
    let journal_revision = JournalRevision::new(journal_revision).to_sqlite()?.get();
    let relation_present = i64::from(derived.relation_present);

    tx.execute(
        "INSERT INTO semantic_commits (
            persona_scope,semantic_revision,journal_revision,relation_present,relation_scope,
            event_id,event_digest,evidence_digest,estimator_digest,incarnation_id,manifest_digest,
            route_digest,formula_digest,state_before,state_digest,graph_before,graph_digest,
            authority_digest,telemetry_digest,snapshot_wire_digest,graph_wire_digest,
            receipt_wire_digest,telemetry_wire_digest,origin_digest,commitment_digest
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,
            ?18,?19,?20,?21,?22,?23,?24,?25)",
        params![
            blob(candidate.persona_scope),
            semantic_revision,
            journal_revision,
            relation_present,
            blob(derived.relation_storage_scope),
            blob(candidate.event_id),
            blob(candidate.event_digest),
            blob(candidate.evidence_digest),
            blob(candidate.estimator_digest),
            blob(candidate.incarnation_id),
            blob(candidate.manifest_digest),
            blob(candidate.route_digest),
            blob(candidate.formula_digest),
            blob(derived.state_before),
            blob(candidate.state_digest),
            blob(derived.graph_before),
            blob(candidate.graph_digest),
            blob(derived.authority_digest),
            blob(derived.telemetry_digest),
            blob(derived.snapshot_wire_digest),
            blob(derived.graph_wire_digest),
            blob(derived.receipt_wire_digest),
            blob(derived.telemetry_wire_digest),
            blob(derived.origin.origin_digest),
            blob(derived.commitment_digest),
        ],
    )?;

    let common = |payload_sql: &str, payload: Vec<u8>| -> Result<(), StoreError> {
        tx.execute(
            payload_sql,
            params![
                blob(candidate.persona_scope),
                semantic_revision,
                blob(candidate.incarnation_id),
                blob(candidate.manifest_digest),
                blob(candidate.route_digest),
                blob(candidate.formula_digest),
                blob(candidate.graph_digest),
                blob(candidate.state_digest),
                blob(candidate.evidence_digest),
                blob(candidate.estimator_digest),
                blob(candidate.event_digest),
                blob(derived.commitment_digest),
                payload,
            ],
        )?;
        Ok(())
    };
    common(
        "INSERT INTO semantic_snapshots (
            persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
            formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
            event_digest,commitment_digest,snapshot_bytes
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        candidate.snapshot_bytes.clone(),
    )?;
    common(
        "INSERT INTO semantic_graphs (
            persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
            formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
            event_digest,commitment_digest,graph_bytes
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        derived.graph_bytes.clone(),
    )?;
    common(
        "INSERT INTO semantic_receipts (
            persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
            formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
            event_digest,commitment_digest,receipt_bytes
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        candidate.receipt_bytes.clone(),
    )?;
    common(
        "INSERT INTO semantic_telemetry (
            persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
            formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
            event_digest,commitment_digest,telemetry_bytes
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
        candidate.telemetry_bytes.clone(),
    )?;
    tx.execute(
        "INSERT INTO semantic_evidence_authority (
            persona_scope,semantic_revision,relation_present,relation_scope,event_id,
            incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,state_digest,
            evidence_digest,estimator_digest,event_digest,commitment_digest,evidence_bytes
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
        params![
            blob(candidate.persona_scope),
            semantic_revision,
            relation_present,
            blob(derived.relation_storage_scope),
            blob(candidate.event_id),
            blob(candidate.incarnation_id),
            blob(candidate.manifest_digest),
            blob(candidate.route_digest),
            blob(candidate.formula_digest),
            blob(candidate.graph_digest),
            blob(candidate.state_digest),
            blob(candidate.evidence_digest),
            blob(candidate.estimator_digest),
            blob(candidate.event_digest),
            blob(derived.commitment_digest),
            candidate.journal.event_bytes.clone(),
        ],
    )?;
    Ok(())
}

fn advance_semantic_cursor_tx(
    tx: &Transaction<'_>,
    candidate: &PairedSemanticCommitV1,
    derived: &DerivedSemanticCommitV1,
    journal_revision: u64,
    had_cursor: bool,
) -> Result<(), StoreError> {
    let semantic_revision = JournalRevision::new(derived.semantic_revision)
        .to_sqlite()?
        .get();
    let journal_revision = JournalRevision::new(journal_revision).to_sqlite()?.get();
    let changed = if had_cursor {
        tx.execute(
            "UPDATE semantic_cursor SET
                semantic_revision=?2,journal_revision=?3,incarnation_id=?4,manifest_digest=?5,
                route_digest=?6,formula_digest=?7,graph_digest=?8,state_digest=?9,
                evidence_digest=?10,estimator_digest=?11,event_digest=?12,commitment_digest=?13,
                transition_kind='perception',time_anchor_semantic_revision=?2,
                time_anchor_state_digest=?9,time_awake_ticks=0,time_drowsy_ticks=0,
                time_asleep_ticks=0,time_awake_remainder_ms=0,time_drowsy_remainder_ms=0,
                time_asleep_remainder_ms=0
             WHERE persona_scope=?1 AND semantic_revision=?14",
            params![
                blob(candidate.persona_scope),
                semantic_revision,
                journal_revision,
                blob(candidate.incarnation_id),
                blob(candidate.manifest_digest),
                blob(candidate.route_digest),
                blob(candidate.formula_digest),
                blob(candidate.graph_digest),
                blob(candidate.state_digest),
                blob(candidate.evidence_digest),
                blob(candidate.estimator_digest),
                blob(candidate.event_digest),
                blob(derived.commitment_digest),
                JournalRevision::new(candidate.semantic_base_revision)
                    .to_sqlite()?
                    .get(),
            ],
        )?
    } else {
        tx.execute(
            "INSERT INTO semantic_cursor (
                persona_scope,semantic_revision,journal_revision,incarnation_id,manifest_digest,
                route_digest,formula_digest,graph_digest,state_digest,evidence_digest,
                estimator_digest,event_digest,commitment_digest,transition_kind,
                time_anchor_semantic_revision,time_anchor_state_digest,time_awake_ticks,
                time_drowsy_ticks,time_asleep_ticks,time_awake_remainder_ms,
                time_drowsy_remainder_ms,time_asleep_remainder_ms
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,'perception',
                ?2,?9,0,0,0,0,0,0)",
            params![
                blob(candidate.persona_scope),
                semantic_revision,
                journal_revision,
                blob(candidate.incarnation_id),
                blob(candidate.manifest_digest),
                blob(candidate.route_digest),
                blob(candidate.formula_digest),
                blob(candidate.graph_digest),
                blob(candidate.state_digest),
                blob(candidate.evidence_digest),
                blob(candidate.estimator_digest),
                blob(candidate.event_digest),
                blob(derived.commitment_digest),
            ],
        )?
    };
    if changed != 1 {
        return Err(StoreError::ContinuityFence("semantic_cursor_cas"));
    }
    Ok(())
}

fn advance_semantic_budget_checkpoint_tx(
    tx: &Transaction<'_>,
    persona_scope: Digest,
    semantic_revision: u64,
    commitment_digest: Digest,
    budget: SemanticBudgetAdvanceV1,
) -> Result<(), StoreError> {
    let row_count = JournalRevision::new(budget.row_count).to_sqlite()?.get();
    let aggregate_bytes = JournalRevision::new(budget.aggregate_bytes)
        .to_sqlite()?
        .get();
    let head_revision = JournalRevision::new(semantic_revision).to_sqlite()?.get();
    let changed = if budget.had_checkpoint {
        tx.execute(
            "UPDATE semantic_budget_checkpoint SET
                row_count=?2,aggregate_bytes=?3,head_revision=?4,head_commitment_digest=?5
             WHERE persona_scope=?1 AND head_revision=?6",
            params![
                blob(persona_scope),
                row_count,
                aggregate_bytes,
                head_revision,
                blob(commitment_digest),
                JournalRevision::new(semantic_revision - 1)
                    .to_sqlite()?
                    .get(),
            ],
        )?
    } else {
        tx.execute(
            "INSERT INTO semantic_budget_checkpoint(
                persona_scope,row_count,aggregate_bytes,head_revision,head_commitment_digest
             ) VALUES(?1,?2,?3,?4,?5)",
            params![
                blob(persona_scope),
                row_count,
                aggregate_bytes,
                head_revision,
                blob(commitment_digest),
            ],
        )?
    };
    if changed != 1 {
        return Err(StoreError::ContinuityFence("semantic_budget_cas"));
    }
    Ok(())
}

enum ProductionSemanticPreparation {
    Existing(CommittedSemanticV1),
    Candidate(PairedSemanticCommitV1),
}

fn prepare_production_semantic_tx(
    tx: &Transaction<'_>,
    event: &CanonicalEvent,
) -> Result<ProductionSemanticPreparation, StoreError> {
    let stimulus = validated_stimulus(event)?;
    let event_bytes = wire::encode_event_checked(event)
        .map_err(|_| StoreError::SemanticInvalid("canonical_event_wire"))?;
    let event_digest = wire::event_digest(event);
    let evidence_digest = semantic_evidence_digest_v1(event)?;
    let authority_digest = ae_authority::authority_projection_digest(event);
    let (persona_scope, relation_scope, relation_storage_scope) = semantic_scopes(stimulus);

    let identity = active_identity_for_scope_tx(tx, &stimulus.scope)?;
    let (journal_revision, journal_chain) =
        attest_persona_journal_tx(tx, persona_scope, identity.initial_snapshot_digest)?;

    // Exact retries are resolved only after both durable histories and active
    // Genesis have re-attested, but before either independent cursor CAS.
    if let Some(committed) = exact_semantic_retry_tx(tx, event, persona_scope, relation_scope)? {
        return Ok(ProductionSemanticPreparation::Existing(committed));
    }
    if stimulus.causal.base_revision != journal_revision {
        return Err(StoreError::StaleRevision {
            expected: stimulus.causal.base_revision,
            actual: journal_revision,
        });
    }
    let journal_next = JournalRevision::new(journal_revision).checked_next()?.get();

    let route_digest = phase0_semantic_route_digest_v1();
    let formula_digest = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let origin = derive_origin_tx(
        tx,
        &stimulus.scope,
        persona_scope,
        identity.incarnation_id,
        identity.manifest_digest,
        route_digest,
        formula_digest,
        &identity,
    )?;
    let (semantic_base_revision, field, graph) =
        semantic_state_for_derivation_tx(tx, persona_scope, &origin, &identity)?;
    let (field, graph) = crate::embodiment_clock::semantic_input(tx, &ae_contracts::PersonaScopeRef { bot_token: stimulus.scope.bot_token, persona_token: stimulus.scope.persona_token })?.unwrap_or((field, graph));
    let canonical_nonce = canonical_semantic_nonce_v1(
        &event_bytes,
        &persona_scope,
        &relation_storage_scope,
        &identity.incarnation_id,
        semantic_base_revision,
    );
    let proposal = PerceptionProposalV1 {
        schema_version: PerceptionProposalV1::SCHEMA_VERSION,
        origin_digest: event_digest,
        dimensions: stimulus.evidence.dimensions.clone(),
        estimator_confidence: stimulus.evidence.estimator_confidence,
        protocol_version: PerceptionProposalV1::PROTOCOL_VERSION,
        request_nonce_digest: canonical_nonce,
    };
    let derived = derive_user_stimulus_transition_v1(UserStimulusTransitionInputV1 {
        field: &field,
        baseline: &identity.baseline_field,
        graph: &graph,
        manifest_digest: identity.manifest_digest,
        development_seed_digest: identity.development_seed_digest,
        proposal: &proposal,
        formula_digest,
        scope_digest: persona_scope,
        event_digest,
        source_digest: evidence_digest,
        authority_digest,
        semantic_base_revision,
    })
    .map_err(semantic_core_input_error)?;
    if derived.route_digest != route_digest
        || derived.state_before_digest != state_digest(&field, &formula_digest)
        || derived.graph_before_digest != graph_digest(&graph)
    {
        return Err(StoreError::ContinuityFence("semantic_core_identity"));
    }

    let journal_receipt = TransitionReceipt {
        schema_version: 1,
        formula_digest,
        scope_digest: persona_scope,
        event_digest,
        authority_digest,
        base_revision: journal_revision,
        next_revision: journal_next,
        state_before: derived.state_before_digest,
        state_after: derived.state_after_digest,
        graph_after: derived.graph_after_digest,
        action_contract: None,
        active_nodes: derived.active_nodes,
        active_edges: derived.active_edges,
        residuals: derived.telemetry.residuals.clone(),
        status: CommitStatus::Committed,
    };
    Ok(ProductionSemanticPreparation::Candidate(
        PairedSemanticCommitV1 {
            journal: CommitEnvelope {
                event_kind: wire::event_kind_name(event).to_owned(),
                event_bytes,
                receipt: journal_receipt,
                chain_seed: journal_chain,
                delta_bytes: Vec::new(),
            },
            persona_scope,
            relation_scope,
            semantic_base_revision,
            event_id: stimulus.event_id,
            event_digest,
            evidence_digest,
            estimator_digest: stimulus.evidence.estimator_digest,
            incarnation_id: identity.incarnation_id,
            manifest_digest: identity.manifest_digest,
            route_digest,
            formula_digest,
            state_digest: derived.state_after_digest,
            snapshot_bytes: derived.snapshot_bytes,
            graph_digest: derived.graph_after_digest,
            receipt_bytes: derived.semantic_receipt_bytes,
            telemetry_bytes: derived.telemetry_bytes,
        },
    ))
}

enum SemanticTransactionOutcome {
    Existing(CommittedSemanticV1),
    Inserted {
        persona_scope: Digest,
        semantic_revision: u64,
    },
}

fn matrix_phase_from_sleep_v1(sleep: SleepStateV1) -> MatrixSleepPhaseV1 {
    match sleep {
        SleepStateV1::Awake => MatrixSleepPhaseV1::Awake,
        SleepStateV1::Drowsy => MatrixSleepPhaseV1::Drowsy,
        SleepStateV1::Asleep => MatrixSleepPhaseV1::Asleep,
    }
}

fn autonomy_state_digest_v1(state: &AutonomousRuntimeStateV1) -> Result<Digest, StoreError> {
    let bytes = serde_json::to_vec(state)
        .map_err(|_| StoreError::ContinuityFence("autonomy_state_wire"))?;
    Ok(wire::domain_hash(b"ae.autonomy.snapshot.v1", &[&bytes]))
}

pub(crate) fn semantic_cursor_epoch_v1(
    conn: &Connection,
    persona_scope: Digest,
    fallback_revision: u64,
    fallback_state: Digest,
) -> Result<(bool, MatrixTimeEpochV1), StoreError> {
    type RawEpoch = (i64, Option<Vec<u8>>, i64, i64, i64, i64, i64, i64, i64);
    let row: Option<RawEpoch> = conn
        .query_row(
            "SELECT time_anchor_semantic_revision,
                    CASE WHEN typeof(time_anchor_state_digest)='blob'
                              AND length(time_anchor_state_digest)=32
                         THEN time_anchor_state_digest END,
                    CASE WHEN typeof(time_anchor_state_digest)='blob'
                         THEN length(time_anchor_state_digest) ELSE -1 END,
                     time_awake_ticks,time_drowsy_ticks,time_asleep_ticks,
                    time_awake_remainder_ms,time_drowsy_remainder_ms,time_asleep_remainder_ms
             FROM semantic_cursor WHERE persona_scope=?1",
            params![blob(persona_scope)],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                ))
            },
        )
        .optional()?;
    let Some(raw) = row else {
        return Ok((
            false,
            MatrixTimeEpochV1 {
                schema_version: MatrixTimeEpochV1::SCHEMA_VERSION,
                anchor_semantic_revision: fallback_revision,
                anchor_state_digest: fallback_state,
                awake_ticks: 0,
                drowsy_ticks: 0,
                asleep_ticks: 0,
                awake_remainder_ms: 0,
                drowsy_remainder_ms: 0,
                asleep_remainder_ms: 0,
            },
        ));
    };
    let to_u64 = |value: i64| {
        u64::try_from(value).map_err(|_| StoreError::ContinuityFence("semantic_time_epoch"))
    };
    let epoch = MatrixTimeEpochV1 {
        schema_version: MatrixTimeEpochV1::SCHEMA_VERSION,
        anchor_semantic_revision: to_u64(raw.0)?,
        anchor_state_digest: digest_from_vec(
            bounded_typed_value(
                raw.1,
                raw.2,
                32,
                "semantic_cursor.time_anchor_state_digest",
                "semantic_time_anchor_state_type",
            )?,
            "semantic_time.anchor_state",
        )?,
        awake_ticks: to_u64(raw.3)?,
        drowsy_ticks: to_u64(raw.4)?,
        asleep_ticks: to_u64(raw.5)?,
        awake_remainder_ms: to_u64(raw.6)?,
        drowsy_remainder_ms: to_u64(raw.7)?,
        asleep_remainder_ms: to_u64(raw.8)?,
    };
    if epoch.anchor_semantic_revision > fallback_revision
        || epoch.awake_remainder_ms >= ae_semantic_core::MATRIX_TIME_QUANTUM_MS
        || epoch.drowsy_remainder_ms >= ae_semantic_core::MATRIX_TIME_QUANTUM_MS
        || epoch.asleep_remainder_ms >= ae_semantic_core::MATRIX_TIME_QUANTUM_MS
    {
        return Err(StoreError::ContinuityFence("semantic_time_epoch"));
    }
    Ok((true, epoch))
}

fn semantic_anchor_field_v1(
    conn: &Connection,
    persona_scope: Digest,
    epoch: &MatrixTimeEpochV1,
    origin: &SemanticOriginV1,
    identity: &ActiveSemanticIdentityV1,
) -> Result<NeuralField, StoreError> {
    semantic_time_anchor_state_v1(conn, persona_scope, epoch, origin, identity)
        .map(|(field, _)| field)
}

/// Insert the evidence-free semantic half of a TimeAdvance inside the caller's
/// already-open wake transaction. No value supplied by Runtime can select the
/// sleep phase, matrix field, epoch, formula, graph or resulting state.
pub(crate) fn commit_time_semantic_tx_v1(
    tx: &Transaction<'_>,
    event: &TimeAdvanceV1,
    autonomy_before: &AutonomousRuntimeStateV1,
    autonomy_after: &AutonomousRuntimeStateV1,
    journal_revision: u64,
    journal_delta_digest: Digest,
) -> Result<Option<u64>, StoreError> {
    if event.frozen.effective_now_utc_ms != event.frozen.observed_now_utc_ms {
        return Err(StoreError::SemanticInvalid("time_effective_not_observed"));
    }
    let raw_elapsed_ms = event
        .frozen
        .effective_now_utc_ms
        .checked_sub(autonomy_before.last_advanced_at_utc_ms)
        .ok_or(StoreError::SemanticInvalid("time_clock_rollback"))?;
    if raw_elapsed_ms == 0 {
        return Ok(None);
    }
    let persona_scope =
        wire::persona_scope_digest(&event.scope.bot_token, &event.scope.persona_token, None);
    if autonomy_before.persona_scope != persona_scope
        || autonomy_after.persona_scope != persona_scope
        || event.scope.relation_token.is_some()
    {
        return Err(StoreError::SemanticInvalid("time_persona_scope"));
    }
    let canonical_event = CanonicalEvent::TimeAdvance(event.clone());
    let event_bytes = wire::encode_event_checked(&canonical_event)
        .map_err(|_| StoreError::SemanticInvalid("time_event_wire"))?;
    let event_digest = wire::event_digest(&canonical_event);
    let journal =
        query_bounded_journal_row(tx, &persona_scope, JournalRevision::new(journal_revision))?
            .ok_or(StoreError::ContinuityFence("semantic_time_journal_missing"))?;
    if journal.event_bytes != event_bytes || journal.event_digest != event_digest {
        return Err(StoreError::ContinuityFence("semantic_time_journal_binding"));
    }

    let identity = active_identity_for_scope_tx(tx, &event.scope)?;
    let route_digest = phase0_semantic_route_digest_v1();
    let formula_digest = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let origin = derive_origin_tx(
        tx,
        &event.scope,
        persona_scope,
        identity.incarnation_id,
        identity.manifest_digest,
        route_digest,
        formula_digest,
        &identity,
    )?;
    let (semantic_base_revision, current_field, graph) =
        semantic_state_for_derivation_tx(tx, persona_scope, &origin, &identity)?;
    let state_before = state_digest(&current_field, &formula_digest);
    let graph_state = graph_digest(&graph);
    let (had_cursor, epoch_before) =
        semantic_cursor_epoch_v1(tx, persona_scope, semantic_base_revision, state_before)?;
    let anchor_field =
        semantic_anchor_field_v1(tx, persona_scope, &epoch_before, &origin, &identity)?;
    let advanced = advance_matrix_time_v1(MatrixTimeInputV1 {
        anchor_field: &anchor_field,
        genesis_baseline: &identity.baseline_field,
        epoch: &epoch_before,
        elapsed_ms: raw_elapsed_ms,
        phase: matrix_phase_from_sleep_v1(autonomy_before.sleep_state),
        semantic_formula_digest: formula_digest,
    })
    .map_err(semantic_core_input_error)?;
    let semantic_revision =
        semantic_base_revision
            .checked_add(1)
            .ok_or(StoreError::RevisionOutOfRange {
                revision: semantic_base_revision,
            })?;
    JournalRevision::new(semantic_revision).to_sqlite()?;
    let state_after = state_digest(&advanced.field, &formula_digest);
    let time_formula_digest = matrix_time_formula_digest_v1(&formula_digest);
    let autonomy_state_before = autonomy_state_digest_v1(autonomy_before)?;
    let autonomy_state_after = autonomy_state_digest_v1(autonomy_after)?;
    let authority = SemanticTimeAuthorityV1 {
        schema_version: SemanticTimeAuthorityV1::SCHEMA_VERSION,
        persona_scope,
        semantic_base_revision,
        semantic_revision,
        journal_revision,
        event_id: event.event_id,
        event_digest,
        pre_sleep_phase: matrix_phase_from_sleep_v1(autonomy_before.sleep_state),
        interval_start_utc_ms: autonomy_before.last_advanced_at_utc_ms,
        interval_end_utc_ms: event.frozen.effective_now_utc_ms,
        raw_elapsed_ms,
        applied_elapsed_ms: advanced.applied_elapsed_ms,
        capped_gap: advanced.capped_gap,
        incarnation_id: identity.incarnation_id,
        manifest_digest: identity.manifest_digest,
        route_digest,
        semantic_formula_digest: formula_digest,
        time_formula_digest,
        state_before,
        state_after,
        graph_digest: graph_state,
        epoch_before: epoch_before.clone(),
        epoch_after: advanced.epoch.clone(),
        autonomy_state_before,
        autonomy_state_after,
        journal_delta_digest,
    };
    if !authority.validate_v1() {
        return Err(StoreError::ContinuityFence("semantic_time_authority"));
    }
    let authority_bytes = serde_json::to_vec(&authority)
        .map_err(|_| StoreError::ContinuityFence("semantic_time_authority_wire"))?;
    enforce_byte_budget(
        "semantic.time_authority_bytes",
        u64::try_from(authority_bytes.len()).unwrap_or(u64::MAX),
        16_384,
    )?;
    let time_authority_digest = wire::domain_hash(
        b"astr-embodiment/semantic-time-authority-v1",
        &[&authority_bytes],
    );
    let event_authority = ae_authority::authority_projection_digest(&canonical_event);
    let commitment_digest = time_commitment_digest_v1(
        &persona_scope,
        semantic_revision,
        journal_revision,
        &event.event_id,
        &event_digest,
        &identity.incarnation_id,
        &identity.manifest_digest,
        &route_digest,
        &formula_digest,
        &state_before,
        &state_after,
        &graph_state,
        &event_authority,
        &origin.origin_digest,
        &time_authority_digest,
    );
    let snapshot_bytes = encode_time_snapshot_v1(&DecodedTimeSnapshotV1 {
        semantic_formula_digest: formula_digest,
        time_formula_digest,
        epoch_before: epoch_before.clone(),
        epoch_after: advanced.epoch.clone(),
        requested_elapsed_ms: raw_elapsed_ms,
        applied_elapsed_ms: advanced.applied_elapsed_ms,
        capped_gap: advanced.capped_gap,
        pre_sleep_phase: matrix_phase_from_sleep_v1(autonomy_before.sleep_state),
        state_before,
        state_after,
        graph_digest: graph_state,
        authority_digest: event_authority,
        commitment_digest,
    })
    .map_err(semantic_core_input_error)?;
    enforce_byte_budget(
        "semantic.snapshot_bytes",
        u64::try_from(snapshot_bytes.len()).unwrap_or(u64::MAX),
        1_024,
    )?;
    let snapshot_wire_digest =
        wire::domain_hash(SEMANTIC_SNAPSHOT_WIRE_DOMAIN_V1, &[&snapshot_bytes]);
    let budget = preflight_semantic_budget_tx(
        tx,
        persona_scope,
        origin.source_revision,
        semantic_base_revision,
        [
            u64::try_from(snapshot_bytes.len()).unwrap_or(u64::MAX),
            0,
            0,
            0,
            u64::try_from(authority_bytes.len()).unwrap_or(u64::MAX),
        ],
    )?;
    insert_origin_tx(tx, &origin)?;
    let semantic_sql = JournalRevision::new(semantic_revision).to_sqlite()?.get();
    let journal_sql = JournalRevision::new(journal_revision).to_sqlite()?.get();
    let zero = [0_u8; 32];
    tx.execute(
        "INSERT INTO semantic_commits(
            persona_scope,semantic_revision,journal_revision,relation_present,relation_scope,
            event_id,event_digest,evidence_digest,estimator_digest,incarnation_id,manifest_digest,
            route_digest,formula_digest,state_before,state_digest,graph_before,graph_digest,
            authority_digest,telemetry_digest,snapshot_wire_digest,graph_wire_digest,
            receipt_wire_digest,telemetry_wire_digest,origin_digest,commitment_digest,transition_kind
         ) VALUES(?1,?2,?3,0,?1,?4,?5,?6,?6,?7,?8,?9,?10,?11,?12,?13,?13,
                  ?14,?6,?15,?16,?6,?6,?17,?18,'time')",
        params![
            blob(persona_scope), semantic_sql, journal_sql, blob(event.event_id), blob(event_digest),
            blob(zero), blob(identity.incarnation_id), blob(identity.manifest_digest),
            blob(route_digest), blob(formula_digest), blob(state_before), blob(state_after),
            blob(graph_state), blob(event_authority), blob(snapshot_wire_digest),
            blob(zero), blob(origin.origin_digest), blob(commitment_digest),
        ],
    )?;
    let insert_common = |table: &str, column: &str, payload: &[u8]| -> Result<(), StoreError> {
        let sql = format!(
            "INSERT INTO {table}(
                persona_scope,semantic_revision,incarnation_id,manifest_digest,route_digest,
                formula_digest,graph_digest,state_digest,evidence_digest,estimator_digest,
                event_digest,commitment_digest,{column}
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?9,?10,?11,?12)"
        );
        tx.execute(
            &sql,
            params![
                blob(persona_scope),
                semantic_sql,
                blob(identity.incarnation_id),
                blob(identity.manifest_digest),
                blob(route_digest),
                blob(formula_digest),
                blob(graph_state),
                blob(state_after),
                blob(zero),
                blob(event_digest),
                blob(commitment_digest),
                payload,
            ],
        )?;
        Ok(())
    };
    insert_common("semantic_snapshots", "snapshot_bytes", &snapshot_bytes)?;
    tx.execute(
        "INSERT INTO semantic_time_authority(
            persona_scope,semantic_revision,journal_revision,event_id,event_digest,
            authority_digest,authority_bytes
         ) VALUES(?1,?2,?3,?4,?5,?6,?7)",
        params![
            blob(persona_scope),
            semantic_sql,
            journal_sql,
            blob(event.event_id),
            blob(event_digest),
            blob(time_authority_digest),
            authority_bytes,
        ],
    )?;
    let epoch = &advanced.epoch;
    let epoch_values = [
        epoch.anchor_semantic_revision,
        epoch.awake_ticks,
        epoch.drowsy_ticks,
        epoch.asleep_ticks,
        epoch.awake_remainder_ms,
        epoch.drowsy_remainder_ms,
        epoch.asleep_remainder_ms,
    ]
    .map(|value| JournalRevision::new(value).to_sqlite().map(|sql| sql.get()))
    .into_iter()
    .collect::<Result<Vec<_>, _>>()?;
    let changed = if had_cursor {
        tx.execute(
            "UPDATE semantic_cursor SET
               semantic_revision=?2,journal_revision=?3,incarnation_id=?4,manifest_digest=?5,
               route_digest=?6,formula_digest=?7,graph_digest=?8,state_digest=?9,
               evidence_digest=?10,estimator_digest=?10,event_digest=?11,commitment_digest=?12,
               transition_kind='time',time_anchor_semantic_revision=?13,
               time_anchor_state_digest=?14,time_awake_ticks=?15,time_drowsy_ticks=?16,
               time_asleep_ticks=?17,time_awake_remainder_ms=?18,
               time_drowsy_remainder_ms=?19,time_asleep_remainder_ms=?20
             WHERE persona_scope=?1 AND semantic_revision=?21",
            params![
                blob(persona_scope),
                semantic_sql,
                journal_sql,
                blob(identity.incarnation_id),
                blob(identity.manifest_digest),
                blob(route_digest),
                blob(formula_digest),
                blob(graph_state),
                blob(state_after),
                blob(zero),
                blob(event_digest),
                blob(commitment_digest),
                epoch_values[0],
                blob(epoch.anchor_state_digest),
                epoch_values[1],
                epoch_values[2],
                epoch_values[3],
                epoch_values[4],
                epoch_values[5],
                epoch_values[6],
                JournalRevision::new(semantic_base_revision)
                    .to_sqlite()?
                    .get(),
            ],
        )?
    } else {
        tx.execute(
            "INSERT INTO semantic_cursor(
               persona_scope,semantic_revision,journal_revision,incarnation_id,manifest_digest,
               route_digest,formula_digest,graph_digest,state_digest,evidence_digest,
               estimator_digest,event_digest,commitment_digest,transition_kind,
               time_anchor_semantic_revision,time_anchor_state_digest,time_awake_ticks,
               time_drowsy_ticks,time_asleep_ticks,time_awake_remainder_ms,
               time_drowsy_remainder_ms,time_asleep_remainder_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,?11,?12,'time',
                      ?13,?14,?15,?16,?17,?18,?19,?20)",
            params![
                blob(persona_scope),
                semantic_sql,
                journal_sql,
                blob(identity.incarnation_id),
                blob(identity.manifest_digest),
                blob(route_digest),
                blob(formula_digest),
                blob(graph_state),
                blob(state_after),
                blob(zero),
                blob(event_digest),
                blob(commitment_digest),
                epoch_values[0],
                blob(epoch.anchor_state_digest),
                epoch_values[1],
                epoch_values[2],
                epoch_values[3],
                epoch_values[4],
                epoch_values[5],
                epoch_values[6],
            ],
        )?
    };
    if changed != 1 {
        return Err(StoreError::ContinuityFence("semantic_time_cursor_cas"));
    }
    advance_semantic_budget_checkpoint_tx(
        tx,
        persona_scope,
        semantic_revision,
        commitment_digest,
        budget,
    )?;
    Ok(Some(semantic_revision))
}

fn commit_semantic_candidate_tx(
    tx: &Transaction<'_>,
    untrusted_candidate: &PairedSemanticCommitV1,
    #[cfg(test)] fault: Option<SemanticFaultPoint>,
) -> Result<SemanticTransactionOutcome, StoreError> {
    let (candidate, derived) = derive_candidate_tx(tx, untrusted_candidate)?;
    let candidate = &candidate;
    // Identity lookup deliberately precedes both CAS checks. A match must also
    // match the complete Store-derived sidecar commitment.
    let duplicate: Option<(i64, Vec<u8>, Vec<u8>)> = tx
        .query_row(
            "SELECT semantic_revision,
                CASE WHEN typeof(event_digest)='blob' AND length(event_digest)=32 THEN event_digest ELSE zeroblob(0) END,
                CASE WHEN typeof(commitment_digest)='blob' AND length(commitment_digest)=32 THEN commitment_digest ELSE zeroblob(0) END
             FROM semantic_evidence_authority
             WHERE persona_scope=?1 AND relation_present=?2 AND relation_scope=?3 AND event_id=?4",
            params![
                blob(candidate.persona_scope),
                i64::from(derived.relation_present),
                blob(derived.relation_storage_scope),
                blob(candidate.event_id),
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    if let Some((revision, event, commitment)) = duplicate {
        let revision = semantic_revision_from_sql(revision)?;
        let event = digest_from_vec(event, "semantic_evidence.event")?;
        let commitment = digest_from_vec(commitment, "semantic_evidence.commitment")?;
        if event != candidate.event_digest || commitment != derived.commitment_digest {
            return Err(StoreError::SemanticIdentityConflict);
        }
        let (head_revision, _, _, _) = semantic_head_tx(tx, candidate.persona_scope)?
            .ok_or(StoreError::ContinuityFence("semantic_cursor_missing"))?;
        if head_revision < revision {
            return Err(StoreError::ContinuityFence("semantic_cursor_order"));
        }
        return Ok(SemanticTransactionOutcome::Existing(read_semantic_commit(
            tx,
            candidate.persona_scope,
            revision,
        )?));
    }

    let had_cursor = validate_semantic_cas_tx(tx, candidate, &derived)?;
    let budget = preflight_semantic_budget_tx(
        tx,
        candidate.persona_scope,
        derived.origin.source_revision,
        candidate.semantic_base_revision,
        [
            u64::try_from(candidate.snapshot_bytes.len()).unwrap_or(u64::MAX),
            u64::try_from(derived.graph_bytes.len()).unwrap_or(u64::MAX),
            u64::try_from(candidate.receipt_bytes.len()).unwrap_or(u64::MAX),
            u64::try_from(candidate.telemetry_bytes.len()).unwrap_or(u64::MAX),
            u64::try_from(candidate.journal.event_bytes.len()).unwrap_or(u64::MAX),
        ],
    )?;
    let journal_prepared = Store::prepare_journal_commit_tx(
        tx,
        &candidate.journal,
        JournalCommitLane::PairedSemantic,
    )?;
    if journal_prepared.revision.get() != candidate.journal.receipt.next_revision
        || journal_prepared.event_digest != candidate.event_digest
    {
        return Err(StoreError::SemanticInvalid("journal_semantic_binding"));
    }
    Store::insert_journal_commit_tx(tx, &candidate.journal, &journal_prepared)?;

    #[cfg(test)]
    if fault == Some(SemanticFaultPoint::AfterJournalInsert) {
        return Err(StoreError::SemanticTestFault("after_journal_insert"));
    }

    insert_origin_tx(tx, &derived.origin)?;
    insert_semantic_rows_tx(tx, candidate, &derived, journal_prepared.revision.get())?;
    advance_semantic_cursor_tx(
        tx,
        candidate,
        &derived,
        journal_prepared.revision.get(),
        had_cursor,
    )?;
    advance_semantic_budget_checkpoint_tx(
        tx,
        candidate.persona_scope,
        derived.semantic_revision,
        derived.commitment_digest,
        budget,
    )?;

    #[cfg(test)]
    if fault == Some(SemanticFaultPoint::AfterSemanticInsert) {
        return Err(StoreError::SemanticTestFault("after_semantic_insert"));
    }

    Ok(SemanticTransactionOutcome::Inserted {
        persona_scope: candidate.persona_scope,
        semantic_revision: derived.semantic_revision,
    })
}

fn reply_affect_for_persona_tx_v1(
    conn: &Connection,
    persona_scope: Digest,
) -> Result<Option<ReplyAffectV1>, StoreError> {
    let Some(input) = attested_affect_projection_input_tx_v1(conn, persona_scope)? else {
        return Ok(None);
    };
    crate::alpha3::projection::build_reply_affect_v1(input).map(Some)
}

fn semantic_appraisal_reserved_tokens_tx_v1(
    conn: &Connection,
    scope: &ScopeRef,
    request_nonce_digest: Digest,
) -> Result<u32, StoreError> {
    let raw: (Vec<u8>, Vec<u8>, i64, Option<i64>) = conn
        .query_row(
            "SELECT persona_scope,origin_event_digest,reserved_tokens,settled_at_ms
             FROM semantic_appraisal_claim WHERE request_nonce_digest=?1",
            params![blob(request_nonce_digest)],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or(StoreError::SemanticInvalid(
            "semantic_appraisal_claim_missing",
        ))?;
    let persona_scope = digest_from_vec(raw.0, "semantic_appraisal.persona")?;
    let expected_persona = wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
    let origin_event = digest_from_vec(raw.1, "semantic_appraisal.origin_event")?;
    let origin = committed_perception_origin_v1(conn, origin_event, Some(persona_scope))?.origin;
    if persona_scope != expected_persona || origin.scope != *scope || raw.3.is_some() {
        return Err(StoreError::SemanticInvalid(
            "semantic_appraisal_claim_consumed",
        ));
    }
    u32::try_from(raw.2)
        .map_err(|_| StoreError::ContinuityFence("semantic_appraisal_claim_reserved"))
}

impl Store {
    pub fn reply_affect_v1(&self, scope: &ScopeRef) -> Result<Option<ReplyAffectV1>, StoreError> {
        if !valid_perception_scope(scope) {
            return Err(StoreError::SemanticInvalid("reply_affect_scope"));
        }
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        reply_affect_for_persona_tx_v1(self.connection()?, persona_scope)
    }

    /// Host-internal authority bridge. `origin_event_digest` must identify an
    /// already committed `InboundObserved/AstrbotMetadata` interaction fact.
    /// No FFI entry point exposes this method: the unavoidable trust root is
    /// the Host adapter that constructs and commits that interaction batch.
    #[cfg(any(test, feature = "legacy-semantic-test-api"))]
    pub fn mint_perception_challenge_from_committed_inbound_v1(
        &mut self,
        origin_event_digest: Digest,
    ) -> Result<PerceptionChallengeV1, StoreError> {
        if !is_nonzero(&origin_event_digest) {
            return Err(StoreError::SemanticInvalid("perception_origin_identity"));
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let authoritative_now_ms = now_ms();
        cleanup_perception_challenges_v1(&tx, authoritative_now_ms)?;
        let attested_origin = committed_perception_origin_v1(&tx, origin_event_digest, None)?;
        if attested_origin.current_journal_revision
            != attested_origin.origin.origin_journal_revision
        {
            return Err(StoreError::SemanticInvalid("perception_origin_not_current"));
        }
        let origin = attested_origin.origin;
        let existing_nonce: Option<Vec<u8>> = tx
            .query_row(
                "SELECT request_nonce_digest FROM perception_challenges
                 WHERE origin_digest=?1 AND base_revision=?2 AND incarnation_id=?3",
                params![
                    blob(origin.origin_digest),
                    JournalRevision::new(origin.canonical_base_revision)
                        .to_sqlite()?
                        .get(),
                    blob(origin.incarnation_id),
                ],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing_nonce) = existing_nonce {
            let existing_nonce = digest_from_vec(existing_nonce, "perception_challenge.nonce")?;
            let existing = read_perception_challenge_v1(&tx, existing_nonce)?
                .ok_or(StoreError::ContinuityFence("perception_challenge_missing"))?;
            if existing.origin != origin || existing.expires_at_ms <= authoritative_now_ms {
                return Err(StoreError::ContinuityFence("perception_challenge_origin"));
            }
            tx.commit()?;
            return Ok(PerceptionChallengeV1 {
                schema_version: PerceptionChallengeV1::SCHEMA_VERSION,
                origin,
                created_at_ms: existing.created_at_ms,
                expires_at_ms: existing.expires_at_ms,
                request_nonce_digest: existing_nonce,
            });
        }
        enforce_perception_quota_v1(&tx, origin.persona_scope)?;
        let mut secret = [0_u8; 32];
        getrandom::fill(&mut secret).map_err(|_| StoreError::PerceptionEntropyUnavailable)?;
        if !is_nonzero(&secret) {
            return Err(StoreError::PerceptionEntropyUnavailable);
        }
        let secret_commitment = perception_challenge_secret_commitment_v1(&secret);
        let expires_at_ms = authoritative_now_ms
            .checked_add(PERCEPTION_CHALLENGE_TTL_MS)
            .ok_or(StoreError::RevisionOutOfRange {
                revision: authoritative_now_ms,
            })?;
        let request_nonce_digest = perception_challenge_nonce_v1(
            &secret_commitment,
            &origin.origin_digest,
            authoritative_now_ms,
            expires_at_ms,
        );
        if !is_nonzero(&request_nonce_digest) {
            return Err(StoreError::PerceptionEntropyUnavailable);
        }
        let origin_bytes = serde_json::to_vec(&origin)
            .map_err(|_| StoreError::ContinuityFence("perception_origin_wire"))?;
        enforce_byte_budget(
            "perception_challenge.origin_bytes",
            u64::try_from(origin_bytes.len()).unwrap_or(u64::MAX),
            MAX_PERCEPTION_ORIGIN_BYTES,
        )?;
        let inserted = tx.execute(
            "INSERT INTO perception_challenges(
                request_nonce_digest,challenge_secret,origin_digest,origin_bytes,persona_scope,
                bot_token,persona_token,origin_event_digest,origin_journal_revision,base_revision,
                incarnation_id,manifest_digest,created_at_ms,expires_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
            params![
                blob(request_nonce_digest),
                blob(secret_commitment),
                blob(origin.origin_digest),
                origin_bytes,
                blob(origin.persona_scope),
                blob(origin.scope.bot_token),
                blob(origin.scope.persona_token),
                blob(origin.origin_event_digest),
                JournalRevision::new(origin.origin_journal_revision)
                    .to_sqlite()?
                    .get(),
                JournalRevision::new(origin.canonical_base_revision)
                    .to_sqlite()?
                    .get(),
                blob(origin.incarnation_id),
                blob(origin.manifest_digest),
                i64::try_from(authoritative_now_ms).map_err(|_| {
                    StoreError::RevisionOutOfRange {
                        revision: authoritative_now_ms,
                    }
                })?,
                i64::try_from(expires_at_ms).map_err(|_| StoreError::RevisionOutOfRange {
                    revision: expires_at_ms
                })?,
            ],
        )?;
        if inserted != 1 {
            return Err(StoreError::ContinuityFence("perception_challenge_insert"));
        }
        let stored = read_perception_challenge_v1(&tx, request_nonce_digest)?
            .ok_or(StoreError::ContinuityFence("perception_challenge_missing"))?;
        let expected = perception_challenge_nonce_v1(
            &stored.secret_commitment,
            &stored.origin.origin_digest,
            stored.created_at_ms,
            stored.expires_at_ms,
        );
        if stored.origin != origin
            || !constant_time_eq::constant_time_eq_n(&request_nonce_digest, &expected)
        {
            return Err(StoreError::ContinuityFence("perception_challenge_nonce"));
        }
        tx.commit()?;
        Ok(PerceptionChallengeV1 {
            schema_version: PerceptionChallengeV1::SCHEMA_VERSION,
            origin,
            created_at_ms: authoritative_now_ms,
            expires_at_ms,
            request_nonce_digest,
        })
    }

    /// Authenticate and atomically consume one Store-minted challenge while
    /// appending the canonical UserStimulus and its complete semantic sidecar.
    #[cfg(any(test, feature = "legacy-semantic-test-api"))]
    pub fn commit_perception_proposal_v1(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
    ) -> Result<SemanticCommitResultV1, StoreError> {
        #[cfg(test)]
        {
            self.commit_perception_proposal_inner_v1(scope, proposal, None, None)
                .map(|(result, _, _)| result)
        }
        #[cfg(not(test))]
        {
            self.commit_perception_proposal_inner_v1(scope, proposal, None)
                .map(|(result, _, _)| result)
        }
    }

    /// Consume one durable appraisal claim exactly once.  Success may append
    /// semantics only when the proposal is closed and Provider accounting is
    /// known within the reservation; every other expected outcome atomically
    /// charges the claim while leaving semantic state untouched.
    pub fn settle_semantic_appraisal_v1(
        &mut self,
        request: &SemanticAppraisalSettleRequestV1,
    ) -> Result<SemanticAppraisalSettlementStoreOutcomeV1, StoreError> {
        if !request.validate_v1() || !valid_perception_scope(&request.scope) {
            return Err(StoreError::SemanticInvalid("semantic_appraisal_settlement"));
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let wall_now_ms = now_ms();
        preflight_semantic_appraisal_storage_v1(&tx, SemanticAppraisalStorageLayoutV1::V3)?;
        let authoritative_now_ms = semantic_appraisal_authoritative_now_tx_v1(&tx, wall_now_ms)?;
        let target_mutated = maintain_semantic_appraisal_target_nonce_v1(
            &tx,
            request.request_nonce_digest,
            authoritative_now_ms,
        )?;
        maintain_semantic_appraisal_retention_v1(
            &tx,
            authoritative_now_ms,
            u64::from(target_mutated),
        )?;
        preflight_semantic_appraisal_storage_v1(&tx, SemanticAppraisalStorageLayoutV1::V3)?;
        if target_mutated {
            tx.commit()?;
            return Ok(SemanticAppraisalSettlementStoreOutcomeV1::RetryExpiredOrUnknown);
        }

        let terminal = terminal_semantic_appraisal_v1(&tx, request, authoritative_now_ms)?;
        if let Some(terminal) = terminal {
            let result = if let Some(semantic_revision) = terminal.receipt.semantic_revision {
                let proposal = request
                    .proposal
                    .as_ref()
                    .filter(|proposal| {
                        request.outcome == SemanticAppraisalOutcomeV1::Success
                            && proposal.request_nonce_digest == request.request_nonce_digest
                            && proposal.validate_v1().is_ok()
                    })
                    .ok_or(StoreError::SemanticInvalid(
                        "semantic_appraisal_claim_consumed",
                    ))?;
                let committed = exact_perception_retry_v1(&tx, &request.scope, proposal)?.ok_or(
                    StoreError::ContinuityFence("semantic_appraisal_retry_receipt"),
                )?;
                if committed.semantic_revision != semantic_revision
                    || committed.journal_revision != terminal.receipt.canonical_revision
                {
                    return Err(StoreError::ContinuityFence(
                        "semantic_appraisal_retry_receipt",
                    ));
                }
                SemanticAppraisalStoreSettlementV1::Committed {
                    result: SemanticCommitResultV1 {
                        disposition: SemanticCommitDispositionV1::Existing,
                        committed,
                    },
                    charged_tokens: terminal.receipt.charged_tokens,
                    reply_affect: terminal.reply_affect,
                }
            } else {
                SemanticAppraisalStoreSettlementV1::ZeroMutation {
                    canonical_revision: terminal.receipt.canonical_revision,
                    charged_tokens: terminal.receipt.charged_tokens,
                    reply_affect: terminal.reply_affect,
                }
            };
            preflight_semantic_appraisal_storage_v1(&tx, SemanticAppraisalStorageLayoutV1::V3)?;
            tx.commit()?;
            return Ok(SemanticAppraisalSettlementStoreOutcomeV1::Completed(result));
        }

        let pending_claim: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM semantic_appraisal_claim
             WHERE request_nonce_digest=?1 AND settled_at_ms IS NULL)",
            params![blob(request.request_nonce_digest)],
            |row| row.get(0),
        )?;
        if !pending_claim {
            preflight_semantic_appraisal_storage_v1(&tx, SemanticAppraisalStorageLayoutV1::V3)?;
            tx.commit()?;
            return Ok(SemanticAppraisalSettlementStoreOutcomeV1::RetryExpiredOrUnknown);
        }

        if request.outcome == SemanticAppraisalOutcomeV1::Success {
            let proposal = request
                .proposal
                .as_ref()
                .ok_or(StoreError::SemanticInvalid(
                    "semantic_appraisal_proposal_missing",
                ))?;
            let proposal_is_closed = proposal.request_nonce_digest == request.request_nonce_digest
                && proposal.validate_v1().is_ok();
            let reserved_tokens = semantic_appraisal_reserved_tokens_tx_v1(
                &tx,
                &request.scope,
                request.request_nonce_digest,
            )?;
            let usage_is_authoritative = matches!(
                (request.provider_usage.known, request.provider_usage.used_tokens),
                (true, Some(actual)) if actual <= reserved_tokens
            );
            if proposal_is_closed && usage_is_authoritative {
                #[cfg(test)]
                let (result, charged_tokens, reply_affect) =
                    Self::commit_perception_proposal_tx_v1(
                        &tx,
                        &request.scope,
                        proposal,
                        Some(&request.provider_usage),
                        authoritative_now_ms,
                        None,
                    )?;
                #[cfg(not(test))]
                let (result, charged_tokens, reply_affect) =
                    Self::commit_perception_proposal_tx_v1(
                        &tx,
                        &request.scope,
                        proposal,
                        Some(&request.provider_usage),
                        authoritative_now_ms,
                    )?;
                let charged_tokens = charged_tokens.ok_or(StoreError::ContinuityFence(
                    "semantic_appraisal_charge_missing",
                ))?;
                preflight_semantic_appraisal_storage_v1(&tx, SemanticAppraisalStorageLayoutV1::V3)?;
                tx.commit()?;
                return Ok(SemanticAppraisalSettlementStoreOutcomeV1::Completed(
                    SemanticAppraisalStoreSettlementV1::Committed {
                        result,
                        charged_tokens,
                        reply_affect,
                    },
                ));
            }
        }
        let persona_scope = wire::persona_scope_digest(
            &request.scope.bot_token,
            &request.scope.persona_token,
            None,
        );
        let current_raw: i64 = tx.query_row(
            "SELECT COALESCE(MAX(logical_revision),0) FROM journal WHERE scope_digest=?1",
            params![blob(persona_scope)],
            |row| row.get(0),
        )?;
        let canonical_revision = JournalRevision::try_from(current_raw)?.get();
        let reply_affect = reply_affect_for_persona_tx_v1(&tx, persona_scope)?;
        let challenge_token =
            closed_perception_challenge_token_v1(&tx, request.request_nonce_digest)?
                .ok_or(StoreError::SemanticInvalid("perception_challenge_missing"))?;
        let charged_tokens = settle_semantic_appraisal_claim_tx_v1(
            &tx,
            &request.scope,
            request.request_nonce_digest,
            semantic_appraisal_outcome_code_v1(request.outcome),
            &request.provider_usage,
            request.proposal.as_ref(),
            canonical_revision,
            None,
            &reply_affect,
            authoritative_now_ms,
        )?;
        delete_perception_challenge_token_v1(&tx, &challenge_token)?;
        preflight_semantic_appraisal_storage_v1(&tx, SemanticAppraisalStorageLayoutV1::V3)?;
        tx.commit()?;
        Ok(SemanticAppraisalSettlementStoreOutcomeV1::Completed(
            SemanticAppraisalStoreSettlementV1::ZeroMutation {
                canonical_revision,
                charged_tokens,
                reply_affect,
            },
        ))
    }

    #[cfg(test)]
    pub(crate) fn commit_perception_proposal_test_fault_v1(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
        fault: SemanticFaultPoint,
    ) -> Result<SemanticCommitResultV1, StoreError> {
        self.commit_perception_proposal_inner_v1(scope, proposal, None, Some(fault))
            .map(|(result, _, _)| result)
    }

    #[cfg(any(test, feature = "legacy-semantic-test-api"))]
    fn commit_perception_proposal_inner_v1(
        &mut self,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
        appraisal_usage: Option<&SemanticAppraisalProviderUsageV1>,
        #[cfg(test)] fault: Option<SemanticFaultPoint>,
    ) -> Result<(SemanticCommitResultV1, Option<u64>, Option<ReplyAffectV1>), StoreError> {
        proposal
            .validate_v1()
            .map_err(|_| StoreError::SemanticInvalid("semantic_perception_proposal"))?;
        if !valid_perception_scope(scope) {
            return Err(StoreError::SemanticInvalid("perception_scope"));
        }
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let authoritative_now_ms = now_ms();
        #[cfg(test)]
        let result = Self::commit_perception_proposal_tx_v1(
            &tx,
            scope,
            proposal,
            appraisal_usage,
            authoritative_now_ms,
            fault,
        )?;
        #[cfg(not(test))]
        let result = Self::commit_perception_proposal_tx_v1(
            &tx,
            scope,
            proposal,
            appraisal_usage,
            authoritative_now_ms,
        )?;
        tx.commit()?;
        Ok(result)
    }

    fn commit_perception_proposal_tx_v1(
        tx: &Transaction<'_>,
        scope: &ScopeRef,
        proposal: &PerceptionProposalV1,
        appraisal_usage: Option<&SemanticAppraisalProviderUsageV1>,
        authoritative_now_ms: u64,
        #[cfg(test)] fault: Option<SemanticFaultPoint>,
    ) -> Result<(SemanticCommitResultV1, Option<u64>, Option<ReplyAffectV1>), StoreError> {
        proposal
            .validate_v1()
            .map_err(|_| StoreError::SemanticInvalid("semantic_perception_proposal"))?;
        if !valid_perception_scope(scope) {
            return Err(StoreError::SemanticInvalid("perception_scope"));
        }
        if appraisal_usage.is_none() {
            cleanup_perception_challenges_v1(tx, authoritative_now_ms)?;
        }
        if let Some(committed) = exact_perception_retry_v1(tx, scope, proposal)? {
            let reply_affect = reply_affect_for_persona_tx_v1(tx, committed.persona_scope)?;
            return Ok((
                SemanticCommitResultV1 {
                    disposition: SemanticCommitDispositionV1::Existing,
                    committed,
                },
                None,
                reply_affect,
            ));
        }
        let challenge_token =
            closed_perception_challenge_token_v1(tx, proposal.request_nonce_digest)?
                .ok_or(StoreError::SemanticInvalid("perception_challenge_missing"))?;
        let challenge = challenge_token
            .challenge
            .as_ref()
            .ok_or(StoreError::ContinuityFence("perception_challenge_layout"))?;
        let appraisal_claim_pending: bool = tx.query_row(
            "SELECT EXISTS(
               SELECT 1 FROM semantic_appraisal_claim
               WHERE request_nonce_digest=?1 AND settled_at_ms IS NULL
             )",
            params![blob(proposal.request_nonce_digest)],
            |row| row.get(0),
        )?;
        if appraisal_usage.is_none() && appraisal_claim_pending {
            return Err(StoreError::SemanticInvalid(
                "semantic_appraisal_settlement_required",
            ));
        }
        if appraisal_usage.is_some() && !appraisal_claim_pending {
            return Err(StoreError::SemanticInvalid(
                "semantic_appraisal_claim_missing",
            ));
        }
        if challenge.expires_at_ms <= authoritative_now_ms {
            return Err(StoreError::SemanticInvalid("perception_challenge_expired"));
        }
        let authoritative_origin = committed_perception_origin_v1(
            tx,
            challenge.origin.origin_event_digest,
            Some(challenge.origin.persona_scope),
        )?;
        let commit_base_revision = authoritative_origin.current_journal_revision;
        let expected_nonce = perception_challenge_nonce_v1(
            &challenge.secret_commitment,
            &challenge.origin.origin_digest,
            challenge.created_at_ms,
            challenge.expires_at_ms,
        );
        if !constant_time_eq::constant_time_eq_n(&proposal.request_nonce_digest, &expected_nonce)
            || proposal.origin_digest != challenge.origin.origin_digest
            || challenge.origin != authoritative_origin.origin
            || challenge.origin.scope != *scope
        {
            return Err(StoreError::SemanticInvalid("perception_challenge_context"));
        }
        let estimator_digest = proposal.estimator_digest_v1();
        if !is_nonzero(&estimator_digest) {
            return Err(StoreError::SemanticInvalid("perception_estimator_digest"));
        }
        let origin = &challenge.origin;
        let event = CanonicalEvent::UserStimulus(UserStimulus {
            event_id: origin.event_id,
            scope: origin.scope.clone(),
            causal: CausalRef {
                turn_id: origin.turn_id,
                action_id: None,
                delivery_id: None,
                claim_id: None,
                // The immutable origin remains the committed inbound
                // revision. Native alone rebases the resulting UserStimulus
                // to the journal head held by this immediate transaction.
                base_revision: commit_base_revision,
            },
            observed_at_ms: origin.observed_at_ms,
            evidence: SemanticEstimate {
                schema_version: 1,
                dimensions: proposal.dimensions.clone(),
                estimator_confidence: proposal.estimator_confidence,
                estimator_digest,
            },
        });
        let candidate = match prepare_production_semantic_tx(tx, &event)? {
            ProductionSemanticPreparation::Existing(_) => {
                return Err(StoreError::SemanticIdentityConflict)
            }
            ProductionSemanticPreparation::Candidate(candidate) => candidate,
        };
        #[cfg(test)]
        let outcome = commit_semantic_candidate_tx(tx, &candidate, fault)?;
        #[cfg(not(test))]
        let outcome = commit_semantic_candidate_tx(tx, &candidate)?;
        let (committed_scope, semantic_revision) = match outcome {
            SemanticTransactionOutcome::Existing(_) => {
                return Err(StoreError::SemanticIdentityConflict)
            }
            SemanticTransactionOutcome::Inserted {
                persona_scope,
                semantic_revision,
            } => (persona_scope, semantic_revision),
        };
        let proposal_digest = proposal.estimator_digest_v1();
        let bound = tx.execute(
            "UPDATE semantic_evidence_authority SET
                perception_nonce_digest=?3,perception_proposal_digest=?4,
                perception_origin_digest=?5,perception_origin_bytes=?6
             WHERE persona_scope=?1 AND semantic_revision=?2
               AND perception_nonce_digest IS NULL",
            params![
                blob(committed_scope),
                JournalRevision::new(semantic_revision).to_sqlite()?.get(),
                blob(proposal.request_nonce_digest),
                blob(proposal_digest),
                blob(challenge.origin.origin_digest),
                challenge.origin_bytes.clone(),
            ],
        )?;
        if bound != 1 {
            return Err(StoreError::ContinuityFence("perception_evidence_binding"));
        }
        let committed = read_semantic_commit(tx, committed_scope, semantic_revision)?;
        crate::embodiment_clock::commit_semantic_anchor(tx, &ae_contracts::PersonaScopeRef { bot_token: scope.bot_token, persona_token: scope.persona_token }, &committed)?;
        let reply_affect = reply_affect_for_persona_tx_v1(tx, committed_scope)?;
        let charged_tokens = appraisal_usage
            .map(|usage| {
                settle_semantic_appraisal_claim_tx_v1(
                    tx,
                    scope,
                    proposal.request_nonce_digest,
                    semantic_appraisal_outcome_code_v1(SemanticAppraisalOutcomeV1::Success),
                    usage,
                    Some(proposal),
                    candidate.journal.receipt.next_revision,
                    Some(semantic_revision),
                    &reply_affect,
                    authoritative_now_ms,
                )
            })
            .transpose()?;
        delete_perception_challenge_token_v1(tx, &challenge_token)?;
        let committed = read_semantic_commit(tx, committed_scope, semantic_revision)?;
        Ok((
            SemanticCommitResultV1 {
                disposition: SemanticCommitDispositionV1::Inserted,
                committed,
            },
            charged_tokens,
            reply_affect,
        ))
    }

    /// Store-owned production lane for a canonical user stimulus. The caller
    /// provides no state, graph, cursor, identity, receipt or telemetry. All of
    /// those are authenticated/derived after `BEGIN IMMEDIATE` and committed
    /// with the canonical journal row in the same transaction.
    pub(crate) fn commit_user_stimulus_semantics_v1(
        &mut self,
        event: &CanonicalEvent,
    ) -> Result<CommittedSemanticV1, StoreError> {
        #[cfg(test)]
        {
            self.commit_user_stimulus_semantics_inner_v1(event, None)
        }
        #[cfg(not(test))]
        {
            self.commit_user_stimulus_semantics_inner_v1(event)
        }
    }

    #[cfg(test)]
    pub(crate) fn commit_user_stimulus_semantics_test_fault_v1(
        &mut self,
        event: &CanonicalEvent,
        fault: SemanticFaultPoint,
    ) -> Result<CommittedSemanticV1, StoreError> {
        self.commit_user_stimulus_semantics_inner_v1(event, Some(fault))
    }

    fn commit_user_stimulus_semantics_inner_v1(
        &mut self,
        event: &CanonicalEvent,
        #[cfg(test)] fault: Option<SemanticFaultPoint>,
    ) -> Result<CommittedSemanticV1, StoreError> {
        validated_stimulus(event)?;
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let candidate = match prepare_production_semantic_tx(&tx, event)? {
            ProductionSemanticPreparation::Existing(committed) => return Ok(committed),
            ProductionSemanticPreparation::Candidate(candidate) => candidate,
        };
        #[cfg(test)]
        let outcome = commit_semantic_candidate_tx(&tx, &candidate, fault)?;
        #[cfg(not(test))]
        let outcome = commit_semantic_candidate_tx(&tx, &candidate)?;
        let (persona_scope, semantic_revision) = match outcome {
            SemanticTransactionOutcome::Existing(committed) => return Ok(committed),
            SemanticTransactionOutcome::Inserted {
                persona_scope,
                semantic_revision,
            } => (persona_scope, semantic_revision),
        };
        tx.commit()?;
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        read_semantic_commit(conn, persona_scope, semantic_revision)
    }

    /// Internal-only transition authority. No feature can make the candidate
    /// or this method externally constructible/callable.
    pub(crate) fn commit_event_with_semantic_v1(
        &mut self,
        candidate: &PairedSemanticCommitV1,
    ) -> Result<CommittedSemanticV1, StoreError> {
        self.commit_event_with_semantic_v1_inner(candidate, None)
    }

    #[cfg(test)]
    pub(crate) fn commit_event_with_semantic_test_fault_v1(
        &mut self,
        candidate: &PairedSemanticCommitV1,
        fault: SemanticFaultPoint,
    ) -> Result<CommittedSemanticV1, StoreError> {
        self.commit_event_with_semantic_v1_inner(candidate, Some(fault))
    }

    /// Atomically append one canonical journal event and one persona semantic
    /// transition. Exact retries precede CAS, but only after the complete
    /// stored semantic history has re-attested.
    fn commit_event_with_semantic_v1_inner(
        &mut self,
        untrusted_candidate: &PairedSemanticCommitV1,
        #[cfg(test)] fault: Option<SemanticFaultPoint>,
        #[cfg(not(test))] _fault: Option<()>,
    ) -> Result<CommittedSemanticV1, StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        #[cfg(test)]
        let outcome = commit_semantic_candidate_tx(&tx, untrusted_candidate, fault)?;
        #[cfg(not(test))]
        let outcome = commit_semantic_candidate_tx(&tx, untrusted_candidate)?;
        let (persona_scope, semantic_revision) = match outcome {
            SemanticTransactionOutcome::Existing(committed) => return Ok(committed),
            SemanticTransactionOutcome::Inserted {
                persona_scope,
                semantic_revision,
            } => (persona_scope, semantic_revision),
        };
        tx.commit()?;

        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        read_semantic_commit(conn, persona_scope, semantic_revision)
    }

    pub fn semantic_revision_v1(&self, persona_scope: &Digest) -> Result<u64, StoreError> {
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        Ok(semantic_head(conn, *persona_scope)?
            .map(|(revision, _, _, _)| revision)
            .unwrap_or(0))
    }

    pub fn latest_semantic_v1(
        &self,
        persona_scope: &Digest,
    ) -> Result<Option<CommittedSemanticV1>, StoreError> {
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        let Some((revision, _, _, commitment)) = semantic_head(conn, *persona_scope)? else {
            return Ok(None);
        };
        let persisted = read_semantic_commit(conn, *persona_scope, revision)?;
        if persisted.commitment_digest != commitment {
            return Err(StoreError::ContinuityFence("semantic_cursor_commitment"));
        }
        Ok(Some(persisted))
    }

    /// Reconstruct the current semantic field without requiring time rows to
    /// duplicate the 16K-node field or its graph.  The latest compact epoch is
    /// applied to the authenticated perception/Genesis anchor, so this stays
    /// O(1) in the number of time revisions.
    pub fn hydrated_semantic_state_v1(
        &self,
        scope: &PersonaScopeRef,
    ) -> Result<Option<HydratedSemanticStateV1>, StoreError> {
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        if let Some((field, graph)) = crate::embodiment_clock::semantic_input(conn, scope)? {
            let identity = active_identity_for_scope_tx(conn, &ScopeRef { bot_token: scope.bot_token, persona_token: scope.persona_token, relation_token: None, session_token: [0;16] })?;
            let formula_digest = phase0_canonical_formula_digest_v1(&identity.formula_digest);
            return Ok(Some(HydratedSemanticStateV1 { semantic_revision: semantic_head(conn, persona_scope)?.map(|v|v.0).unwrap_or(0), formula_digest, state_digest: state_digest(&field,&formula_digest), graph_digest: graph_digest(&graph), field, graph }));
        }
        let Some(origin) = stored_origin(conn, persona_scope)? else {
            return Ok(None);
        };
        let identity_scope = ScopeRef {
            bot_token: scope.bot_token,
            persona_token: scope.persona_token,
            relation_token: None,
            session_token: [0; 16],
        };
        let identity = active_identity_for_scope_tx(conn, &identity_scope)?;
        let (semantic_revision, field, graph) =
            semantic_state_for_derivation_tx(conn, persona_scope, &origin, &identity)?;
        let formula_digest = phase0_canonical_formula_digest_v1(&identity.formula_digest);
        let state_digest = state_digest(&field, &formula_digest);
        let graph_digest = graph_digest(&graph);
        if semantic_revision == origin.source_revision {
            if state_digest != origin.state_digest || graph_digest != origin.graph_digest {
                return Err(StoreError::ContinuityFence("semantic_hydrated_origin"));
            }
        } else {
            let (head_revision, head_state, head_graph, _) = semantic_head(conn, persona_scope)?
                .ok_or(StoreError::ContinuityFence("semantic_hydrated_cursor"))?;
            if semantic_revision != head_revision
                || state_digest != head_state
                || graph_digest != head_graph
            {
                return Err(StoreError::ContinuityFence("semantic_hydrated_cursor"));
            }
        }
        Ok(Some(HydratedSemanticStateV1 {
            semantic_revision,
            formula_digest,
            state_digest,
            graph_digest,
            field,
            graph,
        }))
    }

    /// Explicit O(history) integrity audit. Normal appends deliberately use
    /// only the authenticated cursor/checkpoint and direct predecessor; call
    /// this API (or reopen the Store) when historic tamper detection is
    /// required.
    pub fn audit_semantic_integrity_v1(&mut self) -> Result<(), StoreError> {
        let conn = self.conn.as_mut().ok_or(StoreError::Closed)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Deferred)?;
        // Public audit is a read-only snapshot. Resource/canonicality gates are
        // deliberately first so an externally enlarged live table is rejected
        // before any historic walk; audit never advances clocks or repairs it.
        verify_core_semantic_history(&tx)?;
        tx.commit()?;
        Ok(())
    }

    pub fn semantic_origin_v1(
        &self,
        persona_scope: &Digest,
    ) -> Result<Option<SemanticOriginV1>, StoreError> {
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        stored_origin(conn, *persona_scope)
    }

    pub fn semantic_counts(&self) -> Result<(u64, u64, u64, u64), StoreError> {
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        let raw: (i64, i64, i64, i64) = conn.query_row(
            "SELECT
                (SELECT COUNT(*) FROM semantic_snapshots),
                (SELECT COUNT(*) FROM semantic_graphs),
                (SELECT COUNT(*) FROM semantic_receipts),
                (SELECT COUNT(*) FROM semantic_telemetry)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        Ok((
            sqlite_length(raw.0, "semantic_snapshots.rows")?,
            sqlite_length(raw.1, "semantic_graphs.rows")?,
            sqlite_length(raw.2, "semantic_receipts.rows")?,
            sqlite_length(raw.3, "semantic_telemetry.rows")?,
        ))
    }

    pub fn semantic_evidence_count(&self) -> Result<u64, StoreError> {
        let conn = self.conn.as_ref().ok_or(StoreError::Closed)?;
        let raw: i64 = conn.query_row(
            "SELECT COUNT(*) FROM semantic_evidence_authority",
            [],
            |row| row.get(0),
        )?;
        sqlite_length(raw, "semantic_evidence_authority.rows")
    }
}

pub(crate) fn verify_core_semantic_history(conn: &Transaction<'_>) -> Result<(), StoreError> {
    preflight_semantic_appraisal_storage_v1(conn, SemanticAppraisalStorageLayoutV1::V3)?;
    preflight_perception_challenges_v1(conn, PerceptionChallengeStorageLayoutV1::V3)?;
    verify_committed_semantic_migration_rows(conn)?;
    verify_or_backfill_semantic_budget_checkpoints(conn, false)?;
    verify_all_perception_challenges(conn)
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_semantic_schema_objects_v1, checked_semantic_aggregate_bytes,
        enforce_perception_quota_v1, for_each_semantic_time_persona_v5,
        maintain_semantic_appraisal_retention_v1, migrate_schema, next_semantic_persona_scope_v1,
        preflight_semantic_budget_tx, preflight_semantic_history_layout_v1,
        preflight_semantic_v3_history_with_limits_v1, read_semantic_budget_checkpoint_v1,
        scan_semantic_appraisal_phase_one_v1, semantic_appraisal_budget_compaction_leaf_v1,
        semantic_appraisal_claim_capacity_decision_v1, semantic_appraisal_compaction_chain_v1,
        semantic_appraisal_pending_expired_v1, semantic_appraisal_terminal_compaction_leaf_v1,
        semantic_appraisal_terminal_retry_live_v1, verify_semantic_time_rows_per_persona_v5,
        SemanticAppraisalBudgetCompactionRowV1, SemanticAppraisalStorageBudgetV1,
        SemanticAppraisalStorageLayoutV1, SemanticAppraisalTerminalCompactionRowV1,
        SemanticHistoryScanBudgetV1, SemanticHistoryScanLimitsV1, SemanticHistoryScanTableV1,
        MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL, MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA,
        MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA, MAX_SEMANTIC_APPRAISAL_PENDING_PAYLOAD_BYTES,
        MAX_SEMANTIC_GRAPH_BYTES, MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
        MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL, MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
        MAX_SEMANTIC_RECEIPT_BYTES, MAX_SEMANTIC_TELEMETRY_BYTES,
        PERCEPTION_CHALLENGE_PHASE_ONE_V3_SQL, PERCEPTION_CHALLENGE_PHASE_TWO_V3_SQL,
        SEMANTIC_APPRAISAL_BUDGET_COMPACTION_CHAIN_DOMAIN_V1,
        SEMANTIC_APPRAISAL_BUDGET_PHASE_ONE_SCAN_V1_SQL,
        SEMANTIC_APPRAISAL_BUDGET_PHASE_TWO_SCAN_V1_SQL,
        SEMANTIC_APPRAISAL_BUDGET_RETENTION_CANDIDATES_V1_SQL,
        SEMANTIC_APPRAISAL_BUDGET_RETENTION_ELIGIBLE_V1_SQL,
        SEMANTIC_APPRAISAL_CLAIM_COMPACTION_CHAIN_DOMAIN_V1,
        SEMANTIC_APPRAISAL_CLAIM_PHASE_ONE_SCAN_V1_SQL, SEMANTIC_APPRAISAL_CLAIM_SCAN_V2_SQL,
        SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE, SEMANTIC_APPRAISAL_SCHEMA_V2_SQL,
        SEMANTIC_APPRAISAL_SCHEMA_V3_SQL, SEMANTIC_APPRAISAL_UTC_DAY_MS,
        SEMANTIC_HISTORY_SCOPE_TABLES_V1, SEMANTIC_SCHEMA_V3_SQL,
    };
    use crate::{StoreError, MAX_JOURNAL_EVENT_BYTES, MAX_SNAPSHOT_STATE_BYTES};
    use ae_contracts::{
        wire, MatrixSleepPhaseV1, MatrixTimeEpochV1, SemanticAppraisalBeginStatusV1,
        SemanticAppraisalBudgetReceiptV1, SemanticAppraisalCapacityReasonV1,
    };
    use ae_semantic_core::{
        encode_time_snapshot_v1, matrix_time_formula_digest_v1, DecodedTimeSnapshotV1,
        TIME_SNAPSHOT_WIRE_LEN_V1,
    };
    use rusqlite::{limits::Limit, params, Connection, OpenFlags, Transaction};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn measured_oversized_catalog(unrelated_objects: u64) -> (StoreError, usize, usize) {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "writable_schema", "ON")
            .unwrap();
        {
            let transaction = connection.transaction().unwrap();
            let mut insert = transaction
                .prepare(
                    "INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)
                     VALUES('table',?1,?1,0,NULL)",
                )
                .unwrap();
            for ordinal in 0..unrelated_objects {
                let name = format!("unrelated_{ordinal:05}");
                insert.execute(params![name]).unwrap();
            }
            drop(insert);
            transaction.commit().unwrap();
        }
        connection
            .pragma_update(None, "writable_schema", "OFF")
            .unwrap();

        let progress = Arc::new(AtomicUsize::new(0));
        let observed_progress = Arc::clone(&progress);
        connection.progress_handler(
            1,
            Some(move || {
                observed_progress.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
        let writes = Arc::new(AtomicUsize::new(0));
        let observed_writes = Arc::clone(&writes);
        connection.update_hook(Some(
            move |_: rusqlite::hooks::Action, _: &str, _: &str, _: i64| {
                observed_writes.fetch_add(1, Ordering::SeqCst);
            },
        ));
        let transaction = connection.transaction().unwrap();
        let error = migrate_schema(&transaction).unwrap_err();
        transaction.rollback().unwrap();
        connection.progress_handler(0, None::<fn() -> bool>);
        connection.update_hook(None::<fn(rusqlite::hooks::Action, &str, &str, i64)>);
        (
            error,
            progress.load(Ordering::Relaxed),
            writes.load(Ordering::SeqCst),
        )
    }

    fn install_current_v3_challenge_schema(connection: &Connection) {
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection
            .execute_batch(
                "CREATE TABLE perception_challenges (
                   request_nonce_digest BLOB PRIMARY KEY CHECK(typeof(request_nonce_digest)='blob' AND length(request_nonce_digest)=32),
                   challenge_secret BLOB NOT NULL CHECK(typeof(challenge_secret)='blob' AND length(challenge_secret)=32),
                   origin_digest BLOB NOT NULL CHECK(typeof(origin_digest)='blob' AND length(origin_digest)=32),
                   origin_bytes BLOB NOT NULL CHECK(typeof(origin_bytes)='blob' AND length(origin_bytes)<=4096),
                   persona_scope BLOB NOT NULL CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
                   bot_token BLOB NOT NULL CHECK(typeof(bot_token)='blob' AND length(bot_token)=16),
                   persona_token BLOB NOT NULL CHECK(typeof(persona_token)='blob' AND length(persona_token)=16),
                   origin_event_digest BLOB NOT NULL CHECK(typeof(origin_event_digest)='blob' AND length(origin_event_digest)=32),
                   origin_journal_revision INTEGER NOT NULL CHECK(typeof(origin_journal_revision)='integer' AND origin_journal_revision>0),
                   base_revision INTEGER NOT NULL CHECK(typeof(base_revision)='integer' AND base_revision>=0),
                   incarnation_id BLOB NOT NULL CHECK(typeof(incarnation_id)='blob' AND length(incarnation_id)=32),
                   manifest_digest BLOB NOT NULL CHECK(typeof(manifest_digest)='blob' AND length(manifest_digest)=32),
                   created_at_ms INTEGER NOT NULL CHECK(typeof(created_at_ms)='integer' AND created_at_ms>0),
                   expires_at_ms INTEGER NOT NULL CHECK(typeof(expires_at_ms)='integer' AND expires_at_ms>created_at_ms),
                   UNIQUE(origin_digest,base_revision,incarnation_id),
                   FOREIGN KEY(persona_scope,origin_event_digest)
                     REFERENCES applied_events(scope_digest,event_digest)
                 );
                 CREATE INDEX perception_challenge_context_v1
                   ON perception_challenges(origin_digest,base_revision,incarnation_id);
                 CREATE INDEX perception_challenge_persona_pending_v1
                   ON perception_challenges(persona_scope);
                 CREATE INDEX perception_challenge_expiry_v1
                   ON perception_challenges(expires_at_ms);",
            )
            .unwrap();
    }

    fn quota_id_v1(tag: u8, ordinal: u64) -> [u8; 16] {
        let mut value = [tag; 16];
        value[8..].copy_from_slice(&ordinal.to_le_bytes());
        value
    }

    fn quota_digest_v1(tag: u8, ordinal: u64) -> [u8; 32] {
        let ordinal = ordinal.to_le_bytes();
        wire::domain_hash(b"astr-embodiment/test/quota-row-v1", &[&[tag], &ordinal])
    }

    fn insert_current_v3_challenge_row(
        tx: &Transaction<'_>,
        ordinal: u64,
        persona_ordinal: u64,
    ) -> [u8; 32] {
        let scope = ae_contracts::ScopeRef {
            bot_token: quota_id_v1(0x11, persona_ordinal),
            persona_token: quota_id_v1(0x22, persona_ordinal),
            relation_token: Some(quota_id_v1(0x33, ordinal)),
            session_token: quota_id_v1(0x44, ordinal),
        };
        let persona_scope =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        let relation_scope = wire::persona_scope_digest(
            &scope.bot_token,
            &scope.persona_token,
            scope.relation_token.as_ref(),
        );
        let mut origin = ae_contracts::PerceptionOriginCommitmentV1 {
            schema_version: ae_contracts::PerceptionOriginCommitmentV1::SCHEMA_VERSION,
            source_authority: ae_contracts::SourceAuthority::UserObserved,
            source_digest: quota_digest_v1(0x51, ordinal),
            model_digest: quota_digest_v1(0x52, ordinal),
            provider_digest: quota_digest_v1(0x53, ordinal),
            scope,
            scope_digest: [0; 32],
            persona_scope,
            relation_present: true,
            relation_scope,
            event_id: quota_id_v1(0x61, ordinal),
            turn_id: quota_id_v1(0x62, ordinal),
            observed_at_ms: ordinal.saturating_add(1),
            canonical_base_revision: 1,
            incarnation_id: quota_digest_v1(0x71, persona_ordinal),
            manifest_digest: quota_digest_v1(0x72, persona_ordinal),
            origin_event_digest: quota_digest_v1(0x73, ordinal),
            origin_journal_revision: 1,
            origin_digest: [0; 32],
        };
        origin.scope_digest = wire::scope_digest(&origin.scope);
        origin.origin_digest = origin.digest_v1();
        assert!(origin.validate_v1());
        let origin_bytes = serde_json::to_vec(&origin).unwrap();
        let challenge_secret = quota_digest_v1(0x81, ordinal);
        let created_at_ms = 1_u64;
        let expires_at_ms = created_at_ms + super::PERCEPTION_CHALLENGE_TTL_MS;
        let request_nonce_digest = super::perception_challenge_nonce_v1(
            &challenge_secret,
            &origin.origin_digest,
            created_at_ms,
            expires_at_ms,
        );
        tx.execute(
            "INSERT INTO perception_challenges(
               request_nonce_digest,challenge_secret,origin_digest,origin_bytes,
               persona_scope,bot_token,persona_token,origin_event_digest,
               origin_journal_revision,base_revision,incarnation_id,manifest_digest,
               created_at_ms,expires_at_ms
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,1,1,?9,?10,?11,?12)",
            params![
                request_nonce_digest.to_vec(),
                challenge_secret.to_vec(),
                origin.origin_digest.to_vec(),
                origin_bytes,
                persona_scope.to_vec(),
                origin.scope.bot_token.to_vec(),
                origin.scope.persona_token.to_vec(),
                origin.origin_event_digest.to_vec(),
                origin.incarnation_id.to_vec(),
                origin.manifest_digest.to_vec(),
                i64::try_from(created_at_ms).unwrap(),
                i64::try_from(expires_at_ms).unwrap(),
            ],
        )
        .unwrap();
        persona_scope
    }

    #[test]
    fn global_catalog_gate_rejects_before_write_with_constant_bounded_work() {
        let (boundary_error, boundary_steps, boundary_writes) = measured_oversized_catalog(4_097);
        let (tail_error, tail_steps, tail_writes) = measured_oversized_catalog(8_192);
        for error in [boundary_error, tail_error] {
            assert!(matches!(
                error,
                StoreError::StorageBudgetExceeded {
                    resource: "sqlite_schema.objects",
                    limit: 4_096,
                    actual: 4_097,
                }
            ));
        }
        assert_eq!(boundary_writes, 0);
        assert_eq!(tail_writes, 0);
        assert!(
            tail_steps <= boundary_steps.saturating_add(64),
            "catalog work grew with attacker-controlled tail: boundary={boundary_steps}, tail={tail_steps}"
        );
    }

    fn insert_nonpositive_v3_evidence_row(connection: &Connection, rowid: i64) {
        let persona = [0x91_u8; 32];
        let digest = [0x92_u8; 32];
        connection
            .pragma_update(None, "foreign_keys", "OFF")
            .unwrap();
        connection
            .execute(
                "INSERT INTO semantic_evidence_authority(
                   rowid,persona_scope,semantic_revision,relation_present,relation_scope,event_id,
                   incarnation_id,manifest_digest,route_digest,formula_digest,graph_digest,
                   state_digest,evidence_digest,estimator_digest,event_digest,commitment_digest,
                   evidence_bytes,perception_nonce_digest,perception_proposal_digest,
                   perception_origin_digest,perception_origin_bytes
                 ) VALUES(?1,?2,1,0,?2,?3,?4,?4,?4,?4,?4,?4,?4,?4,?4,?4,X'',NULL,NULL,NULL,NULL)",
                params![rowid, persona.to_vec(), vec![0x93_u8; 16], digest.to_vec()],
            )
            .unwrap();
        assert_eq!(
            connection
                .query_row("SELECT rowid FROM semantic_evidence_authority", [], |row| {
                    row.get::<_, i64>(0)
                },)
                .unwrap(),
            rowid
        );
    }

    #[test]
    fn migrate_rejects_nonpositive_evidence_rowids_before_any_write() {
        for rowid in [-1_i64, 0_i64] {
            let mut connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch(super::SEMANTIC_SCHEMA_V3_SQL)
                .unwrap();
            insert_nonpositive_v3_evidence_row(&connection, rowid);
            let writes = Arc::new(AtomicUsize::new(0));
            let observed_writes = Arc::clone(&writes);
            connection.update_hook(Some(
                move |_: rusqlite::hooks::Action, _: &str, _: &str, _: i64| {
                    observed_writes.fetch_add(1, Ordering::SeqCst);
                },
            ));
            let transaction = connection.transaction().unwrap();
            assert!(matches!(
                migrate_schema(&transaction),
                Err(StoreError::ContinuityFence(
                    "semantic_schema_migration_rowid_domain"
                ))
            ));
            assert_eq!(writes.load(Ordering::SeqCst), 0);
            transaction.rollback().unwrap();
        }
    }

    #[test]
    fn migration_keysets_reject_nonpositive_staging_and_target_rowids() {
        for table in [
            "__ae_semantic_evidence_authority_v2",
            "semantic_evidence_authority",
        ] {
            for rowid in [-1_i64, 0_i64] {
                let mut connection = Connection::open_in_memory().unwrap();
                connection
                    .execute_batch(&format!("CREATE TABLE {table}(evidence_bytes BLOB);"))
                    .unwrap();
                connection
                    .execute(
                        &format!("INSERT INTO {table}(rowid,evidence_bytes) VALUES(?1,X'')"),
                        params![rowid],
                    )
                    .unwrap();
                let writes = Arc::new(AtomicUsize::new(0));
                let observed_writes = Arc::clone(&writes);
                connection.update_hook(Some(
                    move |_: rusqlite::hooks::Action, _: &str, _: &str, _: i64| {
                        observed_writes.fetch_add(1, Ordering::SeqCst);
                    },
                ));
                let transaction = connection.transaction().unwrap();
                assert!(matches!(
                    super::bounded_table_row_count_v1(&transaction, table),
                    Err(StoreError::ContinuityFence(
                        "semantic_schema_migration_rowid_domain"
                    ))
                ));
                assert_eq!(writes.load(Ordering::SeqCst), 0);
                transaction.rollback().unwrap();
            }
        }
    }

    #[test]
    fn perception_receipt_audit_rejects_negative_evidence_rowid_without_writes() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(super::SEMANTIC_SCHEMA_V3_SQL)
            .unwrap();
        insert_nonpositive_v3_evidence_row(&connection, -1);
        let writes = Arc::new(AtomicUsize::new(0));
        let observed_writes = Arc::clone(&writes);
        connection.update_hook(Some(
            move |_: rusqlite::hooks::Action, _: &str, _: &str, _: i64| {
                observed_writes.fetch_add(1, Ordering::SeqCst);
            },
        ));
        let transaction = connection.transaction().unwrap();
        assert!(matches!(
            super::verify_all_perception_challenges(&transaction),
            Err(StoreError::ContinuityFence(
                "perception_receipt_rowid_domain"
            ))
        ));
        assert_eq!(writes.load(Ordering::SeqCst), 0);
        transaction.rollback().unwrap();
    }

    #[test]
    fn appraisal_scans_are_global_rowid_keysets_without_temp_sort() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch(SEMANTIC_SCHEMA_V3_SQL).unwrap();
        connection
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V2_SQL)
            .unwrap();

        for sql in [
            SEMANTIC_APPRAISAL_BUDGET_PHASE_ONE_SCAN_V1_SQL,
            SEMANTIC_APPRAISAL_CLAIM_PHASE_ONE_SCAN_V1_SQL,
            SEMANTIC_APPRAISAL_BUDGET_PHASE_TWO_SCAN_V1_SQL,
            SEMANTIC_APPRAISAL_CLAIM_SCAN_V2_SQL,
        ] {
            let explain = format!("EXPLAIN QUERY PLAN {sql}");
            let mut statement = connection.prepare(&explain).unwrap();
            let details = statement
                .query_map(params![0_i64, 129_i64], |row| row.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            assert!(
                details
                    .iter()
                    .any(|detail| detail.contains("USING INTEGER PRIMARY KEY (rowid>?)")),
                "missing rowid keyset plan for {sql}: {details:?}"
            );
            assert!(
                details
                    .iter()
                    .all(|detail| !detail.contains("USE TEMP B-TREE")),
                "temporary sort in appraisal scan for {sql}: {details:?}"
            );
        }

        for (sql, parameters) in [
            (
                PERCEPTION_CHALLENGE_PHASE_ONE_V3_SQL,
                vec![
                    rusqlite::types::Value::Integer(0),
                    rusqlite::types::Value::Integer(129),
                ],
            ),
            (
                PERCEPTION_CHALLENGE_PHASE_TWO_V3_SQL,
                vec![
                    rusqlite::types::Value::Integer(0),
                    rusqlite::types::Value::Integer(128),
                    rusqlite::types::Value::Integer(4_096),
                ],
            ),
        ] {
            let explain = format!("EXPLAIN QUERY PLAN {sql}");
            let mut statement = connection.prepare(&explain).unwrap();
            let details = statement
                .query_map(rusqlite::params_from_iter(parameters), |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>();
            assert!(
                details
                    .iter()
                    .any(|detail| detail.contains("USING INTEGER PRIMARY KEY (rowid>?)")),
                "missing rowid keyset plan for challenge scan: {details:?}"
            );
            assert!(
                details
                    .iter()
                    .all(|detail| !detail.contains("USE TEMP B-TREE")),
                "temporary sort in challenge scan: {details:?}"
            );
        }
    }

    #[test]
    fn appraisal_budget_retention_candidate_visits_are_bounded_before_eligibility() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V3_SQL)
            .unwrap();
        {
            let transaction = connection.transaction().unwrap();
            let mut insert = transaction
                .prepare(
                    "INSERT INTO semantic_appraisal_budget(
                       persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
                       blocked,updated_at_ms,compacted_claim_rows,compacted_charged_tokens,
                       compacted_chain_digest
                     ) VALUES(?1,0,1000000,0,1,0,1,0,0,zeroblob(32))",
                )
                .unwrap();
            for ordinal in 0_u64..4_096 {
                let mut persona = [0xE1_u8; 32];
                persona[24..].copy_from_slice(&ordinal.to_be_bytes());
                insert.execute([persona.to_vec()]).unwrap();
            }
            drop(insert);
            transaction.commit().unwrap();
        }

        let candidate_plan = {
            let mut statement = connection
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN {SEMANTIC_APPRAISAL_BUDGET_RETENTION_CANDIDATES_V1_SQL}"
                ))
                .unwrap();
            statement
                .query_map(params![1_i64, 1_i64], |row| row.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n")
        };
        assert!(candidate_plan.contains("semantic_appraisal_budget_retention_v3"));
        assert!(candidate_plan.contains("utc_day<?"));
        assert!(!candidate_plan.contains("USE TEMP B-TREE"));
        assert!(!candidate_plan.contains("semantic_appraisal_claim"));
        assert!(!candidate_plan.contains("CORRELATED"));

        let eligibility_plan = {
            let mut statement = connection
                .prepare(&format!(
                    "EXPLAIN QUERY PLAN {SEMANTIC_APPRAISAL_BUDGET_RETENTION_ELIGIBLE_V1_SQL}"
                ))
                .unwrap();
            statement
                .query_map(params![1_i64, vec![0_u8; 32], 0_i64], |row| {
                    row.get::<_, String>(3)
                })
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("\n")
        };
        assert!(eligibility_plan.contains("INTEGER PRIMARY KEY (rowid=?)"));
        assert!(eligibility_plan.contains("semantic_appraisal_claim_budget_v3"));
        assert!(!eligibility_plan.contains("CORRELATED"));
        assert!(!eligibility_plan.contains("SCAN semantic_appraisal"));

        let progress = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&progress);
        connection.progress_handler(
            1,
            Some(move || {
                observed.fetch_add(1, Ordering::Relaxed);
                false
            }),
        );
        let transaction = connection.transaction().unwrap();
        assert_eq!(
            maintain_semantic_appraisal_retention_v1(
                &transaction,
                SEMANTIC_APPRAISAL_UTC_DAY_MS,
                SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE - 1,
            )
            .unwrap(),
            SEMANTIC_APPRAISAL_RETENTION_MUTATIONS_PER_WRITE - 1
        );
        transaction.rollback().unwrap();
        connection.progress_handler(0, None::<fn() -> bool>);
        let visits = progress.load(Ordering::Relaxed);
        assert!(
            visits < 500,
            "one remaining candidate slot visited {visits} SQLite operations"
        );
    }

    #[test]
    fn appraisal_v3_catalog_is_exact_and_seeds_one_rollup() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V3_SQL)
            .unwrap();

        let objects = bounded_semantic_schema_objects_v1(&connection, true).unwrap();
        assert_eq!(objects.len(), 9);
        for name in [
            "semantic_appraisal_claim_retention_v3",
            "semantic_appraisal_claim_budget_v3",
            "semantic_appraisal_budget_retention_v3",
        ] {
            assert!(objects.iter().any(|object| object.name == name));
        }
        let rollup: (i64, i64, i64, i64, Vec<u8>, i64) = connection
            .query_row(
                "SELECT singleton,compacted_budget_rows,compacted_claim_rows,
                        compacted_charged_tokens,compacted_chain_digest,last_authoritative_now_ms
                 FROM semantic_appraisal_rollup",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(rollup, (1, 0, 0, 0, vec![0; 32], 0));
    }

    #[test]
    fn appraisal_retry_and_pending_expiry_flip_at_exact_five_minutes() {
        let created_or_settled = 1_000_000_u64;
        assert!(semantic_appraisal_terminal_retry_live_v1(
            created_or_settled,
            created_or_settled + 299_999
        ));
        assert!(!semantic_appraisal_terminal_retry_live_v1(
            created_or_settled,
            created_or_settled + 300_000
        ));
        assert!(!semantic_appraisal_pending_expired_v1(
            created_or_settled,
            created_or_settled + 299_999
        ));
        assert!(semantic_appraisal_pending_expired_v1(
            created_or_settled,
            created_or_settled + 300_000
        ));
    }

    #[test]
    fn appraisal_compaction_chain_transitions_have_fixed_vectors() {
        let claim = SemanticAppraisalTerminalCompactionRowV1 {
            rowid: 1,
            request_nonce_digest: vec![0x01; 32],
            persona_scope: vec![0x02; 32],
            utc_day: 3,
            origin_event_digest: vec![0x04; 32],
            origin_digest: vec![0x05; 32],
            provider_digest: vec![0x06; 32],
            reserved_tokens: 768,
            created_at_ms: 259_200_123,
            settled_at_ms: 259_200_124,
            charged_tokens: 321,
            outcome_code: "success".into(),
            canonical_revision: 9,
            semantic_revision: Some(4),
            usage_known: Some(1),
            usage_tokens: Some(321),
            proposal_identity_digest: Some(vec![0x07; 32]),
            settlement_identity_digest: Some(vec![0x08; 32]),
            reply_affect_bytes: Some(b"null".to_vec()),
            reply_affect_digest: Some(vec![0x09; 32]),
            terminal_receipt_bytes: Some(b"{}".to_vec()),
            terminal_receipt_digest: Some(vec![0x0A; 32]),
        };
        let claim_leaf = semantic_appraisal_terminal_compaction_leaf_v1(&claim)
            .unwrap()
            .3;
        let claim_chain = semantic_appraisal_compaction_chain_v1(
            SEMANTIC_APPRAISAL_CLAIM_COMPACTION_CHAIN_DOMAIN_V1,
            [0x11; 32],
            7,
            claim_leaf,
        );
        let budget = SemanticAppraisalBudgetCompactionRowV1 {
            rowid: 2,
            persona_scope: vec![0x02; 32],
            utc_day: 3,
            daily_token_limit: 1_000_000,
            charged_tokens: 321,
            reserved_tokens: 0,
            blocked: 0,
            updated_at_ms: 259_200_124,
            compacted_claim_rows: 1,
            compacted_charged_tokens: 321,
            compacted_chain_digest: vec![0x33; 32],
        };
        let budget_leaf = semantic_appraisal_budget_compaction_leaf_v1(&budget)
            .unwrap()
            .4;
        let budget_chain = semantic_appraisal_compaction_chain_v1(
            SEMANTIC_APPRAISAL_BUDGET_COMPACTION_CHAIN_DOMAIN_V1,
            [0x22; 32],
            3,
            budget_leaf,
        );
        assert_eq!(
            (claim_leaf, claim_chain, budget_leaf, budget_chain),
            (
                [
                    146, 206, 49, 241, 127, 20, 140, 129, 11, 85, 230, 251, 252, 254, 137, 130, 28,
                    84, 20, 203, 193, 109, 142, 80, 58, 53, 72, 18, 169, 129, 162, 1,
                ],
                [
                    165, 104, 47, 13, 25, 119, 10, 160, 140, 213, 45, 18, 174, 37, 120, 186, 9,
                    115, 96, 98, 190, 237, 39, 165, 60, 78, 126, 103, 255, 148, 85, 45,
                ],
                [
                    45, 232, 47, 211, 245, 38, 65, 28, 4, 128, 126, 76, 129, 1, 161, 127, 226, 129,
                    241, 7, 130, 103, 193, 109, 173, 220, 227, 88, 63, 65, 206, 189,
                ],
                [
                    96, 107, 13, 124, 64, 119, 232, 172, 12, 218, 184, 21, 245, 209, 43, 218, 134,
                    204, 41, 80, 48, 6, 192, 159, 73, 155, 159, 49, 122, 144, 108, 222,
                ],
            ),
        );
    }

    #[test]
    fn appraisal_global_pending_capacity_accepts_255_and_defers_256() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V3_SQL)
            .unwrap();
        for (persona_byte, pending) in [(0x61_u8, 64_i64), (0x62, 64), (0x63, 64), (0x64, 63)] {
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_budget(
                       persona_scope,utc_day,daily_token_limit,charged_tokens,reserved_tokens,
                       blocked,updated_at_ms,compacted_claim_rows,compacted_charged_tokens,
                       compacted_chain_digest
                     ) VALUES(?1,1,1000000,0,?2,0,?3,0,0,zeroblob(32))",
                    params![
                        vec![persona_byte; 32],
                        pending * 768,
                        i64::try_from(SEMANTIC_APPRAISAL_UTC_DAY_MS + 1).unwrap(),
                    ],
                )
                .unwrap();
        }
        for ordinal in 0_u64..255 {
            let persona_byte = 0x61_u8 + u8::try_from(ordinal / 64).unwrap();
            let mut nonce = [0x71_u8; 32];
            nonce[..8].copy_from_slice(&(ordinal + 1).to_le_bytes());
            let mut origin_event = [0x72_u8; 32];
            origin_event[..8].copy_from_slice(&(ordinal + 1).to_le_bytes());
            connection
                .execute(
                    "INSERT INTO semantic_appraisal_claim(
                       request_nonce_digest,persona_scope,utc_day,origin_event_digest,
                       origin_digest,provider_digest,reserved_tokens,created_at_ms
                     ) VALUES(?1,?2,1,?3,?4,?5,768,?6)",
                    params![
                        nonce.to_vec(),
                        vec![persona_byte; 32],
                        origin_event.to_vec(),
                        vec![0x73_u8; 32],
                        vec![0x74_u8; 32],
                        i64::try_from(SEMANTIC_APPRAISAL_UTC_DAY_MS + ordinal + 1).unwrap(),
                    ],
                )
                .unwrap();
            if ordinal == 253 {
                let storage = scan_semantic_appraisal_phase_one_v1(
                    &connection,
                    SemanticAppraisalStorageLayoutV1::V3,
                )
                .unwrap();
                assert_eq!(storage.claim_rows, 254);
                assert!(semantic_appraisal_claim_capacity_decision_v1(
                    storage, false, false, None, None,
                )
                .is_none());
            }
        }
        let storage =
            scan_semantic_appraisal_phase_one_v1(&connection, SemanticAppraisalStorageLayoutV1::V3)
                .unwrap();
        assert_eq!(
            storage,
            SemanticAppraisalStorageBudgetV1 {
                personas: 4,
                budget_rows: 4,
                claim_rows: 255,
                aggregate_bytes: 255 * MAX_SEMANTIC_APPRAISAL_PENDING_PAYLOAD_BYTES,
            }
        );
        let deferred = semantic_appraisal_claim_capacity_decision_v1(
            storage,
            false,
            false,
            Some(SemanticAppraisalBudgetReceiptV1 {
                utc_day: 1,
                daily_token_limit: 1_000_000,
                reserved_tokens: 63 * 768,
                charged_tokens: 0,
                remaining_tokens: 1_000_000 - 63 * 768,
                blocked: false,
            }),
            None,
        )
        .expect("the production admission branch must defer claim 256");
        assert_eq!(
            deferred.status,
            SemanticAppraisalBeginStatusV1::CapacityDeferred
        );
        assert_eq!(
            deferred.capacity_reason,
            Some(SemanticAppraisalCapacityReasonV1::RetentionCapacityUnavailable)
        );
        assert!(deferred.challenge.is_none());
        assert!(deferred.budget.is_some());
        let mut statement = connection
            .prepare(
                "SELECT COUNT(*) FROM semantic_appraisal_claim
                 GROUP BY persona_scope ORDER BY COUNT(*)",
            )
            .unwrap();
        let counts = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(counts, vec![63, 64, 64, 64]);
    }

    #[test]
    fn appraisal_catalog_rejects_tenth_object_and_oversized_fields_before_materialization() {
        let tenth = Connection::open_in_memory().unwrap();
        tenth
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V2_SQL)
            .unwrap();
        let base = bounded_semantic_schema_objects_v1(&tenth, true)
            .unwrap()
            .len();
        for ordinal in 0..(10_usize.saturating_sub(base)) {
            tenth
                .execute_batch(&format!(
                    "CREATE TRIGGER appraisal_extra_{ordinal} AFTER INSERT
                     ON semantic_appraisal_budget BEGIN SELECT 1; END;"
                ))
                .unwrap();
        }
        assert!(matches!(
            bounded_semantic_schema_objects_v1(&tenth, true),
            Err(StoreError::ContinuityFence(
                "semantic_appraisal_schema_object_set"
            ))
        ));

        let huge_name = Connection::open_in_memory().unwrap();
        huge_name
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V2_SQL)
            .unwrap();
        huge_name
            .execute_batch(&format!(
                "CREATE INDEX \"{}\" ON semantic_appraisal_budget(utc_day);",
                "n".repeat(97)
            ))
            .unwrap();
        assert!(matches!(
            bounded_semantic_schema_objects_v1(&huge_name, true),
            Err(StoreError::ContinuityFence(
                "semantic_appraisal_schema_object_type"
            ))
        ));

        let huge_sql = Connection::open_in_memory().unwrap();
        huge_sql
            .execute_batch(SEMANTIC_APPRAISAL_SCHEMA_V2_SQL)
            .unwrap();
        huge_sql
            .execute_batch(&format!(
                "CREATE TRIGGER appraisal_huge_sql AFTER INSERT ON semantic_appraisal_budget
                 BEGIN SELECT '{}'; END;",
                "x".repeat(65_536)
            ))
            .unwrap();
        assert!(matches!(
            bounded_semantic_schema_objects_v1(&huge_sql, true),
            Err(StoreError::ContinuityFence(
                "semantic_appraisal_schema_sql_size"
            ))
        ));
    }

    #[test]
    fn aggregate_budget_checked_add_is_exact_and_overflow_safe() {
        assert_eq!(
            checked_semantic_aggregate_bytes(
                MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA - 5,
                [1, 1, 1, 1, 1],
            )
            .unwrap(),
            MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA
        );
        assert!(matches!(
            checked_semantic_aggregate_bytes(
                MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
                [1, 0, 0, 0, 0],
            ),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.bytes",
                limit: MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
                actual
            }) if actual == MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA + 1
        ));
        assert!(matches!(
            checked_semantic_aggregate_bytes(u64::MAX, [1, 0, 0, 0, 0]),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.bytes",
                limit: MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA,
                actual: u64::MAX
            })
        ));
    }

    #[test]
    fn semantic_history_keyset_and_global_scan_budgets_reject_first_forbidden_work() {
        let connection = Connection::open_in_memory().unwrap();
        for table in SEMANTIC_HISTORY_SCOPE_TABLES_V1 {
            connection
                .execute_batch(&format!(
                    "CREATE TABLE {table}(persona_scope BLOB PRIMARY KEY NOT NULL);"
                ))
                .unwrap();
        }
        for ordinal in 0..=MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL {
            let mut persona_scope = [0_u8; 32];
            persona_scope[..8].copy_from_slice(&ordinal.to_be_bytes());
            connection
                .execute(
                    "INSERT INTO semantic_origins(persona_scope) VALUES(?1)",
                    params![persona_scope.to_vec()],
                )
                .unwrap();
        }

        let mut budget = SemanticHistoryScanBudgetV1::default();
        let mut last_scope = None;
        let mut admitted_heavy_scopes = 0_u64;
        let rejected = loop {
            let scope = next_semantic_persona_scope_v1(&connection, last_scope)
                .unwrap()
                .expect("the malicious extra scope must be visible");
            match budget.admit_persona() {
                Ok(()) => {
                    // Represents the expensive per-persona aggregate/closure
                    // that production calls only after this admission.
                    admitted_heavy_scopes += 1;
                    last_scope = Some(scope);
                }
                Err(error) => break error,
            }
        };
        assert_eq!(admitted_heavy_scopes, MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL);
        assert_eq!(budget.personas, MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL);
        assert!(matches!(
            rejected,
            StoreError::StorageBudgetExceeded {
                resource: "semantic.history.global_personas",
                limit: MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
                actual,
            } if actual == MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL + 1
        ));

        let mut exact = SemanticHistoryScanBudgetV1::default();
        exact
            .admit_history(
                MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
                MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL,
            )
            .unwrap();
        let frozen = exact;
        assert!(matches!(
            exact.admit_history(1, 0),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.global_scan_rows",
                limit: MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL,
                actual,
            }) if actual == MAX_SEMANTIC_HISTORY_SCAN_ROWS_GLOBAL + 1
        ));
        assert_eq!(exact, frozen);
        assert!(matches!(
            exact.admit_history(0, 1),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.global_scan_bytes",
                limit: MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL,
                actual,
            }) if actual == MAX_SEMANTIC_HISTORY_SCAN_BYTES_GLOBAL + 1
        ));
        assert_eq!(exact, frozen);
    }

    #[test]
    fn legacy_preflight_rejects_exact_row_and_byte_overage_without_writes() {
        let persona_scope = [0x44_u8; 32];
        let row_connection = Connection::open_in_memory().unwrap();
        row_connection
            .execute_batch(
                "CREATE TABLE semantic_commits(
                   persona_scope BLOB NOT NULL,
                   semantic_revision INTEGER NOT NULL
                 );",
            )
            .unwrap();
        for revision in 1_i64..=3 {
            row_connection
                .execute(
                    "INSERT INTO semantic_commits VALUES(?1,?2)",
                    params![persona_scope.to_vec(), revision],
                )
                .unwrap();
        }
        let row_changes = row_connection.total_changes();
        let row_limits = SemanticHistoryScanLimitsV1 {
            personas_global: 1,
            table_rows_per_persona: 2,
            scan_rows_per_persona: 2,
            rows_global: 2,
            bytes_per_persona: 16,
            bytes_global: 16,
        };
        assert!(matches!(
            preflight_semantic_history_layout_v1(
                &row_connection,
                &["semantic_commits"],
                &[SemanticHistoryScanTableV1 {
                    table: "semantic_commits",
                    payload_column: None,
                    is_commit_table: true,
                }],
                row_limits,
            ),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.table_rows",
                limit: 2,
                actual: 3,
            })
        ));
        assert_eq!(row_connection.total_changes(), row_changes);

        let byte_connection = Connection::open_in_memory().unwrap();
        byte_connection
            .execute_batch(
                "CREATE TABLE semantic_snapshots(
                   persona_scope BLOB NOT NULL,
                   semantic_revision INTEGER NOT NULL,
                   snapshot_bytes BLOB NOT NULL
                 );",
            )
            .unwrap();
        byte_connection
            .execute(
                "INSERT INTO semantic_snapshots VALUES(?1,1,?2)",
                params![persona_scope.to_vec(), vec![0_u8; 3]],
            )
            .unwrap();
        let byte_changes = byte_connection.total_changes();
        let byte_limits = SemanticHistoryScanLimitsV1 {
            personas_global: 1,
            table_rows_per_persona: 1,
            scan_rows_per_persona: 1,
            rows_global: 1,
            bytes_per_persona: 2,
            bytes_global: 2,
        };
        assert!(matches!(
            preflight_semantic_history_layout_v1(
                &byte_connection,
                &["semantic_snapshots"],
                &[SemanticHistoryScanTableV1 {
                    table: "semantic_snapshots",
                    payload_column: Some("snapshot_bytes"),
                    is_commit_table: false,
                }],
                byte_limits,
            ),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.bytes",
                limit: 2,
                actual: 3,
            })
        ));
        assert_eq!(byte_connection.total_changes(), byte_changes);
    }

    #[test]
    fn direct_v3_preflight_rejects_exact_row_and_byte_overage_without_writes() {
        fn v3_scan_shape() -> Connection {
            let connection = Connection::open_in_memory().unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE semantic_origins(persona_scope BLOB NOT NULL);
                     CREATE TABLE semantic_commits(
                       persona_scope BLOB NOT NULL, semantic_revision INTEGER NOT NULL);
                     CREATE TABLE semantic_cursor(persona_scope BLOB NOT NULL);
                     CREATE TABLE semantic_snapshots(
                       persona_scope BLOB NOT NULL, semantic_revision INTEGER NOT NULL,
                       snapshot_bytes BLOB NOT NULL);
                     CREATE TABLE semantic_graphs(
                       persona_scope BLOB NOT NULL, semantic_revision INTEGER NOT NULL,
                       graph_bytes BLOB NOT NULL);
                     CREATE TABLE semantic_receipts(
                       persona_scope BLOB NOT NULL, semantic_revision INTEGER NOT NULL,
                       receipt_bytes BLOB NOT NULL);
                     CREATE TABLE semantic_telemetry(
                       persona_scope BLOB NOT NULL, semantic_revision INTEGER NOT NULL,
                       telemetry_bytes BLOB NOT NULL);
                     CREATE TABLE semantic_evidence_authority(
                       persona_scope BLOB NOT NULL, semantic_revision INTEGER NOT NULL,
                       evidence_bytes BLOB NOT NULL);
                     CREATE TABLE semantic_budget_checkpoint(persona_scope BLOB NOT NULL);",
                )
                .unwrap();
            connection
        }

        let persona_scope = [0x45_u8; 32];
        let row_connection = v3_scan_shape();
        for revision in 1_i64..=3 {
            row_connection
                .execute(
                    "INSERT INTO semantic_commits VALUES(?1,?2)",
                    params![persona_scope.to_vec(), revision],
                )
                .unwrap();
        }
        let row_changes = row_connection.total_changes();
        let row_limits = SemanticHistoryScanLimitsV1 {
            personas_global: 1,
            table_rows_per_persona: 2,
            scan_rows_per_persona: 2,
            rows_global: 2,
            bytes_per_persona: 16,
            bytes_global: 16,
        };
        assert!(matches!(
            preflight_semantic_v3_history_with_limits_v1(&row_connection, row_limits),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.table_rows",
                limit: 2,
                actual: 3,
            })
        ));
        assert_eq!(row_connection.total_changes(), row_changes);

        let byte_connection = v3_scan_shape();
        byte_connection
            .execute(
                "INSERT INTO semantic_snapshots VALUES(?1,1,?2)",
                params![persona_scope.to_vec(), vec![0_u8; 3]],
            )
            .unwrap();
        let byte_changes = byte_connection.total_changes();
        let byte_limits = SemanticHistoryScanLimitsV1 {
            personas_global: 1,
            table_rows_per_persona: 1,
            scan_rows_per_persona: 1,
            rows_global: 1,
            bytes_per_persona: 2,
            bytes_global: 2,
        };
        assert!(matches!(
            preflight_semantic_v3_history_with_limits_v1(&byte_connection, byte_limits),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.bytes",
                limit: 2,
                actual: 3,
            })
        ));
        assert_eq!(byte_connection.total_changes(), byte_changes);
    }

    #[test]
    fn oversized_checkpoint_digest_is_rejected_before_blob_materialization() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE semantic_budget_checkpoint(
                   persona_scope BLOB PRIMARY KEY,
                   row_count INTEGER NOT NULL,
                   aggregate_bytes INTEGER NOT NULL,
                   head_revision INTEGER NOT NULL,
                   head_commitment_digest BLOB NOT NULL
                 );",
            )
            .unwrap();
        let persona_scope = [0x73_u8; 32];
        connection
            .execute(
                "INSERT INTO semantic_budget_checkpoint
                 VALUES(?1,1,0,1,zeroblob(65536))",
                params![persona_scope.to_vec()],
            )
            .unwrap();
        let old_limit = connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, 1_024);
        assert!(old_limit > 65_536);
        let result = read_semantic_budget_checkpoint_v1(&connection, persona_scope);
        assert!(
            matches!(
                &result,
                Err(StoreError::InvalidStoredDigest {
                    field: "semantic_budget.commitment",
                    actual,
                }) if *actual == 65_536
            ),
            "unexpected checkpoint read: {result:?}"
        );
    }

    #[test]
    fn hot_preflight_gates_time_and_perception_checkpoint_digest_before_allocation() {
        let uri = "file:semantic-budget-hot-preflight-v1?mode=memory&cache=shared";
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI;
        let mut primary = Connection::open_with_flags(uri, flags).unwrap();
        let external = Connection::open_with_flags(uri, flags).unwrap();
        primary
            .execute_batch(
                "CREATE TABLE semantic_budget_checkpoint(
                   persona_scope BLOB PRIMARY KEY CHECK(typeof(persona_scope)='blob' AND length(persona_scope)=32),
                   row_count INTEGER NOT NULL,
                   aggregate_bytes INTEGER NOT NULL,
                   head_revision INTEGER NOT NULL,
                   head_commitment_digest BLOB NOT NULL
                     CHECK(typeof(head_commitment_digest)='blob' AND length(head_commitment_digest)=32)
                 );",
            )
            .unwrap();
        let persona_scope = [0x74_u8; 32];
        primary
            .execute(
                "INSERT INTO semantic_budget_checkpoint VALUES(?1,1,10,1,?2)",
                params![persona_scope.to_vec(), vec![0x75_u8; 32]],
            )
            .unwrap();
        external
            .pragma_update(None, "ignore_check_constraints", "ON")
            .unwrap();
        external
            .execute(
                "UPDATE semantic_budget_checkpoint
                 SET head_commitment_digest=zeroblob(65536)
                 WHERE persona_scope=?1",
                params![persona_scope.to_vec()],
            )
            .unwrap();

        let old_limit = primary.set_limit(Limit::SQLITE_LIMIT_LENGTH, 1_024);
        assert!(old_limit > 65_536);
        let changes_before = primary.total_changes();
        let tx = primary.transaction().unwrap();
        let time_result =
            preflight_semantic_budget_tx(&tx, persona_scope, 0, 1, [431, 0, 0, 0, 512]);
        assert!(
            matches!(
                &time_result,
                Err(StoreError::InvalidStoredDigest {
                    field: "semantic_budget.commitment",
                    actual,
                }) if *actual == 65_536
            ),
            "unexpected time preflight result: {time_result:?}"
        );
        assert_eq!(tx.total_changes(), changes_before);
        drop(tx);

        external
            .execute(
                "UPDATE semantic_budget_checkpoint
                 SET head_commitment_digest='wrong-storage-class'
                 WHERE persona_scope=?1",
                params![persona_scope.to_vec()],
            )
            .unwrap();
        let tx = primary.transaction().unwrap();
        let perception_result =
            preflight_semantic_budget_tx(&tx, persona_scope, 0, 1, [1, 1, 1, 1, 1]);
        assert!(
            matches!(
                perception_result,
                Err(StoreError::ContinuityFence(
                    "semantic_budget_commitment_type"
                ))
            ),
            "unexpected perception preflight result: {perception_result:?}"
        );
        assert_eq!(tx.total_changes(), changes_before);
        let stored_shape: (String, i64) = tx
            .query_row(
                "SELECT typeof(head_commitment_digest),length(head_commitment_digest)
                 FROM semantic_budget_checkpoint WHERE persona_scope=?1",
                params![persona_scope.to_vec()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_shape, ("text".to_owned(), 19));
    }

    #[test]
    fn compact_projection_budget_survives_1009_wakes_deterministically() {
        let semantic_formula_digest = [0x31; 32];
        let time_formula_digest = matrix_time_formula_digest_v1(&semantic_formula_digest);
        let state_digest = [0x41; 32];
        let graph_digest = [0x51; 32];
        let mut epoch = MatrixTimeEpochV1 {
            schema_version: MatrixTimeEpochV1::SCHEMA_VERSION,
            anchor_semantic_revision: 1,
            anchor_state_digest: state_digest,
            awake_ticks: 0,
            drowsy_ticks: 0,
            asleep_ticks: 0,
            awake_remainder_ms: 0,
            drowsy_remainder_ms: 0,
            asleep_remainder_ms: 0,
        };
        // Begin after charging a maximally sized perception/Genesis anchor,
        // not from an unrealistically empty ledger.
        let anchor_charge = [
            MAX_SNAPSHOT_STATE_BYTES,
            MAX_SEMANTIC_GRAPH_BYTES,
            MAX_SEMANTIC_RECEIPT_BYTES,
            MAX_SEMANTIC_TELEMETRY_BYTES,
            MAX_JOURNAL_EVENT_BYTES,
        ];
        let anchor_bytes = anchor_charge.into_iter().sum::<u64>();
        let mut aggregate = checked_semantic_aggregate_bytes(0, anchor_charge).unwrap();
        let mut final_wire = Vec::new();
        for wake in 1_u64..=1_009 {
            let epoch_before = epoch.clone();
            epoch.awake_remainder_ms += 1;
            let wake_bytes = wake.to_le_bytes();
            let projection = DecodedTimeSnapshotV1 {
                semantic_formula_digest,
                time_formula_digest,
                epoch_before,
                epoch_after: epoch.clone(),
                requested_elapsed_ms: 1,
                applied_elapsed_ms: 1,
                capped_gap: false,
                pre_sleep_phase: MatrixSleepPhaseV1::Awake,
                state_before: state_digest,
                state_after: state_digest,
                graph_digest,
                authority_digest: wire::domain_hash(b"test/time-authority", &[&wake_bytes]),
                commitment_digest: wire::domain_hash(b"test/time-commitment", &[&wake_bytes]),
            };
            let wire = encode_time_snapshot_v1(&projection).unwrap();
            assert_eq!(wire.len(), TIME_SNAPSHOT_WIRE_LEN_V1);
            assert_eq!(wire, encode_time_snapshot_v1(&projection).unwrap());
            final_wire = wire;
            // Charge the maximum legal authority sidecar for every wake. If
            // this conservative bound fits, actual canonical JSON cannot
            // exhaust the per-persona byte budget at this cadence.
            aggregate = checked_semantic_aggregate_bytes(
                aggregate,
                [TIME_SNAPSHOT_WIRE_LEN_V1 as u64, 0, 0, 0, 16_384],
            )
            .unwrap();
            if matches!(wake, 56 | 455 | 1_009) {
                assert!(aggregate < MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA);
            }
        }
        assert_eq!(epoch.awake_remainder_ms, 1_009);
        assert_eq!(final_wire.len(), 431);
        assert_eq!(
            aggregate,
            anchor_bytes + 1_009 * (TIME_SNAPSHOT_WIRE_LEN_V1 as u64 + 16_384)
        );
        assert!(aggregate < MAX_SEMANTIC_AGGREGATE_BYTES_PER_PERSONA);
    }

    #[test]
    fn semantic_v4_to_v5_transaction_installs_compact_composite_lane() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value BLOB NOT NULL);")
            .unwrap();
        connection
            .execute_batch(super::SEMANTIC_SCHEMA_V3_SQL)
            .unwrap();
        super::apply_semantic_schema_v4(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES('semantic_schema_version',?1)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![vec![super::SEMANTIC_SCHEMA_VERSION_V4]],
            )
            .unwrap();

        let tx = connection.transaction().unwrap();
        super::apply_semantic_schema_v5(&tx).unwrap();
        super::compact_semantic_time_rows_v5(&tx).unwrap();
        super::verify_semantic_schema_v5(&tx).unwrap();
        tx.execute(
            "UPDATE meta SET value=?1 WHERE key='semantic_schema_version'",
            params![vec![super::SEMANTIC_SCHEMA_VERSION_V5]],
        )
        .unwrap();
        tx.commit().unwrap();
        super::require_exact_semantic_schema_v5(&connection).unwrap();
        let installed: Vec<u8> = connection
            .query_row(
                "SELECT value FROM meta WHERE key='semantic_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(installed, vec![super::SEMANTIC_SCHEMA_VERSION_V5]);
        let journal_unique_columns: String = connection
            .query_row(
                "SELECT group_concat(info.name, ',')
                 FROM pragma_index_list('semantic_time_authority') AS list
                 JOIN pragma_index_info(list.name) AS info
                 WHERE list.[unique]=1
                 GROUP BY list.name
                 HAVING group_concat(info.name, ',')='persona_scope,journal_revision'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(journal_unique_columns, "persona_scope,journal_revision");

        // Both rows intentionally use logical journal revision 1. V4's
        // global UNIQUE(journal_revision) rejected the second persona.
        connection
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .unwrap();
        for seed in [0x61_u8, 0x62] {
            connection
                .execute(
                    "INSERT INTO semantic_time_authority(
                       persona_scope,semantic_revision,journal_revision,event_id,event_digest,
                       authority_digest,authority_bytes)
                     VALUES(?1,1,1,?2,?3,?4,?5)",
                    params![
                        vec![seed; 32],
                        vec![seed; 16],
                        vec![seed.wrapping_add(1); 32],
                        vec![seed.wrapping_add(2); 32],
                        vec![seed.wrapping_add(3)],
                    ],
                )
                .unwrap();
        }
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM semantic_time_authority WHERE journal_revision=1",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
    }

    #[test]
    fn over_budget_v4_migration_fails_before_schema_or_row_rewrite() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch("CREATE TABLE meta(key TEXT PRIMARY KEY,value BLOB NOT NULL);")
            .unwrap();
        connection
            .execute_batch(super::SEMANTIC_SCHEMA_V3_SQL)
            .unwrap();
        super::apply_semantic_schema_v4(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO meta(key,value) VALUES('semantic_schema_version',?1)",
                params![vec![super::SEMANTIC_SCHEMA_VERSION_V4]],
            )
            .unwrap();
        {
            let tx = connection.transaction().unwrap();
            {
                let fixed_digest = [0x55_u8; 32];
                let mut insert = tx
                    .prepare(
                        "INSERT INTO semantic_origins(
                           persona_scope,source_scope_digest,source_revision,legacy_migrated,
                           incarnation_id,manifest_digest,route_digest,formula_digest,
                           state_digest,graph_digest,origin_digest
                         ) VALUES(?1,?2,0,0,?2,?2,?2,?2,?2,?2,?2)",
                    )
                    .unwrap();
                for ordinal in 0..=MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL {
                    let mut persona_scope = [0_u8; 32];
                    persona_scope[..8].copy_from_slice(&ordinal.to_be_bytes());
                    insert
                        .execute(params![persona_scope.to_vec(), fixed_digest.to_vec()])
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        }

        let schema_before = super::semantic_schema_objects_v1(&connection).unwrap();
        let origin_shape_before: (i64, i64) = connection
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(
                    length(persona_scope)+length(source_scope_digest)+length(incarnation_id)+
                    length(manifest_digest)+length(route_digest)+length(formula_digest)+
                    length(state_digest)+length(graph_digest)+length(origin_digest)
                 ),0) FROM semantic_origins",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let version_before: Vec<u8> = connection
            .query_row(
                "SELECT value FROM meta WHERE key='semantic_schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let user_version_before: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();

        let tx = connection.transaction().unwrap();
        assert!(matches!(
            super::migrate_schema(&tx),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic.history.global_personas",
                limit: MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
                actual,
            }) if actual == MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL + 1
        ));
        assert_eq!(
            super::semantic_schema_objects_v1(&tx).unwrap(),
            schema_before
        );
        assert_eq!(
            tx.query_row(
                "SELECT COUNT(*),COALESCE(SUM(
                    length(persona_scope)+length(source_scope_digest)+length(incarnation_id)+
                    length(manifest_digest)+length(route_digest)+length(formula_digest)+
                    length(state_digest)+length(graph_digest)+length(origin_digest)
                 ),0) FROM semantic_origins",
                [],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            )
            .unwrap(),
            origin_shape_before
        );
        assert_eq!(
            tx.query_row(
                "SELECT value FROM meta WHERE key='semantic_schema_version'",
                [],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .unwrap(),
            version_before
        );
        assert_eq!(
            tx.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            user_version_before
        );
        drop(tx);
        super::require_exact_semantic_schema_v4(&connection).unwrap();
    }

    #[test]
    fn semantic_v4_time_limit_is_persona_local_not_database_global() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE semantic_commits(
                   persona_scope BLOB NOT NULL,
                   semantic_revision INTEGER NOT NULL,
                   transition_kind TEXT NOT NULL
                 );",
            )
            .unwrap();
        let first = [0x01_u8; 32];
        let second = [0x02_u8; 32];
        for (persona_scope, revision) in [(first, 1_i64), (first, 2), (second, 1), (second, 2)] {
            connection
                .execute(
                    "INSERT INTO semantic_commits VALUES(?1,?2,'time')",
                    params![persona_scope.to_vec(), revision],
                )
                .unwrap();
        }
        let tx = connection.transaction().unwrap();
        verify_semantic_time_rows_per_persona_v5(&tx, 2).unwrap();
        tx.execute(
            "INSERT INTO semantic_commits VALUES(?1,3,'time')",
            params![second.to_vec()],
        )
        .unwrap();
        assert!(matches!(
            verify_semantic_time_rows_per_persona_v5(&tx, 2),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic_v5.time_rows.persona",
                limit: 2,
                actual: 3,
            })
        ));
    }

    #[test]
    fn semantic_v4_time_walk_is_keyset_scoped_by_persona() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE semantic_commits(
                   persona_scope BLOB NOT NULL,
                   semantic_revision INTEGER NOT NULL,
                   transition_kind TEXT NOT NULL
                 );",
            )
            .unwrap();
        let first = [0x10_u8; 32];
        let second = [0x20_u8; 32];
        let third = [0x30_u8; 32];
        for (persona, revision, kind) in [
            (second, 1_i64, "time"),
            (first, 2, "time"),
            (third, 1, "time"),
            (first, 1, "time"),
            (third, 2, "time"),
            (first, 3, "time"),
            ([0x05; 32], 1, "perception"),
        ] {
            connection
                .execute(
                    "INSERT INTO semantic_commits VALUES(?1,?2,?3)",
                    params![persona.to_vec(), revision, kind],
                )
                .unwrap();
        }
        let tx = connection.transaction().unwrap();
        let mut visited = Vec::new();
        let stats = for_each_semantic_time_persona_v5(&tx, |conn, persona_scope| {
            let raw: i64 = conn.query_row(
                "SELECT COUNT(*) FROM semantic_commits
                 WHERE transition_kind='time' AND persona_scope=?1",
                params![persona_scope.to_vec()],
                |row| row.get(0),
            )?;
            let count = u64::try_from(raw).unwrap();
            visited.push((persona_scope, count));
            Ok(count)
        })
        .unwrap();
        assert_eq!(visited, vec![(first, 3), (second, 1), (third, 2)]);
        assert_eq!(stats.persona_count, 3);
        assert_eq!(stats.total_rows, 6);
        assert_eq!(stats.max_persona_rows, 3);
    }

    #[test]
    fn semantic_v4_time_walk_rejects_first_forbidden_persona_before_visit() {
        let mut connection = Connection::open_in_memory().unwrap();
        connection
            .execute_batch(
                "CREATE TABLE semantic_commits(
                   persona_scope BLOB NOT NULL,
                   semantic_revision INTEGER NOT NULL,
                   transition_kind TEXT NOT NULL
                 );",
            )
            .unwrap();
        {
            let tx = connection.transaction().unwrap();
            {
                let mut insert = tx
                    .prepare(
                        "INSERT INTO semantic_commits
                         VALUES(?1,1,'time')",
                    )
                    .unwrap();
                for ordinal in 0..=MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL {
                    let mut persona_scope = [0_u8; 32];
                    persona_scope[..8].copy_from_slice(&ordinal.to_be_bytes());
                    insert.execute(params![persona_scope.to_vec()]).unwrap();
                }
            }
            tx.commit().unwrap();
        }

        let tx = connection.transaction().unwrap();
        let mut visits = 0_u64;
        assert!(matches!(
            for_each_semantic_time_persona_v5(&tx, |_conn, _persona_scope| {
                visits += 1;
                Ok(1)
            }),
            Err(StoreError::StorageBudgetExceeded {
                resource: "semantic_v5.time_personas.global",
                limit: MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL,
                actual,
            }) if actual == MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL + 1
        ));
        assert_eq!(visits, MAX_SEMANTIC_HISTORY_PERSONAS_GLOBAL);
    }

    #[test]
    fn consumed_history_at_retired_global_limit_does_not_block_a_new_persona() {
        let mut connection = Connection::open_in_memory().unwrap();
        install_current_v3_challenge_schema(&connection);
        connection
            .execute_batch(
                "CREATE VIEW semantic_evidence_authority AS
                 WITH RECURSIVE consumed(n) AS (
                   VALUES(1) UNION ALL SELECT n + 1 FROM consumed WHERE n < 65536
                 )
                 SELECT CAST(n AS BLOB) AS persona_scope,
                        zeroblob(32) AS perception_nonce_digest
                 FROM consumed;",
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();

        // The recursive view compactly represents the old lifetime cutoff
        // without allocating 65,536 semantic commitments.  Consumed history
        // is immutable and is bounded by the semantic/database budgets, not
        // by the pending challenge admission quota.
        enforce_perception_quota_v1(&transaction, [0xA5; 32]).unwrap();
    }

    #[test]
    fn pending_challenge_limits_reject_at_boundary_and_recover_below_it() {
        let mut connection = Connection::open_in_memory().unwrap();
        install_current_v3_challenge_schema(&connection);
        let transaction = connection.transaction().unwrap();
        let persona_ordinal = 0xB6_u64;
        let persona = wire::persona_scope_digest(
            &quota_id_v1(0x11, persona_ordinal),
            &quota_id_v1(0x22, persona_ordinal),
            None,
        );

        for nonce in 0..MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA {
            assert_eq!(
                insert_current_v3_challenge_row(&transaction, nonce, persona_ordinal),
                persona
            );
        }
        assert!(matches!(
            enforce_perception_quota_v1(&transaction, persona),
            Err(StoreError::StorageBudgetExceeded {
                resource: "perception_challenges.persona_pending",
                limit: MAX_PENDING_PERCEPTION_CHALLENGES_PER_PERSONA,
                ..
            })
        ));
        transaction
            .execute("DELETE FROM perception_challenges", [])
            .unwrap();
        enforce_perception_quota_v1(&transaction, persona).unwrap();

        for nonce in 0..MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL {
            insert_current_v3_challenge_row(&transaction, nonce, nonce);
        }
        assert!(matches!(
            enforce_perception_quota_v1(&transaction, persona),
            Err(StoreError::StorageBudgetExceeded {
                resource: "perception_challenges.global_pending",
                limit: MAX_PENDING_PERCEPTION_CHALLENGES_GLOBAL,
                ..
            })
        ));
        transaction
            .execute("DELETE FROM perception_challenges", [])
            .unwrap();
        enforce_perception_quota_v1(&transaction, persona).unwrap();
    }
}
