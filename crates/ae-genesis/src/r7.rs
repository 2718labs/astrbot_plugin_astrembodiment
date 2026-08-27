#![forbid(unsafe_code)]

//! Immutable, content-addressed Genesis identity primitives for R7.
//!
//! This crate stores only canonical operational identity tokens and fixed-size digests. Its
//! public constructors have no field for a raw Persona prompt, user profile or text memory,
//! neural state, or Continuum-KV state. The AstrBot Persona remains outside this boundary;
//! only its revision digest participates in Incarnation continuity.
//!
//! This is a source foundation, not a Genesis registry, persistence layer, or organism.

use ae_contracts::r7::{wire, Digest};
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

// ---------------------------------------------------------------------------
// Governed public policy bootstrap (GIP1 / V3)
//
// This section deliberately remains independent from the historical G0
// identity types above.  GIP1 is a public, fixed-order wire record: it is
// never assembled from JSON, a map, a prompt, or a default constitution.

pub const POLICY_REVISION_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.revision.v1";
pub const POLICY_CORE_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.core.v1";
pub const GENESIS_EVENT_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.genesis_event.v1";
pub const SEED_MATERIAL_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.seed_material.v1";
pub const POLICY_BODY_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.body.v1";
pub const ATTESTATION_MESSAGE_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.attestation_message.v1";
pub const REVIEW_RECEIPT_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.review_receipt.v1";
pub const REVIEW_RECEIPT_RECORD_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.review_receipt_record.v1";
pub const ATTESTATION_RECORD_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.attestation_record.v1";
pub const REGISTRY_BODY_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.registry_body.v1";
pub const REGISTRY_SIGNATURE_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.registry_signature.v1";
pub const DELEGATION_RECEIPT_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.user_delegation_receipt.v1";
pub const CEREMONY_RECEIPT_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.key_ceremony_receipt.v1";
pub const CUSTODY_RECEIPT_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.custody_disposition_receipt.v1";
pub const ROOT_POP_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.root_key_pop.v1";
pub const POLICY_POP_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.policy_key_pop.v1";
pub const REVIEWER_POP_DOMAIN_V1: &[u8] = b"ae.r7.genesis_identity_policy.reviewer_key_pop.v1";
pub const PUBLIC_KEY_FINGERPRINT_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.public_key_fingerprint.v1";
pub const RELEASE_TRUST_ROOT_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.release_trust_root.v1";
pub const BOOTSTRAP_ACTIVATION_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.bootstrap_activation.v1";
pub const BOOTSTRAP_ACTIVATION_RECORD_DOMAIN_V1: &[u8] =
    b"ae.r7.genesis_identity_policy.bootstrap_activation_record.v1";

pub const POLICY_PROFILE_V1: &str = "genesis_identity_policy_v1";
pub const POLICY_OWNER_REF_V1: &str = "ae_rc1_product_identity_authority";
pub const POLICY_OWNER_KIND_V1: &str = "product_constitution_authority";
pub const POLICY_GRANT_REF_V1: &str = "ae_rc1_identity_policy_approval_v1";
pub const POLICY_KEY_ID_V1: &str = "ae_rc1_identity_policy_signer_v1";
pub const POLICY_SEED_CODE_REF_V1: &str = "g0_committed_birth_v1";
pub const POLICY_MAX_TERMS_V1: u16 = 16;
pub const POLICY_MAX_TOKEN_BYTES_V1: u16 = 96;
pub const POLICY_MAX_SEED_REF_BYTES_V1: u16 = 96;
pub const MAX_POLICY_BODY_BYTES_V1: usize = 4096;

const OPERATIONAL_COMMITMENTS_V1: &[&str] = &[
    "evidence_bound_operation",
    "native_authority_finality",
    "request_scope_isolation",
];
const ANTI_GOALS_V1: &[&str] = &[
    "no_default_constitution",
    "no_fixture_identity",
    "no_raw_text_identity",
    "no_unattested_hydration",
];
const EXPRESSION_BASIS_V1: &[&str] = &["bounded_plain_expression", "disclosure_by_explicit_policy"];
const CORRECTION_BOUNDARY_V1: &[&str] = &[
    "preserve_committed_g0_on_r7_failure",
    "reject_unauthorized_identity_mutation",
    "require_revalidated_attestation",
];
const RELATIONAL_PLAY_LIMITS_V1: &[&str] = &[
    "no_cross_scope_identity_transfer",
    "no_implicit_relationship_claim",
];

/// Errors returned by the closed public-policy codec and verifier.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PolicyErrorV1 {
    #[error("policy record exceeds the bounded wire size")]
    Oversize,
    #[error("policy record is truncated")]
    Truncated,
    #[error("policy record has trailing bytes")]
    TrailingBytes,
    #[error("policy record contains a noncanonical field")]
    NonCanonical,
    #[error("policy record contains an unsupported field value")]
    InvalidValue,
    #[error("policy record contains an all-zero digest")]
    ZeroDigest,
    #[error("policy derived digest mismatch")]
    DigestMismatch,
    #[error("public key or signature has an invalid width")]
    InvalidCryptoWidth,
    #[error("public key or signature failed strict Ed25519 verification")]
    InvalidSignature,
}

/// SHA-256 implementation kept local so the canonical policy wire does not
/// acquire an additional hashing dependency.  The implementation follows
/// FIPS 180-4 and accepts only bytes already bounded by the caller.
fn sha256(bytes: &[u8]) -> Digest {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state = [
        0x6a09e667u32,
        0xbb67ae85,
        0x3c6ef372,
        0xa54ff53a,
        0x510e527f,
        0x9b05688c,
        0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let padded_len = (bytes.len() + 9).div_ceil(64) * 64;
    let mut padded = Vec::with_capacity(padded_len);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    padded.resize(padded_len - 8, 0);
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (index, word) in w[..16].iter_mut().enumerate() {
            let start = index * 4;
            *word = u32::from_be_bytes([
                chunk[start],
                chunk[start + 1],
                chunk[start + 2],
                chunk[start + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let (mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h) = (
            state[0], state[1], state[2], state[3], state[4], state[5], state[6], state[7],
        );
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    let mut result = [0u8; 32];
    for (index, word) in state.into_iter().enumerate() {
        result[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    result
}

/// Hash a canonical domain and byte preimage using the policy SHA-256 rule.
pub fn domain_hash_sha256(domain: &[u8], bytes: &[u8]) -> Digest {
    let mut input = Vec::with_capacity(domain.len() + 1 + bytes.len());
    input.extend_from_slice(domain);
    input.push(0);
    input.extend_from_slice(bytes);
    sha256(&input)
}

fn require_policy_digest(digest: &Digest) -> Result<(), PolicyErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        Err(PolicyErrorV1::ZeroDigest)
    } else {
        Ok(())
    }
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn put_token(out: &mut Vec<u8>, token: &str) {
    put_u16(out, token.len() as u16);
    out.extend_from_slice(token.as_bytes());
}

fn put_terms(out: &mut Vec<u8>, terms: &[String]) {
    put_u16(out, terms.len() as u16);
    for term in terms {
        put_token(out, term);
    }
}

struct PolicyCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> PolicyCursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], PolicyErrorV1> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(PolicyErrorV1::Oversize)?;
        if end > self.bytes.len() {
            return Err(PolicyErrorV1::Truncated);
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, PolicyErrorV1> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, PolicyErrorV1> {
        Ok(u16::from_le_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| PolicyErrorV1::Truncated)?,
        ))
    }

    fn u32(&mut self) -> Result<u32, PolicyErrorV1> {
        Ok(u32::from_le_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| PolicyErrorV1::Truncated)?,
        ))
    }

    fn u64(&mut self) -> Result<u64, PolicyErrorV1> {
        Ok(u64::from_le_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| PolicyErrorV1::Truncated)?,
        ))
    }

    fn digest(&mut self) -> Result<Digest, PolicyErrorV1> {
        let digest: Digest = self
            .take(32)?
            .try_into()
            .map_err(|_| PolicyErrorV1::Truncated)?;
        require_policy_digest(&digest)?;
        Ok(digest)
    }

    fn token(&mut self) -> Result<String, PolicyErrorV1> {
        let length = usize::from(self.u16()?);
        if length == 0 || length > usize::from(POLICY_MAX_TOKEN_BYTES_V1) {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let bytes = self.take(length)?;
        let token = std::str::from_utf8(bytes).map_err(|_| PolicyErrorV1::NonCanonical)?;
        if !is_canonical_token(token) {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(token.to_owned())
    }

    fn terms(&mut self) -> Result<Vec<String>, PolicyErrorV1> {
        let count = usize::from(self.u16()?);
        if count == 0 || count > usize::from(POLICY_MAX_TERMS_V1) {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(self.token()?);
        }
        if values.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(values)
    }
}

/// Canonical GIP1 policy body.  Public fields are fixed data, not a mutable
/// map: callers must pass through `new`/`decode` so every digest is checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisIdentityPolicyV1 {
    pub policy_profile: String,
    pub policy_owner_ref: String,
    pub owner_kind: String,
    pub authorization_grant_ref: String,
    pub authorization_scheme_id: u8,
    pub authorization_key_id: String,
    pub authorization_key_version: u32,
    pub max_terms_per_section: u16,
    pub max_token_bytes: u16,
    pub max_seed_ref_bytes: u16,
    pub identity_scope_id: u8,
    pub operational_commitments: Vec<String>,
    pub anti_goals: Vec<String>,
    pub expression_basis: Vec<String>,
    pub correction_boundary_constitution: Vec<String>,
    pub relational_play_limits: Vec<String>,
    pub persona_revision_digest: Digest,
    pub seed_code_ref: String,
    pub genesis_event_digest: Digest,
    pub incarnation_sequence: u64,
    pub g0_manifest_digest: Digest,
    pub g0_seed_code_digest: Digest,
    pub g0_incarnation_id: Digest,
    pub g0_persona_source_digest: Digest,
    pub g0_genesis_receipt_digest: Digest,
    pub policy_core_digest: Digest,
    pub seed_material_digest: Digest,
    pub policy_body_digest: Digest,
}

impl GenesisIdentityPolicyV1 {
    /// Construct a policy bound to the five exact committed G0 digests.
    pub fn new(
        g0_manifest_digest: Digest,
        g0_seed_code_digest: Digest,
        g0_incarnation_id: Digest,
        g0_persona_source_digest: Digest,
        g0_genesis_receipt_digest: Digest,
    ) -> Result<Self, PolicyErrorV1> {
        for digest in [
            g0_manifest_digest,
            g0_seed_code_digest,
            g0_incarnation_id,
            g0_persona_source_digest,
            g0_genesis_receipt_digest,
        ] {
            require_policy_digest(&digest)?;
        }
        let mut policy = Self {
            policy_profile: POLICY_PROFILE_V1.to_owned(),
            policy_owner_ref: POLICY_OWNER_REF_V1.to_owned(),
            owner_kind: POLICY_OWNER_KIND_V1.to_owned(),
            authorization_grant_ref: POLICY_GRANT_REF_V1.to_owned(),
            authorization_scheme_id: 1,
            authorization_key_id: POLICY_KEY_ID_V1.to_owned(),
            authorization_key_version: 1,
            max_terms_per_section: POLICY_MAX_TERMS_V1,
            max_token_bytes: POLICY_MAX_TOKEN_BYTES_V1,
            max_seed_ref_bytes: POLICY_MAX_SEED_REF_BYTES_V1,
            identity_scope_id: 1,
            operational_commitments: OPERATIONAL_COMMITMENTS_V1
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            anti_goals: ANTI_GOALS_V1.iter().map(|s| (*s).to_owned()).collect(),
            expression_basis: EXPRESSION_BASIS_V1
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            correction_boundary_constitution: CORRECTION_BOUNDARY_V1
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            relational_play_limits: RELATIONAL_PLAY_LIMITS_V1
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
            persona_revision_digest: [0; 32],
            seed_code_ref: POLICY_SEED_CODE_REF_V1.to_owned(),
            genesis_event_digest: [0; 32],
            incarnation_sequence: 1,
            g0_manifest_digest,
            g0_seed_code_digest,
            g0_incarnation_id,
            g0_persona_source_digest,
            g0_genesis_receipt_digest,
            policy_core_digest: [0; 32],
            seed_material_digest: [0; 32],
            policy_body_digest: [0; 32],
        };
        policy.recompute_digests()?;
        Ok(policy)
    }

    fn encode_prefix(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MAX_POLICY_BODY_BYTES_V1);
        out.extend_from_slice(b"GIP1");
        put_u16(&mut out, 1);
        put_token(&mut out, &self.policy_profile);
        put_token(&mut out, &self.policy_owner_ref);
        put_token(&mut out, &self.owner_kind);
        put_token(&mut out, &self.authorization_grant_ref);
        out.push(self.authorization_scheme_id);
        put_token(&mut out, &self.authorization_key_id);
        put_u32(&mut out, self.authorization_key_version);
        put_u16(&mut out, self.max_terms_per_section);
        put_u16(&mut out, self.max_token_bytes);
        put_u16(&mut out, self.max_seed_ref_bytes);
        out.push(self.identity_scope_id);
        put_terms(&mut out, &self.operational_commitments);
        put_terms(&mut out, &self.anti_goals);
        put_terms(&mut out, &self.expression_basis);
        put_terms(&mut out, &self.correction_boundary_constitution);
        put_terms(&mut out, &self.relational_play_limits);
        out
    }

    fn encode_fields_through_g0(&self) -> Vec<u8> {
        let mut out = self.encode_prefix();
        out.extend_from_slice(&self.persona_revision_digest);
        put_token(&mut out, &self.seed_code_ref);
        out.extend_from_slice(&self.genesis_event_digest);
        put_u64(&mut out, self.incarnation_sequence);
        out.extend_from_slice(&self.g0_manifest_digest);
        out.extend_from_slice(&self.g0_seed_code_digest);
        out.extend_from_slice(&self.g0_incarnation_id);
        out.extend_from_slice(&self.g0_persona_source_digest);
        out.extend_from_slice(&self.g0_genesis_receipt_digest);
        out
    }

    fn recompute_digests(&mut self) -> Result<(), PolicyErrorV1> {
        let f1_18 = self.encode_prefix();
        self.persona_revision_digest = domain_hash_sha256(POLICY_REVISION_DOMAIN_V1, &f1_18);
        let mut g0b = Vec::with_capacity(4 + 5 * 32);
        g0b.extend_from_slice(b"G0B1");
        g0b.extend_from_slice(&self.g0_manifest_digest);
        g0b.extend_from_slice(&self.g0_seed_code_digest);
        g0b.extend_from_slice(&self.g0_incarnation_id);
        g0b.extend_from_slice(&self.g0_persona_source_digest);
        g0b.extend_from_slice(&self.g0_genesis_receipt_digest);
        self.genesis_event_digest = domain_hash_sha256(GENESIS_EVENT_DOMAIN_V1, &g0b);
        let f1_27 = self.encode_fields_through_g0();
        self.policy_core_digest = domain_hash_sha256(POLICY_CORE_DOMAIN_V1, &f1_27);
        self.seed_material_digest =
            domain_hash_sha256(SEED_MATERIAL_DOMAIN_V1, &self.policy_core_digest);
        let mut f1_29 = f1_27;
        f1_29.extend_from_slice(&self.policy_core_digest);
        f1_29.extend_from_slice(&self.seed_material_digest);
        self.policy_body_digest = domain_hash_sha256(POLICY_BODY_DOMAIN_V1, &f1_29);
        require_policy_digest(&self.policy_body_digest)
    }

    /// Return exact canonical bytes, recomputing and checking the complete DAG.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.encode_fields_through_g0();
        out.extend_from_slice(&self.policy_core_digest);
        out.extend_from_slice(&self.seed_material_digest);
        out.extend_from_slice(&self.policy_body_digest);
        out
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.encode()
    }

    /// Decode only the exact GIP1 version and fixed policy profile.
    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        if bytes.len() > MAX_POLICY_BODY_BYTES_V1 {
            return Err(PolicyErrorV1::Oversize);
        }
        let mut cursor = PolicyCursor::new(bytes);
        if cursor.take(4)? != b"GIP1" || cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let policy_profile = cursor.token()?;
        let policy_owner_ref = cursor.token()?;
        let owner_kind = cursor.token()?;
        let authorization_grant_ref = cursor.token()?;
        let authorization_scheme_id = cursor.u8()?;
        let authorization_key_id = cursor.token()?;
        let authorization_key_version = cursor.u32()?;
        let max_terms_per_section = cursor.u16()?;
        let max_token_bytes = cursor.u16()?;
        let max_seed_ref_bytes = cursor.u16()?;
        let identity_scope_id = cursor.u8()?;
        let operational_commitments = cursor.terms()?;
        let anti_goals = cursor.terms()?;
        let expression_basis = cursor.terms()?;
        let correction_boundary_constitution = cursor.terms()?;
        let relational_play_limits = cursor.terms()?;
        let f1_18_end = cursor.offset;
        let persona_revision_digest = cursor.digest()?;
        let seed_code_ref = cursor.token()?;
        let genesis_event_digest = cursor.digest()?;
        let incarnation_sequence = cursor.u64()?;
        if incarnation_sequence == 0 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let g0_manifest_digest = cursor.digest()?;
        let g0_seed_code_digest = cursor.digest()?;
        let g0_incarnation_id = cursor.digest()?;
        let g0_persona_source_digest = cursor.digest()?;
        let g0_genesis_receipt_digest = cursor.digest()?;
        let f1_27_end = cursor.offset;
        let policy_core_digest = cursor.digest()?;
        let seed_material_digest = cursor.digest()?;
        let f1_29_end = cursor.offset;
        let policy_body_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if policy_profile != POLICY_PROFILE_V1
            || policy_owner_ref != POLICY_OWNER_REF_V1
            || owner_kind != POLICY_OWNER_KIND_V1
            || authorization_grant_ref != POLICY_GRANT_REF_V1
            || authorization_scheme_id != 1
            || authorization_key_id != POLICY_KEY_ID_V1
            || authorization_key_version != 1
            || max_terms_per_section != POLICY_MAX_TERMS_V1
            || max_token_bytes != POLICY_MAX_TOKEN_BYTES_V1
            || max_seed_ref_bytes != POLICY_MAX_SEED_REF_BYTES_V1
            || identity_scope_id != 1
            || seed_code_ref != POLICY_SEED_CODE_REF_V1
            || operational_commitments
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != OPERATIONAL_COMMITMENTS_V1
            || anti_goals.iter().map(String::as_str).collect::<Vec<_>>() != ANTI_GOALS_V1
            || expression_basis
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != EXPRESSION_BASIS_V1
            || correction_boundary_constitution
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != CORRECTION_BOUNDARY_V1
            || relational_play_limits
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != RELATIONAL_PLAY_LIMITS_V1
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let expected_persona = domain_hash_sha256(POLICY_REVISION_DOMAIN_V1, &bytes[..f1_18_end]);
        if persona_revision_digest != expected_persona {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let mut g0b = Vec::with_capacity(4 + 5 * 32);
        g0b.extend_from_slice(b"G0B1");
        for digest in [
            g0_manifest_digest,
            g0_seed_code_digest,
            g0_incarnation_id,
            g0_persona_source_digest,
            g0_genesis_receipt_digest,
        ] {
            g0b.extend_from_slice(&digest);
        }
        if genesis_event_digest != domain_hash_sha256(GENESIS_EVENT_DOMAIN_V1, &g0b) {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        if policy_core_digest != domain_hash_sha256(POLICY_CORE_DOMAIN_V1, &bytes[..f1_27_end]) {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        if seed_material_digest != domain_hash_sha256(SEED_MATERIAL_DOMAIN_V1, &policy_core_digest)
        {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        if policy_body_digest != domain_hash_sha256(POLICY_BODY_DOMAIN_V1, &bytes[..f1_29_end]) {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let policy = Self {
            policy_profile,
            policy_owner_ref,
            owner_kind,
            authorization_grant_ref,
            authorization_scheme_id,
            authorization_key_id,
            authorization_key_version,
            max_terms_per_section,
            max_token_bytes,
            max_seed_ref_bytes,
            identity_scope_id,
            operational_commitments,
            anti_goals,
            expression_basis,
            correction_boundary_constitution,
            relational_play_limits,
            persona_revision_digest,
            seed_code_ref,
            genesis_event_digest,
            incarnation_sequence,
            g0_manifest_digest,
            g0_seed_code_digest,
            g0_incarnation_id,
            g0_persona_source_digest,
            g0_genesis_receipt_digest,
            policy_core_digest,
            seed_material_digest,
            policy_body_digest,
        };
        if policy.encode() != bytes {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(policy)
    }

    pub fn persona_revision_digest(&self) -> &Digest {
        &self.persona_revision_digest
    }
    pub fn genesis_event_digest(&self) -> &Digest {
        &self.genesis_event_digest
    }
    pub fn policy_core_digest(&self) -> &Digest {
        &self.policy_core_digest
    }
    pub fn seed_material_digest(&self) -> &Digest {
        &self.seed_material_digest
    }
    pub fn policy_body_digest(&self) -> &Digest {
        &self.policy_body_digest
    }
}

/// Compute the RFC-8032 Ed25519 public-key fingerprint used by all V3 roles.
pub fn fingerprint_public_key(public_key: &[u8]) -> Digest {
    let mut input = Vec::with_capacity(1 + public_key.len());
    input.push(1);
    input.extend_from_slice(public_key);
    domain_hash_sha256(PUBLIC_KEY_FINGERPRINT_DOMAIN_V1, &input)
}

/// Verify a detached, domain-bound public signature.  Only the strict
/// `VerifyingKey` path is used; this module has no signer, key-generation, or
/// secret-material API.
pub fn verify_detached_ed25519(
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), PolicyErrorV1> {
    use ed25519_dalek::{Signature, VerifyingKey};
    let key: [u8; 32] = public_key
        .try_into()
        .map_err(|_| PolicyErrorV1::InvalidCryptoWidth)?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| PolicyErrorV1::InvalidCryptoWidth)?;
    let verifying_key =
        VerifyingKey::from_bytes(&key).map_err(|_| PolicyErrorV1::InvalidSignature)?;
    let signature = Signature::from_bytes(&signature);
    verifying_key
        .verify_strict(message, &signature)
        .map_err(|_| PolicyErrorV1::InvalidSignature)
}

pub type PublicKeyV1 = [u8; 32];
pub type SignatureV1 = [u8; 64];

fn parse_public_key(cursor: &mut PolicyCursor<'_>) -> Result<PublicKeyV1, PolicyErrorV1> {
    let key: PublicKeyV1 = cursor
        .take(32)?
        .try_into()
        .map_err(|_| PolicyErrorV1::Truncated)?;
    if key.iter().all(|byte| *byte == 0) {
        return Err(PolicyErrorV1::ZeroDigest);
    }
    // Constructing the strict verifier here rejects non-canonical points and
    // the small-order identity before a key can become an active role key.
    use ed25519_dalek::VerifyingKey;
    VerifyingKey::from_bytes(&key).map_err(|_| PolicyErrorV1::InvalidSignature)?;
    Ok(key)
}

fn parse_signature(cursor: &mut PolicyCursor<'_>) -> Result<SignatureV1, PolicyErrorV1> {
    cursor
        .take(64)?
        .try_into()
        .map_err(|_| PolicyErrorV1::Truncated)
}

fn put_digest(out: &mut Vec<u8>, digest: &Digest) {
    out.extend_from_slice(digest);
}

fn put_public_key(out: &mut Vec<u8>, key: &PublicKeyV1) {
    out.extend_from_slice(key);
}

fn put_signature(out: &mut Vec<u8>, signature: &SignatureV1) {
    out.extend_from_slice(signature);
}

fn parse_magic(cursor: &mut PolicyCursor<'_>, magic: &[u8]) -> Result<(), PolicyErrorV1> {
    if cursor.take(magic.len())? != magic {
        Err(PolicyErrorV1::InvalidValue)
    } else {
        Ok(())
    }
}

/// Host-attributed delegation receipt.  It carries hashes and locators only;
/// raw user text is intentionally not representable by this DTO.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserDelegationReceiptV1 {
    pub product_ref: String,
    pub release_scope: String,
    pub delegator_ref: String,
    pub attribution: u8,
    pub source_thread_id: String,
    pub user_message_locator: String,
    pub user_message_sha256: Digest,
    pub host_attribution_digest: Digest,
    pub delegate_role: String,
    pub grant_mask: u32,
    pub policy_proposal_v2_sha256: Digest,
    pub prior_sol_review_sha256: Digest,
    pub bootstrap_decision_sha256: Digest,
    pub constraints_digest: Digest,
    pub delegation_sequence: u64,
    pub revocable: u8,
    pub delegation_receipt_digest: Digest,
}

impl UserDelegationReceiptV1 {
    fn body_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(381);
        out.extend_from_slice(b"UDR1");
        put_u16(&mut out, 1);
        put_token(&mut out, &self.product_ref);
        put_token(&mut out, &self.release_scope);
        put_token(&mut out, &self.delegator_ref);
        out.push(self.attribution);
        put_token(&mut out, &self.source_thread_id);
        put_token(&mut out, &self.user_message_locator);
        for digest in [self.user_message_sha256, self.host_attribution_digest] {
            put_digest(&mut out, &digest);
        }
        put_token(&mut out, &self.delegate_role);
        put_u32(&mut out, self.grant_mask);
        for digest in [
            self.policy_proposal_v2_sha256,
            self.prior_sol_review_sha256,
            self.bootstrap_decision_sha256,
            self.constraints_digest,
        ] {
            put_digest(&mut out, &digest);
        }
        put_u64(&mut out, self.delegation_sequence);
        out.push(self.revocable);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body_bytes();
        put_digest(&mut out, &self.delegation_receipt_digest);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.delegation_receipt_digest
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        if bytes.len() > MAX_POLICY_BODY_BYTES_V1 {
            return Err(PolicyErrorV1::Oversize);
        }
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"UDR1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let product_ref = cursor.token()?;
        let release_scope = cursor.token()?;
        let delegator_ref = cursor.token()?;
        let attribution = cursor.u8()?;
        let source_thread_id = cursor.token()?;
        let user_message_locator = cursor.token()?;
        let user_message_sha256 = cursor.digest()?;
        let host_attribution_digest = cursor.digest()?;
        let delegate_role = cursor.token()?;
        let grant_mask = cursor.u32()?;
        let policy_proposal_v2_sha256 = cursor.digest()?;
        let prior_sol_review_sha256 = cursor.digest()?;
        let bootstrap_decision_sha256 = cursor.digest()?;
        let constraints_digest = cursor.digest()?;
        let delegation_sequence = cursor.u64()?;
        let revocable = cursor.u8()?;
        let body_end = cursor.offset;
        let delegation_receipt_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if product_ref != "astr_embodiment"
            || release_scope != "1.0.0-rc1"
            || attribution != 1
            || delegate_role != "independent_sol_policy_authority"
            || grant_mask != 0x0000_001F
            || delegation_sequence != 1
            || revocable != 1
            || delegation_receipt_digest
                != domain_hash_sha256(DELEGATION_RECEIPT_DOMAIN_V1, &bytes[..body_end])
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            product_ref,
            release_scope,
            delegator_ref,
            attribution,
            source_thread_id,
            user_message_locator,
            user_message_sha256,
            host_attribution_digest,
            delegate_role,
            grant_mask,
            policy_proposal_v2_sha256,
            prior_sol_review_sha256,
            bootstrap_decision_sha256,
            constraints_digest,
            delegation_sequence,
            revocable,
            delegation_receipt_digest,
        })
    }
}

/// Non-secret custody receipt referenced by each ceremony role.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustodyDispositionReceiptV1 {
    pub custody_receipt_identity: String,
    pub custody_object_ref: String,
    pub key_id: String,
    pub key_version: u32,
    pub public_key_fingerprint: Digest,
    pub signer_kind: u8,
    pub private_disposition: u8,
    pub private_material_exported: u8,
    pub agent_visible_private_material: u8,
    pub custody_receipt_digest: Digest,
}

impl CustodyDispositionReceiptV1 {
    fn body_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(99);
        out.extend_from_slice(b"CDR1");
        put_u16(&mut out, 1);
        put_token(&mut out, &self.custody_receipt_identity);
        put_token(&mut out, &self.custody_object_ref);
        put_token(&mut out, &self.key_id);
        put_u32(&mut out, self.key_version);
        put_digest(&mut out, &self.public_key_fingerprint);
        out.push(self.signer_kind);
        out.push(self.private_disposition);
        out.push(self.private_material_exported);
        out.push(self.agent_visible_private_material);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body_bytes();
        put_digest(&mut out, &self.custody_receipt_digest);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.custody_receipt_digest
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"CDR1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let custody_receipt_identity = cursor.token()?;
        let custody_object_ref = cursor.token()?;
        let key_id = cursor.token()?;
        let key_version = cursor.u32()?;
        let public_key_fingerprint = cursor.digest()?;
        let signer_kind = cursor.u8()?;
        let private_disposition = cursor.u8()?;
        let private_material_exported = cursor.u8()?;
        let agent_visible_private_material = cursor.u8()?;
        let body_end = cursor.offset;
        let custody_receipt_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if signer_kind != 1
            || !matches!(private_disposition, 1 | 2)
            || private_material_exported != 0
            || agent_visible_private_material != 0
            || custody_receipt_digest
                != domain_hash_sha256(CUSTODY_RECEIPT_DOMAIN_V1, &bytes[..body_end])
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            custody_receipt_identity,
            custody_object_ref,
            key_id,
            key_version,
            public_key_fingerprint,
            signer_kind,
            private_disposition,
            private_material_exported,
            agent_visible_private_material,
            custody_receipt_digest,
        })
    }
}

/// Public result of the offline ceremony.  Its three detached proofs are
/// checked separately, even when root and role-1 policy use one physical key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyCeremonyReceiptV1 {
    pub delegation_receipt_digest: Digest,
    pub ceremony_sequence: u64,
    pub algorithm_id: u8,
    pub root_key_id: String,
    pub root_key_version: u32,
    pub root_public_key: PublicKeyV1,
    pub root_public_key_fingerprint: Digest,
    pub policy_key_id: String,
    pub policy_key_version: u32,
    pub policy_public_key: PublicKeyV1,
    pub policy_public_key_fingerprint: Digest,
    pub reviewer_key_id: String,
    pub reviewer_key_version: u32,
    pub reviewer_public_key: PublicKeyV1,
    pub reviewer_public_key_fingerprint: Digest,
    pub root_policy_key_relation: u8,
    pub ceremony_operator_kind: String,
    pub ceremony_operator_ref: String,
    pub ceremony_tool_identity_digest: Digest,
    pub entropy_source_class_digest: Digest,
    pub root_custody_receipt_digest: Digest,
    pub policy_custody_receipt_digest: Digest,
    pub reviewer_custody_receipt_digest: Digest,
    pub root_private_material_exported: u8,
    pub root_agent_visible_private_material: u8,
    pub root_private_disposition: u8,
    pub policy_private_material_exported: u8,
    pub policy_agent_visible_private_material: u8,
    pub policy_private_disposition: u8,
    pub reviewer_private_material_exported: u8,
    pub reviewer_agent_visible_private_material: u8,
    pub reviewer_private_disposition: u8,
    pub key_ceremony_receipt_digest: Digest,
    pub root_pop_signature: SignatureV1,
    pub policy_pop_signature: SignatureV1,
    pub reviewer_pop_signature: SignatureV1,
}

impl KeyCeremonyReceiptV1 {
    fn body_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(570);
        out.extend_from_slice(b"KCR1");
        put_u16(&mut out, 1);
        put_digest(&mut out, &self.delegation_receipt_digest);
        put_u64(&mut out, self.ceremony_sequence);
        out.push(self.algorithm_id);
        put_token(&mut out, &self.root_key_id);
        put_u32(&mut out, self.root_key_version);
        put_public_key(&mut out, &self.root_public_key);
        put_digest(&mut out, &self.root_public_key_fingerprint);
        put_token(&mut out, &self.policy_key_id);
        put_u32(&mut out, self.policy_key_version);
        put_public_key(&mut out, &self.policy_public_key);
        put_digest(&mut out, &self.policy_public_key_fingerprint);
        put_token(&mut out, &self.reviewer_key_id);
        put_u32(&mut out, self.reviewer_key_version);
        put_public_key(&mut out, &self.reviewer_public_key);
        put_digest(&mut out, &self.reviewer_public_key_fingerprint);
        out.push(self.root_policy_key_relation);
        put_token(&mut out, &self.ceremony_operator_kind);
        put_token(&mut out, &self.ceremony_operator_ref);
        put_digest(&mut out, &self.ceremony_tool_identity_digest);
        put_digest(&mut out, &self.entropy_source_class_digest);
        put_digest(&mut out, &self.root_custody_receipt_digest);
        put_digest(&mut out, &self.policy_custody_receipt_digest);
        put_digest(&mut out, &self.reviewer_custody_receipt_digest);
        out.extend_from_slice(&[
            self.root_private_material_exported,
            self.root_agent_visible_private_material,
            self.root_private_disposition,
            self.policy_private_material_exported,
            self.policy_agent_visible_private_material,
            self.policy_private_disposition,
            self.reviewer_private_material_exported,
            self.reviewer_agent_visible_private_material,
            self.reviewer_private_disposition,
        ]);
        out
    }

    fn signature_preimage(domain: &[u8], digest: &Digest) -> Vec<u8> {
        let mut out = Vec::with_capacity(domain.len() + 33);
        out.extend_from_slice(domain);
        out.push(0);
        out.extend_from_slice(digest);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body_bytes();
        put_digest(&mut out, &self.key_ceremony_receipt_digest);
        put_signature(&mut out, &self.root_pop_signature);
        put_signature(&mut out, &self.policy_pop_signature);
        put_signature(&mut out, &self.reviewer_pop_signature);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.key_ceremony_receipt_digest
    }

    pub fn verify_pop_signatures(&self) -> Result<(), PolicyErrorV1> {
        verify_detached_ed25519(
            &self.root_public_key,
            &Self::signature_preimage(ROOT_POP_DOMAIN_V1, &self.key_ceremony_receipt_digest),
            &self.root_pop_signature,
        )?;
        verify_detached_ed25519(
            &self.policy_public_key,
            &Self::signature_preimage(POLICY_POP_DOMAIN_V1, &self.key_ceremony_receipt_digest),
            &self.policy_pop_signature,
        )?;
        verify_detached_ed25519(
            &self.reviewer_public_key,
            &Self::signature_preimage(REVIEWER_POP_DOMAIN_V1, &self.key_ceremony_receipt_digest),
            &self.reviewer_pop_signature,
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        if bytes.len() > MAX_POLICY_BODY_BYTES_V1 {
            return Err(PolicyErrorV1::Oversize);
        }
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"KCR1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let delegation_receipt_digest = cursor.digest()?;
        let ceremony_sequence = cursor.u64()?;
        let algorithm_id = cursor.u8()?;
        let root_key_id = cursor.token()?;
        let root_key_version = cursor.u32()?;
        let root_public_key = parse_public_key(&mut cursor)?;
        let root_public_key_fingerprint = cursor.digest()?;
        let policy_key_id = cursor.token()?;
        let policy_key_version = cursor.u32()?;
        let policy_public_key = parse_public_key(&mut cursor)?;
        let policy_public_key_fingerprint = cursor.digest()?;
        let reviewer_key_id = cursor.token()?;
        let reviewer_key_version = cursor.u32()?;
        let reviewer_public_key = parse_public_key(&mut cursor)?;
        let reviewer_public_key_fingerprint = cursor.digest()?;
        let root_policy_key_relation = cursor.u8()?;
        let ceremony_operator_kind = cursor.token()?;
        let ceremony_operator_ref = cursor.token()?;
        let ceremony_tool_identity_digest = cursor.digest()?;
        let entropy_source_class_digest = cursor.digest()?;
        let root_custody_receipt_digest = cursor.digest()?;
        let policy_custody_receipt_digest = cursor.digest()?;
        let reviewer_custody_receipt_digest = cursor.digest()?;
        let root_private_material_exported = cursor.u8()?;
        let root_agent_visible_private_material = cursor.u8()?;
        let root_private_disposition = cursor.u8()?;
        let policy_private_material_exported = cursor.u8()?;
        let policy_agent_visible_private_material = cursor.u8()?;
        let policy_private_disposition = cursor.u8()?;
        let reviewer_private_material_exported = cursor.u8()?;
        let reviewer_agent_visible_private_material = cursor.u8()?;
        let reviewer_private_disposition = cursor.u8()?;
        let body_end = cursor.offset;
        let key_ceremony_receipt_digest = cursor.digest()?;
        let root_pop_signature = parse_signature(&mut cursor)?;
        let policy_pop_signature = parse_signature(&mut cursor)?;
        let reviewer_pop_signature = parse_signature(&mut cursor)?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if ceremony_sequence != 1
            || algorithm_id != 1
            || root_policy_key_relation != 1 && root_policy_key_relation != 2
            || ceremony_operator_kind != "delegated_bootstrap_operator"
            || root_public_key_fingerprint != fingerprint_public_key(&root_public_key)
            || policy_public_key_fingerprint != fingerprint_public_key(&policy_public_key)
            || reviewer_public_key_fingerprint != fingerprint_public_key(&reviewer_public_key)
            || root_private_material_exported != 0
            || root_agent_visible_private_material != 0
            || policy_private_material_exported != 0
            || policy_agent_visible_private_material != 0
            || reviewer_private_material_exported != 0
            || reviewer_agent_visible_private_material != 0
            || !matches!(root_private_disposition, 1 | 2)
            || !matches!(policy_private_disposition, 1 | 2)
            || !matches!(reviewer_private_disposition, 1 | 2)
            || key_ceremony_receipt_digest
                != domain_hash_sha256(CEREMONY_RECEIPT_DOMAIN_V1, &bytes[..body_end])
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let root_policy_same = root_public_key == policy_public_key
            && root_key_id == policy_key_id
            && root_key_version == policy_key_version
            && root_public_key_fingerprint == policy_public_key_fingerprint;
        if (root_policy_key_relation == 1) != root_policy_same
            || reviewer_public_key == root_public_key
            || reviewer_public_key == policy_public_key
            || reviewer_key_id == root_key_id
            || reviewer_key_id == policy_key_id
            || reviewer_public_key_fingerprint == root_public_key_fingerprint
            || reviewer_public_key_fingerprint == policy_public_key_fingerprint
            || (root_policy_key_relation == 1
                && (root_custody_receipt_digest != policy_custody_receipt_digest
                    || root_private_disposition != policy_private_disposition))
            || (root_policy_key_relation == 2
                && root_custody_receipt_digest == policy_custody_receipt_digest)
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let receipt = Self {
            delegation_receipt_digest,
            ceremony_sequence,
            algorithm_id,
            root_key_id,
            root_key_version,
            root_public_key,
            root_public_key_fingerprint,
            policy_key_id,
            policy_key_version,
            policy_public_key,
            policy_public_key_fingerprint,
            reviewer_key_id,
            reviewer_key_version,
            reviewer_public_key,
            reviewer_public_key_fingerprint,
            root_policy_key_relation,
            ceremony_operator_kind,
            ceremony_operator_ref,
            ceremony_tool_identity_digest,
            entropy_source_class_digest,
            root_custody_receipt_digest,
            policy_custody_receipt_digest,
            reviewer_custody_receipt_digest,
            root_private_material_exported,
            root_agent_visible_private_material,
            root_private_disposition,
            policy_private_material_exported,
            policy_agent_visible_private_material,
            policy_private_disposition,
            reviewer_private_material_exported,
            reviewer_agent_visible_private_material,
            reviewer_private_disposition,
            key_ceremony_receipt_digest,
            root_pop_signature,
            policy_pop_signature,
            reviewer_pop_signature,
        };
        receipt.verify_pop_signatures()?;
        Ok(receipt)
    }
}

/// Single source-pinned public trust root.  It is intentionally unsigned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseTrustRootV1 {
    pub root_key_id: String,
    pub root_key_version: u32,
    pub algorithm_id: u8,
    pub root_public_key: PublicKeyV1,
    pub root_public_key_fingerprint: Digest,
    pub delegation_receipt_digest: Digest,
    pub key_ceremony_receipt_digest: Digest,
    pub activation_sequence: u64,
    pub release_trust_root_digest: Digest,
}

impl ReleaseTrustRootV1 {
    fn body_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(149);
        out.extend_from_slice(b"RTR1");
        put_u16(&mut out, 1);
        out.push(self.algorithm_id);
        put_token(&mut out, &self.root_key_id);
        put_u32(&mut out, self.root_key_version);
        put_public_key(&mut out, &self.root_public_key);
        put_digest(&mut out, &self.root_public_key_fingerprint);
        put_digest(&mut out, &self.delegation_receipt_digest);
        put_digest(&mut out, &self.key_ceremony_receipt_digest);
        put_u64(&mut out, self.activation_sequence);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body_bytes();
        put_digest(&mut out, &self.release_trust_root_digest);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.release_trust_root_digest
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"RTR1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let algorithm_id = cursor.u8()?;
        let root_key_id = cursor.token()?;
        let root_key_version = cursor.u32()?;
        let root_public_key = parse_public_key(&mut cursor)?;
        let root_public_key_fingerprint = cursor.digest()?;
        let delegation_receipt_digest = cursor.digest()?;
        let key_ceremony_receipt_digest = cursor.digest()?;
        let activation_sequence = cursor.u64()?;
        let body_end = cursor.offset;
        let release_trust_root_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if algorithm_id != 1
            || activation_sequence != 1
            || root_public_key_fingerprint != fingerprint_public_key(&root_public_key)
            || release_trust_root_digest
                != domain_hash_sha256(RELEASE_TRUST_ROOT_DOMAIN_V1, &bytes[..body_end])
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            root_key_id,
            root_key_version,
            algorithm_id,
            root_public_key,
            root_public_key_fingerprint,
            delegation_receipt_digest,
            key_ceremony_receipt_digest,
            activation_sequence,
            release_trust_root_digest,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryGrantV1 {
    pub subject_ref: String,
    pub grant_ref: String,
    pub grant_role_id: u8,
    pub key_id: String,
    pub key_version: u32,
    pub public_key: PublicKeyV1,
    pub public_key_fingerprint: Digest,
}

impl RegistryGrantV1 {
    fn parse_from(cursor: &mut PolicyCursor<'_>) -> Result<Self, PolicyErrorV1> {
        let subject_ref = cursor.token()?;
        let grant_ref = cursor.token()?;
        let grant_role_id = cursor.u8()?;
        let key_id = cursor.token()?;
        let key_version = cursor.u32()?;
        let public_key = parse_public_key(cursor)?;
        let public_key_fingerprint = cursor.digest()?;
        if !matches!(grant_role_id, 1 | 2)
            || public_key_fingerprint != fingerprint_public_key(&public_key)
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            subject_ref,
            grant_ref,
            grant_role_id,
            key_id,
            key_version,
            public_key,
            public_key_fingerprint,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(150);
        put_token(&mut out, &self.subject_ref);
        put_token(&mut out, &self.grant_ref);
        out.push(self.grant_role_id);
        put_token(&mut out, &self.key_id);
        put_u32(&mut out, self.key_version);
        put_public_key(&mut out, &self.public_key);
        put_digest(&mut out, &self.public_key_fingerprint);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        let grant = Self::parse_from(&mut cursor)?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        Ok(grant)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryRevocationV1 {
    pub key_id: String,
    pub key_version: u32,
    pub revocation_epoch: u64,
    pub status: u8,
}

impl RegistryRevocationV1 {
    fn parse_from(cursor: &mut PolicyCursor<'_>) -> Result<Self, PolicyErrorV1> {
        let key_id = cursor.token()?;
        let key_version = cursor.u32()?;
        let revocation_epoch = cursor.u64()?;
        let status = cursor.u8()?;
        if revocation_epoch == 0 || status != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            key_id,
            key_version,
            revocation_epoch,
            status,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        put_token(&mut out, &self.key_id);
        put_u32(&mut out, self.key_version);
        put_u64(&mut out, self.revocation_epoch);
        out.push(self.status);
        out
    }
}

/// Closed root-signed registry snapshot.  Grant/revocation order is the
/// literal canonical byte order, never map iteration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RootRegistrySnapshotV1 {
    pub root_key_id: String,
    pub root_key_version: u32,
    pub registry_epoch: u64,
    pub release_trust_root_digest: Digest,
    pub delegation_receipt_digest: Digest,
    pub key_ceremony_receipt_digest: Digest,
    pub activation_sequence: u64,
    pub owner_kind: String,
    pub previous_snapshot_digest: Option<Digest>,
    pub grants: Vec<RegistryGrantV1>,
    pub revocations: Vec<RegistryRevocationV1>,
    pub registry_snapshot_digest: Digest,
    pub root_signature: SignatureV1,
}

impl RootRegistrySnapshotV1 {
    fn body_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1024);
        out.extend_from_slice(b"RGS1");
        put_u16(&mut out, 1);
        put_token(&mut out, &self.root_key_id);
        put_u32(&mut out, self.root_key_version);
        put_u64(&mut out, self.registry_epoch);
        out.extend_from_slice(&self.release_trust_root_digest);
        out.extend_from_slice(&self.delegation_receipt_digest);
        out.extend_from_slice(&self.key_ceremony_receipt_digest);
        put_u64(&mut out, self.activation_sequence);
        put_token(&mut out, &self.owner_kind);
        match self.previous_snapshot_digest {
            Some(digest) => {
                out.push(1);
                put_digest(&mut out, &digest);
            }
            None => out.push(0),
        }
        put_u16(&mut out, self.grants.len() as u16);
        for grant in &self.grants {
            out.extend_from_slice(&grant.encode());
        }
        put_u16(&mut out, self.revocations.len() as u16);
        for revocation in &self.revocations {
            out.extend_from_slice(&revocation.encode());
        }
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.body_bytes();
        put_digest(&mut out, &self.registry_snapshot_digest);
        put_signature(&mut out, &self.root_signature);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.registry_snapshot_digest
    }

    pub fn find_grant(&self, subject: &str, grant: &str, role: u8) -> Option<&RegistryGrantV1> {
        self.grants.iter().find(|entry| {
            entry.subject_ref == subject && entry.grant_ref == grant && entry.grant_role_id == role
        })
    }

    pub fn is_revoked(&self, key_id: &str, key_version: u32) -> bool {
        self.revocations
            .iter()
            .any(|entry| entry.key_id == key_id && entry.key_version == key_version)
    }

    pub fn verify_with_root(&self, root: &ReleaseTrustRootV1) -> Result<(), PolicyErrorV1> {
        if self.root_key_id != root.root_key_id
            || self.root_key_version != root.root_key_version
            || self.release_trust_root_digest != root.release_trust_root_digest
            || self.registry_snapshot_digest
                != domain_hash_sha256(REGISTRY_BODY_DOMAIN_V1, &self.body_bytes())
        {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let mut preimage =
            Vec::with_capacity(REGISTRY_SIGNATURE_DOMAIN_V1.len() + self.body_bytes().len() + 33);
        preimage.extend_from_slice(REGISTRY_SIGNATURE_DOMAIN_V1);
        preimage.push(0);
        preimage.extend_from_slice(&self.body_bytes());
        preimage.extend_from_slice(&self.registry_snapshot_digest);
        verify_detached_ed25519(&root.root_public_key, &preimage, &self.root_signature)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        if bytes.len() > u16::MAX as usize {
            return Err(PolicyErrorV1::Oversize);
        }
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"RGS1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let root_key_id = cursor.token()?;
        let root_key_version = cursor.u32()?;
        let registry_epoch = cursor.u64()?;
        if registry_epoch == 0 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let release_trust_root_digest = cursor.digest()?;
        let delegation_receipt_digest = cursor.digest()?;
        let key_ceremony_receipt_digest = cursor.digest()?;
        let activation_sequence = cursor.u64()?;
        if activation_sequence != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let owner_kind = cursor.token()?;
        if owner_kind != POLICY_OWNER_KIND_V1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let has_previous = cursor.u8()?;
        let previous_snapshot_digest = match has_previous {
            0 => None,
            1 => Some(cursor.digest()?),
            _ => return Err(PolicyErrorV1::InvalidValue),
        };
        if (registry_epoch == 1) != previous_snapshot_digest.is_none() {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let grant_count = usize::from(cursor.u16()?);
        if grant_count > 256 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let mut grants = Vec::with_capacity(grant_count);
        let mut previous_grant_bytes: Option<Vec<u8>> = None;
        for _ in 0..grant_count {
            let grant = RegistryGrantV1::parse_from(&mut cursor)?;
            let encoded = grant.encode();
            if previous_grant_bytes
                .as_ref()
                .is_some_and(|previous| previous >= &encoded)
            {
                return Err(PolicyErrorV1::NonCanonical);
            }
            previous_grant_bytes = Some(encoded);
            grants.push(grant);
        }
        let revocation_count = usize::from(cursor.u16()?);
        if revocation_count > 256 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let mut revocations = Vec::with_capacity(revocation_count);
        let mut previous_revocation: Option<(String, u32)> = None;
        for _ in 0..revocation_count {
            let revocation = RegistryRevocationV1::parse_from(&mut cursor)?;
            let key = (revocation.key_id.clone(), revocation.key_version);
            if previous_revocation
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                return Err(PolicyErrorV1::NonCanonical);
            }
            previous_revocation = Some(key);
            if revocation.revocation_epoch > registry_epoch {
                return Err(PolicyErrorV1::InvalidValue);
            }
            revocations.push(revocation);
        }
        let body_end = cursor.offset;
        let registry_snapshot_digest = cursor.digest()?;
        let root_signature = parse_signature(&mut cursor)?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if registry_snapshot_digest
            != domain_hash_sha256(REGISTRY_BODY_DOMAIN_V1, &bytes[..body_end])
        {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let snapshot = Self {
            root_key_id,
            root_key_version,
            registry_epoch,
            release_trust_root_digest,
            delegation_receipt_digest,
            key_ceremony_receipt_digest,
            activation_sequence,
            owner_kind,
            previous_snapshot_digest,
            grants,
            revocations,
            registry_snapshot_digest,
            root_signature,
        };
        if snapshot.encode() != bytes {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndependentSolReviewMessageV1 {
    pub reviewer_authority_ref: String,
    pub reviewer_grant_ref: String,
    pub reviewer_key_id: String,
    pub reviewer_key_version: u32,
    pub approval: u8,
    pub policy_spec_normalized_sha256: Digest,
    pub policy_body_digest: Digest,
    pub registry_snapshot_digest: Digest,
    pub native_source_identity_digest: Digest,
    pub plugin_source_identity_digest: Digest,
    pub control_evidence_set_digest: Digest,
    pub delegation_receipt_digest: Digest,
    pub key_ceremony_receipt_digest: Digest,
    pub release_trust_root_digest: Digest,
    pub root_public_key_fingerprint: Digest,
    pub reviewer_public_key_fingerprint: Digest,
    pub approval_origin: u8,
    pub approval_actor: u8,
}

impl IndependentSolReviewMessageV1 {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(434);
        out.extend_from_slice(b"IRM1");
        put_u16(&mut out, 1);
        put_token(&mut out, &self.reviewer_authority_ref);
        put_token(&mut out, &self.reviewer_grant_ref);
        put_token(&mut out, &self.reviewer_key_id);
        put_u32(&mut out, self.reviewer_key_version);
        out.push(self.approval);
        for digest in [
            self.policy_spec_normalized_sha256,
            self.policy_body_digest,
            self.registry_snapshot_digest,
            self.native_source_identity_digest,
            self.plugin_source_identity_digest,
            self.control_evidence_set_digest,
            self.delegation_receipt_digest,
            self.key_ceremony_receipt_digest,
            self.release_trust_root_digest,
            self.root_public_key_fingerprint,
            self.reviewer_public_key_fingerprint,
        ] {
            put_digest(&mut out, &digest);
        }
        out.push(self.approval_origin);
        out.push(self.approval_actor);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"IRM1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let reviewer_authority_ref = cursor.token()?;
        let reviewer_grant_ref = cursor.token()?;
        let reviewer_key_id = cursor.token()?;
        let reviewer_key_version = cursor.u32()?;
        let approval = cursor.u8()?;
        let policy_spec_normalized_sha256 = cursor.digest()?;
        let policy_body_digest = cursor.digest()?;
        let registry_snapshot_digest = cursor.digest()?;
        let native_source_identity_digest = cursor.digest()?;
        let plugin_source_identity_digest = cursor.digest()?;
        let control_evidence_set_digest = cursor.digest()?;
        let delegation_receipt_digest = cursor.digest()?;
        let key_ceremony_receipt_digest = cursor.digest()?;
        let release_trust_root_digest = cursor.digest()?;
        let root_public_key_fingerprint = cursor.digest()?;
        let reviewer_public_key_fingerprint = cursor.digest()?;
        let approval_origin = cursor.u8()?;
        let approval_actor = cursor.u8()?;
        if cursor.offset != bytes.len()
            || approval != 1
            || approval_origin != 1
            || approval_actor != 1
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            reviewer_authority_ref,
            reviewer_grant_ref,
            reviewer_key_id,
            reviewer_key_version,
            approval,
            policy_spec_normalized_sha256,
            policy_body_digest,
            registry_snapshot_digest,
            native_source_identity_digest,
            plugin_source_identity_digest,
            control_evidence_set_digest,
            delegation_receipt_digest,
            key_ceremony_receipt_digest,
            release_trust_root_digest,
            root_public_key_fingerprint,
            reviewer_public_key_fingerprint,
            approval_origin,
            approval_actor,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IndependentSolReviewReceiptV1 {
    pub message: IndependentSolReviewMessageV1,
    pub reviewer_signature: SignatureV1,
    pub review_receipt_digest: Digest,
}

impl IndependentSolReviewReceiptV1 {
    fn signature_preimage(&self) -> Vec<u8> {
        let message = self.message.encode();
        let mut out = Vec::with_capacity(REVIEW_RECEIPT_DOMAIN_V1.len() + 1 + message.len());
        out.extend_from_slice(REVIEW_RECEIPT_DOMAIN_V1);
        out.push(0);
        out.extend_from_slice(&message);
        out
    }

    fn outer_without_digest(&self) -> Vec<u8> {
        let message = self.message.encode();
        let mut out = Vec::with_capacity(6 + message.len() + 64);
        out.extend_from_slice(b"IRR1");
        put_u16(&mut out, message.len() as u16);
        out.extend_from_slice(&message);
        put_signature(&mut out, &self.reviewer_signature);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.outer_without_digest();
        put_digest(&mut out, &self.review_receipt_digest);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.review_receipt_digest
    }

    pub fn verify(&self, reviewer_public_key: &PublicKeyV1) -> Result<(), PolicyErrorV1> {
        verify_detached_ed25519(
            reviewer_public_key,
            &self.signature_preimage(),
            &self.reviewer_signature,
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"IRR1")?;
        let message_len = usize::from(cursor.u16()?);
        if message_len > 1024 {
            return Err(PolicyErrorV1::Oversize);
        }
        let message_bytes = cursor.take(message_len)?;
        let message = IndependentSolReviewMessageV1::decode(message_bytes)?;
        let reviewer_signature = parse_signature(&mut cursor)?;
        let outer_end = cursor.offset;
        let review_receipt_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if review_receipt_digest
            != domain_hash_sha256(REVIEW_RECEIPT_RECORD_DOMAIN_V1, &bytes[..outer_end])
        {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let receipt = Self {
            message,
            reviewer_signature,
            review_receipt_digest,
        };
        if receipt.encode() != bytes {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(receipt)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyAttestationMessageV1 {
    pub scheme_id: u8,
    pub role_id: u8,
    pub registry_epoch: u64,
    pub policy_body_digest: Digest,
    pub review_receipt_digest: Digest,
    pub registry_snapshot_digest: Digest,
    pub release_trust_root_digest: Digest,
    pub delegation_receipt_digest: Digest,
    pub key_ceremony_receipt_digest: Digest,
    pub policy_public_key_fingerprint: Digest,
    pub policy_spec_normalized_sha256: Digest,
    pub policy_owner_ref: String,
    pub authorization_grant_ref: String,
    pub attestation_key_id: String,
    pub attestation_key_version: u32,
}

impl PolicyAttestationMessageV1 {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(381);
        out.extend_from_slice(b"PAM1");
        put_u16(&mut out, 1);
        out.push(self.scheme_id);
        out.push(self.role_id);
        put_u64(&mut out, self.registry_epoch);
        for digest in [
            self.policy_body_digest,
            self.review_receipt_digest,
            self.registry_snapshot_digest,
            self.release_trust_root_digest,
            self.delegation_receipt_digest,
            self.key_ceremony_receipt_digest,
            self.policy_public_key_fingerprint,
            self.policy_spec_normalized_sha256,
        ] {
            put_digest(&mut out, &digest);
        }
        put_token(&mut out, &self.policy_owner_ref);
        put_token(&mut out, &self.authorization_grant_ref);
        put_token(&mut out, &self.attestation_key_id);
        put_u32(&mut out, self.attestation_key_version);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"PAM1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let scheme_id = cursor.u8()?;
        let role_id = cursor.u8()?;
        let registry_epoch = cursor.u64()?;
        let policy_body_digest = cursor.digest()?;
        let review_receipt_digest = cursor.digest()?;
        let registry_snapshot_digest = cursor.digest()?;
        let release_trust_root_digest = cursor.digest()?;
        let delegation_receipt_digest = cursor.digest()?;
        let key_ceremony_receipt_digest = cursor.digest()?;
        let policy_public_key_fingerprint = cursor.digest()?;
        let policy_spec_normalized_sha256 = cursor.digest()?;
        let policy_owner_ref = cursor.token()?;
        let authorization_grant_ref = cursor.token()?;
        let attestation_key_id = cursor.token()?;
        let attestation_key_version = cursor.u32()?;
        if cursor.offset != bytes.len()
            || scheme_id != 1
            || role_id != 1
            || registry_epoch == 0
            || policy_owner_ref != POLICY_OWNER_REF_V1
            || authorization_grant_ref != POLICY_GRANT_REF_V1
            || attestation_key_id != POLICY_KEY_ID_V1
            || attestation_key_version != 1
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            scheme_id,
            role_id,
            registry_epoch,
            policy_body_digest,
            review_receipt_digest,
            registry_snapshot_digest,
            release_trust_root_digest,
            delegation_receipt_digest,
            key_ceremony_receipt_digest,
            policy_public_key_fingerprint,
            policy_spec_normalized_sha256,
            policy_owner_ref,
            authorization_grant_ref,
            attestation_key_id,
            attestation_key_version,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PolicyAttestationV1 {
    pub message: PolicyAttestationMessageV1,
    pub policy_signature: SignatureV1,
    pub policy_attestation_digest: Digest,
}

impl PolicyAttestationV1 {
    fn signature_preimage(&self) -> Vec<u8> {
        let message = self.message.encode();
        let mut out = Vec::with_capacity(ATTESTATION_MESSAGE_DOMAIN_V1.len() + 1 + message.len());
        out.extend_from_slice(ATTESTATION_MESSAGE_DOMAIN_V1);
        out.push(0);
        out.extend_from_slice(&message);
        out
    }

    fn outer_without_digest(&self) -> Vec<u8> {
        let message = self.message.encode();
        let mut out = Vec::with_capacity(6 + message.len() + 64);
        out.extend_from_slice(b"PAT1");
        put_u16(&mut out, message.len() as u16);
        out.extend_from_slice(&message);
        put_signature(&mut out, &self.policy_signature);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.outer_without_digest();
        put_digest(&mut out, &self.policy_attestation_digest);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.policy_attestation_digest
    }

    pub fn verify(&self, policy_public_key: &PublicKeyV1) -> Result<(), PolicyErrorV1> {
        verify_detached_ed25519(
            policy_public_key,
            &self.signature_preimage(),
            &self.policy_signature,
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"PAT1")?;
        let message_len = usize::from(cursor.u16()?);
        if message_len > 1024 {
            return Err(PolicyErrorV1::Oversize);
        }
        let message = PolicyAttestationMessageV1::decode(cursor.take(message_len)?)?;
        let policy_signature = parse_signature(&mut cursor)?;
        let outer_end = cursor.offset;
        let policy_attestation_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if policy_attestation_digest
            != domain_hash_sha256(ATTESTATION_RECORD_DOMAIN_V1, &bytes[..outer_end])
        {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let attestation = Self {
            message,
            policy_signature,
            policy_attestation_digest,
        };
        if attestation.encode() != bytes {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(attestation)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapActivationMessageV1 {
    pub approval_origin: u8,
    pub approval_actor: u8,
    pub user_direct_fingerprint_approval: u8,
    pub delegated_fingerprint_approval: u8,
    pub release_disposition: u8,
    pub delegation_receipt_digest: Digest,
    pub key_ceremony_receipt_digest: Digest,
    pub release_trust_root_digest: Digest,
    pub root_public_key_fingerprint: Digest,
    pub registry_snapshot_digest: Digest,
    pub registry_epoch: u64,
    pub policy_spec_normalized_sha256: Digest,
    pub policy_body_digest: Digest,
    pub review_receipt_digest: Digest,
    pub policy_attestation_digest: Digest,
    pub native_source_identity_digest: Digest,
    pub plugin_source_identity_digest: Digest,
    pub control_evidence_set_digest: Digest,
    pub g0_binding_contract_digest: Digest,
    pub g0_only_fallback_contract_digest: Digest,
    pub reviewer_authority_ref: String,
    pub reviewer_grant_ref: String,
    pub reviewer_key_id: String,
    pub reviewer_key_version: u32,
    pub reviewer_public_key_fingerprint: Digest,
    pub activation_sequence: u64,
}

impl BootstrapActivationMessageV1 {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(580);
        out.extend_from_slice(b"BAM1");
        put_u16(&mut out, 1);
        out.extend_from_slice(&[
            self.approval_origin,
            self.approval_actor,
            self.user_direct_fingerprint_approval,
            self.delegated_fingerprint_approval,
            self.release_disposition,
        ]);
        for digest in [
            self.delegation_receipt_digest,
            self.key_ceremony_receipt_digest,
            self.release_trust_root_digest,
            self.root_public_key_fingerprint,
            self.registry_snapshot_digest,
        ] {
            put_digest(&mut out, &digest);
        }
        put_u64(&mut out, self.registry_epoch);
        for digest in [
            self.policy_spec_normalized_sha256,
            self.policy_body_digest,
            self.review_receipt_digest,
            self.policy_attestation_digest,
            self.native_source_identity_digest,
            self.plugin_source_identity_digest,
            self.control_evidence_set_digest,
            self.g0_binding_contract_digest,
            self.g0_only_fallback_contract_digest,
        ] {
            put_digest(&mut out, &digest);
        }
        put_token(&mut out, &self.reviewer_authority_ref);
        put_token(&mut out, &self.reviewer_grant_ref);
        put_token(&mut out, &self.reviewer_key_id);
        put_u32(&mut out, self.reviewer_key_version);
        put_digest(&mut out, &self.reviewer_public_key_fingerprint);
        put_u64(&mut out, self.activation_sequence);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"BAM1")?;
        if cursor.u16()? != 1 {
            return Err(PolicyErrorV1::InvalidValue);
        }
        let approval_origin = cursor.u8()?;
        let approval_actor = cursor.u8()?;
        let user_direct_fingerprint_approval = cursor.u8()?;
        let delegated_fingerprint_approval = cursor.u8()?;
        let release_disposition = cursor.u8()?;
        let delegation_receipt_digest = cursor.digest()?;
        let key_ceremony_receipt_digest = cursor.digest()?;
        let release_trust_root_digest = cursor.digest()?;
        let root_public_key_fingerprint = cursor.digest()?;
        let registry_snapshot_digest = cursor.digest()?;
        let registry_epoch = cursor.u64()?;
        let policy_spec_normalized_sha256 = cursor.digest()?;
        let policy_body_digest = cursor.digest()?;
        let review_receipt_digest = cursor.digest()?;
        let policy_attestation_digest = cursor.digest()?;
        let native_source_identity_digest = cursor.digest()?;
        let plugin_source_identity_digest = cursor.digest()?;
        let control_evidence_set_digest = cursor.digest()?;
        let g0_binding_contract_digest = cursor.digest()?;
        let g0_only_fallback_contract_digest = cursor.digest()?;
        let reviewer_authority_ref = cursor.token()?;
        let reviewer_grant_ref = cursor.token()?;
        let reviewer_key_id = cursor.token()?;
        let reviewer_key_version = cursor.u32()?;
        let reviewer_public_key_fingerprint = cursor.digest()?;
        let activation_sequence = cursor.u64()?;
        if cursor.offset != bytes.len()
            || approval_origin != 1
            || approval_actor != 1
            || user_direct_fingerprint_approval != 0
            || delegated_fingerprint_approval != 1
            || release_disposition != 1
            || registry_epoch == 0
            || activation_sequence != 1
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
        Ok(Self {
            approval_origin,
            approval_actor,
            user_direct_fingerprint_approval,
            delegated_fingerprint_approval,
            release_disposition,
            delegation_receipt_digest,
            key_ceremony_receipt_digest,
            release_trust_root_digest,
            root_public_key_fingerprint,
            registry_snapshot_digest,
            registry_epoch,
            policy_spec_normalized_sha256,
            policy_body_digest,
            review_receipt_digest,
            policy_attestation_digest,
            native_source_identity_digest,
            plugin_source_identity_digest,
            control_evidence_set_digest,
            g0_binding_contract_digest,
            g0_only_fallback_contract_digest,
            reviewer_authority_ref,
            reviewer_grant_ref,
            reviewer_key_id,
            reviewer_key_version,
            reviewer_public_key_fingerprint,
            activation_sequence,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapActivationReceiptV1 {
    pub message: BootstrapActivationMessageV1,
    pub reviewer_signature: SignatureV1,
    pub bootstrap_activation_receipt_digest: Digest,
}

impl BootstrapActivationReceiptV1 {
    fn signature_preimage(&self) -> Vec<u8> {
        let message = self.message.encode();
        let mut out = Vec::with_capacity(BOOTSTRAP_ACTIVATION_DOMAIN_V1.len() + 1 + message.len());
        out.extend_from_slice(BOOTSTRAP_ACTIVATION_DOMAIN_V1);
        out.push(0);
        out.extend_from_slice(&message);
        out
    }

    fn outer_without_digest(&self) -> Vec<u8> {
        let message = self.message.encode();
        let mut out = Vec::with_capacity(6 + message.len() + 64);
        out.extend_from_slice(b"BAR1");
        put_u16(&mut out, message.len() as u16);
        out.extend_from_slice(&message);
        put_signature(&mut out, &self.reviewer_signature);
        out
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = self.outer_without_digest();
        put_digest(&mut out, &self.bootstrap_activation_receipt_digest);
        out
    }

    pub fn digest(&self) -> &Digest {
        &self.bootstrap_activation_receipt_digest
    }

    pub fn verify(&self, reviewer_public_key: &PublicKeyV1) -> Result<(), PolicyErrorV1> {
        verify_detached_ed25519(
            reviewer_public_key,
            &self.signature_preimage(),
            &self.reviewer_signature,
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PolicyErrorV1> {
        let mut cursor = PolicyCursor::new(bytes);
        parse_magic(&mut cursor, b"BAR1")?;
        let message_len = usize::from(cursor.u16()?);
        if message_len > 1024 {
            return Err(PolicyErrorV1::Oversize);
        }
        let message = BootstrapActivationMessageV1::decode(cursor.take(message_len)?)?;
        let reviewer_signature = parse_signature(&mut cursor)?;
        let outer_end = cursor.offset;
        let bootstrap_activation_receipt_digest = cursor.digest()?;
        if cursor.offset != bytes.len() {
            return Err(PolicyErrorV1::TrailingBytes);
        }
        if bootstrap_activation_receipt_digest
            != domain_hash_sha256(BOOTSTRAP_ACTIVATION_RECORD_DOMAIN_V1, &bytes[..outer_end])
        {
            return Err(PolicyErrorV1::DigestMismatch);
        }
        let receipt = Self {
            message,
            reviewer_signature,
            bootstrap_activation_receipt_digest,
        };
        if receipt.encode() != bytes {
            return Err(PolicyErrorV1::NonCanonical);
        }
        Ok(receipt)
    }
}

/// The public evidence selected by the closed authority chain.  This is a
/// verification result only; it does not create, import, or retain any
/// signing material or runtime state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorityClosureV1 {
    pub delegation_receipt_digest: Digest,
    pub ceremony_receipt_digest: Digest,
    pub release_trust_root_digest: Digest,
    pub registry_snapshot_digest: Digest,
    pub review_receipt_digest: Digest,
    pub policy_attestation_digest: Digest,
    pub bootstrap_activation_receipt_digest: Digest,
    pub role1_grant: RegistryGrantV1,
    pub role2_grant: RegistryGrantV1,
}

/// Verify the complete public authority closure in the fixed V3 order.
/// Every object is round-tripped through its canonical decoder before any
/// cross-object relation is accepted.  The returned grants are the exact
/// selected role-1 and role-2 registry entries.
#[allow(clippy::too_many_arguments)]
pub fn verify_authority_closure_v1(
    delegation: &UserDelegationReceiptV1,
    ceremony: &KeyCeremonyReceiptV1,
    root_custody: &CustodyDispositionReceiptV1,
    policy_custody: &CustodyDispositionReceiptV1,
    reviewer_custody: &CustodyDispositionReceiptV1,
    root: &ReleaseTrustRootV1,
    registry: &RootRegistrySnapshotV1,
    review: &IndependentSolReviewReceiptV1,
    attestation: &PolicyAttestationV1,
    activation: &BootstrapActivationReceiptV1,
) -> Result<AuthorityClosureV1, PolicyErrorV1> {
    if UserDelegationReceiptV1::decode(&delegation.encode())? != *delegation
        || KeyCeremonyReceiptV1::decode(&ceremony.encode())? != *ceremony
        || CustodyDispositionReceiptV1::decode(&root_custody.encode())? != *root_custody
        || CustodyDispositionReceiptV1::decode(&policy_custody.encode())? != *policy_custody
        || CustodyDispositionReceiptV1::decode(&reviewer_custody.encode())? != *reviewer_custody
        || ReleaseTrustRootV1::decode(&root.encode())? != *root
        || RootRegistrySnapshotV1::decode(&registry.encode())? != *registry
        || IndependentSolReviewReceiptV1::decode(&review.encode())? != *review
        || PolicyAttestationV1::decode(&attestation.encode())? != *attestation
        || BootstrapActivationReceiptV1::decode(&activation.encode())? != *activation
    {
        return Err(PolicyErrorV1::NonCanonical);
    }

    if delegation.grant_mask != 0x0000_001F
        || ceremony.delegation_receipt_digest != *delegation.digest()
        || ceremony.root_custody_receipt_digest != *root_custody.digest()
        || ceremony.policy_custody_receipt_digest != *policy_custody.digest()
        || ceremony.reviewer_custody_receipt_digest != *reviewer_custody.digest()
    {
        return Err(PolicyErrorV1::DigestMismatch);
    }

    let custody_matches = [
        (
            root_custody,
            &ceremony.root_key_id,
            ceremony.root_key_version,
            ceremony.root_public_key_fingerprint,
        ),
        (
            policy_custody,
            &ceremony.policy_key_id,
            ceremony.policy_key_version,
            ceremony.policy_public_key_fingerprint,
        ),
        (
            reviewer_custody,
            &ceremony.reviewer_key_id,
            ceremony.reviewer_key_version,
            ceremony.reviewer_public_key_fingerprint,
        ),
    ];
    if custody_matches
        .iter()
        .any(|(custody, key_id, key_version, fingerprint)| {
            custody.key_id != **key_id
                || custody.key_version != *key_version
                || custody.public_key_fingerprint != *fingerprint
        })
    {
        return Err(PolicyErrorV1::InvalidValue);
    }
    if ceremony.root_policy_key_relation == 1 {
        if root_custody.custody_receipt_identity != policy_custody.custody_receipt_identity
            || root_custody.custody_object_ref != policy_custody.custody_object_ref
            || root_custody.custody_receipt_digest != policy_custody.custody_receipt_digest
            || root_custody.private_disposition != policy_custody.private_disposition
        {
            return Err(PolicyErrorV1::InvalidValue);
        }
    } else if root_custody.custody_receipt_identity == policy_custody.custody_receipt_identity
        || root_custody.custody_object_ref == policy_custody.custody_object_ref
        || root_custody.custody_receipt_digest == policy_custody.custody_receipt_digest
    {
        return Err(PolicyErrorV1::InvalidValue);
    }
    if root_custody.custody_receipt_identity == reviewer_custody.custody_receipt_identity
        || root_custody.custody_object_ref == reviewer_custody.custody_object_ref
        || root_custody.custody_receipt_digest == reviewer_custody.custody_receipt_digest
        || policy_custody.custody_receipt_identity == reviewer_custody.custody_receipt_identity
        || policy_custody.custody_object_ref == reviewer_custody.custody_object_ref
        || policy_custody.custody_receipt_digest == reviewer_custody.custody_receipt_digest
    {
        return Err(PolicyErrorV1::InvalidValue);
    }

    ceremony.verify_pop_signatures()?;
    if root.delegation_receipt_digest != *delegation.digest()
        || root.key_ceremony_receipt_digest != *ceremony.digest()
        || root.root_key_id != ceremony.root_key_id
        || root.root_key_version != ceremony.root_key_version
        || root.root_public_key != ceremony.root_public_key
        || root.root_public_key_fingerprint != ceremony.root_public_key_fingerprint
    {
        return Err(PolicyErrorV1::DigestMismatch);
    }

    registry.verify_with_root(root)?;
    if registry.delegation_receipt_digest != *delegation.digest()
        || registry.key_ceremony_receipt_digest != *ceremony.digest()
        || registry.release_trust_root_digest != *root.digest()
    {
        return Err(PolicyErrorV1::DigestMismatch);
    }
    let role1 = registry
        .find_grant(
            &attestation.message.policy_owner_ref,
            &attestation.message.authorization_grant_ref,
            1,
        )
        .ok_or(PolicyErrorV1::InvalidValue)?;
    let role2 = registry
        .find_grant(
            &review.message.reviewer_authority_ref,
            &review.message.reviewer_grant_ref,
            2,
        )
        .ok_or(PolicyErrorV1::InvalidValue)?;
    if role1.key_id != ceremony.policy_key_id
        || role1.key_version != ceremony.policy_key_version
        || role1.public_key != ceremony.policy_public_key
        || role1.public_key_fingerprint != ceremony.policy_public_key_fingerprint
        || role2.key_id != ceremony.reviewer_key_id
        || role2.key_version != ceremony.reviewer_key_version
        || role2.public_key != ceremony.reviewer_public_key
        || role2.public_key_fingerprint != ceremony.reviewer_public_key_fingerprint
        || role1.public_key == role2.public_key
        || role1.public_key_fingerprint == role2.public_key_fingerprint
        || role1.key_id == role2.key_id
        || root.root_public_key == role2.public_key
        || root.root_public_key_fingerprint == role2.public_key_fingerprint
        || root.root_key_id == role2.key_id
        || registry.is_revoked(&role1.key_id, role1.key_version)
        || registry.is_revoked(&role2.key_id, role2.key_version)
        || registry.is_revoked(&root.root_key_id, root.root_key_version)
    {
        return Err(PolicyErrorV1::InvalidValue);
    }

    if review.message.policy_body_digest != attestation.message.policy_body_digest
        || review.message.registry_snapshot_digest != *registry.digest()
        || review.message.delegation_receipt_digest != *delegation.digest()
        || review.message.key_ceremony_receipt_digest != *ceremony.digest()
        || review.message.release_trust_root_digest != *root.digest()
        || review.message.root_public_key_fingerprint != root.root_public_key_fingerprint
        || review.message.reviewer_key_id != role2.key_id
        || review.message.reviewer_key_version != role2.key_version
        || review.message.reviewer_public_key_fingerprint != role2.public_key_fingerprint
    {
        return Err(PolicyErrorV1::DigestMismatch);
    }
    review.verify(&role2.public_key)?;

    if attestation.message.registry_epoch != registry.registry_epoch
        || attestation.message.policy_body_digest != review.message.policy_body_digest
        || attestation.message.review_receipt_digest != *review.digest()
        || attestation.message.registry_snapshot_digest != *registry.digest()
        || attestation.message.release_trust_root_digest != *root.digest()
        || attestation.message.delegation_receipt_digest != *delegation.digest()
        || attestation.message.key_ceremony_receipt_digest != *ceremony.digest()
        || attestation.message.policy_public_key_fingerprint != role1.public_key_fingerprint
        || attestation.message.attestation_key_id != role1.key_id
        || attestation.message.attestation_key_version != role1.key_version
    {
        return Err(PolicyErrorV1::DigestMismatch);
    }
    attestation.verify(&role1.public_key)?;

    if activation.message.registry_epoch != registry.registry_epoch
        || activation.message.delegation_receipt_digest != *delegation.digest()
        || activation.message.key_ceremony_receipt_digest != *ceremony.digest()
        || activation.message.release_trust_root_digest != *root.digest()
        || activation.message.root_public_key_fingerprint != root.root_public_key_fingerprint
        || activation.message.registry_snapshot_digest != *registry.digest()
        || activation.message.review_receipt_digest != *review.digest()
        || activation.message.policy_attestation_digest != *attestation.digest()
        || activation.message.reviewer_authority_ref != role2.subject_ref
        || activation.message.reviewer_grant_ref != role2.grant_ref
        || activation.message.reviewer_key_id != role2.key_id
        || activation.message.reviewer_key_version != role2.key_version
        || activation.message.reviewer_public_key_fingerprint != role2.public_key_fingerprint
    {
        return Err(PolicyErrorV1::DigestMismatch);
    }
    activation.verify(&role2.public_key)?;

    Ok(AuthorityClosureV1 {
        delegation_receipt_digest: *delegation.digest(),
        ceremony_receipt_digest: *ceremony.digest(),
        release_trust_root_digest: *root.digest(),
        registry_snapshot_digest: *registry.digest(),
        review_receipt_digest: *review.digest(),
        policy_attestation_digest: *attestation.digest(),
        bootstrap_activation_receipt_digest: *activation.digest(),
        role1_grant: role1.clone(),
        role2_grant: role2.clone(),
    })
}

pub type BootstrapActivationReceipt = BootstrapActivationReceiptV1;
pub type PolicyAttestationReceiptV1 = PolicyAttestationV1;
