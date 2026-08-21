use ae_contracts::r7::{wire::domain_hash, Digest};
use std::collections::HashMap;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeKvError {
    Validation(ContinuumValidationError),
    FenceEpochMismatch { expected: u64, actual: u64 },
    RevisionMismatch { expected: u64, actual: u64 },
    ValueDigestMismatch,
    ScanGap { expected: u64, actual: u64 },
    AmbiguousLogicalStream,
    RangeIncomplete { expected: u64, actual: Option<u64> },
    ScanLimitExceeded { requested: u32, available: usize },
    RevisionOverflow,
    InvalidRange,
}

impl From<ContinuumValidationError> for NativeKvError {
    fn from(error: ContinuumValidationError) -> Self {
        Self::Validation(error)
    }
}

/// A native, revision-preserving KV authority used by the R7 producer lane.
/// It never accepts caller-provided digests as authority: key and value digests
/// are recomputed from their canonical bytes before a row is committed.
#[derive(Clone, Debug, Default)]
pub struct NativeContinuumKv {
    fence_epoch: u64,
    rows: HashMap<Digest, (ContinuumKey, Vec<VersionedValue>)>,
}

impl NativeContinuumKv {
    pub fn new(fence_epoch: u64) -> Self {
        Self {
            fence_epoch,
            rows: HashMap::new(),
        }
    }

    pub fn fence_epoch(&self) -> u64 {
        self.fence_epoch
    }
}

impl ContinuumKv for NativeContinuumKv {
    type Error = NativeKvError;

    fn get(&self, key: &ContinuumKey) -> Result<Option<VersionedValue>, Self::Error> {
        validate_key(key)?;
        Ok(self
            .rows
            .get(&key_digest(key)?)
            .and_then(|(_, rows)| rows.last().cloned()))
    }

    fn scan_contiguous(
        &self,
        scope_digest: &Digest,
        kind: ContinuumObjectKind,
        from_exclusive: u64,
        through_inclusive: u64,
        limit: u32,
    ) -> Result<Vec<VersionedValue>, Self::Error> {
        validate_scan_limit(limit)?;
        if through_inclusive < from_exclusive {
            return Err(NativeKvError::InvalidRange);
        }
        let matching: Vec<_> = self
            .rows
            .values()
            .filter(|(key, _)| key.scope_digest == *scope_digest && key.kind == kind)
            .collect();
        if matching.len() > 1 {
            return Err(NativeKvError::AmbiguousLogicalStream);
        }
        if through_inclusive == from_exclusive {
            return Ok(Vec::new());
        }

        let first_expected = from_exclusive
            .checked_add(1)
            .ok_or(NativeKvError::RevisionOverflow)?;
        let Some((_, rows)) = matching.first() else {
            return Err(NativeKvError::RangeIncomplete {
                expected: first_expected,
                actual: None,
            });
        };
        let mut ordered = rows.clone();
        ordered.sort_by_key(|value| value.revision);
        let mut out = Vec::new();
        let mut expected = first_expected;
        let Some(first) = ordered.iter().find(|value| value.revision == expected) else {
            let actual = ordered
                .iter()
                .find(|value| value.revision > expected)
                .map(|value| value.revision);
            return Err(NativeKvError::RangeIncomplete { expected, actual });
        };
        out.push(first.clone());
        if expected == through_inclusive {
            if out.len() > limit as usize {
                return Err(NativeKvError::ScanLimitExceeded {
                    requested: limit,
                    available: out.len(),
                });
            }
            return Ok(out);
        }
        loop {
            expected = expected
                .checked_add(1)
                .ok_or(NativeKvError::RevisionOverflow)?;
            let Some(value) = ordered.iter().find(|value| value.revision == expected) else {
                let actual = ordered
                    .iter()
                    .find(|value| value.revision > expected)
                    .map(|value| value.revision);
                if expected == through_inclusive {
                    return Err(NativeKvError::RangeIncomplete { expected, actual });
                }
                return Err(NativeKvError::ScanGap {
                    expected,
                    actual: actual.unwrap_or(expected),
                });
            };
            out.push(value.clone());
            if expected == through_inclusive {
                break;
            }
        }
        if out.len() > limit as usize {
            return Err(NativeKvError::ScanLimitExceeded {
                requested: limit,
                available: out.len(),
            });
        }
        Ok(out)
    }

    fn compare_and_swap(&mut self, request: &CompareAndSwap) -> Result<CasOutcome, Self::Error> {
        validate_key(&request.key)?;
        validate_value(&request.candidate)?;
        if request.fence_epoch != self.fence_epoch {
            return Err(NativeKvError::FenceEpochMismatch {
                expected: self.fence_epoch,
                actual: request.fence_epoch,
            });
        }
        let digest = key_digest(&request.key)?;
        let (_, rows) = self
            .rows
            .entry(digest)
            .or_insert_with(|| (request.key.clone(), Vec::new()));
        let current = rows.last();
        if let Some(current) = current {
            if current.revision == request.candidate.revision
                && current.value_digest == request.candidate.value_digest
            {
                return Ok(CasOutcome::Duplicate(current.clone()));
            }
        }
        let actual = current.map_or(0, |value| value.revision);
        if actual != request.expected_revision {
            return Ok(CasOutcome::Conflict {
                current_revision: actual,
                current_value_digest: current.map(|value| value.value_digest),
            });
        }
        let expected_next = request
            .expected_revision
            .checked_add(1)
            .ok_or(NativeKvError::RevisionOverflow)?;
        if request.candidate.revision != expected_next {
            return Err(NativeKvError::RevisionMismatch {
                expected: expected_next,
                actual: request.candidate.revision,
            });
        }
        if request.expected_value_digest != current.map(|value| value.value_digest) {
            return Ok(CasOutcome::Conflict {
                current_revision: actual,
                current_value_digest: current.map(|value| value.value_digest),
            });
        }
        rows.push(request.candidate.clone());
        Ok(CasOutcome::Committed(request.candidate.clone()))
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod native_tests {
    use super::*;

    fn key() -> ContinuumKey {
        ContinuumKey {
            scope_digest: [7; 32],
            kind: ContinuumObjectKind::Snapshot,
            logical_id: b"state".to_vec(),
        }
    }

    fn candidate(revision: u64, bytes: &[u8]) -> VersionedValue {
        VersionedValue {
            revision,
            value_digest: value_digest(bytes).unwrap(),
            canonical_bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn native_cas_reopen_and_replay_are_byte_stable() {
        let mut kv = NativeContinuumKv::new(11);
        let first = candidate(1, b"canonical-state");
        let request = CompareAndSwap {
            key: key(),
            expected_revision: 0,
            expected_value_digest: None,
            fence_epoch: 11,
            candidate: first.clone(),
        };
        assert_eq!(
            kv.compare_and_swap(&request),
            Ok(CasOutcome::Committed(first.clone()))
        );
        assert_eq!(
            kv.compare_and_swap(&request),
            Ok(CasOutcome::Duplicate(first.clone()))
        );
        assert_eq!(kv.get(&key()).unwrap(), Some(first));
    }

    #[test]
    fn native_cas_rejects_foreign_fence_and_bad_digest() {
        let mut kv = NativeContinuumKv::new(11);
        let request = CompareAndSwap {
            key: key(),
            expected_revision: 0,
            expected_value_digest: None,
            fence_epoch: 10,
            candidate: candidate(1, b"canonical-state"),
        };
        assert!(matches!(
            kv.compare_and_swap(&request),
            Err(NativeKvError::FenceEpochMismatch { .. })
        ));
        let mut bad = candidate(1, b"canonical-state");
        bad.value_digest = [0; 32];
        let request = CompareAndSwap {
            key: key(),
            expected_revision: 0,
            expected_value_digest: None,
            fence_epoch: 11,
            candidate: bad,
        };
        assert!(matches!(
            kv.compare_and_swap(&request),
            Err(NativeKvError::Validation(
                ContinuumValidationError::ValueDigestMismatch
            ))
        ));
    }

    #[test]
    fn native_cas_rejects_stale_revision() {
        let mut kv = NativeContinuumKv::new(11);
        let first = candidate(1, b"a");
        let request = CompareAndSwap {
            key: key(),
            expected_revision: 0,
            expected_value_digest: None,
            fence_epoch: 11,
            candidate: first.clone(),
        };
        kv.compare_and_swap(&request).unwrap();
        let stale = CompareAndSwap {
            key: key(),
            expected_revision: 0,
            expected_value_digest: None,
            fence_epoch: 11,
            candidate: candidate(1, b"b"),
        };
        assert!(matches!(
            kv.compare_and_swap(&stale),
            Ok(CasOutcome::Conflict { .. })
        ));
    }

    fn second_key() -> ContinuumKey {
        ContinuumKey {
            scope_digest: [7; 32],
            kind: ContinuumObjectKind::Snapshot,
            logical_id: b"other-state".to_vec(),
        }
    }

    #[test]
    fn scan_rejects_multiple_logical_streams() {
        let mut kv = NativeContinuumKv::new(11);
        for (key, bytes) in [(key(), b"a".as_slice()), (second_key(), b"b".as_slice())] {
            let candidate = candidate(1, bytes);
            kv.rows
                .insert(key_digest(&key).unwrap(), (key, vec![candidate]));
        }
        assert_eq!(
            kv.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 0, 1, 2),
            Err(NativeKvError::AmbiguousLogicalStream)
        );
    }

    #[test]
    fn scan_rejects_incomplete_range_before_limit_truncation() {
        let mut kv = NativeContinuumKv::new(11);
        let k = key();
        kv.rows.insert(
            key_digest(&k).unwrap(),
            (k, vec![candidate(1, b"a"), candidate(3, b"c")]),
        );
        assert_eq!(
            kv.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 0, 3, 1),
            Err(NativeKvError::ScanGap {
                expected: 2,
                actual: 3,
            })
        );
        assert_eq!(
            kv.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 1, 3, 2),
            Err(NativeKvError::RangeIncomplete {
                expected: 2,
                actual: Some(3),
            })
        );
        assert_eq!(
            kv.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 0, 4, 4),
            Err(NativeKvError::ScanGap {
                expected: 2,
                actual: 3,
            })
        );

        let mut complete_prefix = NativeContinuumKv::new(11);
        let k = key();
        complete_prefix.rows.insert(
            key_digest(&k).unwrap(),
            (k, vec![candidate(1, b"a"), candidate(2, b"b")]),
        );
        assert_eq!(
            complete_prefix.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 0, 3, 4,),
            Err(NativeKvError::RangeIncomplete {
                expected: 3,
                actual: None,
            })
        );
    }

    #[test]
    fn scan_rejects_invalid_bounds_and_excess_limit() {
        let mut kv = NativeContinuumKv::new(11);
        let k = key();
        kv.rows.insert(
            key_digest(&k).unwrap(),
            (k, vec![candidate(1, b"a"), candidate(2, b"b")]),
        );
        assert_eq!(
            kv.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 2, 1, 2),
            Err(NativeKvError::InvalidRange)
        );
        assert_eq!(
            kv.scan_contiguous(&[7; 32], ContinuumObjectKind::Snapshot, 0, 2, 1),
            Err(NativeKvError::ScanLimitExceeded {
                requested: 1,
                available: 2,
            })
        );
    }

    #[test]
    fn cas_revision_overflow_is_a_stable_error() {
        let mut kv = NativeContinuumKv::new(11);
        let k = key();
        kv.rows.insert(
            key_digest(&k).unwrap(),
            (k.clone(), vec![candidate(u64::MAX, b"max")]),
        );
        let request = CompareAndSwap {
            key: k,
            expected_revision: u64::MAX,
            expected_value_digest: Some(value_digest(b"max").unwrap()),
            fence_epoch: 11,
            candidate: candidate(0, b"next"),
        };
        assert_eq!(
            kv.compare_and_swap(&request),
            Err(NativeKvError::RevisionOverflow)
        );
    }
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
