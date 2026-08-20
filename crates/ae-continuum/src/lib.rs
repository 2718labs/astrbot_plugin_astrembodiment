#![forbid(unsafe_code)]

mod kv;

pub use kv::{
    key_digest, validate_canonical_value_len, validate_key, validate_scan_limit, validate_value,
    value_digest, CasOutcome, CompareAndSwap, ContinuumKey, ContinuumKv, ContinuumObjectKind,
    ContinuumValidationError, VersionedValue, KEY_DOMAIN, MAX_CANONICAL_VALUE_BYTES,
    MAX_LOGICAL_ID_BYTES, MAX_SCAN_LIMIT, VALUE_DOMAIN,
};

use ae_contracts::{Digest, TransitionReceipt};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalCoordinate {
    pub revision: u64,
    pub high_water_mark: u64,
    pub journal_digest: Digest,
    pub formula_digest: Digest,
}

pub fn hash_chain(previous: &Digest, payload: &[u8]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(previous);
    hasher.update(payload);
    *hasher.finalize().as_bytes()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitEnvelope {
    pub receipt: TransitionReceipt,
    pub delta_bytes: Vec<u8>,
}
