#![forbid(unsafe_code)]

//! Immutable, content-addressed Genesis identity primitives for R7.
//!
//! This crate stores only canonical operational identity tokens and fixed-size digests. Its
//! public constructors have no field for a raw Persona prompt, user profile or text memory,
//! neural state, or Continuum-KV state. The AstrBot Persona remains outside this boundary;
//! only its revision digest participates in Incarnation continuity.
//!
//! This is a source foundation, not a Genesis registry, persistence layer, or organism.

use ae_contracts::{wire, Digest};
use serde::{Serialize, Serializer};
use std::cmp::Ordering;
use thiserror::Error;

pub const SEED_CODE_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/seed-code-v1";
pub const INCARNATION_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/incarnation-v1";
pub const IDENTITY_CONSTITUTION_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/identity-constitution-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentitySectionV1 {
    OperationalCommitments,
    AntiGoals,
    ExpressionBasis,
    CorrectionBoundaryConstitution,
    RelationalPlayLimits,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum GenesisErrorV1 {
    #[error("{field} bound must be nonzero")]
    ZeroBound { field: &'static str },
    #[error("identity section {section:?} must not be empty")]
    EmptySection { section: IdentitySectionV1 },
    #[error("identity section {section:?} has {actual_terms} terms, above {max_terms}")]
    TooManyTerms {
        section: IdentitySectionV1,
        max_terms: u16,
        actual_terms: usize,
    },
    #[error("identity section {section:?} contains an empty token at {index}")]
    EmptyToken {
        section: IdentitySectionV1,
        index: usize,
    },
    #[error("identity section {section:?} token at {index} exceeds {max_bytes} bytes")]
    TokenTooLong {
        section: IdentitySectionV1,
        index: usize,
        max_bytes: u16,
        actual_bytes: usize,
    },
    #[error("identity section {section:?} token at {index} is not canonical")]
    NonCanonicalToken {
        section: IdentitySectionV1,
        index: usize,
    },
    #[error("identity section {section:?} has a duplicate at {index}")]
    DuplicateTerm {
        section: IdentitySectionV1,
        index: usize,
    },
    #[error("identity section {section:?} is not canonically ordered at {index}")]
    NonCanonicalOrder {
        section: IdentitySectionV1,
        index: usize,
    },
    #[error("seed_code_ref must not be empty")]
    EmptySeedCodeRef,
    #[error("seed_code_ref exceeds {max_bytes} bytes")]
    SeedCodeRefTooLong { max_bytes: u16, actual_bytes: usize },
    #[error("seed_code_ref is not a canonical token")]
    NonCanonicalSeedCodeRef,
    #[error("zero digest is not valid for {field}")]
    ZeroDigest { field: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdentityBoundsV1 {
    max_terms_per_section: u16,
    max_token_bytes: u16,
}

impl IdentityBoundsV1 {
    pub fn new(max_terms_per_section: u16, max_token_bytes: u16) -> Result<Self, GenesisErrorV1> {
        if max_terms_per_section == 0 {
            return Err(GenesisErrorV1::ZeroBound {
                field: "max_terms_per_section",
            });
        }
        if max_token_bytes == 0 {
            return Err(GenesisErrorV1::ZeroBound {
                field: "max_token_bytes",
            });
        }
        Ok(Self {
            max_terms_per_section,
            max_token_bytes,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CanonicalTermsV1 {
    section: IdentitySectionV1,
    values: Vec<String>,
}

impl CanonicalTermsV1 {
    fn new(
        section: IdentitySectionV1,
        values: Vec<String>,
        bounds: IdentityBoundsV1,
    ) -> Result<Self, GenesisErrorV1> {
        if values.is_empty() {
            return Err(GenesisErrorV1::EmptySection { section });
        }
        if values.len() > usize::from(bounds.max_terms_per_section) {
            return Err(GenesisErrorV1::TooManyTerms {
                section,
                max_terms: bounds.max_terms_per_section,
                actual_terms: values.len(),
            });
        }
        for (index, value) in values.iter().enumerate() {
            if value.is_empty() {
                return Err(GenesisErrorV1::EmptyToken { section, index });
            }
            if value.len() > usize::from(bounds.max_token_bytes) {
                return Err(GenesisErrorV1::TokenTooLong {
                    section,
                    index,
                    max_bytes: bounds.max_token_bytes,
                    actual_bytes: value.len(),
                });
            }
            if !is_canonical_token(value) {
                return Err(GenesisErrorV1::NonCanonicalToken { section, index });
            }
        }
        for (offset, pair) in values.windows(2).enumerate() {
            match pair[0].cmp(&pair[1]) {
                Ordering::Equal => {
                    return Err(GenesisErrorV1::DuplicateTerm {
                        section,
                        index: offset + 1,
                    });
                }
                Ordering::Greater => {
                    return Err(GenesisErrorV1::NonCanonicalOrder {
                        section,
                        index: offset + 1,
                    });
                }
                Ordering::Less => {}
            }
        }
        Ok(Self { section, values })
    }

    fn as_slice(&self) -> &[String] {
        &self.values
    }

    fn content_digest(&self) -> Digest {
        let fields: Vec<&[u8]> = self.values.iter().map(|value| value.as_bytes()).collect();
        wire::domain_hash(section_domain(self.section), &fields)
    }
}

impl Serialize for CanonicalTermsV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

macro_rules! identity_section_type {
    ($name:ident, $section:expr) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        #[serde(transparent)]
        pub struct $name(CanonicalTermsV1);

        impl $name {
            pub fn new(
                values: Vec<String>,
                bounds: IdentityBoundsV1,
            ) -> Result<Self, GenesisErrorV1> {
                CanonicalTermsV1::new($section, values, bounds).map(Self)
            }

            pub fn as_slice(&self) -> &[String] {
                self.0.as_slice()
            }

            fn content_digest(&self) -> Digest {
                self.0.content_digest()
            }
        }
    };
}

identity_section_type!(
    OperationalCommitmentsV1,
    IdentitySectionV1::OperationalCommitments
);
identity_section_type!(AntiGoalsV1, IdentitySectionV1::AntiGoals);
identity_section_type!(ExpressionBasisV1, IdentitySectionV1::ExpressionBasis);
identity_section_type!(
    CorrectionBoundaryConstitutionV1,
    IdentitySectionV1::CorrectionBoundaryConstitution
);
identity_section_type!(
    RelationalPlayLimitsV1,
    IdentitySectionV1::RelationalPlayLimits
);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SeedCodeV1 {
    seed_code_ref: String,
    genesis_event_digest: Digest,
    seed_material_digest: Digest,
    seed_code_digest: Digest,
}

impl SeedCodeV1 {
    pub fn new(
        seed_code_ref: String,
        max_ref_bytes: u16,
        genesis_event_digest: Digest,
        seed_material_digest: Digest,
    ) -> Result<Self, GenesisErrorV1> {
        if max_ref_bytes == 0 {
            return Err(GenesisErrorV1::ZeroBound {
                field: "max_ref_bytes",
            });
        }
        if seed_code_ref.is_empty() {
            return Err(GenesisErrorV1::EmptySeedCodeRef);
        }
        if seed_code_ref.len() > usize::from(max_ref_bytes) {
            return Err(GenesisErrorV1::SeedCodeRefTooLong {
                max_bytes: max_ref_bytes,
                actual_bytes: seed_code_ref.len(),
            });
        }
        if !is_canonical_token(&seed_code_ref) {
            return Err(GenesisErrorV1::NonCanonicalSeedCodeRef);
        }
        require_digest(&genesis_event_digest, "genesis_event_digest")?;
        require_digest(&seed_material_digest, "seed_material_digest")?;
        let seed_code_digest = wire::domain_hash(
            SEED_CODE_DOMAIN_V1,
            &[
                seed_code_ref.as_bytes(),
                &genesis_event_digest,
                &seed_material_digest,
            ],
        );
        Ok(Self {
            seed_code_ref,
            genesis_event_digest,
            seed_material_digest,
            seed_code_digest,
        })
    }

    pub fn seed_code_digest(&self) -> &Digest {
        &self.seed_code_digest
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IncarnationRefV1 {
    seed_code_digest: Digest,
    persona_revision_digest: Digest,
    incarnation_sequence: u64,
    incarnation_digest: Digest,
}

impl IncarnationRefV1 {
    pub fn derive(
        seed_code: &SeedCodeV1,
        persona_revision_digest: Digest,
        incarnation_sequence: u64,
    ) -> Result<Self, GenesisErrorV1> {
        require_digest(&persona_revision_digest, "persona_revision_digest")?;
        let sequence = incarnation_sequence.to_be_bytes();
        let incarnation_digest = wire::domain_hash(
            INCARNATION_DOMAIN_V1,
            &[
                seed_code.seed_code_digest(),
                &persona_revision_digest,
                &sequence,
            ],
        );
        Ok(Self {
            seed_code_digest: *seed_code.seed_code_digest(),
            persona_revision_digest,
            incarnation_sequence,
            incarnation_digest,
        })
    }

    pub fn persona_revision_digest(&self) -> &Digest {
        &self.persona_revision_digest
    }

    pub fn incarnation_digest(&self) -> &Digest {
        &self.incarnation_digest
    }
}

/// The complete stable identity material allowed to enter a cognitive envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityConstitutionV1 {
    operational_commitments: OperationalCommitmentsV1,
    anti_goals: AntiGoalsV1,
    expression_basis: ExpressionBasisV1,
    correction_boundary_constitution: CorrectionBoundaryConstitutionV1,
    relational_play_limits: RelationalPlayLimitsV1,
    persona_revision_digest: Digest,
    incarnation_digest: Digest,
    constitution_digest: Digest,
}

impl IdentityConstitutionV1 {
    pub fn derive(
        incarnation: &IncarnationRefV1,
        operational_commitments: OperationalCommitmentsV1,
        anti_goals: AntiGoalsV1,
        expression_basis: ExpressionBasisV1,
        correction_boundary_constitution: CorrectionBoundaryConstitutionV1,
        relational_play_limits: RelationalPlayLimitsV1,
    ) -> Result<Self, GenesisErrorV1> {
        require_digest(
            incarnation.persona_revision_digest(),
            "persona_revision_digest",
        )?;
        require_digest(incarnation.incarnation_digest(), "incarnation_digest")?;
        let commitments_digest = operational_commitments.content_digest();
        let anti_goals_digest = anti_goals.content_digest();
        let expression_digest = expression_basis.content_digest();
        let correction_boundary_digest = correction_boundary_constitution.content_digest();
        let relational_play_digest = relational_play_limits.content_digest();
        let constitution_digest = wire::domain_hash(
            IDENTITY_CONSTITUTION_DOMAIN_V1,
            &[
                incarnation.persona_revision_digest(),
                incarnation.incarnation_digest(),
                &commitments_digest,
                &anti_goals_digest,
                &expression_digest,
                &correction_boundary_digest,
                &relational_play_digest,
            ],
        );
        Ok(Self {
            operational_commitments,
            anti_goals,
            expression_basis,
            correction_boundary_constitution,
            relational_play_limits,
            persona_revision_digest: *incarnation.persona_revision_digest(),
            incarnation_digest: *incarnation.incarnation_digest(),
            constitution_digest,
        })
    }

    pub fn operational_commitments(&self) -> &OperationalCommitmentsV1 {
        &self.operational_commitments
    }

    pub fn anti_goals(&self) -> &AntiGoalsV1 {
        &self.anti_goals
    }

    pub fn expression_basis(&self) -> &ExpressionBasisV1 {
        &self.expression_basis
    }

    pub fn correction_boundary_constitution(&self) -> &CorrectionBoundaryConstitutionV1 {
        &self.correction_boundary_constitution
    }

    pub fn relational_play_limits(&self) -> &RelationalPlayLimitsV1 {
        &self.relational_play_limits
    }

    pub fn persona_revision_digest(&self) -> &Digest {
        &self.persona_revision_digest
    }

    pub fn incarnation_digest(&self) -> &Digest {
        &self.incarnation_digest
    }

    pub fn constitution_digest(&self) -> &Digest {
        &self.constitution_digest
    }
}

fn require_digest(digest: &Digest, field: &'static str) -> Result<(), GenesisErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(GenesisErrorV1::ZeroDigest { field });
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

fn section_domain(section: IdentitySectionV1) -> &'static [u8] {
    match section {
        IdentitySectionV1::OperationalCommitments => {
            b"astr-embodiment/r7/identity-section/operational-commitments-v1"
        }
        IdentitySectionV1::AntiGoals => b"astr-embodiment/r7/identity-section/anti-goals-v1",
        IdentitySectionV1::ExpressionBasis => {
            b"astr-embodiment/r7/identity-section/expression-basis-v1"
        }
        IdentitySectionV1::CorrectionBoundaryConstitution => {
            b"astr-embodiment/r7/identity-section/correction-boundary-constitution-v1"
        }
        IdentitySectionV1::RelationalPlayLimits => {
            b"astr-embodiment/r7/identity-section/relational-play-limits-v1"
        }
    }
}
