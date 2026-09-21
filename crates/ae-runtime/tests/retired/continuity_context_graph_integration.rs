use ae_contracts::{
    AllostaticSetpoints, CanonicalEvent, CausalRef, EpistemicPriors, EvidenceVector,
    ExpressionPhenotype, GenesisManifestProposal, PersonaGenesisRequest, PersonaScopeRef,
    PersonaSelectionKind, PersonaSourceRef, PersonalityVector, ScopeRef, SemanticEstimate,
    SocialPriors, UserStimulus,
};
use ae_fixed::Fixed;
use ae_runtime::AstrRuntime;

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
        prompt_chars: 10,
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
        expression: ExpressionPhenotype::default(),
        allostasis: AllostaticSetpoints::default(),
        epistemic: EpistemicPriors::default(),
        social: SocialPriors::default(),
        compiler_protocol_digest: [seed.wrapping_add(4); 32],
        compiler_model_digest: [seed.wrapping_add(5); 32],
    };
    PersonaGenesisRequest {
        source,
        proposal,
        formula_digest: [seed.wrapping_add(6); 32],
        incarnation_nonce: [seed.wrapping_add(7); 32],
        parent_incarnation_id: None,
        observed_at_ms: 1_700_000_000_000,
    }
}

fn scope(seed: u8) -> ScopeRef {
    ScopeRef {
        bot_token: [seed; 16],
        persona_token: [seed.wrapping_add(1); 16],
        relation_token: Some([seed.wrapping_add(8); 16]),
        session_token: [seed.wrapping_add(9); 16],
    }
}

fn stimulus(seed: u8, revision: u64) -> CanonicalEvent {
    CanonicalEvent::UserStimulus(UserStimulus {
        event_id: [seed.wrapping_add(10); 16],
        scope: scope(seed),
        causal: CausalRef {
            turn_id: [seed.wrapping_add(11); 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: revision,
        },
        observed_at_ms: 1_700_000_000_100,
        evidence: SemanticEstimate {
            schema_version: 1,
            dimensions: EvidenceVector {
                positive: Fixed::from_raw(400_000),
                engagement: Fixed::from_raw(600_000),
                ..EvidenceVector::default()
            },
            estimator_confidence: Fixed::from_raw(700_000),
            estimator_digest: [seed.wrapping_add(12); 32],
        },
    })
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "ae-runtime-context-{name}-{}-{}",
        std::process::id(),
        std::thread::current().name().unwrap_or("integration")
    ));
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn committed_turn_projects_aggregate_context_and_deduplicates_after_reopen() {
    let directory = temp_dir("reopen");
    let path = directory.join("store.db");
    let request = request(21);
    let scope = scope(21);
    let event = stimulus(21, 0);

    let mut first_runtime = AstrRuntime::open(&path).unwrap();
    first_runtime.ensure_genesis(&request).unwrap();
    let first = first_runtime.apply_event(&scope, &event).unwrap();

    assert_eq!(first.revision, 1);
    assert_eq!(first.context_summary.source_continuum_revision, 1);
    assert_eq!(first.context_summary.summary_revision, 1);
    let summary_digest = first.context_summary.summary_digest;
    drop(first_runtime);

    let mut reopened = AstrRuntime::open(&path).unwrap();
    let duplicate = reopened.apply_event(&scope, &event).unwrap();

    assert!(duplicate.deduplicated);
    assert_eq!(duplicate.revision, 1);
    assert_eq!(duplicate.context_summary.source_continuum_revision, 1);
    assert_eq!(duplicate.context_summary.summary_digest, summary_digest);
    assert_eq!(
        reopened
            .context_summary_for_scope(&scope)
            .unwrap()
            .unwrap()
            .summary_digest,
        summary_digest
    );

    drop(reopened);
    std::fs::remove_dir_all(&directory).unwrap();
}
