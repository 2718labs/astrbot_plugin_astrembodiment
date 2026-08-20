#![forbid(unsafe_code)]

//! Bounded native MORPH effector/affordance classifications for R7 projection inputs.
//!
//! R7 names availability, reliability, side-effect class, confirmation requirements,
//! latency, cost, and reversibility overlays, but does not define their concrete native
//! vocabularies or thresholds. This core therefore requires callers to supply finite,
//! canonical, closed classification vocabularies. It does not infer or invent action
//! semantics. The two binary states that R7 does define operationally -- availability and
//! confirmation requirement -- are fixed enums.
//!
//! This is not a sensor compiler, body-schema model, or effect executor. Raw Persona/user/
//! provider text, neural/KV arrays, and effect payloads have no input field and are also
//! rejected as classification tokens.

use ae_contracts::{wire, Digest};
use std::cmp::Ordering;
use thiserror::Error;

pub const MORPH_AFFORDANCE_MAX_ITEMS_V1: u16 = 64;

const MORPH_STATE_BINDING_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/morph-state-binding-v1";
const MORPH_CLASS_SET_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/morph-class-set-v1";
const MORPH_VOCABULARY_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/morph-vocabulary-v1";
const MORPH_EFFECTOR_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/morph-effector-v1";
const MORPH_CATALOG_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/morph-affordance-catalog-v1";

const PROHIBITED_TOKEN_FRAGMENTS: &[&str] = &[
    "raw_user_text",
    "user_conversation",
    "provider_payload",
    "provider_text",
    "visible_text",
    "persona_prompt",
    "persona_text",
    "raw_persona",
    "raw_evidence",
    "neural_array",
    "neural_state",
    "continuum_kv",
    "raw_kv",
    "kv_array",
    "effect_payload",
];

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MorphErrorV1 {
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
    #[error("{field} contains a prohibited raw-content marker")]
    ProhibitedToken { field: &'static str },
    #[error("{field} digest must not be zero")]
    ZeroDigest { field: &'static str },
    #[error("unknown MORPH availability")]
    UnknownAvailability,
    #[error("unknown MORPH confirmation requirement")]
    UnknownConfirmationRequirement,
    #[error("{axis} vocabulary must contain at least one classification")]
    EmptyClassificationVocabulary { axis: &'static str },
    #[error("{axis} vocabulary has {actual_items} items, above {max_items}")]
    TooManyClassifications {
        axis: &'static str,
        max_items: u16,
        actual_items: usize,
    },
    #[error("duplicate {axis} classification at index {index}")]
    DuplicateClassification { axis: &'static str, index: usize },
    #[error("{axis} classifications are not canonically ordered at index {index}")]
    NonCanonicalClassificationOrder { axis: &'static str, index: usize },
    #[error("entry uses a classification not declared by the {axis} vocabulary")]
    UndeclaredClassification { axis: &'static str },
    #[error("catalog must contain at least one effector classification")]
    EmptyCatalog,
    #[error("catalog bound {actual_bound} exceeds schema maximum {max_items}")]
    CatalogBoundExceedsSchema { max_items: u16, actual_bound: u16 },
    #[error("catalog has {actual_items} entries, above {max_items}")]
    TooManyEntries { max_items: u16, actual_items: usize },
    #[error("duplicate effector at index {index}")]
    DuplicateEffector { index: usize },
    #[error("effectors are not canonically ordered at index {index}")]
    NonCanonicalEffectorOrder { index: usize },
    #[error("effector at index {index} is bound to another native state")]
    StateBindingMismatch { index: usize },
    #[error("effector at index {index} is bound to another classification vocabulary")]
    VocabularyBindingMismatch { index: usize },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphAvailabilityV1 {
    Available,
    Unavailable,
}

impl MorphAvailabilityV1 {
    pub fn parse(value: &str) -> Result<Self, MorphErrorV1> {
        match value {
            "available" => Ok(Self::Available),
            "unavailable" => Ok(Self::Unavailable),
            _ => Err(MorphErrorV1::UnknownAvailability),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphConfirmationRequirementV1 {
    NotRequired,
    Required,
}

impl MorphConfirmationRequirementV1 {
    pub fn parse(value: &str) -> Result<Self, MorphErrorV1> {
        match value {
            "not_required" => Ok(Self::NotRequired),
            "required" => Ok(Self::Required),
            _ => Err(MorphErrorV1::UnknownConfirmationRequirement),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::Required => "required",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MorphVocabularyBoundsV1 {
    max_classes_per_axis: u16,
    max_token_bytes: u16,
}

impl MorphVocabularyBoundsV1 {
    pub fn new(max_classes_per_axis: u16, max_token_bytes: u16) -> Result<Self, MorphErrorV1> {
        if max_classes_per_axis == 0 {
            return Err(MorphErrorV1::ZeroBound {
                field: "max_classes_per_axis",
            });
        }
        if max_token_bytes == 0 {
            return Err(MorphErrorV1::ZeroBound {
                field: "max_token_bytes",
            });
        }
        Ok(Self {
            max_classes_per_axis,
            max_token_bytes,
        })
    }

    pub fn max_classes_per_axis(self) -> u16 {
        self.max_classes_per_axis
    }

    pub fn max_token_bytes(self) -> u16 {
        self.max_token_bytes
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphClassificationVocabularyInputV1 {
    pub capability_classes: Vec<String>,
    pub safety_classes: Vec<String>,
    pub reliability_classes: Vec<String>,
    pub side_effect_classes: Vec<String>,
    pub latency_classes: Vec<String>,
    pub cost_classes: Vec<String>,
    pub reversibility_classes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphClassificationVocabularyV1 {
    capability_classes: Vec<String>,
    safety_classes: Vec<String>,
    reliability_classes: Vec<String>,
    side_effect_classes: Vec<String>,
    latency_classes: Vec<String>,
    cost_classes: Vec<String>,
    reversibility_classes: Vec<String>,
    max_token_bytes: u16,
    vocabulary_digest: Digest,
}

impl MorphClassificationVocabularyV1 {
    pub fn new(
        input: MorphClassificationVocabularyInputV1,
        bounds: MorphVocabularyBoundsV1,
    ) -> Result<Self, MorphErrorV1> {
        validate_classifications("capability_class", &input.capability_classes, bounds)?;
        validate_classifications("safety_class", &input.safety_classes, bounds)?;
        validate_classifications("reliability_class", &input.reliability_classes, bounds)?;
        validate_classifications("side_effect_class", &input.side_effect_classes, bounds)?;
        validate_classifications("latency_class", &input.latency_classes, bounds)?;
        validate_classifications("cost_class", &input.cost_classes, bounds)?;
        validate_classifications("reversibility_class", &input.reversibility_classes, bounds)?;

        let capability_digest = class_set_digest("capability_class", &input.capability_classes);
        let safety_digest = class_set_digest("safety_class", &input.safety_classes);
        let reliability_digest = class_set_digest("reliability_class", &input.reliability_classes);
        let side_effect_digest = class_set_digest("side_effect_class", &input.side_effect_classes);
        let latency_digest = class_set_digest("latency_class", &input.latency_classes);
        let cost_digest = class_set_digest("cost_class", &input.cost_classes);
        let reversibility_digest =
            class_set_digest("reversibility_class", &input.reversibility_classes);
        let vocabulary_digest = wire::domain_hash(
            MORPH_VOCABULARY_DOMAIN_V1,
            &[
                &capability_digest,
                &safety_digest,
                &reliability_digest,
                &side_effect_digest,
                &latency_digest,
                &cost_digest,
                &reversibility_digest,
            ],
        );

        Ok(Self {
            capability_classes: input.capability_classes,
            safety_classes: input.safety_classes,
            reliability_classes: input.reliability_classes,
            side_effect_classes: input.side_effect_classes,
            latency_classes: input.latency_classes,
            cost_classes: input.cost_classes,
            reversibility_classes: input.reversibility_classes,
            max_token_bytes: bounds.max_token_bytes,
            vocabulary_digest,
        })
    }

    pub fn capability_classes(&self) -> &[String] {
        &self.capability_classes
    }

    pub fn safety_classes(&self) -> &[String] {
        &self.safety_classes
    }

    pub fn reliability_classes(&self) -> &[String] {
        &self.reliability_classes
    }

    pub fn side_effect_classes(&self) -> &[String] {
        &self.side_effect_classes
    }

    pub fn latency_classes(&self) -> &[String] {
        &self.latency_classes
    }

    pub fn cost_classes(&self) -> &[String] {
        &self.cost_classes
    }

    pub fn reversibility_classes(&self) -> &[String] {
        &self.reversibility_classes
    }

    pub fn vocabulary_digest(&self) -> &Digest {
        &self.vocabulary_digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphStateBindingV1 {
    revision: u64,
    identity_constitution_digest: Digest,
    source_state_digest: Digest,
    binding_digest: Digest,
}

impl MorphStateBindingV1 {
    pub fn new(
        revision: u64,
        identity_constitution_digest: Digest,
        source_state_digest: Digest,
    ) -> Result<Self, MorphErrorV1> {
        require_digest(
            "identity_constitution_digest",
            &identity_constitution_digest,
        )?;
        require_digest("source_state_digest", &source_state_digest)?;
        let revision_bytes = revision.to_be_bytes();
        let binding_digest = wire::domain_hash(
            MORPH_STATE_BINDING_DOMAIN_V1,
            &[
                &revision_bytes,
                &identity_constitution_digest,
                &source_state_digest,
            ],
        );
        Ok(Self {
            revision,
            identity_constitution_digest,
            source_state_digest,
            binding_digest,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn identity_constitution_digest(&self) -> &Digest {
        &self.identity_constitution_digest
    }

    pub fn source_state_digest(&self) -> &Digest {
        &self.source_state_digest
    }

    pub fn binding_digest(&self) -> &Digest {
        &self.binding_digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphEffectorInputV1 {
    pub effector_id: String,
    pub capability_class: String,
    pub availability: MorphAvailabilityV1,
    pub safety_class: String,
    pub reliability_class: String,
    pub side_effect_class: String,
    pub confirmation_requirement: MorphConfirmationRequirementV1,
    pub latency_class: String,
    pub cost_class: String,
    pub reversibility_class: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphEffectorV1 {
    effector_id: String,
    capability_class: String,
    availability: MorphAvailabilityV1,
    safety_class: String,
    reliability_class: String,
    side_effect_class: String,
    confirmation_requirement: MorphConfirmationRequirementV1,
    latency_class: String,
    cost_class: String,
    reversibility_class: String,
    state_binding_digest: Digest,
    vocabulary_digest: Digest,
    effector_digest: Digest,
}

impl MorphEffectorV1 {
    pub fn new(
        input: MorphEffectorInputV1,
        max_effector_id_bytes: u16,
        vocabulary: &MorphClassificationVocabularyV1,
        binding: &MorphStateBindingV1,
    ) -> Result<Self, MorphErrorV1> {
        require_token("effector_id", &input.effector_id, max_effector_id_bytes)?;
        require_declared(
            "capability_class",
            &input.capability_class,
            &vocabulary.capability_classes,
            vocabulary.max_token_bytes,
        )?;
        require_declared(
            "safety_class",
            &input.safety_class,
            &vocabulary.safety_classes,
            vocabulary.max_token_bytes,
        )?;
        require_declared(
            "reliability_class",
            &input.reliability_class,
            &vocabulary.reliability_classes,
            vocabulary.max_token_bytes,
        )?;
        require_declared(
            "side_effect_class",
            &input.side_effect_class,
            &vocabulary.side_effect_classes,
            vocabulary.max_token_bytes,
        )?;
        require_declared(
            "latency_class",
            &input.latency_class,
            &vocabulary.latency_classes,
            vocabulary.max_token_bytes,
        )?;
        require_declared(
            "cost_class",
            &input.cost_class,
            &vocabulary.cost_classes,
            vocabulary.max_token_bytes,
        )?;
        require_declared(
            "reversibility_class",
            &input.reversibility_class,
            &vocabulary.reversibility_classes,
            vocabulary.max_token_bytes,
        )?;

        let effector_digest = wire::domain_hash(
            MORPH_EFFECTOR_DOMAIN_V1,
            &[
                input.effector_id.as_bytes(),
                input.capability_class.as_bytes(),
                input.availability.as_str().as_bytes(),
                input.safety_class.as_bytes(),
                input.reliability_class.as_bytes(),
                input.side_effect_class.as_bytes(),
                input.confirmation_requirement.as_str().as_bytes(),
                input.latency_class.as_bytes(),
                input.cost_class.as_bytes(),
                input.reversibility_class.as_bytes(),
                binding.binding_digest(),
                vocabulary.vocabulary_digest(),
            ],
        );
        Ok(Self {
            effector_id: input.effector_id,
            capability_class: input.capability_class,
            availability: input.availability,
            safety_class: input.safety_class,
            reliability_class: input.reliability_class,
            side_effect_class: input.side_effect_class,
            confirmation_requirement: input.confirmation_requirement,
            latency_class: input.latency_class,
            cost_class: input.cost_class,
            reversibility_class: input.reversibility_class,
            state_binding_digest: *binding.binding_digest(),
            vocabulary_digest: *vocabulary.vocabulary_digest(),
            effector_digest,
        })
    }

    pub fn effector_id(&self) -> &str {
        &self.effector_id
    }

    pub fn capability_class(&self) -> &str {
        &self.capability_class
    }

    pub fn availability(&self) -> MorphAvailabilityV1 {
        self.availability
    }

    pub fn safety_class(&self) -> &str {
        &self.safety_class
    }

    pub fn reliability_class(&self) -> &str {
        &self.reliability_class
    }

    pub fn side_effect_class(&self) -> &str {
        &self.side_effect_class
    }

    pub fn confirmation_requirement(&self) -> MorphConfirmationRequirementV1 {
        self.confirmation_requirement
    }

    pub fn latency_class(&self) -> &str {
        &self.latency_class
    }

    pub fn cost_class(&self) -> &str {
        &self.cost_class
    }

    pub fn reversibility_class(&self) -> &str {
        &self.reversibility_class
    }

    pub fn effector_digest(&self) -> &Digest {
        &self.effector_digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MorphAffordanceCatalogV1 {
    catalog_ref: String,
    binding: MorphStateBindingV1,
    vocabulary: MorphClassificationVocabularyV1,
    effectors: Vec<MorphEffectorV1>,
    catalog_digest: Digest,
}

impl MorphAffordanceCatalogV1 {
    pub fn new(
        catalog_ref: String,
        max_ref_bytes: u16,
        binding: MorphStateBindingV1,
        vocabulary: MorphClassificationVocabularyV1,
        effectors: Vec<MorphEffectorV1>,
        max_items: u16,
    ) -> Result<Self, MorphErrorV1> {
        if max_items == 0 {
            return Err(MorphErrorV1::ZeroBound { field: "max_items" });
        }
        if max_items > MORPH_AFFORDANCE_MAX_ITEMS_V1 {
            return Err(MorphErrorV1::CatalogBoundExceedsSchema {
                max_items: MORPH_AFFORDANCE_MAX_ITEMS_V1,
                actual_bound: max_items,
            });
        }
        require_token("catalog_ref", &catalog_ref, max_ref_bytes)?;
        if effectors.is_empty() {
            return Err(MorphErrorV1::EmptyCatalog);
        }
        if effectors.len() > usize::from(max_items) {
            return Err(MorphErrorV1::TooManyEntries {
                max_items,
                actual_items: effectors.len(),
            });
        }
        for (offset, pair) in effectors.windows(2).enumerate() {
            match pair[0].effector_id.cmp(&pair[1].effector_id) {
                Ordering::Equal => {
                    return Err(MorphErrorV1::DuplicateEffector { index: offset + 1 });
                }
                Ordering::Greater => {
                    return Err(MorphErrorV1::NonCanonicalEffectorOrder { index: offset + 1 });
                }
                Ordering::Less => {}
            }
        }
        for (index, effector) in effectors.iter().enumerate() {
            if effector.state_binding_digest != *binding.binding_digest() {
                return Err(MorphErrorV1::StateBindingMismatch { index });
            }
            if effector.vocabulary_digest != *vocabulary.vocabulary_digest() {
                return Err(MorphErrorV1::VocabularyBindingMismatch { index });
            }
        }

        let count = u64::try_from(effectors.len())
            .expect("bounded MORPH effector count fits u64")
            .to_be_bytes();
        let mut fields = Vec::with_capacity(effectors.len() + 4);
        fields.push(catalog_ref.as_bytes());
        fields.push(binding.binding_digest().as_slice());
        fields.push(vocabulary.vocabulary_digest().as_slice());
        fields.push(count.as_slice());
        fields.extend(
            effectors
                .iter()
                .map(|effector| effector.effector_digest.as_slice()),
        );
        let catalog_digest = wire::domain_hash(MORPH_CATALOG_DOMAIN_V1, &fields);
        Ok(Self {
            catalog_ref,
            binding,
            vocabulary,
            effectors,
            catalog_digest,
        })
    }

    pub fn catalog_ref(&self) -> &str {
        &self.catalog_ref
    }

    pub fn revision(&self) -> u64 {
        self.binding.revision()
    }

    pub fn identity_constitution_digest(&self) -> &Digest {
        self.binding.identity_constitution_digest()
    }

    pub fn source_state_digest(&self) -> &Digest {
        self.binding.source_state_digest()
    }

    pub fn state_binding_digest(&self) -> &Digest {
        self.binding.binding_digest()
    }

    pub fn classification_vocabulary(&self) -> &MorphClassificationVocabularyV1 {
        &self.vocabulary
    }

    pub fn effectors(&self) -> &[MorphEffectorV1] {
        &self.effectors
    }

    pub fn available_effectors(&self) -> impl Iterator<Item = &MorphEffectorV1> {
        self.effectors
            .iter()
            .filter(|effector| effector.availability == MorphAvailabilityV1::Available)
    }

    pub fn catalog_digest(&self) -> &Digest {
        &self.catalog_digest
    }
}

fn validate_classifications(
    axis: &'static str,
    values: &[String],
    bounds: MorphVocabularyBoundsV1,
) -> Result<(), MorphErrorV1> {
    if values.is_empty() {
        return Err(MorphErrorV1::EmptyClassificationVocabulary { axis });
    }
    if values.len() > usize::from(bounds.max_classes_per_axis) {
        return Err(MorphErrorV1::TooManyClassifications {
            axis,
            max_items: bounds.max_classes_per_axis,
            actual_items: values.len(),
        });
    }
    for value in values {
        require_token(axis, value, bounds.max_token_bytes)?;
    }
    for (offset, pair) in values.windows(2).enumerate() {
        match pair[0].cmp(&pair[1]) {
            Ordering::Equal => {
                return Err(MorphErrorV1::DuplicateClassification {
                    axis,
                    index: offset + 1,
                });
            }
            Ordering::Greater => {
                return Err(MorphErrorV1::NonCanonicalClassificationOrder {
                    axis,
                    index: offset + 1,
                });
            }
            Ordering::Less => {}
        }
    }
    Ok(())
}

fn class_set_digest(axis: &'static str, values: &[String]) -> Digest {
    let count = u64::try_from(values.len())
        .expect("bounded MORPH vocabulary count fits u64")
        .to_be_bytes();
    let mut fields = Vec::with_capacity(values.len() + 2);
    fields.push(axis.as_bytes());
    fields.push(count.as_slice());
    fields.extend(values.iter().map(|value| value.as_bytes()));
    wire::domain_hash(MORPH_CLASS_SET_DOMAIN_V1, &fields)
}

fn require_declared(
    axis: &'static str,
    value: &str,
    declared: &[String],
    max_token_bytes: u16,
) -> Result<(), MorphErrorV1> {
    require_token(axis, value, max_token_bytes)?;
    if declared
        .binary_search_by(|candidate| candidate.as_str().cmp(value))
        .is_err()
    {
        return Err(MorphErrorV1::UndeclaredClassification { axis });
    }
    Ok(())
}

fn require_token(field: &'static str, value: &str, max_bytes: u16) -> Result<(), MorphErrorV1> {
    if max_bytes == 0 {
        return Err(MorphErrorV1::ZeroBound { field: "max_bytes" });
    }
    if value.is_empty() {
        return Err(MorphErrorV1::EmptyToken { field });
    }
    if value.len() > usize::from(max_bytes) {
        return Err(MorphErrorV1::TokenTooLong {
            field,
            max_bytes,
            actual_bytes: value.len(),
        });
    }
    if !is_canonical_token(value) {
        return Err(MorphErrorV1::NonCanonicalToken { field });
    }
    if PROHIBITED_TOKEN_FRAGMENTS
        .iter()
        .any(|fragment| value.contains(fragment))
    {
        return Err(MorphErrorV1::ProhibitedToken { field });
    }
    Ok(())
}

fn require_digest(field: &'static str, digest: &Digest) -> Result<(), MorphErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(MorphErrorV1::ZeroDigest { field });
    }
    Ok(())
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
