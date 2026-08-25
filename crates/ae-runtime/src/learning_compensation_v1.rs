#![forbid(unsafe_code)]

//! Text-free, append-only Phase 0 teacher compensation.
//!
//! This module owns the native side of the V2 bridge: source provenance is
//! immutable at enqueue, while the semantic cursor/telemetry/checkpoint are
//! claimed atomically and CAS-verified before Candidate B can append `u`.

use crate::{semantic, validate_perception_scope, AstrRuntime, RuntimeError};
use ae_contracts::{
    canonical_one_learning_compensation_policy_digest_v1, evidence_vector_from_values,
    learning_compensation_formula_digest_v1, perception_dimension_values, wire, Digest,
    EvidenceVector, LearningCompensationCommitStatusV1, LearningCompensationEnqueueReceiptV1,
    LearningCompensationNoChangeReceiptV1, LearningCompensationReceiptV1,
    LearningCompensationTerminalReceiptV1, LearningCompensationTerminalStatusV1,
    NativeTelemetryReceiptV1, ScopeRef, SemanticLearningCompensationApplyV1,
    SemanticLearningCompensationClaimV1, SemanticLearningCompensationEnqueueV1,
    SemanticLearningCompensationTerminalV1, LEARNING_COMPENSATION_CANONICAL_ONE_POLICY_BODY_V1,
    LEARNING_COMPENSATION_CANONICAL_ONE_POLICY_DIGEST_V1,
    LEARNING_COMPENSATION_ENQUEUE_RECEIPT_SCHEMA_V1, LEARNING_COMPENSATION_POLICY_SHA256_DOMAIN_V1,
    LEARNING_COMPENSATION_RECEIPT_SCHEMA_V1, LEARNING_COMPENSATION_TERMINAL_SCHEMA_V1,
};
use ae_fixed::Fixed;
use ae_neurofield::{initial_state_from_manifest, NeuralField, SparseGraph, REGION_LAYOUT};
use ae_store::{
    LearningCompensationClaimBindingV1, LearningCompensationCommitV1,
    LearningCompensationEnqueueOutcomeV1, LearningCompensationEnqueueReceiptRowV1,
    LearningCompensationJobRowV1, LearningCompensationJobStatusV1,
    LearningCompensationTerminalCommitV1, NewLearningCompensationJobV1, StoreError,
};
use sha2::{Digest as Sha2Digest, Sha256};

const LEARNING_JOB_DOMAIN_V1: &[u8] = b"astr-embodiment/phase0-learning-job-v1";
const LEARNING_COMPENSATION_VECTOR_DOMAIN_V1: &[u8] =
    b"astr-embodiment/phase0-learning-compensation-v1";
const LEARNING_NO_CHANGE_REASON_DOMAIN_V1: &[u8] =
    b"astr-embodiment/phase0-learning-no-change-reason-v1";

const TEACHER_CONF_MIN: i64 = 900_000;
const ENTER: i64 = 200_000;
const EXIT: i64 = 100_000;
const U_MAX: i64 = 250_000;
const RISE_MAX: i64 = 50_000;
const FALL_MAX: i64 = 100_000;

/// Verify the only admissible policy artifact from the frozen canonical JSON
/// and parameters before accepting any host-supplied digest. The returned
/// digest is deliberately SHA-256 (the cross-runtime policy commitment), not
/// the BLAKE3 receipt/hash domain used elsewhere in native wire contracts.
fn verified_canonical_one_policy_digest_v1() -> Result<Digest, RuntimeError> {
    let mut hasher = Sha256::new();
    hasher.update(LEARNING_COMPENSATION_POLICY_SHA256_DOMAIN_V1);
    hasher.update(LEARNING_COMPENSATION_CANONICAL_ONE_POLICY_BODY_V1);
    let digest: Digest = hasher.finalize().into();
    if digest != LEARNING_COMPENSATION_CANONICAL_ONE_POLICY_DIGEST_V1
        || digest != canonical_one_learning_compensation_policy_digest_v1()
    {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(digest)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningCompensationEnqueueAvailabilityV1 {
    Available,
    UnavailableLocalConfidence,
    Unavailable,
}

impl LearningCompensationEnqueueAvailabilityV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "AVAILABLE",
            Self::UnavailableLocalConfidence => "UNAVAILABLE_LOCAL_CONFIDENCE",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningCompensationEnqueueStatusV1 {
    Queued,
    Replayed,
    Unavailable,
}

impl LearningCompensationEnqueueStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "QUEUED",
            Self::Replayed => "REPLAYED",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LearningCompensationEnqueueDecisionV1 {
    pub availability: LearningCompensationEnqueueAvailabilityV1,
    pub status: LearningCompensationEnqueueStatusV1,
    pub job_id: Option<Digest>,
    pub source_event_digest: Digest,
    pub source_text_digest: Digest,
    pub source_revision: u64,
    pub formula_digest: Digest,
    pub local_estimator_formula_digest: Digest,
    pub learning_formula_digest: Option<Digest>,
    pub policy_digest: Digest,
    pub request_digest: Option<Digest>,
    pub terminal_status: Option<LearningCompensationTerminalStatusV1>,
    pub receipt_digest: Option<Digest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningCompensationClaimStatusV1 {
    Claimed,
    Replayed,
    Terminal,
    Unavailable,
}

impl LearningCompensationClaimStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claimed => "CLAIMED",
            Self::Replayed => "REPLAYED",
            Self::Terminal => "TERMINAL",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Clone, Debug)]
pub struct LearningCompensationClaimDecisionV1 {
    pub job_id: Digest,
    pub status: LearningCompensationClaimStatusV1,
    pub lease_token: Option<Digest>,
    pub lease_epoch: Option<u64>,
    pub source_event_digest: Option<Digest>,
    pub source_text_digest: Option<Digest>,
    pub source_revision: Option<u64>,
    pub request_digest: Option<Digest>,
    pub base_revision: Option<u64>,
    pub formula_digest: Option<Digest>,
    pub local_estimator_formula_digest: Option<Digest>,
    pub learning_formula_digest: Option<Digest>,
    pub policy_digest: Option<Digest>,
    pub provider_digest: Option<Digest>,
    pub model_digest: Option<Digest>,
    pub prompt_digest: Option<Digest>,
    pub schema_digest: Option<Digest>,
    pub telemetry_digest: Option<Digest>,
    pub checkpoint_digest: Option<Digest>,
    pub terminal_status: Option<LearningCompensationTerminalStatusV1>,
    pub receipt_digest: Option<Digest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LearningCompensationApplyStatusV1 {
    Committed,
    NoChange,
    Replayed,
    StaleRetry,
    Unavailable,
}

impl LearningCompensationApplyStatusV1 {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Committed => "COMMITTED",
            Self::NoChange => "NO_CHANGE",
            Self::Replayed => "REPLAYED",
            Self::StaleRetry => "STALE_RETRY",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Clone, Debug)]
pub enum LearningCompensationApplyDecisionV1 {
    Committed(LearningCompensationReceiptV1),
    NoChange(LearningCompensationNoChangeReceiptV1),
    Replayed(LearningCompensationReceiptV1),
    StaleRetry { job_id: Digest },
    Unavailable { job_id: Digest },
}

impl LearningCompensationApplyDecisionV1 {
    pub fn status(&self) -> LearningCompensationApplyStatusV1 {
        match self {
            Self::Committed(_) => LearningCompensationApplyStatusV1::Committed,
            Self::NoChange(_) => LearningCompensationApplyStatusV1::NoChange,
            Self::Replayed(_) => LearningCompensationApplyStatusV1::Replayed,
            Self::StaleRetry { .. } => LearningCompensationApplyStatusV1::StaleRetry,
            Self::Unavailable { .. } => LearningCompensationApplyStatusV1::Unavailable,
        }
    }

    pub fn job_id(&self) -> Digest {
        match self {
            Self::Committed(receipt) | Self::Replayed(receipt) => receipt.job_id,
            Self::NoChange(receipt) => receipt.job_id,
            Self::StaleRetry { job_id } | Self::Unavailable { job_id } => *job_id,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LearningCompensationTerminalDecisionV1 {
    pub receipt: LearningCompensationTerminalReceiptV1,
}

fn unavailable_claim(job_id: Digest) -> LearningCompensationClaimDecisionV1 {
    LearningCompensationClaimDecisionV1 {
        job_id,
        status: LearningCompensationClaimStatusV1::Unavailable,
        lease_token: None,
        lease_epoch: None,
        source_event_digest: None,
        source_text_digest: None,
        source_revision: None,
        request_digest: None,
        base_revision: None,
        formula_digest: None,
        local_estimator_formula_digest: None,
        learning_formula_digest: None,
        policy_digest: None,
        provider_digest: None,
        model_digest: None,
        prompt_digest: None,
        schema_digest: None,
        telemetry_digest: None,
        checkpoint_digest: None,
        terminal_status: None,
        receipt_digest: None,
    }
}

fn terminal_claim(
    receipt: &LearningCompensationTerminalReceiptV1,
) -> LearningCompensationClaimDecisionV1 {
    LearningCompensationClaimDecisionV1 {
        job_id: receipt.job_id,
        status: LearningCompensationClaimStatusV1::Terminal,
        lease_token: None,
        lease_epoch: None,
        source_event_digest: None,
        source_text_digest: None,
        source_revision: None,
        request_digest: None,
        base_revision: None,
        formula_digest: None,
        local_estimator_formula_digest: None,
        learning_formula_digest: None,
        policy_digest: None,
        provider_digest: None,
        model_digest: None,
        prompt_digest: None,
        schema_digest: None,
        telemetry_digest: None,
        checkpoint_digest: None,
        terminal_status: Some(receipt.status),
        receipt_digest: Some(receipt.receipt_digest),
    }
}

fn unavailable_enqueue(
    request: &SemanticLearningCompensationEnqueueV1,
) -> LearningCompensationEnqueueDecisionV1 {
    LearningCompensationEnqueueDecisionV1 {
        availability: LearningCompensationEnqueueAvailabilityV1::Unavailable,
        status: LearningCompensationEnqueueStatusV1::Unavailable,
        job_id: None,
        source_event_digest: request.source_event_digest,
        source_text_digest: request.source_text_digest,
        source_revision: request.source_revision,
        formula_digest: request.formula_digest,
        local_estimator_formula_digest: request.local_estimator_formula_digest,
        learning_formula_digest: None,
        policy_digest: request.policy_digest,
        request_digest: None,
        terminal_status: None,
        receipt_digest: None,
    }
}

#[derive(Clone)]
struct LearningSemanticContextV1 {
    semantic_scope: Digest,
    formula_digest: Digest,
    baseline_field: NeuralField,
    baseline_graph: SparseGraph,
    revision: u64,
    legacy_unavailable: bool,
}

fn derive_job_id(scope_digest: &Digest, request: &SemanticLearningCompensationEnqueueV1) -> Digest {
    let policy_digest = canonical_one_learning_compensation_policy_digest_v1();
    let learning_formula_digest = learning_compensation_formula_digest_v1(
        &request.formula_digest,
        &request.local_estimator_formula_digest,
        &policy_digest,
    );
    let mut hasher = Sha256::new();
    hasher.update(LEARNING_JOB_DOMAIN_V1);
    hasher.update(scope_digest);
    hasher.update(request.source_event_digest);
    hasher.update(request.source_text_digest);
    hasher.update(request.source_revision.to_le_bytes());
    hasher.update(request.provider_digest);
    hasher.update(request.model_digest);
    hasher.update(request.prompt_digest);
    hasher.update(request.schema_digest);
    hasher.update(request.formula_digest);
    hasher.update(request.local_estimator_formula_digest);
    hasher.update(policy_digest);
    hasher.update(learning_formula_digest);
    hasher.finalize().into()
}

fn compensation_digest(vector: &EvidenceVector) -> Digest {
    let mut bytes = Vec::with_capacity(15 * 8);
    for value in perception_dimension_values(vector) {
        bytes.extend_from_slice(&value.encode());
    }
    wire::domain_hash(LEARNING_COMPENSATION_VECTOR_DOMAIN_V1, &[&bytes])
}

fn vector_bytes(vector: &EvidenceVector) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(15 * 8);
    for value in perception_dimension_values(vector) {
        bytes.extend_from_slice(&value.encode());
    }
    bytes
}

fn decode_vector(bytes: &[u8], signed: bool) -> Result<EvidenceVector, RuntimeError> {
    if bytes.len() != 15 * 8 {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    let mut values = [Fixed::ZERO; 15];
    for (index, chunk) in bytes.chunks_exact(8).enumerate() {
        let mut raw = [0u8; 8];
        raw.copy_from_slice(chunk);
        let value = Fixed::decode(raw);
        let valid = if signed {
            (-U_MAX..=U_MAX).contains(&value.raw())
        } else {
            (0..=Fixed::ONE.raw()).contains(&value.raw())
        };
        if !valid {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        values[index] = value;
    }
    Ok(evidence_vector_from_values(values))
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], RuntimeError> {
        let end = self
            .offset
            .checked_add(count)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, RuntimeError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, RuntimeError> {
        let mut value = [0u8; 2];
        value.copy_from_slice(self.take(2)?);
        Ok(u16::from_le_bytes(value))
    }

    fn u64(&mut self) -> Result<u64, RuntimeError> {
        let mut value = [0u8; 8];
        value.copy_from_slice(self.take(8)?);
        Ok(u64::from_le_bytes(value))
    }

    fn digest(&mut self) -> Result<Digest, RuntimeError> {
        let mut value = [0u8; 32];
        value.copy_from_slice(self.take(32)?);
        Ok(value)
    }

    fn vector(&mut self, signed: bool) -> Result<EvidenceVector, RuntimeError> {
        decode_vector(self.take(15 * 8)?, signed)
    }

    fn eof(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

fn decode_enqueue(bytes: &[u8]) -> Result<SemanticLearningCompensationEnqueueV1, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let schema_version = cursor.u16()?;
    let source_event_digest = cursor.digest()?;
    let source_text_digest = cursor.digest()?;
    let source_revision = cursor.u64()?;
    let local_vector = cursor.vector(false)?;
    let local_confidence_vector = match cursor.u8()? {
        0 => None,
        1 => Some(cursor.vector(false)?),
        _ => return Err(RuntimeError::InvalidLearningCompensation),
    };
    let request = SemanticLearningCompensationEnqueueV1 {
        schema_version,
        source_event_digest,
        source_text_digest,
        source_revision,
        local_vector,
        local_confidence_vector,
        policy_digest: cursor.digest()?,
        provider_digest: cursor.digest()?,
        model_digest: cursor.digest()?,
        prompt_digest: cursor.digest()?,
        schema_digest: cursor.digest()?,
        formula_digest: cursor.digest()?,
        local_estimator_formula_digest: cursor.digest()?,
        source_telemetry_digest: cursor.digest()?,
        source_checkpoint_digest: cursor.digest()?,
    };
    if !cursor.eof()
        || (!request.validate_available() && !request.validate_unavailable_local_confidence())
        || request.canonical_bytes() != bytes
    {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(request)
}

fn frozen_request_from_row(
    row: &LearningCompensationJobRowV1,
) -> Result<SemanticLearningCompensationEnqueueV1, RuntimeError> {
    let request = decode_enqueue(&row.request_bytes)?;
    let canonical_policy_digest = verified_canonical_one_policy_digest_v1()?;
    if row.source_event_digest != request.source_event_digest
        || row.source_text_digest != request.source_text_digest
        || row.source_base_revision != request.source_revision
        || row.policy_digest != request.policy_digest
        || row.schema_digest != request.schema_digest
        || row.formula_digest != request.formula_digest
        || request.policy_digest != canonical_policy_digest
        || row.telemetry_digest != request.source_telemetry_digest
        || row.checkpoint_digest != request.source_checkpoint_digest
        || row.request_digest != request.request_digest(&row.scope_digest)
        || row.job_id != derive_job_id(&row.scope_digest, &request)
    {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(request)
}

fn enqueue_receipt_for_request(
    request: &SemanticLearningCompensationEnqueueV1,
    job_id: Digest,
    request_digest: Digest,
    learning_formula_digest: Digest,
) -> LearningCompensationEnqueueReceiptV1 {
    LearningCompensationEnqueueReceiptV1 {
        schema: LEARNING_COMPENSATION_ENQUEUE_RECEIPT_SCHEMA_V1.to_owned(),
        job_id,
        source_event_digest: request.source_event_digest,
        source_text_digest: request.source_text_digest,
        source_revision: request.source_revision,
        request_digest,
        formula_digest: request.formula_digest,
        local_estimator_formula_digest: request.local_estimator_formula_digest,
        learning_formula_digest,
        source_telemetry_digest: request.source_telemetry_digest,
        source_checkpoint_digest: request.source_checkpoint_digest,
        policy_digest: request.policy_digest,
        provider_digest: request.provider_digest,
        model_digest: request.model_digest,
        prompt_digest: request.prompt_digest,
        schema_digest: request.schema_digest,
        receipt_digest: [0; 32],
    }
    .seal()
}

fn encode_enqueue_receipt(receipt: &LearningCompensationEnqueueReceiptV1) -> Vec<u8> {
    let mut bytes = receipt.canonical_bytes_without_receipt_digest();
    bytes.extend_from_slice(&receipt.receipt_digest);
    bytes
}

fn decode_enqueue_receipt(
    bytes: &[u8],
) -> Result<LearningCompensationEnqueueReceiptV1, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let job_id = cursor.digest()?;
    let source_event_digest = cursor.digest()?;
    let source_text_digest = cursor.digest()?;
    let request_digest = cursor.digest()?;
    let formula_digest = cursor.digest()?;
    let local_estimator_formula_digest = cursor.digest()?;
    let learning_formula_digest = cursor.digest()?;
    let source_telemetry_digest = cursor.digest()?;
    let source_checkpoint_digest = cursor.digest()?;
    let policy_digest = cursor.digest()?;
    let provider_digest = cursor.digest()?;
    let model_digest = cursor.digest()?;
    let prompt_digest = cursor.digest()?;
    let schema_digest = cursor.digest()?;
    let source_revision = cursor.u64()?;
    let receipt = LearningCompensationEnqueueReceiptV1 {
        schema: LEARNING_COMPENSATION_ENQUEUE_RECEIPT_SCHEMA_V1.to_owned(),
        job_id,
        source_event_digest,
        source_text_digest,
        source_revision,
        request_digest,
        formula_digest,
        local_estimator_formula_digest,
        learning_formula_digest,
        source_telemetry_digest,
        source_checkpoint_digest,
        policy_digest,
        provider_digest,
        model_digest,
        prompt_digest,
        schema_digest,
        receipt_digest: cursor.digest()?,
    };
    if !cursor.eof() || !receipt.validate() || encode_enqueue_receipt(&receipt) != bytes {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(receipt)
}

fn enqueue_receipt_from_row(
    row: &LearningCompensationJobRowV1,
    persisted: &LearningCompensationEnqueueReceiptRowV1,
) -> Result<LearningCompensationEnqueueReceiptV1, RuntimeError> {
    if persisted.job_id != row.job_id {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    let request = frozen_request_from_row(row)?;
    let receipt = decode_enqueue_receipt(&persisted.receipt_bytes)?;
    let learning_formula_digest = learning_compensation_formula_digest_v1(
        &request.formula_digest,
        &request.local_estimator_formula_digest,
        &request.policy_digest,
    );
    if persisted.receipt_digest != receipt.receipt_digest
        || receipt.job_id != row.job_id
        || receipt.source_event_digest != row.source_event_digest
        || receipt.source_text_digest != row.source_text_digest
        || receipt.source_revision != row.source_base_revision
        || receipt.request_digest != row.request_digest
        || receipt.formula_digest != request.formula_digest
        || receipt.local_estimator_formula_digest != request.local_estimator_formula_digest
        || receipt.learning_formula_digest != learning_formula_digest
        || receipt.source_telemetry_digest != row.telemetry_digest
        || receipt.source_checkpoint_digest != row.checkpoint_digest
        || receipt.policy_digest != request.policy_digest
        || receipt.provider_digest != request.provider_digest
        || receipt.model_digest != request.model_digest
        || receipt.prompt_digest != request.prompt_digest
        || receipt.schema_digest != request.schema_digest
    {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(receipt)
}

fn encode_receipt(receipt: &LearningCompensationReceiptV1) -> Vec<u8> {
    let mut bytes = receipt.canonical_bytes_without_receipt_digest();
    bytes.extend_from_slice(&receipt.receipt_digest);
    bytes
}

fn decode_receipt(bytes: &[u8]) -> Result<LearningCompensationReceiptV1, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let job_id = cursor.digest()?;
    let source_event_digest = cursor.digest()?;
    let source_text_digest = cursor.digest()?;
    let formula_digest = cursor.digest()?;
    let local_estimator_formula_digest = cursor.digest()?;
    let learning_formula_digest = cursor.digest()?;
    let telemetry_digest = cursor.digest()?;
    let checkpoint_digest = cursor.digest()?;
    let compensation_digest = cursor.digest()?;
    let policy_digest = cursor.digest()?;
    let schema_digest = cursor.digest()?;
    let teacher_digest = cursor.digest()?;
    let provider_digest = cursor.digest()?;
    let model_digest = cursor.digest()?;
    let prompt_digest = cursor.digest()?;
    let request_digest = cursor.digest()?;
    let source_revision = cursor.u64()?;
    let base_revision = cursor.u64()?;
    let next_checkpoint_revision = cursor.u64()?;
    let eligible_dimension_count = cursor.u8()?;
    let changed_dimension_count = cursor.u8()?;
    let u_next = cursor.vector(true)?;
    let status = match cursor.u8()? {
        1 => LearningCompensationCommitStatusV1::Committed,
        2 => LearningCompensationCommitStatusV1::NoChange,
        3 => LearningCompensationCommitStatusV1::Replayed,
        _ => return Err(RuntimeError::InvalidLearningCompensation),
    };
    let receipt = LearningCompensationReceiptV1 {
        schema: LEARNING_COMPENSATION_RECEIPT_SCHEMA_V1.to_owned(),
        job_id,
        source_event_digest,
        source_text_digest,
        source_revision,
        base_revision,
        next_checkpoint_revision,
        formula_digest,
        local_estimator_formula_digest,
        learning_formula_digest,
        telemetry_digest,
        checkpoint_digest,
        compensation_digest,
        policy_digest,
        schema_digest,
        teacher_digest,
        provider_digest,
        model_digest,
        prompt_digest,
        request_digest,
        eligible_dimension_count,
        changed_dimension_count,
        u_next,
        status,
        receipt_digest: cursor.digest()?,
    };
    if !cursor.eof() || !receipt.validate() || encode_receipt(&receipt) != bytes {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(receipt)
}

fn encode_no_change_receipt(receipt: &LearningCompensationNoChangeReceiptV1) -> Vec<u8> {
    let mut bytes = receipt.canonical_bytes_without_receipt_digest();
    bytes.extend_from_slice(&receipt.receipt_digest);
    bytes
}

fn decode_no_change_receipt(
    bytes: &[u8],
) -> Result<LearningCompensationNoChangeReceiptV1, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let job_id = cursor.digest()?;
    let source_event_digest = cursor.digest()?;
    let source_text_digest = cursor.digest()?;
    let formula_digest = cursor.digest()?;
    let local_estimator_formula_digest = cursor.digest()?;
    let learning_formula_digest = cursor.digest()?;
    let telemetry_digest = cursor.digest()?;
    let checkpoint_digest = cursor.digest()?;
    let policy_digest = cursor.digest()?;
    let provider_digest = cursor.digest()?;
    let model_digest = cursor.digest()?;
    let prompt_digest = cursor.digest()?;
    let schema_digest = cursor.digest()?;
    let teacher_digest = cursor.digest()?;
    let request_digest = cursor.digest()?;
    let source_revision = cursor.u64()?;
    let base_revision = cursor.u64()?;
    let eligible_dimension_count = cursor.u8()?;
    let changed_dimension_count = cursor.u8()?;
    let receipt = LearningCompensationNoChangeReceiptV1 {
        schema: LEARNING_COMPENSATION_RECEIPT_SCHEMA_V1.to_owned(),
        job_id,
        source_event_digest,
        source_text_digest,
        source_revision,
        base_revision,
        formula_digest,
        local_estimator_formula_digest,
        learning_formula_digest,
        telemetry_digest,
        checkpoint_digest,
        policy_digest,
        provider_digest,
        model_digest,
        prompt_digest,
        schema_digest,
        teacher_digest,
        request_digest,
        eligible_dimension_count,
        changed_dimension_count,
        receipt_digest: cursor.digest()?,
    };
    if !cursor.eof() || !receipt.validate() || encode_no_change_receipt(&receipt) != bytes {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(receipt)
}

fn encode_terminal_receipt(receipt: &LearningCompensationTerminalReceiptV1) -> Vec<u8> {
    let mut bytes = receipt.canonical_bytes_without_receipt_digest();
    bytes.extend_from_slice(&receipt.receipt_digest);
    bytes
}

fn decode_terminal_receipt(
    bytes: &[u8],
) -> Result<LearningCompensationTerminalReceiptV1, RuntimeError> {
    let mut cursor = Cursor::new(bytes);
    let job_id = cursor.digest()?;
    let source_event_digest = cursor.digest()?;
    let source_text_digest = cursor.digest()?;
    let request_digest = cursor.digest()?;
    let formula_digest = cursor.digest()?;
    let local_estimator_formula_digest = cursor.digest()?;
    let learning_formula_digest = cursor.digest()?;
    let policy_digest = cursor.digest()?;
    let provider_digest = cursor.digest()?;
    let model_digest = cursor.digest()?;
    let prompt_digest = cursor.digest()?;
    let schema_digest = cursor.digest()?;
    let reason_digest = cursor.digest()?;
    let checkpoint_digest = cursor.digest()?;
    let source_revision = cursor.u64()?;
    let status = match cursor.u8()? {
        1 => LearningCompensationTerminalStatusV1::AbandonedInputUnavailable,
        2 => LearningCompensationTerminalStatusV1::Rejected,
        3 => LearningCompensationTerminalStatusV1::Expired,
        _ => return Err(RuntimeError::InvalidLearningCompensation),
    };
    let receipt = LearningCompensationTerminalReceiptV1 {
        schema: LEARNING_COMPENSATION_TERMINAL_SCHEMA_V1.to_owned(),
        job_id,
        status,
        source_event_digest,
        source_text_digest,
        source_revision,
        request_digest,
        formula_digest,
        local_estimator_formula_digest,
        learning_formula_digest,
        policy_digest,
        provider_digest,
        model_digest,
        prompt_digest,
        schema_digest,
        reason_digest,
        checkpoint_digest,
        receipt_digest: cursor.digest()?,
    };
    if !cursor.eof() || !receipt.validate() || encode_terminal_receipt(&receipt) != bytes {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(receipt)
}

fn terminal_receipt_from_row(
    row: &LearningCompensationJobRowV1,
) -> Result<Option<LearningCompensationTerminalReceiptV1>, RuntimeError> {
    if !matches!(
        row.status,
        LearningCompensationJobStatusV1::Rejected
            | LearningCompensationJobStatusV1::Abandoned
            | LearningCompensationJobStatusV1::Expired
    ) {
        return Ok(None);
    }
    let bytes = row
        .receipt_bytes
        .as_deref()
        .ok_or(RuntimeError::InvalidLearningCompensation)?;
    let receipt = decode_terminal_receipt(bytes)?;
    if row.receipt_digest != Some(receipt.receipt_digest)
        || receipt.job_id != row.job_id
        || receipt.request_digest != row.request_digest
    {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    Ok(Some(receipt))
}

fn completed_enqueue_replay_from_row(
    row: &LearningCompensationJobRowV1,
) -> Result<(Option<LearningCompensationTerminalStatusV1>, Digest), RuntimeError> {
    match row.status {
        LearningCompensationJobStatusV1::Committed => {
            let bytes = row
                .receipt_bytes
                .as_deref()
                .ok_or(RuntimeError::InvalidLearningCompensation)?;
            let receipt = decode_receipt(bytes)?;
            if row.receipt_digest != Some(receipt.receipt_digest)
                || receipt.job_id != row.job_id
                || receipt.request_digest != row.request_digest
            {
                return Err(RuntimeError::InvalidLearningCompensation);
            }
            Ok((None, receipt.receipt_digest))
        }
        LearningCompensationJobStatusV1::NoChange => {
            let bytes = row
                .receipt_bytes
                .as_deref()
                .ok_or(RuntimeError::InvalidLearningCompensation)?;
            let receipt = decode_no_change_receipt(bytes)?;
            if row.receipt_digest != Some(receipt.receipt_digest)
                || receipt.job_id != row.job_id
                || receipt.request_digest != row.request_digest
            {
                return Err(RuntimeError::InvalidLearningCompensation);
            }
            Ok((None, receipt.receipt_digest))
        }
        LearningCompensationJobStatusV1::Rejected
        | LearningCompensationJobStatusV1::Abandoned
        | LearningCompensationJobStatusV1::Expired => {
            let receipt =
                terminal_receipt_from_row(row)?.ok_or(RuntimeError::InvalidLearningCompensation)?;
            Ok((Some(receipt.status), receipt.receipt_digest))
        }
        LearningCompensationJobStatusV1::Pending | LearningCompensationJobStatusV1::Claimed => {
            Err(RuntimeError::InvalidLearningCompensation)
        }
    }
}

fn enqueue_decision_from_existing(
    semantic_scope: &Digest,
    request: &SemanticLearningCompensationEnqueueV1,
    request_digest: Digest,
    learning_formula_digest: Digest,
    row: &LearningCompensationJobRowV1,
    persisted_enqueue_receipt: &LearningCompensationEnqueueReceiptRowV1,
) -> Result<LearningCompensationEnqueueDecisionV1, RuntimeError> {
    let frozen = frozen_request_from_row(row)?;
    if row.scope_digest != *semantic_scope
        || row.request_digest != request_digest
        || frozen != *request
        || row.job_id != derive_job_id(semantic_scope, request)
    {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    let enqueue_receipt = enqueue_receipt_from_row(row, persisted_enqueue_receipt)?;
    let expected_enqueue_receipt =
        enqueue_receipt_for_request(request, row.job_id, request_digest, learning_formula_digest);
    if enqueue_receipt != expected_enqueue_receipt {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    let (status, terminal_status, receipt_digest) = match row.status {
        LearningCompensationJobStatusV1::Pending | LearningCompensationJobStatusV1::Claimed => {
            // A still-open job is not a replayed terminal. Its durable enqueue
            // attestation gives a retried host a receipt without inventing a
            // completed outcome.
            (
                LearningCompensationEnqueueStatusV1::Queued,
                None,
                enqueue_receipt.receipt_digest,
            )
        }
        LearningCompensationJobStatusV1::Committed
        | LearningCompensationJobStatusV1::NoChange
        | LearningCompensationJobStatusV1::Rejected
        | LearningCompensationJobStatusV1::Abandoned
        | LearningCompensationJobStatusV1::Expired => {
            let (terminal_status, receipt_digest) = completed_enqueue_replay_from_row(row)?;
            (
                LearningCompensationEnqueueStatusV1::Replayed,
                terminal_status,
                receipt_digest,
            )
        }
    };
    Ok(LearningCompensationEnqueueDecisionV1 {
        availability: LearningCompensationEnqueueAvailabilityV1::Available,
        status,
        job_id: Some(row.job_id),
        source_event_digest: request.source_event_digest,
        source_text_digest: request.source_text_digest,
        source_revision: request.source_revision,
        formula_digest: request.formula_digest,
        local_estimator_formula_digest: request.local_estimator_formula_digest,
        learning_formula_digest: Some(learning_formula_digest),
        policy_digest: request.policy_digest,
        request_digest: Some(request_digest),
        terminal_status,
        receipt_digest: Some(receipt_digest),
    })
}

fn mul6_nonnegative(left: i64, right: i64) -> Result<i64, RuntimeError> {
    if left < 0 || right < 0 {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    let value = (i128::from(left) * i128::from(right) + 500_000) / 1_000_000;
    i64::try_from(value).map_err(|_| RuntimeError::InvalidLearningCompensation)
}

fn smul6(error: i64, gain: i64) -> Result<i64, RuntimeError> {
    if gain < 0 {
        return Err(RuntimeError::InvalidLearningCompensation);
    }
    let magnitude = error
        .checked_abs()
        .ok_or(RuntimeError::InvalidLearningCompensation)?;
    let scaled = mul6_nonnegative(magnitude, gain)?;
    Ok(if error < 0 { -scaled } else { scaled })
}

fn abs_raw(value: i64) -> Result<i64, RuntimeError> {
    value
        .checked_abs()
        .ok_or(RuntimeError::InvalidLearningCompensation)
}

fn clamp_raw(value: i64, lower: i64, upper: i64) -> i64 {
    value.clamp(lower, upper)
}

#[derive(Clone, Debug)]
struct CandidateBResultV1 {
    u_next: EvidenceVector,
    eligible_dimension_count: u8,
    changed_dimension_count: u8,
}

fn candidate_b_v1(
    local: &EvidenceVector,
    local_confidence: &EvidenceVector,
    teacher: &EvidenceVector,
    teacher_confidence: &EvidenceVector,
    previous: &EvidenceVector,
    telemetry: &NativeTelemetryReceiptV1,
) -> Result<CandidateBResultV1, RuntimeError> {
    if !telemetry.validate() || telemetry.native_gate == Fixed::ZERO {
        return Err(RuntimeError::LearningCompensationUnavailable);
    }
    let bottleneck = telemetry
        .energy
        .headroom
        .min(telemetry.capacity.headroom)
        .min(telemetry.residual_health)
        .raw();
    if !(1..=Fixed::ONE.raw()).contains(&bottleneck) {
        return Err(RuntimeError::LearningCompensationUnavailable);
    }
    let local_values = perception_dimension_values(local);
    let local_confidence_values = perception_dimension_values(local_confidence);
    let teacher_values = perception_dimension_values(teacher);
    let teacher_confidence_values = perception_dimension_values(teacher_confidence);
    let previous_values = perception_dimension_values(previous);
    let mut next_values = previous_values;
    let mut eligible_dimension_count = 0u8;
    let mut changed_dimension_count = 0u8;

    for dimension in 0..15 {
        let local_value = local_values[dimension].raw();
        let local_confidence_value = local_confidence_values[dimension].raw();
        let teacher_value = teacher_values[dimension].raw();
        let teacher_confidence_value = teacher_confidence_values[dimension].raw();
        let previous_value = previous_values[dimension].raw();
        let error = teacher_value
            .checked_sub(local_value)
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let abs_error = abs_raw(error)?;
        let eligible = teacher_confidence_value >= TEACHER_CONF_MIN && abs_error >= ENTER;
        if !eligible {
            continue;
        }
        eligible_dimension_count = eligible_dimension_count
            .checked_add(1)
            .ok_or(RuntimeError::InvalidLearningCompensation)?;

        let local_confidence_floor_half = local_confidence_value / 2;
        let certainty_complement = Fixed::ONE
            .raw()
            .checked_sub(local_confidence_floor_half)
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let q = mul6_nonnegative(teacher_confidence_value, certainty_complement)?;
        // The approved Phase 0 policy artifact is the native frozen
        // consistency vector of ONE. A missing policy digest is rejected at
        // enqueue; host never supplies an unverified dynamic gain.
        let gain = mul6_nonnegative(mul6_nonnegative(q, Fixed::ONE.raw())?, bottleneck)?;
        let target = clamp_raw(smul6(error, gain)?, -U_MAX, U_MAX);

        let mut desired = if previous_value == 0 && abs_error < ENTER {
            0
        } else if previous_value != 0 && abs_error <= EXIT {
            0
        } else if abs_error >= ENTER {
            target
        } else {
            previous_value
        };
        if (previous_value > 0 && desired < 0) || (previous_value < 0 && desired > 0) {
            desired = 0;
        }
        let step_cap = if abs_raw(desired)? > abs_raw(previous_value)? {
            mul6_nonnegative(RISE_MAX, bottleneck)?
        } else {
            FALL_MAX
        };
        let delta = desired
            .checked_sub(previous_value)
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let stepped = previous_value
            .checked_add(clamp_raw(delta, -step_cap, step_cap))
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let next = clamp_raw(stepped, -U_MAX, U_MAX);
        if next != previous_value {
            changed_dimension_count = changed_dimension_count
                .checked_add(1)
                .ok_or(RuntimeError::InvalidLearningCompensation)?;
        }
        next_values[dimension] = Fixed::from_raw(next);
    }

    Ok(CandidateBResultV1 {
        u_next: evidence_vector_from_values(next_values),
        eligible_dimension_count,
        changed_dimension_count,
    })
}

fn no_change_reason_digest(candidate: &CandidateBResultV1) -> Digest {
    let body = [
        candidate.eligible_dimension_count,
        candidate.changed_dimension_count,
    ];
    wire::domain_hash(LEARNING_NO_CHANGE_REASON_DOMAIN_V1, &[&body])
}

pub(crate) fn committed_compensation_by_region_v1(
    store: &ae_store::Store,
    semantic_scope: &Digest,
) -> Result<[Fixed; REGION_LAYOUT.len()], RuntimeError> {
    let vector = match store.read_learning_compensation_state_v1(semantic_scope)? {
        Some(state) => {
            let vector = decode_vector(&state.u_bytes, true)?;
            if compensation_digest(&vector) != state.compensation_digest {
                return Err(RuntimeError::InvalidLearningCompensation);
            }
            vector
        }
        None => EvidenceVector::default(),
    };
    semantic::compensation_by_region_from_vector_v1(&vector)
}

impl AstrRuntime {
    fn learning_semantic_scope_v1(&mut self, scope: &ScopeRef) -> Result<Digest, RuntimeError> {
        validate_perception_scope(scope)?;
        let (bot_token, persona_token, semantic_scope) = {
            let hot = self.hot_for(scope)?;
            (hot.bot_token, hot.persona_token, hot.semantic_scope)
        };
        if scope.bot_token != bot_token || scope.persona_token != persona_token {
            return Err(RuntimeError::InvalidPerceptionScope);
        }
        Ok(semantic_scope)
    }

    fn learning_semantic_context_v1(
        &mut self,
        scope: &ScopeRef,
    ) -> Result<LearningSemanticContextV1, RuntimeError> {
        validate_perception_scope(scope)?;
        let (
            bot_token,
            persona_token,
            semantic_scope,
            genesis_formula_digest,
            manifest,
            development_seed_digest,
            revision,
            legacy_unavailable,
        ) = {
            let hot = self.hot_for(scope)?;
            (
                hot.bot_token,
                hot.persona_token,
                hot.semantic_scope,
                hot.formula_digest,
                hot.identity.manifest.clone(),
                hot.identity.development_seed_digest,
                hot.semantic_revision,
                hot.semantic_legacy_unavailable,
            )
        };
        if scope.bot_token != bot_token || scope.persona_token != persona_token {
            return Err(RuntimeError::InvalidPerceptionScope);
        }
        let (baseline_field, baseline_graph) = initial_state_from_manifest(
            &manifest,
            &genesis_formula_digest,
            &development_seed_digest,
        );
        if !baseline_field.validate() || !baseline_graph.validate() {
            return Err(RuntimeError::InvalidNeuralState);
        }
        Ok(LearningSemanticContextV1 {
            semantic_scope,
            formula_digest: semantic::phase0_semantic_formula_digest_v1(&genesis_formula_digest)?,
            baseline_field,
            baseline_graph,
            revision,
            legacy_unavailable,
        })
    }

    fn verified_learning_telemetry_at_v1(
        &self,
        context: &LearningSemanticContextV1,
        revision: u64,
    ) -> Result<NativeTelemetryReceiptV1, RuntimeError> {
        if context.legacy_unavailable || revision == 0 || revision > context.revision {
            return Err(RuntimeError::LearningCompensationUnavailable);
        }
        let (_, _, committed) = self.semantic_snapshot_at(
            &context.semantic_scope,
            &context.formula_digest,
            &context.baseline_field,
            &context.baseline_graph,
            revision,
        )?;
        let (_, telemetry, _) = committed.ok_or(RuntimeError::LearningCompensationUnavailable)?;
        if !telemetry.validate()
            || telemetry.scope_digest != context.semantic_scope
            || telemetry.formula_digest != context.formula_digest
            || telemetry.next_revision != revision
            || telemetry.native_gate == Fixed::ZERO
        {
            return Err(RuntimeError::LearningCompensationUnavailable);
        }
        Ok(telemetry)
    }

    /// Enqueue only a verified, text-free source descriptor. A missing real
    /// per-dimension local confidence is a normal fail-closed response rather
    /// than an invitation to synthesize one from aggregate confidence.
    pub fn enqueue_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        request: &SemanticLearningCompensationEnqueueV1,
    ) -> Result<LearningCompensationEnqueueDecisionV1, RuntimeError> {
        if !request.validate_common() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let canonical_policy_digest = verified_canonical_one_policy_digest_v1()?;
        if request.policy_digest != canonical_policy_digest {
            return Ok(unavailable_enqueue(request));
        }
        if !request.has_local_confidence() {
            if !request.validate_unavailable_local_confidence() {
                return Err(RuntimeError::InvalidLearningCompensation);
            }
            return Ok(LearningCompensationEnqueueDecisionV1 {
                availability: LearningCompensationEnqueueAvailabilityV1::UnavailableLocalConfidence,
                status: LearningCompensationEnqueueStatusV1::Unavailable,
                job_id: None,
                source_event_digest: request.source_event_digest,
                source_text_digest: request.source_text_digest,
                source_revision: request.source_revision,
                formula_digest: request.formula_digest,
                local_estimator_formula_digest: request.local_estimator_formula_digest,
                learning_formula_digest: None,
                policy_digest: request.policy_digest,
                request_digest: None,
                terminal_status: None,
                receipt_digest: None,
            });
        }
        if !request.validate_available() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let context = match self.learning_semantic_context_v1(scope) {
            Ok(value) => value,
            Err(RuntimeError::LearningCompensationUnavailable)
            | Err(RuntimeError::LegacyUnattested) => return Ok(unavailable_enqueue(request)),
            Err(error) => return Err(error),
        };
        let request_digest = request.request_digest(&context.semantic_scope);
        let learning_formula_digest = learning_compensation_formula_digest_v1(
            &request.formula_digest,
            &request.local_estimator_formula_digest,
            &request.policy_digest,
        );
        let job_id = derive_job_id(&context.semantic_scope, request);
        let enqueue_receipt =
            enqueue_receipt_for_request(request, job_id, request_digest, learning_formula_digest);

        // A durable job already carries a sealed acceptance receipt. Re-entering
        // it must be deterministic even when the current runtime cannot rebuild
        // the original source telemetry (for example immediately after restart
        // or a later legacy/gate transition). New jobs still require the source
        // telemetry verification below.
        if let Some(existing) = self.store.read_learning_compensation_job_v1(&job_id)? {
            let persisted_enqueue_receipt = self
                .store
                .read_learning_compensation_enqueue_receipt_v1(&job_id)?
                .ok_or(RuntimeError::InvalidLearningCompensation)?;
            return enqueue_decision_from_existing(
                &context.semantic_scope,
                request,
                request_digest,
                learning_formula_digest,
                &existing,
                &persisted_enqueue_receipt,
            );
        }
        let source_telemetry =
            match self.verified_learning_telemetry_at_v1(&context, request.source_revision) {
                Ok(value) => value,
                Err(RuntimeError::LearningCompensationUnavailable)
                | Err(RuntimeError::LegacyUnattested) => return Ok(unavailable_enqueue(request)),
                Err(error) => return Err(error),
            };
        if request.formula_digest != context.formula_digest
            || source_telemetry.event_digest != request.source_event_digest
            || source_telemetry.formula_digest != request.formula_digest
            || source_telemetry.telemetry_digest != request.source_telemetry_digest
            || source_telemetry.checkpoint_digest != request.source_checkpoint_digest
        {
            return Ok(unavailable_enqueue(request));
        }
        let job = NewLearningCompensationJobV1 {
            job_id,
            scope_digest: context.semantic_scope,
            source_event_digest: request.source_event_digest,
            source_text_digest: request.source_text_digest,
            source_base_revision: request.source_revision,
            request_digest,
            request_bytes: request.canonical_bytes(),
            policy_digest: request.policy_digest,
            schema_digest: request.schema_digest,
            formula_digest: request.formula_digest,
            telemetry_digest: request.source_telemetry_digest,
            checkpoint_digest: request.source_checkpoint_digest,
            enqueue_receipt_bytes: encode_enqueue_receipt(&enqueue_receipt),
            enqueue_receipt_digest: enqueue_receipt.receipt_digest,
        };
        let outcome = self.store.enqueue_learning_compensation_job_v1(&job)?;
        let (row, persisted_enqueue_receipt) = match outcome {
            LearningCompensationEnqueueOutcomeV1::Queued {
                job,
                enqueue_receipt,
            }
            | LearningCompensationEnqueueOutcomeV1::Replayed {
                job,
                enqueue_receipt,
            } => (job, enqueue_receipt),
        };
        enqueue_decision_from_existing(
            &context.semantic_scope,
            request,
            request_digest,
            learning_formula_digest,
            &row,
            &persisted_enqueue_receipt,
        )
    }

    /// Lease a job and atomically bind the returned lease to the current
    /// verified semantic cursor. Reclaiming an existing lease requires the
    /// previous token, which is how one stale retry remains causally closed.
    pub fn claim_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        claim: &SemanticLearningCompensationClaimV1,
    ) -> Result<LearningCompensationClaimDecisionV1, RuntimeError> {
        if !claim.validate() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let canonical_policy_digest = verified_canonical_one_policy_digest_v1()?;
        let semantic_scope = self.learning_semantic_scope_v1(scope)?;
        let existing = self
            .store
            .read_learning_compensation_job_v1(&claim.job_id)?
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        if existing.scope_digest != semantic_scope
            || existing.request_digest != claim.expected_request_digest
        {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let frozen = frozen_request_from_row(&existing)?;
        match existing.status {
            LearningCompensationJobStatusV1::Committed
            | LearningCompensationJobStatusV1::NoChange => {
                return Ok(LearningCompensationClaimDecisionV1 {
                    job_id: existing.job_id,
                    status: LearningCompensationClaimStatusV1::Replayed,
                    lease_token: None,
                    lease_epoch: None,
                    source_event_digest: None,
                    source_text_digest: None,
                    source_revision: None,
                    request_digest: None,
                    base_revision: None,
                    formula_digest: None,
                    local_estimator_formula_digest: None,
                    learning_formula_digest: None,
                    policy_digest: None,
                    provider_digest: None,
                    model_digest: None,
                    prompt_digest: None,
                    schema_digest: None,
                    telemetry_digest: None,
                    checkpoint_digest: None,
                    terminal_status: None,
                    receipt_digest: existing.receipt_digest,
                });
            }
            LearningCompensationJobStatusV1::Rejected
            | LearningCompensationJobStatusV1::Abandoned
            | LearningCompensationJobStatusV1::Expired => {
                let receipt = terminal_receipt_from_row(&existing)?
                    .ok_or(RuntimeError::InvalidLearningCompensation)?;
                return Ok(terminal_claim(&receipt));
            }
            LearningCompensationJobStatusV1::Pending | LearningCompensationJobStatusV1::Claimed => {
            }
        }
        let context = self.learning_semantic_context_v1(scope)?;
        if context.semantic_scope != semantic_scope {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let telemetry = match self.verified_learning_telemetry_at_v1(&context, context.revision) {
            Ok(value) => value,
            Err(RuntimeError::LearningCompensationUnavailable)
            | Err(RuntimeError::LegacyUnattested) => return Ok(unavailable_claim(claim.job_id)),
            Err(error) => return Err(error),
        };
        if existing.formula_digest != frozen.formula_digest
            || frozen.policy_digest != canonical_policy_digest
            || existing.formula_digest != context.formula_digest
            || telemetry.formula_digest != existing.formula_digest
        {
            return Ok(unavailable_claim(claim.job_id));
        }
        let row = self
            .store
            .claim_learning_compensation_job_v1(&claim.job_id, claim.previous_lease_token)?;
        if row.status != LearningCompensationJobStatusV1::Claimed {
            return Ok(unavailable_claim(row.job_id));
        }
        let lease_token = row
            .lease_token
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let learning_formula_digest = learning_compensation_formula_digest_v1(
            &frozen.formula_digest,
            &frozen.local_estimator_formula_digest,
            &frozen.policy_digest,
        );
        self.store
            .bind_learning_compensation_claim_v1(&LearningCompensationClaimBindingV1 {
                job_id: row.job_id,
                lease_token,
                base_revision: context.revision,
                formula_digest: telemetry.formula_digest,
                telemetry_digest: telemetry.telemetry_digest,
                checkpoint_digest: telemetry.checkpoint_digest,
            })?;
        Ok(LearningCompensationClaimDecisionV1 {
            job_id: row.job_id,
            status: LearningCompensationClaimStatusV1::Claimed,
            lease_token: Some(lease_token),
            lease_epoch: Some(row.lease_epoch),
            source_event_digest: Some(row.source_event_digest),
            source_text_digest: Some(row.source_text_digest),
            source_revision: Some(row.source_base_revision),
            request_digest: Some(row.request_digest),
            base_revision: Some(context.revision),
            formula_digest: Some(telemetry.formula_digest),
            local_estimator_formula_digest: Some(frozen.local_estimator_formula_digest),
            learning_formula_digest: Some(learning_formula_digest),
            policy_digest: Some(frozen.policy_digest),
            provider_digest: Some(frozen.provider_digest),
            model_digest: Some(frozen.model_digest),
            prompt_digest: Some(frozen.prompt_digest),
            schema_digest: Some(frozen.schema_digest),
            telemetry_digest: Some(telemetry.telemetry_digest),
            checkpoint_digest: Some(telemetry.checkpoint_digest),
            terminal_status: None,
            receipt_digest: None,
        })
    }

    /// Verify the current claimed telemetry triple, independently recompute
    /// Candidate B, and append only `u`. It deliberately never mutates the
    /// semantic field/graph or any expression projection.
    pub fn apply_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        apply: &SemanticLearningCompensationApplyV1,
    ) -> Result<LearningCompensationApplyDecisionV1, RuntimeError> {
        if !apply.validate() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let canonical_policy_digest = verified_canonical_one_policy_digest_v1()?;
        let context = self.learning_semantic_context_v1(scope)?;
        let row = self
            .store
            .read_learning_compensation_job_v1(&apply.job_id)?
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        if row.scope_digest != context.semantic_scope {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let frozen = frozen_request_from_row(&row)?;
        if apply.expected_request_digest != row.request_digest
            || apply.expected_formula_digest != frozen.formula_digest
            || frozen.policy_digest != canonical_policy_digest
            || apply.provider_digest != frozen.provider_digest
            || apply.model_digest != frozen.model_digest
            || apply.prompt_digest != frozen.prompt_digest
        {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        match row.status {
            LearningCompensationJobStatusV1::Committed => {
                let bytes = row
                    .receipt_bytes
                    .as_deref()
                    .ok_or(RuntimeError::InvalidLearningCompensation)?;
                let receipt = decode_receipt(bytes)?;
                if receipt.request_digest != row.request_digest || receipt.job_id != row.job_id {
                    return Err(RuntimeError::InvalidLearningCompensation);
                }
                return Ok(LearningCompensationApplyDecisionV1::Replayed(receipt));
            }
            LearningCompensationJobStatusV1::NoChange => {
                let bytes = row
                    .receipt_bytes
                    .as_deref()
                    .ok_or(RuntimeError::InvalidLearningCompensation)?;
                let receipt = decode_no_change_receipt(bytes)?;
                if receipt.request_digest != row.request_digest || receipt.job_id != row.job_id {
                    return Err(RuntimeError::InvalidLearningCompensation);
                }
                return Ok(LearningCompensationApplyDecisionV1::NoChange(receipt));
            }
            LearningCompensationJobStatusV1::Rejected
            | LearningCompensationJobStatusV1::Abandoned
            | LearningCompensationJobStatusV1::Expired => {
                return Ok(LearningCompensationApplyDecisionV1::Unavailable { job_id: row.job_id });
            }
            LearningCompensationJobStatusV1::Pending => {
                return Ok(LearningCompensationApplyDecisionV1::Unavailable { job_id: row.job_id });
            }
            LearningCompensationJobStatusV1::Claimed => {}
        }
        if row.lease_token != Some(apply.lease_token) {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let telemetry = match self.verified_learning_telemetry_at_v1(&context, context.revision) {
            Ok(value) => value,
            Err(RuntimeError::LearningCompensationUnavailable)
            | Err(RuntimeError::LegacyUnattested) => {
                return Ok(LearningCompensationApplyDecisionV1::Unavailable { job_id: row.job_id });
            }
            Err(error) => return Err(error),
        };
        if apply.expected_base_revision != context.revision
            || apply.expected_formula_digest != telemetry.formula_digest
            || apply.expected_telemetry_digest != telemetry.telemetry_digest
            || apply.expected_checkpoint_digest != telemetry.checkpoint_digest
        {
            return Ok(LearningCompensationApplyDecisionV1::StaleRetry { job_id: row.job_id });
        }
        let previous_state = self
            .store
            .read_learning_compensation_state_v1(&context.semantic_scope)?;
        let previous = match previous_state.as_ref() {
            Some(state) => {
                let vector = decode_vector(&state.u_bytes, true)?;
                if compensation_digest(&vector) != state.compensation_digest {
                    return Err(RuntimeError::InvalidLearningCompensation);
                }
                vector
            }
            None => EvidenceVector::default(),
        };
        let local_confidence = frozen
            .local_confidence_vector
            .as_ref()
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let candidate = candidate_b_v1(
            &frozen.local_vector,
            local_confidence,
            &apply.teacher_vector,
            &apply.teacher_confidence_vector,
            &previous,
            &telemetry,
        )?;
        let learning_formula_digest = learning_compensation_formula_digest_v1(
            &frozen.formula_digest,
            &frozen.local_estimator_formula_digest,
            &frozen.policy_digest,
        );
        let teacher_digest = apply.teacher_digest(
            &frozen.local_estimator_formula_digest,
            &learning_formula_digest,
        );
        if candidate.eligible_dimension_count == 0 || candidate.changed_dimension_count == 0 {
            let receipt = LearningCompensationNoChangeReceiptV1 {
                schema: LEARNING_COMPENSATION_RECEIPT_SCHEMA_V1.to_owned(),
                job_id: row.job_id,
                source_event_digest: frozen.source_event_digest,
                source_text_digest: frozen.source_text_digest,
                source_revision: frozen.source_revision,
                base_revision: context.revision,
                formula_digest: telemetry.formula_digest,
                local_estimator_formula_digest: frozen.local_estimator_formula_digest,
                learning_formula_digest,
                telemetry_digest: telemetry.telemetry_digest,
                checkpoint_digest: telemetry.checkpoint_digest,
                policy_digest: frozen.policy_digest,
                provider_digest: frozen.provider_digest,
                model_digest: frozen.model_digest,
                prompt_digest: frozen.prompt_digest,
                schema_digest: frozen.schema_digest,
                teacher_digest,
                request_digest: row.request_digest,
                eligible_dimension_count: candidate.eligible_dimension_count,
                changed_dimension_count: candidate.changed_dimension_count,
                receipt_digest: [0; 32],
            }
            .seal();
            if !receipt.validate() {
                return Err(RuntimeError::InvalidLearningCompensation);
            }
            let stored = self.store.terminalize_learning_compensation_v1(
                &LearningCompensationTerminalCommitV1 {
                    job_id: row.job_id,
                    lease_token: apply.lease_token,
                    status: LearningCompensationJobStatusV1::NoChange,
                    reason_digest: no_change_reason_digest(&candidate),
                    checkpoint_digest: apply.expected_checkpoint_digest,
                    receipt_bytes: encode_no_change_receipt(&receipt),
                    receipt_digest: receipt.receipt_digest,
                },
            );
            return match stored {
                Ok(_) => Ok(LearningCompensationApplyDecisionV1::NoChange(receipt)),
                Err(StoreError::LearningCompensationConflict) => {
                    Ok(LearningCompensationApplyDecisionV1::StaleRetry { job_id: row.job_id })
                }
                Err(error) => Err(RuntimeError::Store(error)),
            };
        }
        let next_checkpoint_revision = previous_state
            .as_ref()
            .map(|state| state.checkpoint_revision)
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        let compensation_digest = compensation_digest(&candidate.u_next);
        let receipt = LearningCompensationReceiptV1 {
            schema: LEARNING_COMPENSATION_RECEIPT_SCHEMA_V1.to_owned(),
            job_id: row.job_id,
            source_event_digest: frozen.source_event_digest,
            source_text_digest: frozen.source_text_digest,
            source_revision: frozen.source_revision,
            base_revision: context.revision,
            next_checkpoint_revision,
            formula_digest: telemetry.formula_digest,
            local_estimator_formula_digest: frozen.local_estimator_formula_digest,
            learning_formula_digest,
            telemetry_digest: telemetry.telemetry_digest,
            checkpoint_digest: telemetry.checkpoint_digest,
            compensation_digest,
            policy_digest: frozen.policy_digest,
            schema_digest: frozen.schema_digest,
            teacher_digest,
            provider_digest: frozen.provider_digest,
            model_digest: frozen.model_digest,
            prompt_digest: frozen.prompt_digest,
            request_digest: row.request_digest,
            eligible_dimension_count: candidate.eligible_dimension_count,
            changed_dimension_count: candidate.changed_dimension_count,
            u_next: candidate.u_next,
            status: LearningCompensationCommitStatusV1::Committed,
            receipt_digest: [0; 32],
        }
        .seal();
        if !receipt.validate() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let stored = self
            .store
            .commit_learning_compensation_v1(&LearningCompensationCommitV1 {
                job_id: row.job_id,
                lease_token: apply.lease_token,
                expected_request_digest: row.request_digest,
                expected_semantic_revision: context.revision,
                expected_formula_digest: apply.expected_formula_digest,
                expected_telemetry_digest: apply.expected_telemetry_digest,
                expected_checkpoint_digest: apply.expected_checkpoint_digest,
                next_checkpoint_revision,
                u_bytes: vector_bytes(&receipt.u_next),
                compensation_digest: receipt.compensation_digest,
                receipt_bytes: encode_receipt(&receipt),
                receipt_digest: receipt.receipt_digest,
            });
        match stored {
            Ok(stored) if stored.receipt_digest == Some(receipt.receipt_digest) => {
                Ok(LearningCompensationApplyDecisionV1::Committed(receipt))
            }
            Ok(stored) if stored.status == LearningCompensationJobStatusV1::Committed => {
                let existing = decode_receipt(
                    stored
                        .receipt_bytes
                        .as_deref()
                        .ok_or(RuntimeError::InvalidLearningCompensation)?,
                )?;
                Ok(LearningCompensationApplyDecisionV1::Replayed(existing))
            }
            Ok(_) => Err(RuntimeError::InvalidLearningCompensation),
            Err(StoreError::LearningCompensationConflict) => {
                Ok(LearningCompensationApplyDecisionV1::StaleRetry { job_id: row.job_id })
            }
            Err(error) => Err(RuntimeError::Store(error)),
        }
    }

    pub fn abandon_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        terminal: &SemanticLearningCompensationTerminalV1,
    ) -> Result<LearningCompensationTerminalDecisionV1, RuntimeError> {
        self.terminalize_learning_compensation_v1(
            scope,
            terminal,
            LearningCompensationJobStatusV1::Abandoned,
            LearningCompensationTerminalStatusV1::AbandonedInputUnavailable,
        )
    }

    pub fn reject_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        terminal: &SemanticLearningCompensationTerminalV1,
    ) -> Result<LearningCompensationTerminalDecisionV1, RuntimeError> {
        self.terminalize_learning_compensation_v1(
            scope,
            terminal,
            LearningCompensationJobStatusV1::Rejected,
            LearningCompensationTerminalStatusV1::Rejected,
        )
    }

    /// TTL expiry is a first-class durable terminal state.  It is intentionally
    /// not encoded as abandonment with an arbitrary host reason string.
    pub fn expire_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        terminal: &SemanticLearningCompensationTerminalV1,
    ) -> Result<LearningCompensationTerminalDecisionV1, RuntimeError> {
        self.terminalize_learning_compensation_v1(
            scope,
            terminal,
            LearningCompensationJobStatusV1::Expired,
            LearningCompensationTerminalStatusV1::Expired,
        )
    }

    fn terminalize_learning_compensation_v1(
        &mut self,
        scope: &ScopeRef,
        terminal: &SemanticLearningCompensationTerminalV1,
        store_status: LearningCompensationJobStatusV1,
        receipt_status: LearningCompensationTerminalStatusV1,
    ) -> Result<LearningCompensationTerminalDecisionV1, RuntimeError> {
        if !terminal.validate() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let context = self.learning_semantic_context_v1(scope)?;
        let row = self
            .store
            .read_learning_compensation_job_v1(&terminal.job_id)?
            .ok_or(RuntimeError::InvalidLearningCompensation)?;
        if row.scope_digest != context.semantic_scope
            || row.request_digest != terminal.expected_request_digest
        {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let frozen = frozen_request_from_row(&row)?;
        if row.status.is_terminal() {
            let existing = row
                .receipt_bytes
                .as_deref()
                .ok_or(RuntimeError::InvalidLearningCompensation)?;
            let receipt = decode_terminal_receipt(existing)?;
            if receipt.status != receipt_status || receipt.request_digest != row.request_digest {
                return Err(RuntimeError::InvalidLearningCompensation);
            }
            return Ok(LearningCompensationTerminalDecisionV1 { receipt });
        }
        if row.status != LearningCompensationJobStatusV1::Claimed
            || row.lease_token != Some(terminal.lease_token)
        {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        let receipt = LearningCompensationTerminalReceiptV1 {
            schema: LEARNING_COMPENSATION_TERMINAL_SCHEMA_V1.to_owned(),
            job_id: row.job_id,
            status: receipt_status,
            source_event_digest: frozen.source_event_digest,
            source_text_digest: frozen.source_text_digest,
            source_revision: frozen.source_revision,
            request_digest: row.request_digest,
            formula_digest: frozen.formula_digest,
            local_estimator_formula_digest: frozen.local_estimator_formula_digest,
            learning_formula_digest: learning_compensation_formula_digest_v1(
                &frozen.formula_digest,
                &frozen.local_estimator_formula_digest,
                &frozen.policy_digest,
            ),
            policy_digest: frozen.policy_digest,
            provider_digest: frozen.provider_digest,
            model_digest: frozen.model_digest,
            prompt_digest: frozen.prompt_digest,
            schema_digest: frozen.schema_digest,
            reason_digest: terminal.reason_digest,
            checkpoint_digest: terminal.checkpoint_digest,
            receipt_digest: [0; 32],
        }
        .seal();
        if !receipt.validate() {
            return Err(RuntimeError::InvalidLearningCompensation);
        }
        self.store
            .terminalize_learning_compensation_v1(&LearningCompensationTerminalCommitV1 {
                job_id: row.job_id,
                lease_token: terminal.lease_token,
                status: store_status,
                reason_digest: terminal.reason_digest,
                checkpoint_digest: terminal.checkpoint_digest,
                receipt_bytes: encode_terminal_receipt(&receipt),
                receipt_digest: receipt.receipt_digest,
            })?;
        Ok(LearningCompensationTerminalDecisionV1 { receipt })
    }
}
