use ae_contracts::{
    NodeObservabilityComponentV1, NodeObservabilityContractInfoV1, NodeObservabilityCountsV1,
    NodeObservabilityProjectionWireV2, NodeObservabilityRegionV1, NodeObservabilityResidualStateV1,
    NodeObservabilityResidualsV1,
};

fn component() -> NodeObservabilityComponentV1 {
    NodeObservabilityComponentV1 {
        before_mean_fxp6: 0,
        after_mean_fxp6: 0,
        delta_mean_fxp6: 0,
        changed_node_count: 0,
        nonzero_after_count: 0,
    }
}

#[test]
fn node_observability_contract_accepts_nonactivation_component_change_and_rejects_tampering() {
    let contract = NodeObservabilityContractInfoV1::native_v1();
    assert!(contract.validate());
    assert_eq!(
        contract.schema,
        "astr-embodiment.node-observability-contract-info.v1"
    );
    assert_eq!(
        contract.contract_id,
        "astr-embodiment.node-observability-contract.v2"
    );
    assert_eq!(
        contract.node_observability_schema,
        "astr-embodiment.node-observability.v2"
    );

    let projection = NodeObservabilityProjectionWireV2::new(
        7,
        4,
        NodeObservabilityCountsV1 {
            selected_node_count: 1,
            activated_node_count: 0,
            changed_node_count: 1,
            potential_nonzero_after_count: 0,
            excitation_nonzero_after_count: 0,
            signal_nonzero_after_count: 0,
        },
        NodeObservabilityResidualsV1 {
            state: NodeObservabilityResidualStateV1::NotComputed,
            formula: None,
            values_fxp6: None,
        },
        vec![NodeObservabilityRegionV1 {
            region_id: 0,
            region_name: "test_region".to_owned(),
            node_capacity: 4,
            selected_node_count: 1,
            activated_node_count: 0,
            changed_node_count: 1,
            potential: component(),
            excitation: component(),
        }],
    );

    assert!(projection.validate());

    let mut wrong_contract = projection.clone();
    wrong_contract.contract_id.push('0');
    assert!(!wrong_contract.validate());

    let mut wrong_relation = projection.clone();
    wrong_relation.counts.selected_node_count = 0;
    assert!(!wrong_relation.validate());

    let mut unknown_field = serde_json::to_value(&projection).expect("projection serializes");
    unknown_field["untrusted"] = serde_json::json!(true);
    assert!(serde_json::from_value::<NodeObservabilityProjectionWireV2>(unknown_field).is_err());
}
