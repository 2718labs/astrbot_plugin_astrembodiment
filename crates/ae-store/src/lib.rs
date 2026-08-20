#![forbid(unsafe_code)]

use ae_continuum::CommitEnvelope;
use ae_contracts::Digest;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("stale base revision")]
    StaleRevision,
    #[error("candidate rejected: {0}")]
    CandidateRejected(&'static str),
    #[error("storage unavailable")]
    Unavailable,
}

pub trait Repository {
    fn current_revision(&self, scope: &Digest) -> Result<u64, StoreError>;
    fn commit(&mut self, scope: &Digest, envelope: CommitEnvelope) -> Result<(), StoreError>;
}
