#![forbid(unsafe_code)]

//! Continuum persistence primitives: append-only journal hash chain,
//! commit envelope and mechanical replay verification.
//!
//! The journal never stores raw text: only event kind, encoded canonical
//! event bytes, digests and receipts.

// Alpha's persisted journal ABI remains at this crate root. The authority's
// bounded KV kernel is an additive R7-only namespace so neither side aliases
// the other at the public boundary.
mod kv;

pub mod r7 {
    pub use super::kv::{
        key_digest, validate_canonical_value_len, validate_key, validate_scan_limit,
        validate_value, value_digest, CasOutcome, CompareAndSwap, ContinuumKey, ContinuumKv,
        ContinuumObjectKind, ContinuumValidationError, VersionedValue, KEY_DOMAIN,
        MAX_CANONICAL_VALUE_BYTES, MAX_LOGICAL_ID_BYTES, MAX_SCAN_LIMIT, VALUE_DOMAIN,
    };
}

use ae_contracts::{wire, Digest, TransitionReceipt};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalCoordinate {
    pub revision: u64,
    pub high_water_mark: u64,
    pub journal_digest: Digest,
    pub formula_digest: Digest,
}

/// One link of the journal hash chain: J_n = H(J_{n-1} || event || receipt).
pub fn hash_chain(previous: &Digest, payload: &[u8]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(previous);
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

pub fn chain_link(previous: &Digest, event_bytes: &[u8], receipt_bytes: &[u8]) -> Digest {
    let mut payload = Vec::with_capacity(event_bytes.len() + receipt_bytes.len());
    payload.extend_from_slice(event_bytes);
    payload.extend_from_slice(receipt_bytes);
    hash_chain(previous, &payload)
}

/// A persisted journal row. Receipt bytes are the canonical binary encoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalRow {
    pub revision: u64,
    pub scope_digest: Digest,
    pub base_revision: u64,
    pub event_kind: String,
    pub event_bytes: Vec<u8>,
    pub event_digest: Digest,
    pub receipt_bytes: Vec<u8>,
    pub chain_digest: Digest,
}

impl JournalRow {
    pub fn decode_receipt(&self) -> Result<TransitionReceipt, wire::WireError> {
        wire::decode_transition_receipt(&self.receipt_bytes)
    }
}

/// The candidate a transition produces before it enters the commit lane.
/// delta_bytes is empty in G0 (no-op transitions change no neural state);
/// later gates put the canonical structural delta here.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitEnvelope {
    pub event_kind: String,
    pub event_bytes: Vec<u8>,
    pub receipt: TransitionReceipt,
    /// Chain seed: the committed genesis snapshot digest for the first entry,
    /// otherwise the previous chain digest.
    pub chain_seed: Digest,
    pub delta_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub checked: usize,
    pub ok: bool,
    pub base_revision: u64,
    pub final_revision: u64,
    pub final_chain_digest: Digest,
    pub first_error: Option<String>,
}

/// Mechanically verify a journal: contiguity, per-row event digest, receipt
/// consistency and the hash chain. A candidate failure must never move the
/// active pointer; this function only reads.
pub fn verify_replay(chain_seed: Digest, rows: &[JournalRow]) -> ReplayReport {
    let mut previous_chain = chain_seed;
    let mut expected_revision: Option<u64> = None;
    let mut final_revision = 0;
    let mut first_error: Option<String> = None;

    for row in rows {
        if let Some(expected) = expected_revision {
            if row.revision != expected {
                first_error = Some(format!(
                    "revision gap at row {}: expected {}, got {}",
                    row.revision, expected, row.revision
                ));
                break;
            }
        }
        expected_revision = Some(row.revision + 1);

        let event_ok = match wire::decode_event(&row.event_bytes) {
            Ok(event) => wire::event_digest(&event) == row.event_digest,
            Err(error) => {
                first_error = Some(format!("event decode failed at {}: {error}", row.revision));
                break;
            }
        };
        if !event_ok {
            first_error = Some(format!("event digest mismatch at {}", row.revision));
            break;
        }

        let receipt_ok = match row.decode_receipt() {
            Ok(receipt) => {
                receipt.base_revision == row.base_revision
                    && receipt.event_digest == row.event_digest
                    && receipt.scope_digest == row.scope_digest
            }
            Err(error) => {
                first_error = Some(format!(
                    "receipt decode failed at {}: {error}",
                    row.revision
                ));
                break;
            }
        };
        if !receipt_ok {
            first_error = Some(format!("receipt mismatch at {}", row.revision));
            break;
        }

        let recomputed = chain_link(&previous_chain, &row.event_bytes, &row.receipt_bytes);
        if recomputed != row.chain_digest {
            first_error = Some(format!("hash chain broken at {}", row.revision));
            break;
        }
        previous_chain = recomputed;
        final_revision = row.revision;
    }

    ReplayReport {
        checked: rows.len(),
        ok: first_error.is_none(),
        base_revision: rows.first().map(|row| row.base_revision).unwrap_or(0),
        final_revision,
        final_chain_digest: previous_chain,
        first_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        wire, CanonicalEvent, CommitStatus, InvariantResiduals, ScopeRef, TimeAdvance,
        TransitionReceipt,
    };

    fn scope() -> ScopeRef {
        ScopeRef {
            bot_token: [1; 16],
            persona_token: [2; 16],
            relation_token: None,
            session_token: [3; 16],
        }
    }

    fn receipt(
        base: u64,
        next: u64,
        scope_digest: Digest,
        event_digest: Digest,
    ) -> TransitionReceipt {
        TransitionReceipt {
            schema_version: 1,
            formula_digest: [4; 32],
            scope_digest,
            event_digest,
            authority_digest: [5; 32],
            base_revision: base,
            next_revision: next,
            state_before: [6; 32],
            state_after: [6; 32],
            graph_after: [7; 32],
            action_contract: Some([8; 32]),
            active_nodes: 16_384,
            active_edges: 0,
            residuals: InvariantResiduals::default(),
            status: CommitStatus::Committed,
        }
    }

    fn row(revision: u64, scope_digest: Digest, chain_seed: Digest) -> JournalRow {
        let event = CanonicalEvent::TimeAdvance(TimeAdvance {
            event_id: [revision as u8; 16],
            scope: scope(),
            elapsed_ms: 1,
        });
        let event_bytes = wire::encode_event(&event);
        let event_digest = wire::event_digest(&event);
        let receipt = receipt(
            revision.saturating_sub(1),
            revision,
            scope_digest,
            event_digest,
        );
        let receipt_bytes = wire::encode_transition_receipt(&receipt);
        let chain_digest = chain_link(&chain_seed, &event_bytes, &receipt_bytes);
        JournalRow {
            revision,
            scope_digest,
            base_revision: revision.saturating_sub(1),
            event_kind: "time_advance".to_string(),
            event_bytes,
            event_digest,
            receipt_bytes,
            chain_digest,
        }
    }

    #[test]
    fn valid_chain_verifies() {
        let seed = [9; 32];
        let scope_digest = wire::scope_digest(&scope());
        let rows = vec![
            row(1, scope_digest, seed),
            row(2, scope_digest, seed),
            row(3, scope_digest, seed),
        ];
        // rebuild chain links with the actual previous link
        let mut linked = Vec::with_capacity(3);
        let mut previous = seed;
        for mut entry in rows {
            entry.chain_digest = chain_link(&previous, &entry.event_bytes, &entry.receipt_bytes);
            previous = entry.chain_digest;
            linked.push(entry);
        }
        let report = verify_replay(seed, &linked);
        assert!(report.ok, "{:?}", report.first_error);
        assert_eq!(report.checked, 3);
        assert_eq!(report.final_revision, 3);
        assert_eq!(report.final_chain_digest, previous);
    }

    #[test]
    fn tampered_chain_fails_at_the_right_row() {
        let seed = [9; 32];
        let scope_digest = wire::scope_digest(&scope());
        let mut rows = vec![row(1, scope_digest, seed), row(2, scope_digest, seed)];
        let mut previous = seed;
        for entry in &mut rows {
            entry.chain_digest = chain_link(&previous, &entry.event_bytes, &entry.receipt_bytes);
            previous = entry.chain_digest;
        }
        // Tamper with row 1 event bytes (chain link 2 now breaks).
        rows[0].event_bytes[3] ^= 0xFF;
        let report = verify_replay(seed, &rows);
        assert!(!report.ok);
        assert!(report
            .first_error
            .unwrap()
            .contains("event digest mismatch"));
    }

    #[test]
    fn gap_in_revisions_fails() {
        let seed = [9; 32];
        let scope_digest = wire::scope_digest(&scope());
        let mut rows = vec![row(1, scope_digest, seed), row(3, scope_digest, seed)];
        let mut previous = seed;
        for entry in &mut rows {
            entry.chain_digest = chain_link(&previous, &entry.event_bytes, &entry.receipt_bytes);
            previous = entry.chain_digest;
        }
        let report = verify_replay(seed, &rows);
        assert!(!report.ok);
        assert!(report.first_error.unwrap().contains("revision gap"));
    }

    #[test]
    fn receipt_bytes_decode_round_trip() {
        let seed = [9; 32];
        let scope_digest = wire::scope_digest(&scope());
        let entry = row(1, scope_digest, seed);
        let decoded = entry.decode_receipt().unwrap();
        assert_eq!(decoded.base_revision, 0);
        assert_eq!(decoded.next_revision, 1);
        assert_eq!(decoded.event_digest, entry.event_digest);
        assert_eq!(decoded.status, CommitStatus::Committed);
        assert_eq!(decoded.residuals, InvariantResiduals::default());
    }
}
