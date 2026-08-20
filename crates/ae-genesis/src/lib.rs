#![forbid(unsafe_code)]

//! Genesis Rust core: manifest projection, SeedCode/Incarnation identity,
//! DevelopmentSeed and GenesisCapsule build/verification.
//!
//! Everything here is a pure function of its inputs. The same accepted
//! proposal produces the same Manifest, SeedCode and (for the same birth
//! transaction) IncarnationId on any machine, in any process, independent of
//! Formula revision only where the contract says so.

use ae_contracts::{
    wire, AllostaticSetpoints, Digest, EpistemicPriors, ExpressionPhenotype, GenesisCapsule,
    GenesisManifest, PersonaGenesisRequest, PersonalityVector, SocialPriors,
};
use ae_fixed::{Fixed, SCALE};
use thiserror::Error;

pub const MIN_TRAIT: Fixed = Fixed::from_raw(50_000);
pub const MAX_TRAIT: Fixed = Fixed::from_raw(950_000);

#[derive(Clone, Debug)]
pub struct GenesisPrior {
    pub personality: PersonalityVector,
}

impl Default for GenesisPrior {
    fn default() -> Self {
        Self {
            personality: PersonalityVector {
                baseline_warmth: Fixed::from_raw(520_000),
                baseline_patience: Fixed::from_raw(650_000),
                sensitivity: Fixed::from_raw(500_000),
                irritability: Fixed::from_raw(350_000),
                composure: Fixed::from_raw(620_000),
                epistemic_pride: Fixed::from_raw(480_000),
                epistemic_openness: Fixed::from_raw(650_000),
                boundary_strength: Fixed::from_raw(600_000),
                forgiveness: Fixed::from_raw(550_000),
                attachment_propensity: Fixed::from_raw(420_000),
                expression_drive: Fixed::from_raw(500_000),
                curiosity: Fixed::from_raw(600_000),
            },
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum GenesisError {
    #[error("persona proposal source does not match request source")]
    SourceMismatch,
    #[error("persona proposal could not be canonically encoded")]
    CanonicalEncoding,
    #[error("capsule verification failed: {0}")]
    CapsuleInvalid(&'static str),
    #[error("SeedCode is a content identity, not a portable payload; import requires a verified GenesisCapsule")]
    SeedCodeOnlyImport,
}

/// All derived identities of one birth transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GenesisIdentity {
    pub manifest: GenesisManifest,
    pub manifest_digest: Digest,
    pub seed_code_digest: Digest,
    pub incarnation_id: Digest,
    pub development_seed_digest: Digest,
}

fn hash_domain(domain: &[u8], fields: &[&[u8]]) -> Digest {
    wire::domain_hash(domain, fields)
}

fn blend(prior: Fixed, candidate: Fixed, confidence: Fixed) -> Fixed {
    let c = confidence.clamp(Fixed::ZERO, Fixed::ONE).raw() as i128;
    let delta = i128::from(candidate.raw()) - i128::from(prior.raw());
    let blended = i128::from(prior.raw()) + delta * c / i128::from(SCALE);
    let raw = i64::try_from(blended).unwrap_or_else(|_| {
        if blended.is_negative() {
            i64::MIN
        } else {
            i64::MAX
        }
    });
    Fixed::from_raw(raw).clamp(MIN_TRAIT, MAX_TRAIT)
}

fn project_personality(
    prior: &PersonalityVector,
    candidate: &PersonalityVector,
    confidence: &PersonalityVector,
) -> PersonalityVector {
    PersonalityVector {
        baseline_warmth: blend(
            prior.baseline_warmth,
            candidate.baseline_warmth,
            confidence.baseline_warmth,
        ),
        baseline_patience: blend(
            prior.baseline_patience,
            candidate.baseline_patience,
            confidence.baseline_patience,
        ),
        sensitivity: blend(
            prior.sensitivity,
            candidate.sensitivity,
            confidence.sensitivity,
        ),
        irritability: blend(
            prior.irritability,
            candidate.irritability,
            confidence.irritability,
        ),
        composure: blend(prior.composure, candidate.composure, confidence.composure),
        epistemic_pride: blend(
            prior.epistemic_pride,
            candidate.epistemic_pride,
            confidence.epistemic_pride,
        ),
        epistemic_openness: blend(
            prior.epistemic_openness,
            candidate.epistemic_openness,
            confidence.epistemic_openness,
        ),
        boundary_strength: blend(
            prior.boundary_strength,
            candidate.boundary_strength,
            confidence.boundary_strength,
        ),
        forgiveness: blend(
            prior.forgiveness,
            candidate.forgiveness,
            confidence.forgiveness,
        ),
        attachment_propensity: blend(
            prior.attachment_propensity,
            candidate.attachment_propensity,
            confidence.attachment_propensity,
        ),
        expression_drive: blend(
            prior.expression_drive,
            candidate.expression_drive,
            confidence.expression_drive,
        ),
        curiosity: blend(prior.curiosity, candidate.curiosity, confidence.curiosity),
    }
}

fn clamp_expression(value: &ExpressionPhenotype) -> ExpressionPhenotype {
    ExpressionPhenotype {
        warmth: value.warmth.clamp(Fixed::ZERO, Fixed::ONE),
        directness: value.directness.clamp(Fixed::ZERO, Fixed::ONE),
        verbosity: value.verbosity.clamp(Fixed::ZERO, Fixed::ONE),
        self_disclosure: value.self_disclosure.clamp(Fixed::ZERO, Fixed::ONE),
        humor: value.humor.clamp(Fixed::ZERO, Fixed::ONE),
        formality: value.formality.clamp(Fixed::ZERO, Fixed::ONE),
    }
}

fn clamp_allostasis(value: &AllostaticSetpoints) -> AllostaticSetpoints {
    AllostaticSetpoints {
        energy: value.energy.clamp(Fixed::ZERO, Fixed::ONE),
        arousal: value.arousal.clamp(Fixed::ZERO, Fixed::ONE),
        contact_need: value.contact_need.clamp(Fixed::ZERO, Fixed::ONE),
        quiet_need: value.quiet_need.clamp(Fixed::ZERO, Fixed::ONE),
        expression_pressure: value.expression_pressure.clamp(Fixed::ZERO, Fixed::ONE),
        exploration_drive: value.exploration_drive.clamp(Fixed::ZERO, Fixed::ONE),
    }
}

fn clamp_epistemic(value: &EpistemicPriors) -> EpistemicPriors {
    EpistemicPriors {
        verification_drive: value.verification_drive.clamp(Fixed::ZERO, Fixed::ONE),
        confidence_style: value.confidence_style.clamp(Fixed::ZERO, Fixed::ONE),
        correction_defensiveness: value
            .correction_defensiveness
            .clamp(Fixed::ZERO, Fixed::ONE),
        repair_after_error: value.repair_after_error.clamp(Fixed::ZERO, Fixed::ONE),
    }
}

fn clamp_social(value: &SocialPriors) -> SocialPriors {
    SocialPriors {
        stranger_distance: value.stranger_distance.clamp(Fixed::ZERO, Fixed::ONE),
        approach_threshold: value.approach_threshold.clamp(Fixed::ZERO, Fixed::ONE),
        rejection_sensitivity: value.rejection_sensitivity.clamp(Fixed::ZERO, Fixed::ONE),
        reciprocity_expectation: value.reciprocity_expectation.clamp(Fixed::ZERO, Fixed::ONE),
    }
}

/// Project an untrusted LLM proposal into one canonical, formula-independent
/// GenesisManifest. The same effective Persona source and accepted proposal
/// must produce the same Manifest regardless of machine, Formula revision or
/// incarnation nonce.
pub fn project_manifest(
    request: &PersonaGenesisRequest,
    prior: &GenesisPrior,
) -> Result<GenesisManifest, GenesisError> {
    let proposal = &request.proposal;
    if proposal.source != request.source {
        return Err(GenesisError::SourceMismatch);
    }

    let mut manifest = GenesisManifest {
        schema_version: 1,
        traits: project_personality(
            &prior.personality,
            &proposal.traits,
            &proposal.trait_confidence,
        ),
        expression: clamp_expression(&proposal.expression),
        allostasis: clamp_allostasis(&proposal.allostasis),
        epistemic: clamp_epistemic(&proposal.epistemic),
        social: clamp_social(&proposal.social),
        manifest_digest: [0; 32],
    };
    manifest.manifest_digest = wire::manifest_body_digest(&manifest);
    Ok(manifest)
}

/// Canonical binary encoding of the manifest body. This is the only codec
/// allowed to participate in content identity; JSON never does.
pub fn canonical_manifest_body(manifest: &GenesisManifest) -> Vec<u8> {
    wire::encode_manifest_body(manifest)
}

/// SeedCode is a content identity for the Manifest, not a brain instance and
/// not an authorization capability.
pub fn derive_seed_code_digest(manifest_digest: &Digest) -> Digest {
    hash_domain(b"ae.seed-code.v1", &[manifest_digest])
}

/// One Manifest may be instantiated more than once. Formula, nonce and parent
/// lineage therefore belong to IncarnationId rather than SeedCode.
pub fn derive_incarnation_id(request: &PersonaGenesisRequest, seed_code_digest: &Digest) -> Digest {
    let parent = request.parent_incarnation_id.unwrap_or([0; 32]);
    hash_domain(
        b"ae.incarnation.v1",
        &[
            seed_code_digest,
            &request.formula_digest,
            &request.incarnation_nonce,
            &request.source.scope.bot_token,
            &request.source.scope.persona_token,
            &parent,
        ],
    )
}

/// DevelopmentSeed drives microstructure differences inside the Manifest's
/// allowed envelope. It must never change macro personality targets, safety
/// bounds or initial relation values.
pub fn derive_development_seed(
    seed_code_digest: &Digest,
    incarnation_id: &Digest,
    formula_digest: &Digest,
) -> Digest {
    hash_domain(
        b"ae.neural-development.v1",
        &[seed_code_digest, incarnation_id, formula_digest],
    )
}

/// Project the proposal and derive every identity in one deterministic step.
pub fn derive_identity(
    request: &PersonaGenesisRequest,
    prior: &GenesisPrior,
) -> Result<GenesisIdentity, GenesisError> {
    let manifest = project_manifest(request, prior)?;
    let manifest_digest = manifest.manifest_digest;
    let seed_code_digest = derive_seed_code_digest(&manifest_digest);
    let incarnation_id = derive_incarnation_id(request, &seed_code_digest);
    let development_seed_digest =
        derive_development_seed(&seed_code_digest, &incarnation_id, &request.formula_digest);
    Ok(GenesisIdentity {
        manifest,
        manifest_digest,
        seed_code_digest,
        incarnation_id,
        development_seed_digest,
    })
}

/// Stable lease key for the Genesis singleflight protocol: one durable writer
/// per (Bot, Persona, persona source digest, Formula).
pub fn genesis_scope_key(
    bot_token: &[u8; 16],
    persona_token: &[u8; 16],
    source_digest: &Digest,
    formula_digest: &Digest,
) -> Digest {
    hash_domain(
        b"ae.genesis.lease-key.v1",
        &[bot_token, persona_token, source_digest, formula_digest],
    )
}

// ------------------------------------------------------------------ formatting

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

fn crockford_base32(digest: &Digest) -> String {
    let mut encoded = String::with_capacity(52);
    let mut accumulator: u32 = 0;
    let mut bits: u8 = 0;
    for byte in digest {
        accumulator = (accumulator << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((accumulator >> bits) & 0x1f) as usize;
            encoded.push(char::from(ALPHABET[index]));
            if bits == 0 {
                accumulator = 0;
            } else {
                accumulator &= (1u32 << bits) - 1;
            }
        }
    }
    if bits > 0 {
        let index = ((accumulator << (5 - bits)) & 0x1f) as usize;
        encoded.push(char::from(ALPHABET[index]));
    }
    encoded
}

fn format_digest(prefix: &str, digest: &Digest) -> String {
    let encoded = crockford_base32(digest);
    let mut result = String::with_capacity(prefix.len() + encoded.len() + encoded.len() / 4);
    result.push_str(prefix);
    for (index, character) in encoded.chars().enumerate() {
        if index > 0 && index % 4 == 0 {
            result.push('-');
        }
        result.push(character);
    }
    result
}

pub fn format_seed_code(digest: &Digest) -> String {
    format_digest("AE-S1-", digest)
}

pub fn format_incarnation_id(digest: &Digest) -> String {
    format_digest("AE-I1-", digest)
}

/// Ordinary UI shows a short fingerprint only; full codes are for
/// administrators or explicit export.
pub fn short_fingerprint(digest: &Digest) -> String {
    // 100 display bits per genesis-identity-v1: 20 Crockford characters.
    crockford_base32(digest)[..20].to_string()
}

pub fn format_short_seed_code(digest: &Digest) -> String {
    format!("AE-S1-{}", short_fingerprint(digest))
}

// ------------------------------------------------------------------ capsule

/// Build a verified GenesisCapsule from a canonical Manifest. The capsule is
/// the minimal portable payload; a SeedCode string alone is not portable.
pub fn build_capsule(manifest: &GenesisManifest, provenance_digest: &Digest) -> GenesisCapsule {
    let manifest_digest = wire::manifest_body_digest(manifest);
    let seed_code_digest = derive_seed_code_digest(&manifest_digest);
    let mut capsule = GenesisCapsule {
        schema_version: 1,
        seed_code_digest,
        manifest: manifest.clone(),
        provenance_digest: *provenance_digest,
        capsule_digest: [0; 32],
    };
    capsule.capsule_digest = wire::capsule_digest(&capsule);
    capsule
}

/// Verify a capsule end-to-end: closed schema, byte boundaries, re-encode,
/// manifest digest, SeedCode digest, capsule digest. Any single-bit change in
/// any covered field must fail.
pub fn verify_capsule(capsule: &GenesisCapsule) -> Result<(), GenesisError> {
    if capsule.schema_version != 1 {
        return Err(GenesisError::CapsuleInvalid("unsupported capsule schema"));
    }
    let body = wire::encode_capsule_body(capsule);
    if body.len() != 2 + 32 + wire::MANIFEST_BODY_LEN + 64 {
        return Err(GenesisError::CapsuleInvalid("capsule byte boundary"));
    }
    let recomputed_manifest_digest = wire::manifest_body_digest(&capsule.manifest);
    if capsule.manifest.manifest_digest != recomputed_manifest_digest {
        return Err(GenesisError::CapsuleInvalid("manifest digest mismatch"));
    }
    let recomputed_seed = derive_seed_code_digest(&recomputed_manifest_digest);
    if capsule.seed_code_digest != recomputed_seed {
        return Err(GenesisError::CapsuleInvalid("seed code digest mismatch"));
    }
    let recomputed_capsule = wire::capsule_digest(capsule);
    if capsule.capsule_digest != recomputed_capsule {
        return Err(GenesisError::CapsuleInvalid("capsule digest mismatch"));
    }
    Ok(())
}

/// Explicit fail-closed entry point for the SeedCode-only import attempt.
/// A SeedCode is a stable content fingerprint and cannot reconstruct a
/// Manifest, authorize a birth or clone a brain.
pub fn reject_seed_code_only_import() -> Result<(), GenesisError> {
    Err(GenesisError::SeedCodeOnlyImport)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        hex, GenesisManifestProposal, PersonaScopeRef, PersonaSelectionKind, PersonaSourceRef,
    };

    fn request(seed: u8) -> PersonaGenesisRequest {
        let scope = PersonaScopeRef {
            bot_token: [seed; 16],
            persona_token: [seed.wrapping_add(1); 16],
        };
        let source = PersonaSourceRef {
            scope,
            source_digest: [seed.wrapping_add(2); 32],
            capability_digest: [seed.wrapping_add(3); 32],
            selection: PersonaSelectionKind::Conversation,
            prompt_chars: 100,
            begin_dialog_count: 1,
            mood_dialog_count: 0,
        };
        let proposal = GenesisManifestProposal {
            schema_version: 1,
            source: source.clone(),
            traits: PersonalityVector {
                baseline_warmth: Fixed::from_raw(700_000),
                ..PersonalityVector::default()
            },
            trait_confidence: PersonalityVector {
                baseline_warmth: Fixed::from_raw(500_000),
                ..PersonalityVector::default()
            },
            ..GenesisManifestProposal {
                schema_version: 1,
                source: source.clone(),
                traits: PersonalityVector::default(),
                trait_confidence: PersonalityVector::default(),
                expression: ExpressionPhenotype::default(),
                allostasis: AllostaticSetpoints::default(),
                epistemic: EpistemicPriors::default(),
                social: SocialPriors::default(),
                compiler_protocol_digest: [0; 32],
                compiler_model_digest: [0; 32],
            }
        };
        PersonaGenesisRequest {
            source,
            proposal,
            formula_digest: [seed.wrapping_add(4); 32],
            incarnation_nonce: [seed.wrapping_add(5); 32],
            parent_incarnation_id: None,
            observed_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn same_inputs_give_same_identity() {
        let left = derive_identity(&request(7), &GenesisPrior::default()).unwrap();
        let right = derive_identity(&request(7), &GenesisPrior::default()).unwrap();
        assert_eq!(left, right);
    }

    #[test]
    fn identity_survives_cross_process_style_reconstruction() {
        // Determinism must hold when the same bytes are rebuilt from scratch
        // (as a second process would after reading a file).
        let request = request(7);
        let first = derive_identity(&request, &GenesisPrior::default()).unwrap();
        let json = serde_json::to_string(&request).unwrap();
        let rebuilt: PersonaGenesisRequest = serde_json::from_str(&json).unwrap();
        let second = derive_identity(&rebuilt, &GenesisPrior::default()).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn every_semantic_field_changes_the_manifest_digest() {
        let request = request(1);
        let base = project_manifest(&request, &GenesisPrior::default()).unwrap();
        let mut proposal = request.proposal.clone();
        proposal.traits.curiosity = Fixed::from_raw(810_000);
        proposal.trait_confidence.curiosity = Fixed::ONE;
        let mut modified = request.clone();
        modified.proposal = proposal;
        let changed = project_manifest(&modified, &GenesisPrior::default()).unwrap();
        assert_ne!(base.manifest_digest, changed.manifest_digest);
    }

    #[test]
    fn scope_provenance_and_time_do_not_change_seed_code() {
        // Same phenotype data, different persona binding/compiler/time:
        let mut other = request(1);
        other.source.scope.bot_token = [99; 16];
        other.proposal.source.scope.bot_token = [99; 16];
        other.proposal.compiler_model_digest = [88; 32];
        other.observed_at_ms = 9;
        let a = derive_identity(&request(1), &GenesisPrior::default()).unwrap();
        let b = derive_identity(&other, &GenesisPrior::default()).unwrap();
        assert_eq!(a.seed_code_digest, b.seed_code_digest);
        // ...but the incarnation differs (different Bot binding).
        assert_ne!(a.incarnation_id, b.incarnation_id);
    }

    #[test]
    fn formula_and_nonce_change_incarnation_not_seed() {
        let mut other = request(1);
        other.formula_digest = [77; 32];
        let a = derive_identity(&request(1), &GenesisPrior::default()).unwrap();
        let b = derive_identity(&other, &GenesisPrior::default()).unwrap();
        assert_eq!(a.seed_code_digest, b.seed_code_digest);
        assert_ne!(a.incarnation_id, b.incarnation_id);

        let mut nonce_variant = request(1);
        nonce_variant.incarnation_nonce = [66; 32];
        let c = derive_identity(&nonce_variant, &GenesisPrior::default()).unwrap();
        assert_eq!(a.seed_code_digest, c.seed_code_digest);
        assert_ne!(a.incarnation_id, c.incarnation_id);
    }

    #[test]
    fn source_mismatch_is_rejected() {
        let mut request = request(1);
        request.proposal.source.source_digest = [123; 32];
        assert_eq!(
            project_manifest(&request, &GenesisPrior::default()).unwrap_err(),
            GenesisError::SourceMismatch
        );
    }

    #[test]
    fn formatted_codes_are_stable_and_prefixed() {
        let identity = derive_identity(&request(3), &GenesisPrior::default()).unwrap();
        let seed = format_seed_code(&identity.seed_code_digest);
        let incarnation = format_incarnation_id(&identity.incarnation_id);
        assert!(seed.starts_with("AE-S1-"));
        assert!(incarnation.starts_with("AE-I1-"));
        assert_eq!(seed.len(), "AE-S1-".len() + 52 + 12);
        assert_eq!(
            format_short_seed_code(&identity.seed_code_digest).len(),
            "AE-S1-".len() + 20
        );
        assert_eq!(seed, format_seed_code(&identity.seed_code_digest));
    }

    #[test]
    fn seed_codes_differ_across_manifests() {
        let a = derive_identity(&request(1), &GenesisPrior::default()).unwrap();
        let mut other = request(1);
        other.proposal.traits.curiosity = Fixed::from_raw(810_000);
        other.proposal.trait_confidence.curiosity = Fixed::ONE;
        let b = derive_identity(&other, &GenesisPrior::default()).unwrap();
        assert_ne!(a.seed_code_digest, b.seed_code_digest);
        assert_ne!(
            format_seed_code(&a.seed_code_digest),
            format_seed_code(&b.seed_code_digest)
        );
    }

    #[test]
    fn capsule_round_trip_and_tamper_rejection() {
        let identity = derive_identity(&request(1), &GenesisPrior::default()).unwrap();
        let capsule = build_capsule(&identity.manifest, &[5; 32]);
        verify_capsule(&capsule).unwrap();
        assert_eq!(capsule.seed_code_digest, identity.seed_code_digest);

        // Single-bit change in the manifest body must fail.
        let mut tampered = capsule.clone();
        tampered.manifest.traits.baseline_warmth =
            Fixed::from_raw(tampered.manifest.traits.baseline_warmth.raw() ^ 1);
        assert_eq!(
            verify_capsule(&tampered).unwrap_err(),
            GenesisError::CapsuleInvalid("manifest digest mismatch")
        );

        // Tampered seed field must fail.
        let mut bad_seed = capsule.clone();
        bad_seed.seed_code_digest[0] ^= 1;
        assert_eq!(
            verify_capsule(&bad_seed).unwrap_err(),
            GenesisError::CapsuleInvalid("seed code digest mismatch")
        );

        // Tampered capsule digest must fail.
        let mut bad_capsule = capsule.clone();
        bad_capsule.capsule_digest[0] ^= 1;
        assert_eq!(
            verify_capsule(&bad_capsule).unwrap_err(),
            GenesisError::CapsuleInvalid("capsule digest mismatch")
        );
    }

    #[test]
    fn seed_code_only_import_is_rejected() {
        assert_eq!(
            reject_seed_code_only_import().unwrap_err(),
            GenesisError::SeedCodeOnlyImport
        );
    }

    #[test]
    fn manifest_body_golden_all_zero() {
        // Golden vector: all-zero phenotype, schema 1.
        // Bytes: u16 schema (01 00) + 32 zero i64 values (256 bytes).
        let manifest = GenesisManifest {
            schema_version: 1,
            traits: PersonalityVector::default(),
            expression: ExpressionPhenotype::default(),
            allostasis: AllostaticSetpoints::default(),
            epistemic: EpistemicPriors::default(),
            social: SocialPriors::default(),
            manifest_digest: [0; 32],
        };
        let bytes = canonical_manifest_body(&manifest);
        assert_eq!(bytes.len(), 258);
        assert_eq!(&bytes[..2], &[0x01, 0x00]);
        assert!(bytes[2..].iter().all(|byte| *byte == 0));
        let digest = wire::manifest_body_digest(&manifest);
        assert_eq!(
            hex::encode32(&digest),
            "5c37f92a4d6da11ecbba7143b0f9df8c8dea4530b029e9f0ca38bb329e213a27"
        );
        let seed = derive_seed_code_digest(&digest);
        assert_eq!(
            format_seed_code(&seed),
            "AE-S1-0KJQ-WGPX-M1J1-4JWH-SCSV-F0SM-BQ7S-578R-D4C1-W8MM-K7Q9-WCWG-N6Q0"
        );
    }
}
