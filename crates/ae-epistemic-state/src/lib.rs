#![forbid(unsafe_code)]

//! Bounded epistemic projection sources for R7.
//!
//! The source accepts the existing typed [`EvidenceVector`] and [`SemanticEstimate`] values
//! without recalculating them. It only validates their binding and compiles a closed projection
//! vocabulary. Raw user text, conversation, neural arrays, Continuum-KV banks, floating-point
//! values, and unbounded collections are intentionally unrepresentable at this boundary.

use ae_contracts::r7::{
    wire, CausalRef, Digest, EvidenceVector, Id128, ScopeRef, SemanticEstimate, VerdictKind,
};
use ae_fixed::Fixed;
use ae_genesis::r7::IdentityConstitutionV1;
use std::cmp::Ordering;
use thiserror::Error;

pub const EPISTEMIC_EVIDENCE_DIMENSION_COUNT_V1: usize = 15;
pub const EPISTEMIC_EVIDENCE_GAP_CAPACITY_V1: usize = 3;
pub const EPISTEMIC_EVIDENCE_VECTOR_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/epistemic-evidence-vector-v1";
pub const EPISTEMIC_SOURCE_ESTIMATE_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/epistemic-source-estimate-v1";
pub const EPISTEMIC_CLASSIFICATION_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/epistemic-classification-v1";
pub const EPISTEMIC_PROJECTION_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/epistemic-projection-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EpistemicEvidenceGapV1 {
    InsufficientEvidence,
    ConflictingEvidence,
    VerifierPending,
}

impl EpistemicEvidenceGapV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::InsufficientEvidence => "insufficient_evidence",
            Self::ConflictingEvidence => "conflicting_evidence",
            Self::VerifierPending => "verifier_pending",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifierNeedV1 {
    NotRequired,
    Required,
}

impl VerifierNeedV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Required => "required",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EpistemicStateErrorV1 {
    #[error("caller-selected epistemic classification is not an authority input")]
    CallerSelectedClassificationRejected,
    #[error("zero identifier is not valid for {field}")]
    ZeroIdentifier { field: &'static str },
    #[error("zero digest is not valid for {field}")]
    ZeroDigest { field: &'static str },
    #[error("revision must be nonzero")]
    ZeroRevision,
    #[error("causal turn does not match the epistemic binding")]
    TurnBindingMismatch,
    #[error("causal revision does not match the epistemic binding")]
    RevisionBindingMismatch,
    #[error("semantic estimate schema version must be nonzero")]
    InvalidEstimateSchemaVersion,
    #[error("{field} must be within the inclusive fixed-point range [0, 1]")]
    InvalidConfidenceRange { field: &'static str },
    #[error("too many canonical evidence gaps ({actual_items} > {max_items})")]
    TooManyEvidenceGaps {
        max_items: usize,
        actual_items: usize,
    },
    #[error("duplicate evidence gap at index {index}")]
    DuplicateEvidenceGap { index: usize },
    #[error("noncanonical evidence-gap order at index {index}")]
    NonCanonicalEvidenceGapOrder { index: usize },
    #[error("mandatory correction requires mandatory acknowledgement")]
    CorrectionRequiresAcknowledgement,
}

/// Turn, state, revision, scope, and identity values that bind a source estimate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpistemicSourceBindingV1 {
    scope: ScopeRef,
    turn_id: Id128,
    state_digest: Digest,
    revision: u64,
    identity_digest: Digest,
}

impl EpistemicSourceBindingV1 {
    pub fn new(
        scope: ScopeRef,
        turn_id: Id128,
        state_digest: Digest,
        revision: u64,
        identity: IdentityConstitutionV1,
    ) -> Result<Self, EpistemicStateErrorV1> {
        require_id(&scope.bot_token, "scope.bot_token")?;
        require_id(&scope.persona_token, "scope.persona_token")?;
        if let Some(relation_token) = &scope.relation_token {
            require_id(relation_token, "scope.relation_token")?;
        }
        require_id(&scope.session_token, "scope.session_token")?;
        require_id(&turn_id, "turn_id")?;
        require_digest(&state_digest, "state_digest")?;
        if revision == 0 {
            return Err(EpistemicStateErrorV1::ZeroRevision);
        }
        let identity_digest = *identity.constitution_digest();
        require_digest(&identity_digest, "identity.constitution_digest")?;
        Ok(Self {
            scope,
            turn_id,
            state_digest,
            revision,
            identity_digest,
        })
    }
}

/// Closed caller-provided classifications; this core never infers them from state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerProvidedEpistemicClassificationV1 {
    verdict: VerdictKind,
    evidence_gaps: Vec<EpistemicEvidenceGapV1>,
    verifier_need: VerifierNeedV1,
    confidence_ceiling: Fixed,
    must_acknowledge: bool,
    must_correct: bool,
}

impl CallerProvidedEpistemicClassificationV1 {
    pub fn new(
        verdict: VerdictKind,
        evidence_gaps: Vec<EpistemicEvidenceGapV1>,
        verifier_need: VerifierNeedV1,
        confidence_ceiling: Fixed,
        must_acknowledge: bool,
        must_correct: bool,
    ) -> Result<Self, EpistemicStateErrorV1> {
        if evidence_gaps.len() > EPISTEMIC_EVIDENCE_GAP_CAPACITY_V1 {
            return Err(EpistemicStateErrorV1::TooManyEvidenceGaps {
                max_items: EPISTEMIC_EVIDENCE_GAP_CAPACITY_V1,
                actual_items: evidence_gaps.len(),
            });
        }
        for (offset, pair) in evidence_gaps.windows(2).enumerate() {
            match pair[0].cmp(&pair[1]) {
                Ordering::Equal => {
                    return Err(EpistemicStateErrorV1::DuplicateEvidenceGap { index: offset + 1 });
                }
                Ordering::Greater => {
                    return Err(EpistemicStateErrorV1::NonCanonicalEvidenceGapOrder {
                        index: offset + 1,
                    });
                }
                Ordering::Less => {}
            }
        }
        require_confidence_range(confidence_ceiling, "confidence_ceiling")?;
        if must_correct && !must_acknowledge {
            return Err(EpistemicStateErrorV1::CorrectionRequiresAcknowledgement);
        }
        Ok(Self {
            verdict,
            evidence_gaps,
            verifier_need,
            confidence_ceiling,
            must_acknowledge,
            must_correct,
        })
    }
}

/// Existing typed evidence and an explicit classification bound to one committed source state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpistemicProjectionInputV1 {
    pub binding: EpistemicSourceBindingV1,
    pub causal: CausalRef,
    pub estimate: SemanticEstimate,
    pub classification: CallerProvidedEpistemicClassificationV1,
}

/// Typed evidence and its committed binding. Classification is derived by this crate;
/// no caller-selected verdict, action, text, provider, or control field is accepted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpistemicProjectionEvidenceInputV1 {
    pub binding: EpistemicSourceBindingV1,
    pub causal: CausalRef,
    pub estimate: SemanticEstimate,
}

impl EpistemicProjectionEvidenceInputV1 {
    pub fn new(
        binding: EpistemicSourceBindingV1,
        causal: CausalRef,
        estimate: SemanticEstimate,
    ) -> Self {
        Self {
            binding,
            causal,
            estimate,
        }
    }
}

impl EpistemicProjectionInputV1 {
    pub fn new(
        binding: EpistemicSourceBindingV1,
        causal: CausalRef,
        estimate: SemanticEstimate,
        classification: CallerProvidedEpistemicClassificationV1,
    ) -> Self {
        Self {
            binding,
            causal,
            estimate,
            classification,
        }
    }
}

/// Bounded R7 epistemic projection without raw evidence vectors or source text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpistemicProjectionV1 {
    turn_id: Id128,
    state_digest: Digest,
    revision: u64,
    identity_digest: Digest,
    claim_under_challenge: Option<Id128>,
    source_estimate_digest: Digest,
    classification: CallerProvidedEpistemicClassificationV1,
    caller_provided: bool,
    projection_digest: Digest,
}

impl EpistemicProjectionV1 {
    pub fn turn_id(&self) -> &Id128 {
        &self.turn_id
    }

    pub fn state_digest(&self) -> &Digest {
        &self.state_digest
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn identity_digest(&self) -> &Digest {
        &self.identity_digest
    }

    pub fn claim_under_challenge(&self) -> Option<&Id128> {
        self.claim_under_challenge.as_ref()
    }

    pub fn source_estimate_digest(&self) -> &Digest {
        &self.source_estimate_digest
    }

    pub fn classification_is_caller_provided(&self) -> bool {
        self.caller_provided
    }

    pub fn projection_digest(&self) -> &Digest {
        &self.projection_digest
    }
}

pub fn compile_epistemic_projection_v1(
    input: &EpistemicProjectionInputV1,
) -> Result<EpistemicProjectionV1, EpistemicStateErrorV1> {
    let _ = input;
    Err(EpistemicStateErrorV1::CallerSelectedClassificationRejected)
}

pub fn derive_epistemic_projection_v1(
    input: &EpistemicProjectionEvidenceInputV1,
) -> Result<EpistemicProjectionV1, EpistemicStateErrorV1> {
    let classification = derive_classification(&input.estimate);
    compile_projection(
        input.binding.clone(),
        input.causal.clone(),
        input.estimate.clone(),
        classification,
        false,
    )
}

fn compile_projection(
    binding: EpistemicSourceBindingV1,
    causal: CausalRef,
    estimate: SemanticEstimate,
    classification: CallerProvidedEpistemicClassificationV1,
    caller_provided: bool,
) -> Result<EpistemicProjectionV1, EpistemicStateErrorV1> {
    if causal.turn_id != binding.turn_id {
        return Err(EpistemicStateErrorV1::TurnBindingMismatch);
    }
    if causal.base_revision != binding.revision {
        return Err(EpistemicStateErrorV1::RevisionBindingMismatch);
    }
    require_optional_id(causal.action_id.as_ref(), "causal.action_id")?;
    require_optional_id(causal.delivery_id.as_ref(), "causal.delivery_id")?;
    require_optional_id(causal.claim_id.as_ref(), "causal.claim_id")?;
    if estimate.schema_version == 0 {
        return Err(EpistemicStateErrorV1::InvalidEstimateSchemaVersion);
    }
    require_digest(&estimate.estimator_digest, "estimate.estimator_digest")?;
    require_confidence_range(
        estimate.estimator_confidence,
        "estimate.estimator_confidence",
    )?;

    let evidence_digest = canonical_evidence_vector_digest(&estimate.dimensions);
    let schema_version = estimate.schema_version.to_be_bytes();
    let estimator_confidence = estimate.estimator_confidence.raw().to_be_bytes();
    let source_estimate_digest = wire::domain_hash(
        EPISTEMIC_SOURCE_ESTIMATE_DOMAIN_V1,
        &[
            &schema_version,
            &evidence_digest,
            &estimator_confidence,
            &estimate.estimator_digest,
        ],
    );
    let classification_digest = classification_digest(&classification);
    let revision = binding.revision.to_be_bytes();
    let causal_action = causal.action_id.unwrap_or([0; 16]);
    let causal_delivery = causal.delivery_id.unwrap_or([0; 16]);
    let causal_claim = causal.claim_id.unwrap_or([0; 16]);
    let relation = binding.scope.relation_token.unwrap_or([0; 16]);
    let projection_digest = wire::domain_hash(
        EPISTEMIC_PROJECTION_DOMAIN_V1,
        &[
            &binding.scope.bot_token,
            &binding.scope.persona_token,
            &relation,
            &binding.scope.session_token,
            &binding.turn_id,
            &binding.state_digest,
            &revision,
            &binding.identity_digest,
            &causal_action,
            &causal_delivery,
            &causal_claim,
            &source_estimate_digest,
            &classification_digest,
        ],
    );

    Ok(EpistemicProjectionV1 {
        turn_id: binding.turn_id,
        state_digest: binding.state_digest,
        revision: binding.revision,
        identity_digest: binding.identity_digest,
        claim_under_challenge: causal.claim_id,
        source_estimate_digest,
        classification,
        caller_provided,
        projection_digest,
    })
}

fn derive_classification(estimate: &SemanticEstimate) -> CallerProvidedEpistemicClassificationV1 {
    let mut gaps = Vec::new();
    if estimate.estimator_confidence < Fixed::from_raw(500_000) {
        gaps.push(EpistemicEvidenceGapV1::InsufficientEvidence);
    }
    if estimate.dimensions.epistemic_conflict > Fixed::from_raw(500_000) {
        gaps.push(EpistemicEvidenceGapV1::ConflictingEvidence);
    }
    if estimate.estimator_confidence < Fixed::from_raw(800_000) {
        gaps.push(EpistemicEvidenceGapV1::VerifierPending);
    }
    let verdict = if estimate.dimensions.self_responsibility
        >= estimate.dimensions.other_responsibility
        && estimate.dimensions.positive >= estimate.dimensions.harm
    {
        VerdictKind::ConfirmedSelfError
    } else if estimate.dimensions.epistemic_conflict > Fixed::from_raw(500_000) {
        VerdictKind::SharedAmbiguity
    } else {
        VerdictKind::Unresolved
    };
    let confidence_ceiling = if estimate.estimator_confidence > Fixed::from_raw(700_000) {
        Fixed::from_raw(700_000)
    } else {
        estimate.estimator_confidence
    };
    CallerProvidedEpistemicClassificationV1::new(
        verdict,
        gaps,
        if estimate.estimator_confidence < Fixed::from_raw(800_000) {
            VerifierNeedV1::Required
        } else {
            VerifierNeedV1::NotRequired
        },
        confidence_ceiling,
        matches!(verdict, VerdictKind::ConfirmedSelfError),
        matches!(verdict, VerdictKind::ConfirmedSelfError)
            && estimate.estimator_confidence >= Fixed::from_raw(500_000),
    )
    .expect("derived classification is canonical and bounded")
}

fn canonical_evidence_vector_digest(evidence: &EvidenceVector) -> Digest {
    let raw_values = [
        evidence.positive.raw().to_be_bytes(),
        evidence.affiliation.raw().to_be_bytes(),
        evidence.harm.raw().to_be_bytes(),
        evidence.boundary.raw().to_be_bytes(),
        evidence.repair.raw().to_be_bytes(),
        evidence.repetition.raw().to_be_bytes(),
        evidence.new_information.raw().to_be_bytes(),
        evidence.constraint_instability.raw().to_be_bytes(),
        evidence.epistemic_conflict.raw().to_be_bytes(),
        evidence.self_responsibility.raw().to_be_bytes(),
        evidence.other_responsibility.raw().to_be_bytes(),
        evidence.hostility.raw().to_be_bytes(),
        evidence.publicness.raw().to_be_bytes(),
        evidence.engagement.raw().to_be_bytes(),
        evidence.rejection.raw().to_be_bytes(),
    ];
    let fields = raw_values
        .iter()
        .map(|value| value.as_slice())
        .collect::<Vec<_>>();
    wire::domain_hash(EPISTEMIC_EVIDENCE_VECTOR_DOMAIN_V1, &fields)
}

fn classification_digest(classification: &CallerProvidedEpistemicClassificationV1) -> Digest {
    let gap_count = u64::try_from(classification.evidence_gaps.len())
        .expect("bounded evidence-gap count fits u64")
        .to_be_bytes();
    let confidence_ceiling = classification.confidence_ceiling.raw().to_be_bytes();
    let must_acknowledge = [u8::from(classification.must_acknowledge)];
    let must_correct = [u8::from(classification.must_correct)];
    let mut fields = Vec::with_capacity(classification.evidence_gaps.len() + 6);
    fields.push(verdict_name(classification.verdict).as_bytes());
    fields.push(classification.verifier_need.as_str().as_bytes());
    fields.push(&gap_count);
    fields.push(&confidence_ceiling);
    fields.push(&must_acknowledge);
    fields.push(&must_correct);
    fields.extend(
        classification
            .evidence_gaps
            .iter()
            .map(|gap| gap.as_str().as_bytes()),
    );
    wire::domain_hash(EPISTEMIC_CLASSIFICATION_DOMAIN_V1, &fields)
}

fn require_id(id: &Id128, field: &'static str) -> Result<(), EpistemicStateErrorV1> {
    if id.iter().all(|byte| *byte == 0) {
        return Err(EpistemicStateErrorV1::ZeroIdentifier { field });
    }
    Ok(())
}

fn require_optional_id(
    id: Option<&Id128>,
    field: &'static str,
) -> Result<(), EpistemicStateErrorV1> {
    if let Some(id) = id {
        require_id(id, field)?;
    }
    Ok(())
}

fn require_digest(digest: &Digest, field: &'static str) -> Result<(), EpistemicStateErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(EpistemicStateErrorV1::ZeroDigest { field });
    }
    Ok(())
}

fn require_confidence_range(
    value: Fixed,
    field: &'static str,
) -> Result<(), EpistemicStateErrorV1> {
    if value < Fixed::ZERO || value > Fixed::ONE {
        return Err(EpistemicStateErrorV1::InvalidConfidenceRange { field });
    }
    Ok(())
}

fn verdict_name(verdict: VerdictKind) -> &'static str {
    match verdict {
        VerdictKind::ConfirmedSelfError => "confirmed_self_error",
        VerdictKind::RejectedChallenge => "rejected_challenge",
        VerdictKind::SharedAmbiguity => "shared_ambiguity",
        VerdictKind::HostFailure => "host_failure",
        VerdictKind::Unresolved => "unresolved",
    }
}
