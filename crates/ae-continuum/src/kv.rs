use ae_contracts::{wire::domain_hash, Digest};

pub const KEY_DOMAIN: &[u8] = b"ae.continuum-kv.key.v1";
pub const VALUE_DOMAIN: &[u8] = b"ae.continuum-kv.value.v1";
pub const MAX_LOGICAL_ID_BYTES: usize = 256;
pub const MAX_CANONICAL_VALUE_BYTES: usize = 67_108_864;
pub const MAX_SCAN_LIMIT: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ContinuumObjectKind {
    Snapshot = 1,
    Delta = 2,
    ActiveHead = 3,
    TransitionReceipt = 4,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContinuumKey {
    pub scope_digest: Digest,
    pub kind: ContinuumObjectKind,
    pub logical_id: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedValue {
    pub revision: u64,
    pub value_digest: Digest,
    pub canonical_bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareAndSwap {
    pub key: ContinuumKey,
    pub expected_revision: u64,
    pub expected_value_digest: Option<Digest>,
    pub fence_epoch: u64,
    pub candidate: VersionedValue,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CasOutcome {
    Committed(VersionedValue),
    Duplicate(VersionedValue),
    Conflict {
        current_revision: u64,
        current_value_digest: Option<Digest>,
    },
}

pub trait ContinuumKv {
    type Error;

    fn get(&self, key: &ContinuumKey) -> Result<Option<VersionedValue>, Self::Error>;
    fn scan_contiguous(
        &self,
        scope_digest: &Digest,
        kind: ContinuumObjectKind,
        from_exclusive: u64,
        through_inclusive: u64,
        limit: u32,
    ) -> Result<Vec<VersionedValue>, Self::Error>;
    fn compare_and_swap(&mut self, request: &CompareAndSwap) -> Result<CasOutcome, Self::Error>;
    fn flush(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContinuumValidationError {
    EmptyLogicalId,
    LogicalIdTooLong { actual: usize },
    CanonicalValueTooLong { actual: usize },
    ValueDigestMismatch,
    InvalidScanLimit { actual: u32 },
}

pub fn validate_key(key: &ContinuumKey) -> Result<(), ContinuumValidationError> {
    let actual = key.logical_id.len();
    if actual == 0 {
        Err(ContinuumValidationError::EmptyLogicalId)
    } else if actual > MAX_LOGICAL_ID_BYTES {
        Err(ContinuumValidationError::LogicalIdTooLong { actual })
    } else {
        Ok(())
    }
}

pub fn key_digest(key: &ContinuumKey) -> Result<Digest, ContinuumValidationError> {
    validate_key(key)?;
    let kind = (key.kind as u16).to_le_bytes();
    Ok(domain_hash(
        KEY_DOMAIN,
        &[&key.scope_digest, &kind, &key.logical_id],
    ))
}

pub fn value_digest(canonical_bytes: &[u8]) -> Result<Digest, ContinuumValidationError> {
    validate_canonical_value_len(canonical_bytes.len())?;
    Ok(domain_hash(VALUE_DOMAIN, &[canonical_bytes]))
}

pub fn validate_canonical_value_len(actual: usize) -> Result<(), ContinuumValidationError> {
    if actual > MAX_CANONICAL_VALUE_BYTES {
        Err(ContinuumValidationError::CanonicalValueTooLong { actual })
    } else {
        Ok(())
    }
}

pub fn validate_value(value: &VersionedValue) -> Result<(), ContinuumValidationError> {
    if value_digest(&value.canonical_bytes)? != value.value_digest {
        Err(ContinuumValidationError::ValueDigestMismatch)
    } else {
        Ok(())
    }
}

pub fn validate_scan_limit(limit: u32) -> Result<(), ContinuumValidationError> {
    if (1..=MAX_SCAN_LIMIT).contains(&limit) {
        Ok(())
    } else {
        Err(ContinuumValidationError::InvalidScanLimit { actual: limit })
    }
}
