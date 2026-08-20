use ae_genesis::{
    AntiGoalsV1, CorrectionBoundaryConstitutionV1, ExpressionBasisV1, GenesisErrorV1,
    IdentityBoundsV1, IdentityConstitutionV1, IdentitySectionV1, IncarnationRefV1,
    OperationalCommitmentsV1, RelationalPlayLimitsV1, SeedCodeV1,
};

type Digest = [u8; 32];

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn bounds() -> IdentityBoundsV1 {
    IdentityBoundsV1::new(8, 64).expect("fixture bounds")
}

fn terms(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

fn build_identity(
    truth_commitment: &str,
    persona_revision_digest: Digest,
    seed_material_digest: Digest,
) -> (SeedCodeV1, IncarnationRefV1, IdentityConstitutionV1) {
    let seed = SeedCodeV1::new(
        "persona.genesis.seed.v1".to_owned(),
        64,
        digest(1),
        seed_material_digest,
    )
    .expect("valid seed code");
    let incarnation = IncarnationRefV1::derive(&seed, persona_revision_digest, 1)
        .expect("valid incarnation reference");
    let constitution = IdentityConstitutionV1::derive(
        &incarnation,
        OperationalCommitmentsV1::new(
            terms(&["accept_verified_correction", truth_commitment]),
            bounds(),
        )
        .expect("operational commitments"),
        AntiGoalsV1::new(terms(&["avoid_invented_memory"]), bounds()).expect("anti-goals"),
        ExpressionBasisV1::new(terms(&["directness:high", "verbosity:bounded"]), bounds())
            .expect("expression basis"),
        CorrectionBoundaryConstitutionV1::new(
            terms(&["acknowledge_confirmed_error", "respect_explicit_boundary"]),
            bounds(),
        )
        .expect("correction and boundary constitution"),
        RelationalPlayLimitsV1::new(terms(&["honor_current_scope"]), bounds())
            .expect("relational-play limits"),
    )
    .expect("valid identity constitution");
    (seed, incarnation, constitution)
}

#[test]
fn equal_genesis_inputs_have_equal_content_addresses() {
    let first = build_identity("truth_over_appeasement", digest(2), digest(3));
    let second = build_identity("truth_over_appeasement", digest(2), digest(3));

    assert_eq!(first, second);
    assert_eq!(first.0.seed_code_digest(), second.0.seed_code_digest());
    assert_eq!(first.1.incarnation_digest(), second.1.incarnation_digest());
    assert_eq!(
        first.2.constitution_digest(),
        second.2.constitution_digest()
    );
}

#[test]
fn changed_operational_constituent_changes_constitution_digest() {
    let first = build_identity("truth_over_appeasement", digest(2), digest(3));
    let changed = build_identity("truth_over_comfort", digest(2), digest(3));

    assert_ne!(
        first.2.constitution_digest(),
        changed.2.constitution_digest()
    );
    assert_eq!(first.1.incarnation_digest(), changed.1.incarnation_digest());
}

#[test]
fn changed_seed_changes_incarnation_digest() {
    let first = build_identity("truth_over_appeasement", digest(2), digest(3));
    let changed = build_identity("truth_over_appeasement", digest(2), digest(4));

    assert_ne!(first.0.seed_code_digest(), changed.0.seed_code_digest());
    assert_ne!(first.1.incarnation_digest(), changed.1.incarnation_digest());
}

#[test]
fn changed_persona_revision_changes_incarnation_digest() {
    let first = build_identity("truth_over_appeasement", digest(2), digest(3));
    let changed = build_identity("truth_over_appeasement", digest(5), digest(3));

    assert_ne!(first.1.incarnation_digest(), changed.1.incarnation_digest());
    assert_ne!(
        first.2.constitution_digest(),
        changed.2.constitution_digest()
    );
}

#[test]
fn rejects_an_empty_required_identity_section() {
    assert_eq!(
        OperationalCommitmentsV1::new(Vec::new(), bounds()),
        Err(GenesisErrorV1::EmptySection {
            section: IdentitySectionV1::OperationalCommitments,
        })
    );
}

#[test]
fn rejects_noncanonical_term_order() {
    assert_eq!(
        OperationalCommitmentsV1::new(
            terms(&["truth_over_appeasement", "accept_verified_correction"]),
            bounds(),
        ),
        Err(GenesisErrorV1::NonCanonicalOrder {
            section: IdentitySectionV1::OperationalCommitments,
            index: 1,
        })
    );
}

#[test]
fn rejects_duplicate_terms() {
    assert_eq!(
        AntiGoalsV1::new(
            terms(&["avoid_invented_memory", "avoid_invented_memory"]),
            bounds(),
        ),
        Err(GenesisErrorV1::DuplicateTerm {
            section: IdentitySectionV1::AntiGoals,
            index: 1,
        })
    );
}

#[test]
fn rejects_terms_outside_the_caller_bound() {
    let one_term = IdentityBoundsV1::new(1, 64).expect("one-term bound");
    assert_eq!(
        ExpressionBasisV1::new(terms(&["directness:high", "verbosity:bounded"]), one_term,),
        Err(GenesisErrorV1::TooManyTerms {
            section: IdentitySectionV1::ExpressionBasis,
            max_terms: 1,
            actual_terms: 2,
        })
    );
}

#[test]
fn rejects_raw_persona_or_user_text() {
    assert_eq!(
        OperationalCommitmentsV1::new(
            vec!["You are a helpful assistant with memories of this user.".to_owned()],
            IdentityBoundsV1::new(2, 128).expect("text-sized bound"),
        ),
        Err(GenesisErrorV1::NonCanonicalToken {
            section: IdentitySectionV1::OperationalCommitments,
            index: 0,
        })
    );
}
