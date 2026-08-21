//! External regression for the retired public R7 authority path. The crate
//! doctest proves imports fail; this source assertion prevents an accidental
//! `pub mod r7` restoration from being hidden by internal test compilation.

#[test]
fn runtime_r7_namespace_remains_private() {
    let root = include_str!("../src/lib.rs");
    assert!(root.contains("mod r7;"));
    assert!(!root.contains("pub mod r7;"));
}

#[test]
fn missing_public_material_stays_g0_only_without_a_store_row() {
    use ae_runtime::{AstrRuntime, R7HydrationOutcomeV1};
    use ae_store::R7PolicyBindingKeyV1;

    let root = std::path::PathBuf::from(
        std::env::var("AE_RC1_TASK_TEMP").expect("test runner supplies the G-drive task root"),
    );
    let path = root.join(format!("runtime-g0-only-{}.db", std::process::id()));
    let mut runtime = AstrRuntime::open(&path).expect("open isolated store");
    let key = R7PolicyBindingKeyV1 {
        bot_token: [1; 16],
        persona_token: [2; 16],
        committed_g0_incarnation_id: [3; 32],
        identity_scope_id: 1,
    };
    assert_eq!(
        runtime.hydrate_r7_public_policy(&key, None).unwrap(),
        R7HydrationOutcomeV1::G0Only
    );
    runtime.flush_and_close().unwrap();
    let _ = std::fs::remove_file(path);
}

#[test]
fn missing_validation_context_stays_g0_only() {
    use ae_runtime::{AstrRuntime, R7HydrationOutcomeV1};
    use ae_store::{R7PolicyBindingKeyV1, R7PolicyValidationContextV1};

    let root = std::path::PathBuf::from(
        std::env::var("AE_RC1_TASK_TEMP").expect("test runner supplies the G-drive task root"),
    );
    let path = root.join(format!("runtime-context-g0-only-{}.db", std::process::id()));
    let mut runtime = AstrRuntime::open(&path).expect("open isolated store");
    let key = R7PolicyBindingKeyV1 {
        bot_token: [4; 16],
        persona_token: [5; 16],
        committed_g0_incarnation_id: [6; 32],
        identity_scope_id: 1,
    };
    let context = R7PolicyValidationContextV1 {
        native_source_identity_digest: [7; 32],
        plugin_source_identity_digest: [8; 32],
        control_evidence_set_digest: [9; 32],
        g0_binding_contract_digest: [10; 32],
        g0_only_fallback_contract_digest: [11; 32],
        committed_g0_incarnation_id: key.committed_g0_incarnation_id,
        committed_g0_manifest_digest: [12; 32],
        committed_g0_seed_code_digest: [13; 32],
        committed_g0_persona_source_digest: [14; 32],
        committed_g0_genesis_receipt_digest: [15; 32],
    };
    assert_eq!(
        runtime
            .hydrate_r7_public_policy_with_context(&key, None, Some(&context))
            .unwrap(),
        R7HydrationOutcomeV1::G0Only
    );
    runtime.flush_and_close().unwrap();
    let _ = std::fs::remove_file(path);
}
