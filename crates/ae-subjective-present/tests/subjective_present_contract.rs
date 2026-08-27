use ae_subjective_present::{
    SubjectivePresentErrorV1, SubjectivePresentProjectionV1, SubjectivePresentV1,
};

fn item_json(axis: &str, band: &str) -> String {
    format!(
        r#"{{"axis":"{axis}","band":"{band}","trend":"stable","behavioral_effect":"be_concise","disclosure":"BEHAVIORAL_ONLY","confidence":"high","cause_ref":null}}"#
    )
}

#[test]
fn canonical_item_matches_the_r7_subjective_item_shape() {
    let item = SubjectivePresentV1::from_json(&item_json("irritation", "moderate"))
        .expect("canonical R7 subjective item");

    assert_eq!(
        item.to_canonical_json(),
        r#"{"axis":"irritation","band":"moderate","trend":"stable","behavioral_effect":"be_concise","disclosure":"BEHAVIORAL_ONLY","confidence":"high","cause_ref":null}"#,
    );
    assert_ne!(item.identity_digest(), &[0; 32]);
}

#[test]
fn rejects_unknown_schema_material_and_free_form_emotion_narratives() {
    let unknown_field = r#"{"axis":"irritation","band":"moderate","trend":"stable","behavioral_effect":"be_concise","disclosure":"BEHAVIORAL_ONLY","confidence":"high","neural_state":[1,2,3]}"#;
    assert!(matches!(
        SubjectivePresentV1::from_json(unknown_field),
        Err(SubjectivePresentErrorV1::InvalidJson { .. })
    ));

    let free_form_axis = item_json("i feel angry because you ignored me", "moderate");
    assert!(matches!(
        SubjectivePresentV1::from_json(&free_form_axis),
        Err(SubjectivePresentErrorV1::NonCanonicalToken { field: "axis" })
    ));

    let raw_digest = r#"{"axis":"irritation","band":"moderate","trend":"stable","behavioral_effect":"be_concise","disclosure":"BEHAVIORAL_ONLY","confidence":"high","projection_digest":[0,0,0]}"#;
    assert!(matches!(
        SubjectivePresentV1::from_json(raw_digest),
        Err(SubjectivePresentErrorV1::InvalidJson { .. })
    ));
}

#[test]
fn projection_rejects_duplicate_unsorted_and_over_capacity_axes() {
    let first =
        SubjectivePresentV1::from_json(&item_json("attention_load", "high")).expect("first item");
    let duplicate = SubjectivePresentV1::from_json(&item_json("attention_load", "low"))
        .expect("duplicate axis item");
    assert!(matches!(
        SubjectivePresentProjectionV1::new(vec![first.clone(), duplicate]),
        Err(SubjectivePresentErrorV1::DuplicateAxis { index: 1 })
    ));

    let later = SubjectivePresentV1::from_json(&item_json("zeta", "low")).expect("later");
    let earlier = SubjectivePresentV1::from_json(&item_json("alpha", "low")).expect("earlier");
    assert!(matches!(
        SubjectivePresentProjectionV1::new(vec![later, earlier]),
        Err(SubjectivePresentErrorV1::NonCanonicalAxisOrder { index: 1 })
    ));

    let items = (0..33)
        .map(|index| {
            SubjectivePresentV1::from_json(&item_json(&format!("axis_{index:02}"), "low"))
                .expect("bounded item")
        })
        .collect();
    assert!(matches!(
        SubjectivePresentProjectionV1::new(items),
        Err(SubjectivePresentErrorV1::TooManyItems {
            max_items: 32,
            actual_items: 33,
        })
    ));
}

#[test]
fn projection_identity_is_deterministic_and_domain_bound() {
    let first =
        SubjectivePresentV1::from_json(&item_json("irritation", "moderate")).expect("first item");
    let second = SubjectivePresentV1::from_json(&item_json("fatigue", "low")).expect("second item");
    let projection = SubjectivePresentProjectionV1::new(vec![second.clone(), first.clone()])
        .expect("canonical projection");
    let same_projection =
        SubjectivePresentProjectionV1::new(vec![second, first]).expect("same canonical projection");

    assert_eq!(
        projection.identity_digest(),
        same_projection.identity_digest()
    );
    assert_ne!(projection.identity_digest(), &[0; 32]);
    assert_eq!(
        projection.to_canonical_json(),
        format!(
            "[{},{}]",
            SubjectivePresentV1::from_json(&item_json("fatigue", "low"))
                .expect("fatigue")
                .to_canonical_json(),
            SubjectivePresentV1::from_json(&item_json("irritation", "moderate"))
                .expect("irritation")
                .to_canonical_json(),
        ),
    );
}
