#![forbid(unsafe_code)]

//! Bounded native SOMA state and an explicit-classification bridge to R7 subjective present.
//!
//! The R7 design pack names five SOMA domains: metabolism, autonomic regulation, endocrine
//! fields, immune repair, and rhythm. It does not specify a numeric signal-to-subjective
//! formula. This crate therefore never infers emotion from physiology. Callers provide
//! finite signals with explicit bounds and a closed, state-bound classification for the
//! three SOMA-related subjective axes explicitly named by A41: energy, fatigue, and
//! mobilization/restoration balance.
//!
//! Raw Persona text, user conversation, neural/KV arrays, and provider payload text have no
//! input field. All identifiers and projected strings are canonical tokens.

use ae_contracts::r7::{wire, Digest};
use ae_subjective_present::{
    ConfidenceV1, DisclosureV1, SubjectiveBandV1, SubjectivePresentInputV1,
    SubjectivePresentProjectionV1, SubjectivePresentV1, SubjectiveTrendV1,
    SUBJECTIVE_PRESENT_MAX_ITEMS_V1,
};
use std::cmp::Ordering;
use thiserror::Error;

pub const SOMA_STATE_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/soma-state-v1";
const SOMA_SIGNAL_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/soma-signal-v1";
const SOMA_FIELD_SET_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/soma-field-set-v1";
const SOMA_NATIVE_SOURCE_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/soma-native-source-v1";

/// The only accepted SOMA producer input: a committed native semantic source.
/// It intentionally has no default constructor or caller classification field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeSomaSourceV1 {
    revision: u64,
    identity_constitution_digest: Digest,
    source_state_digest: Digest,
    metabolism: SomaFieldSetV1,
    autonomic_regulation: SomaFieldSetV1,
    endocrine_fields: SomaFieldSetV1,
    immune_repair: SomaFieldSetV1,
    rhythm: SomaFieldSetV1,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SomaErrorV1 {
    #[error("native source revision must be nonzero")]
    ZeroRevision,
    #[error("{field} bound must be nonzero")]
    ZeroBound { field: &'static str },
    #[error("{field} must not be empty")]
    EmptyToken { field: &'static str },
    #[error("{field} exceeds its byte bound ({actual_bytes} > {max_bytes})")]
    TokenTooLong {
        field: &'static str,
        max_bytes: u16,
        actual_bytes: usize,
    },
    #[error("{field} is not a canonical token")]
    NonCanonicalToken { field: &'static str },
    #[error("{field} digest must not be zero")]
    ZeroDigest { field: &'static str },
    #[error("SOMA signal {field} is nonfinite")]
    NonFiniteSignal { field: &'static str },
    #[error("SOMA signal bounds must satisfy lower < upper")]
    InvalidSignalBounds,
    #[error("SOMA signal value is outside its caller-provided bounds")]
    SignalOutOfRange,
    #[error("SOMA field set must contain at least one bounded signal")]
    EmptySignalSet,
    #[error("SOMA field set has {actual_items} signals, above {max_items}")]
    TooManySignals { max_items: u16, actual_items: usize },
    #[error("duplicate SOMA signal at index {index}")]
    DuplicateSignal { index: usize },
    #[error("SOMA signals are not canonically ordered at index {index}")]
    NonCanonicalSignalOrder { index: usize },
    #[error("unknown SOMA subjective axis")]
    UnknownSubjectiveAxis,
    #[error("caller-provided classification is invalid")]
    InvalidClassification,
    #[error("classification ingress must contain at least one item")]
    EmptyClassificationIngress,
    #[error("classification ingress has {actual_items} items, above {max_items}")]
    TooManyClassifications {
        max_items: usize,
        actual_items: usize,
    },
    #[error("duplicate subjective axis at index {index}")]
    DuplicateSubjectiveAxis { index: usize },
    #[error("subjective axes are not canonically ordered at index {index}")]
    NonCanonicalSubjectiveAxisOrder { index: usize },
    #[error("classification ingress state digest does not match SOMA state")]
    StateDigestMismatch,
    #[error("classification ingress revision does not match SOMA state")]
    RevisionMismatch,
    #[error("classification ingress identity does not match SOMA state")]
    IdentityBindingMismatch,
    #[error("typed subjective projection rejected the classifications")]
    ProjectionRejected,
    #[error("native source state digest does not match its canonical fields")]
    SourceStateDigestMismatch,
}

pub fn native_soma_source_state_digest_v1(
    revision: u64,
    identity_constitution_digest: &Digest,
    metabolism: &SomaFieldSetV1,
    autonomic_regulation: &SomaFieldSetV1,
    endocrine_fields: &SomaFieldSetV1,
    immune_repair: &SomaFieldSetV1,
    rhythm: &SomaFieldSetV1,
) -> Digest {
    let revision_bytes = revision.to_be_bytes();
    wire::domain_hash(
        SOMA_NATIVE_SOURCE_DOMAIN_V1,
        &[
            &revision_bytes,
            identity_constitution_digest,
            &metabolism.content_digest,
            &autonomic_regulation.content_digest,
            &endocrine_fields.content_digest,
            &immune_repair.content_digest,
            &rhythm.content_digest,
        ],
    )
}

impl NativeSomaSourceV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        revision: u64,
        identity_constitution_digest: Digest,
        source_state_digest: Digest,
        metabolism: SomaFieldSetV1,
        autonomic_regulation: SomaFieldSetV1,
        endocrine_fields: SomaFieldSetV1,
        immune_repair: SomaFieldSetV1,
        rhythm: SomaFieldSetV1,
    ) -> Result<Self, SomaErrorV1> {
        if revision == 0 {
            return Err(SomaErrorV1::ZeroRevision);
        }
        require_digest(
            "identity_constitution_digest",
            &identity_constitution_digest,
        )?;
        require_digest("source_state_digest", &source_state_digest)?;
        if source_state_digest
            != native_soma_source_state_digest_v1(
                revision,
                &identity_constitution_digest,
                &metabolism,
                &autonomic_regulation,
                &endocrine_fields,
                &immune_repair,
                &rhythm,
            )
        {
            return Err(SomaErrorV1::SourceStateDigestMismatch);
        }
        Ok(Self {
            revision,
            identity_constitution_digest,
            source_state_digest,
            metabolism,
            autonomic_regulation,
            endocrine_fields,
            immune_repair,
            rhythm,
        })
    }
}

pub fn produce_native_soma_snapshot_v1(
    source: NativeSomaSourceV1,
    soma_ref: String,
    max_ref_bytes: u16,
) -> Result<SomaStateV1, SomaErrorV1> {
    SomaStateV1::new_bound_to_source_state(
        soma_ref,
        max_ref_bytes,
        source.revision,
        source.identity_constitution_digest,
        source.metabolism,
        source.autonomic_regulation,
        source.endocrine_fields,
        source.immune_repair,
        source.rhythm,
        source.source_state_digest,
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedSomaSignalV1 {
    signal_id: String,
    value_bits: u64,
    lower_bound_bits: u64,
    upper_bound_bits: u64,
    identity_digest: Digest,
}

impl BoundedSomaSignalV1 {
    pub fn new(
        signal_id: String,
        max_id_bytes: u16,
        value: f64,
        lower_bound: f64,
        upper_bound: f64,
    ) -> Result<Self, SomaErrorV1> {
        require_token("signal_id", &signal_id, max_id_bytes)?;
        require_finite("value", value)?;
        require_finite("lower_bound", lower_bound)?;
        require_finite("upper_bound", upper_bound)?;
        if lower_bound >= upper_bound {
            return Err(SomaErrorV1::InvalidSignalBounds);
        }
        if value < lower_bound || value > upper_bound {
            return Err(SomaErrorV1::SignalOutOfRange);
        }
        let value_bits = canonical_f64_bits(value);
        let lower_bound_bits = canonical_f64_bits(lower_bound);
        let upper_bound_bits = canonical_f64_bits(upper_bound);
        let value_bytes = value_bits.to_be_bytes();
        let lower_bytes = lower_bound_bits.to_be_bytes();
        let upper_bytes = upper_bound_bits.to_be_bytes();
        let identity_digest = wire::domain_hash(
            SOMA_SIGNAL_DOMAIN_V1,
            &[
                signal_id.as_bytes(),
                &value_bytes,
                &lower_bytes,
                &upper_bytes,
            ],
        );
        Ok(Self {
            signal_id,
            value_bits,
            lower_bound_bits,
            upper_bound_bits,
            identity_digest,
        })
    }

    pub fn signal_id(&self) -> &str {
        &self.signal_id
    }

    pub fn value(&self) -> f64 {
        f64::from_bits(self.value_bits)
    }

    pub fn lower_bound(&self) -> f64 {
        f64::from_bits(self.lower_bound_bits)
    }

    pub fn upper_bound(&self) -> f64 {
        f64::from_bits(self.upper_bound_bits)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SomaFieldSetV1 {
    signals: Vec<BoundedSomaSignalV1>,
    content_digest: Digest,
}

impl SomaFieldSetV1 {
    pub fn new(signals: Vec<BoundedSomaSignalV1>, max_items: u16) -> Result<Self, SomaErrorV1> {
        if max_items == 0 {
            return Err(SomaErrorV1::ZeroBound { field: "max_items" });
        }
        if signals.is_empty() {
            return Err(SomaErrorV1::EmptySignalSet);
        }
        if signals.len() > usize::from(max_items) {
            return Err(SomaErrorV1::TooManySignals {
                max_items,
                actual_items: signals.len(),
            });
        }
        for (offset, pair) in signals.windows(2).enumerate() {
            match pair[0].signal_id.cmp(&pair[1].signal_id) {
                Ordering::Equal => {
                    return Err(SomaErrorV1::DuplicateSignal { index: offset + 1 });
                }
                Ordering::Greater => {
                    return Err(SomaErrorV1::NonCanonicalSignalOrder { index: offset + 1 });
                }
                Ordering::Less => {}
            }
        }
        let count = u64::try_from(signals.len())
            .expect("bounded SOMA signal count fits u64")
            .to_be_bytes();
        let mut fields = Vec::with_capacity(signals.len() + 1);
        fields.push(count.as_slice());
        fields.extend(
            signals
                .iter()
                .map(|signal| signal.identity_digest.as_slice()),
        );
        let content_digest = wire::domain_hash(SOMA_FIELD_SET_DOMAIN_V1, &fields);
        Ok(Self {
            signals,
            content_digest,
        })
    }

    pub fn signals(&self) -> &[BoundedSomaSignalV1] {
        &self.signals
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SomaStateV1 {
    soma_ref: String,
    revision: u64,
    identity_constitution_digest: Digest,
    /// Digest of the organism semantic state that this SOMA snapshot was
    /// derived from. It is deliberately distinct from `state_digest`, which
    /// remains the digest of this SOMA state itself.
    source_state_digest: Option<Digest>,
    metabolism: SomaFieldSetV1,
    autonomic_regulation: SomaFieldSetV1,
    endocrine_fields: SomaFieldSetV1,
    immune_repair: SomaFieldSetV1,
    rhythm: SomaFieldSetV1,
    state_digest: Digest,
}

impl SomaStateV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        soma_ref: String,
        max_ref_bytes: u16,
        revision: u64,
        identity_constitution_digest: Digest,
        metabolism: SomaFieldSetV1,
        autonomic_regulation: SomaFieldSetV1,
        endocrine_fields: SomaFieldSetV1,
        immune_repair: SomaFieldSetV1,
        rhythm: SomaFieldSetV1,
    ) -> Result<Self, SomaErrorV1> {
        Self::new_inner(
            soma_ref,
            max_ref_bytes,
            revision,
            identity_constitution_digest,
            metabolism,
            autonomic_regulation,
            endocrine_fields,
            immune_repair,
            rhythm,
            None,
        )
    }

    /// Constructs a SOMA snapshot which faithfully records the committed
    /// organism semantic state it was derived from. The source digest and the
    /// SOMA state's own digest are never interchangeable.
    #[allow(clippy::too_many_arguments)]
    pub fn new_bound_to_source_state(
        soma_ref: String,
        max_ref_bytes: u16,
        revision: u64,
        identity_constitution_digest: Digest,
        metabolism: SomaFieldSetV1,
        autonomic_regulation: SomaFieldSetV1,
        endocrine_fields: SomaFieldSetV1,
        immune_repair: SomaFieldSetV1,
        rhythm: SomaFieldSetV1,
        source_state_digest: Digest,
    ) -> Result<Self, SomaErrorV1> {
        require_digest("source_state_digest", &source_state_digest)?;
        Self::new_inner(
            soma_ref,
            max_ref_bytes,
            revision,
            identity_constitution_digest,
            metabolism,
            autonomic_regulation,
            endocrine_fields,
            immune_repair,
            rhythm,
            Some(source_state_digest),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new_inner(
        soma_ref: String,
        max_ref_bytes: u16,
        revision: u64,
        identity_constitution_digest: Digest,
        metabolism: SomaFieldSetV1,
        autonomic_regulation: SomaFieldSetV1,
        endocrine_fields: SomaFieldSetV1,
        immune_repair: SomaFieldSetV1,
        rhythm: SomaFieldSetV1,
        source_state_digest: Option<Digest>,
    ) -> Result<Self, SomaErrorV1> {
        require_token("soma_ref", &soma_ref, max_ref_bytes)?;
        require_digest(
            "identity_constitution_digest",
            &identity_constitution_digest,
        )?;
        let revision_bytes = revision.to_be_bytes();
        let mut digest_fields = vec![
            soma_ref.as_bytes(),
            &revision_bytes,
            &identity_constitution_digest,
            &metabolism.content_digest,
            &autonomic_regulation.content_digest,
            &endocrine_fields.content_digest,
            &immune_repair.content_digest,
            &rhythm.content_digest,
        ];
        if let Some(source) = source_state_digest.as_ref() {
            digest_fields.push(source);
        }
        let state_digest = wire::domain_hash(SOMA_STATE_DOMAIN_V1, &digest_fields);
        Ok(Self {
            soma_ref,
            revision,
            identity_constitution_digest,
            source_state_digest,
            metabolism,
            autonomic_regulation,
            endocrine_fields,
            immune_repair,
            rhythm,
            state_digest,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn identity_constitution_digest(&self) -> &Digest {
        &self.identity_constitution_digest
    }

    pub fn source_state_digest(&self) -> Option<&Digest> {
        self.source_state_digest.as_ref()
    }

    pub fn metabolism(&self) -> &SomaFieldSetV1 {
        &self.metabolism
    }

    pub fn autonomic_regulation(&self) -> &SomaFieldSetV1 {
        &self.autonomic_regulation
    }

    pub fn endocrine_fields(&self) -> &SomaFieldSetV1 {
        &self.endocrine_fields
    }

    pub fn immune_repair(&self) -> &SomaFieldSetV1 {
        &self.immune_repair
    }

    pub fn rhythm(&self) -> &SomaFieldSetV1 {
        &self.rhythm
    }

    pub fn state_digest(&self) -> &Digest {
        &self.state_digest
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SomaSubjectiveAxisV1 {
    Energy,
    Fatigue,
    MobilizationRestorationBalance,
}

impl SomaSubjectiveAxisV1 {
    pub fn parse(value: &str) -> Result<Self, SomaErrorV1> {
        match value {
            "energy" => Ok(Self::Energy),
            "fatigue" => Ok(Self::Fatigue),
            "mobilization_restoration_balance" => Ok(Self::MobilizationRestorationBalance),
            _ => Err(SomaErrorV1::UnknownSubjectiveAxis),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Energy => "energy",
            Self::Fatigue => "fatigue",
            Self::MobilizationRestorationBalance => "mobilization_restoration_balance",
        }
    }
}

/// A bounded classification supplied by the native caller, not inferred by this crate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerProvidedClassificationV1 {
    axis: SomaSubjectiveAxisV1,
    subjective_present: SubjectivePresentV1,
}

impl CallerProvidedClassificationV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        axis: SomaSubjectiveAxisV1,
        band: SubjectiveBandV1,
        trend: SubjectiveTrendV1,
        behavioral_effect: String,
        disclosure: DisclosureV1,
        confidence: ConfidenceV1,
        cause_ref: Option<String>,
    ) -> Result<Self, SomaErrorV1> {
        let subjective_present = SubjectivePresentV1::try_from_input(SubjectivePresentInputV1 {
            axis: axis.as_str().to_owned(),
            band,
            trend,
            behavioral_effect,
            disclosure,
            confidence,
            cause_ref,
        })
        .map_err(|_| SomaErrorV1::InvalidClassification)?;
        Ok(Self {
            axis,
            subjective_present,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SomaClassificationIngressV1 {
    soma_state_digest: Digest,
    soma_revision: u64,
    identity_constitution_digest: Digest,
    classifications: Vec<CallerProvidedClassificationV1>,
}

impl SomaClassificationIngressV1 {
    pub fn new(
        soma_state_digest: Digest,
        soma_revision: u64,
        identity_constitution_digest: Digest,
        classifications: Vec<CallerProvidedClassificationV1>,
        max_items: u16,
    ) -> Result<Self, SomaErrorV1> {
        require_digest("soma_state_digest", &soma_state_digest)?;
        require_digest(
            "identity_constitution_digest",
            &identity_constitution_digest,
        )?;
        if max_items == 0 {
            return Err(SomaErrorV1::ZeroBound { field: "max_items" });
        }
        if classifications.is_empty() {
            return Err(SomaErrorV1::EmptyClassificationIngress);
        }
        let effective_max = usize::from(max_items).min(SUBJECTIVE_PRESENT_MAX_ITEMS_V1);
        if classifications.len() > effective_max {
            return Err(SomaErrorV1::TooManyClassifications {
                max_items: effective_max,
                actual_items: classifications.len(),
            });
        }
        for (offset, pair) in classifications.windows(2).enumerate() {
            match pair[0].axis.cmp(&pair[1].axis) {
                Ordering::Equal => {
                    return Err(SomaErrorV1::DuplicateSubjectiveAxis { index: offset + 1 });
                }
                Ordering::Greater => {
                    return Err(SomaErrorV1::NonCanonicalSubjectiveAxisOrder { index: offset + 1 });
                }
                Ordering::Less => {}
            }
        }
        Ok(Self {
            soma_state_digest,
            soma_revision,
            identity_constitution_digest,
            classifications,
        })
    }
}

pub fn compile_subjective_present_v1(
    state: &SomaStateV1,
    ingress: &SomaClassificationIngressV1,
) -> Result<SubjectivePresentProjectionV1, SomaErrorV1> {
    if ingress.soma_state_digest != *state.state_digest() {
        return Err(SomaErrorV1::StateDigestMismatch);
    }
    if ingress.soma_revision != state.revision() {
        return Err(SomaErrorV1::RevisionMismatch);
    }
    if ingress.identity_constitution_digest != *state.identity_constitution_digest() {
        return Err(SomaErrorV1::IdentityBindingMismatch);
    }
    SubjectivePresentProjectionV1::new(
        ingress
            .classifications
            .iter()
            .map(|classification| classification.subjective_present.clone())
            .collect(),
    )
    .map_err(|_| SomaErrorV1::ProjectionRejected)
}

fn require_token(field: &'static str, value: &str, max_bytes: u16) -> Result<(), SomaErrorV1> {
    if max_bytes == 0 {
        return Err(SomaErrorV1::ZeroBound { field: "max_bytes" });
    }
    if value.is_empty() {
        return Err(SomaErrorV1::EmptyToken { field });
    }
    if value.len() > usize::from(max_bytes) {
        return Err(SomaErrorV1::TokenTooLong {
            field,
            max_bytes,
            actual_bytes: value.len(),
        });
    }
    if !is_canonical_token(value) {
        return Err(SomaErrorV1::NonCanonicalToken { field });
    }
    Ok(())
}

fn require_digest(field: &'static str, digest: &Digest) -> Result<(), SomaErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(SomaErrorV1::ZeroDigest { field });
    }
    Ok(())
}

fn require_finite(field: &'static str, value: f64) -> Result<(), SomaErrorV1> {
    if !value.is_finite() {
        return Err(SomaErrorV1::NonFiniteSignal { field });
    }
    Ok(())
}

fn canonical_f64_bits(value: f64) -> u64 {
    if value == 0.0 {
        0.0_f64.to_bits()
    } else {
        value.to_bits()
    }
}

fn is_canonical_token(value: &str) -> bool {
    let mut previous_was_separator = true;
    for byte in value.bytes() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_was_separator = false;
        } else if matches!(byte, b'_' | b'-' | b'.' | b':') && !previous_was_separator {
            previous_was_separator = true;
        } else {
            return false;
        }
    }
    !previous_was_separator
}
