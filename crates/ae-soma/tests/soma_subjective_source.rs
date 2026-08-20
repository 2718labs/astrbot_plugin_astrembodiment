use ae_soma::{
    compile_subjective_present_v1, BoundedSomaSignalV1, CallerProvidedClassificationV1,
    SomaClassificationIngressV1, SomaErrorV1, SomaFieldSetV1, SomaStateV1, SomaSubjectiveAxisV1,
};
use ae_subjective_present::{ConfidenceV1, DisclosureV1, SubjectiveBandV1, SubjectiveTrendV1};

type Digest = [u8; 32];

const REVISION: u64 = 17;

fn digest(seed: u8) -> Digest {
    [seed; 32]
}

fn signal(id: &str, value: f64) -> BoundedSomaSignalV1 {
    BoundedSomaSignalV1::new(id.to_owned(), 64, value, 0.0, 1.0).expect("bounded SOMA signal")
}

fn field(signals: Vec<BoundedSomaSignalV1>) -> SomaFieldSetV1 {
    SomaFieldSetV1::new(signals, 8).expect("bounded SOMA field")
}

fn state_with_metabolic_value(value: f64) -> SomaStateV1 {
    SomaStateV1::new(
        "soma:snapshot:17".to_owned(),
        128,
        REVISION,
        digest(7),
        field(vec![signal("energy_availability", value)]),
        field(vec![signal("mobilization_balance", 0.6)]),
        field(vec![signal("endocrine_load", 0.2)]),
        field(vec![signal("repair_pressure", 0.3)]),
        field(vec![signal("circadian_phase", 0.4)]),
    )
    .expect("valid SOMA state")
}

fn classification(axis: SomaSubjectiveAxisV1) -> CallerProvidedClassificationV1 {
    CallerProvidedClassificationV1::new(
        axis,
        SubjectiveBandV1::Moderate,
        SubjectiveTrendV1::Stable,
        "prefer_bounded_effort".to_owned(),
        DisclosureV1::BehavioralOnly,
        ConfidenceV1::High,
        Some("soma:snapshot:17".to_owned()),
    )
    .expect("caller-provided classification")
}

fn ingress(
    state: &SomaStateV1,
    revision: u64,
    identity_digest: Digest,
) -> SomaClassificationIngressV1 {
    SomaClassificationIngressV1::new(
        *state.state_digest(),
        revision,
        identity_digest,
        vec![classification(SomaSubjectiveAxisV1::Energy)],
        8,
    )
    .expect("classification ingress")
}

#[test]
fn soma_state_is_deterministic_content_addressed_and_uses_all_five_domains() {
    let first = state_with_metabolic_value(0.7);
    let second = state_with_metabolic_value(0.7);
    let changed = state_with_metabolic_value(0.8);

    assert_eq!(first, second);
    assert_eq!(first.state_digest(), second.state_digest());
    assert_ne!(first.state_digest(), changed.state_digest());
    assert_eq!(first.revision(), REVISION);
    assert_eq!(first.identity_constitution_digest(), &digest(7));
    assert_eq!(first.metabolism().signals().len(), 1);
    assert_eq!(first.autonomic_regulation().signals().len(), 1);
    assert_eq!(first.endocrine_fields().signals().len(), 1);
    assert_eq!(first.immune_repair().signals().len(), 1);
    assert_eq!(first.rhythm().signals().len(), 1);
}

#[test]
fn compiles_only_an_explicit_classification_bound_to_the_state() {
    let state = state_with_metabolic_value(0.7);
    let ingress = ingress(&state, REVISION, digest(7));

    let first = compile_subjective_present_v1(&state, &ingress).expect("valid compilation");
    let second = compile_subjective_present_v1(&state, &ingress).expect("deterministic");

    assert_eq!(first, second);
    assert_eq!(first.items().len(), 1);
    assert_eq!(first.items()[0].axis(), "energy");
    assert_eq!(first.identity_digest(), second.identity_digest());
}

#[test]
fn rejects_nonfinite_invalid_bound_and_out_of_range_signals() {
    assert_eq!(
        BoundedSomaSignalV1::new("energy".to_owned(), 64, f64::NAN, 0.0, 1.0),
        Err(SomaErrorV1::NonFiniteSignal { field: "value" })
    );
    assert_eq!(
        BoundedSomaSignalV1::new("energy".to_owned(), 64, 0.5, 1.0, 1.0),
        Err(SomaErrorV1::InvalidSignalBounds)
    );
    assert_eq!(
        BoundedSomaSignalV1::new("energy".to_owned(), 64, 1.1, 0.0, 1.0),
        Err(SomaErrorV1::SignalOutOfRange)
    );
}

#[test]
fn rejects_duplicate_unsorted_and_over_capacity_signal_sets() {
    assert_eq!(
        SomaFieldSetV1::new(vec![signal("energy", 0.4), signal("energy", 0.5)], 8),
        Err(SomaErrorV1::DuplicateSignal { index: 1 })
    );
    assert_eq!(
        SomaFieldSetV1::new(vec![signal("zeta", 0.4), signal("alpha", 0.5)], 8),
        Err(SomaErrorV1::NonCanonicalSignalOrder { index: 1 })
    );
    assert_eq!(
        SomaFieldSetV1::new(vec![signal("energy", 0.4), signal("fatigue", 0.5)], 1),
        Err(SomaErrorV1::TooManySignals {
            max_items: 1,
            actual_items: 2,
        })
    );
}

#[test]
fn rejects_wrong_state_revision_and_identity_bindings() {
    let state = state_with_metabolic_value(0.7);
    let wrong_state = SomaClassificationIngressV1::new(
        digest(99),
        REVISION,
        digest(7),
        vec![classification(SomaSubjectiveAxisV1::Energy)],
        8,
    )
    .expect("well-formed but wrong state binding");
    assert_eq!(
        compile_subjective_present_v1(&state, &wrong_state),
        Err(SomaErrorV1::StateDigestMismatch)
    );
    assert_eq!(
        compile_subjective_present_v1(&state, &ingress(&state, REVISION + 1, digest(7))),
        Err(SomaErrorV1::RevisionMismatch)
    );
    assert_eq!(
        compile_subjective_present_v1(&state, &ingress(&state, REVISION, digest(8))),
        Err(SomaErrorV1::IdentityBindingMismatch)
    );
}

#[test]
fn rejects_unknown_subjective_states_and_raw_text() {
    assert_eq!(
        SomaSubjectiveAxisV1::parse("contentment"),
        Err(SomaErrorV1::UnknownSubjectiveAxis)
    );
    assert_eq!(
        CallerProvidedClassificationV1::new(
            SomaSubjectiveAxisV1::Fatigue,
            SubjectiveBandV1::Moderate,
            SubjectiveTrendV1::Stable,
            "raw user conversation and invented emotion".to_owned(),
            DisclosureV1::BehavioralOnly,
            ConfidenceV1::High,
            None,
        ),
        Err(SomaErrorV1::InvalidClassification)
    );
    assert_eq!(
        BoundedSomaSignalV1::new("raw neural array".to_owned(), 64, 0.5, 0.0, 1.0),
        Err(SomaErrorV1::NonCanonicalToken { field: "signal_id" })
    );
}
