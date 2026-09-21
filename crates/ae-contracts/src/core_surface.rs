//! Closed compatibility classifier. It never decodes request bodies or opens storage.
pub const RETIRED_OPERATION_NAME_MANIFEST_V1: &[&str] = &[
    "_autonomy_call", "alpha3_call", "apply_event", "apply_interaction",
    "apply_interaction_v1", "autonomy_status", "begin_semantic_appraisal_v1",
    "bind_outbound_target", "bootstrap_autonomy", "claim_wake", "claim_wake_v2",
    "gate_and_claim_dispatch", "gate_and_claim_dispatch_v2",
    "gate_and_claim_externalization", "gate_and_claim_externalization_v2",
    "gate_externalization", "host_readiness_witness_digest_v1", "integration_availability_v1",
    "observe_body_snapshot_v1", "observe_budget_summary_v1", "observe_execution_receipt_v1",
    "observe_gate_reasons_v1", "pending_autonomy_work", "record_relation_inbound",
    "recover_autonomy", "scope_digests", "settle_dispatch", "settle_dispatch_v2",
    "settle_externalization", "settle_externalization_v2", "settle_wake", "settle_wake_v2",
    "upsert_budget_policy", "verify_autonomy_projection", "wake_caller_incarnation_v2",
];
pub const RETIRED_EVENT_KIND_MANIFEST_V1: &[(u8, &str, u8)] = &[
    (1,"UserStimulusV1",1), (2,"UserReactionV1",1), (3,"CorrectionClaimV1",1),
    (4,"CorrectionVerdictV1",1), (5,"SelfActionCandidateV1",1), (6,"DeliveryOutcomeV1",2),
    (7,"SettlementEvidenceV1",1), (8,"TimeAdvanceV1",3), (9,"AdminActionV1",1),
    (10,"InteractionFactBatchV1",4), (11,"EmbodimentTimeAdvanceRequestV1",5),
];
pub const RETIRED_HOST_IDENTIFIER_MANIFEST_V1: &[&str] = &[
    "_commit_contact_control", "_persist_proactive_migration", "_prepare_proactive_settings",
    "/ae_contact_end", "/ae_contact_pause", "ae_contact_end", "ae_contact_pause", "ae_wake",
    "AutonomousSupervisor", "contact_end_command", "contact_pause_command",
    "emergency_wake_command", "execute_proactive_intention", "submit_proactive_message",
];
/// Rust bypass suffixes are classified separately from the frozen external wire manifest.
pub const RETIRED_STORE_RUNTIME_SYMBOL_MANIFEST_V1: &[&str] = &[
    "apply_interaction_fact_batch_v1", "handle_alpha3", "ensure_relation_authority_tx",
    "load_relation_contact_v1", "load_relation_consent_v1", "relation_contact_v1",
    "relation_consent_v1", "upsert_relation_policy", "store_outbound_target",
    "load_current_outbound_target", "upsert_autonomy_scope", "list_autonomy_scopes",
    "list_autonomy_scopes_for_persona", "rebuild_autonomy_scope_bindings_from_journal",
    "claim_wake_proposal", "load_wake_claim", "load_wake_claim_v2",
    "load_wake_time_settlement_v1", "commit_autonomous_wake", "commit_autonomous_wake_claim_v1",
    "commit_alpha3_wake", "recover_retired_wake_v2_claims", "write_autonomy_snapshot",
    "read_autonomy_snapshot", "pending_outbound", "autonomy_work_scopes",
    "rebuild_autonomy_projection", "recover_orphaned_dispatches", "recover_orphaned_externalizations",
    "externalization_reserved_tokens", "list_pending_autonomy_work", "list_pending_autonomy_work_for_relation",
    "load_outbound_for_intention", "outbound_submission_metrics", "has_autonomy_journal_delta",
    "relation_budget_ledger_v1", "alpha3_authoritative_fingerprint_v1",
];
pub const CORE_PUBLIC_METHOD_MANIFEST_V1: &[(&str,u16)] = &[
    ("advance_embodiment_time_v1",1), ("build_info_v1",1),
    ("commit_core_delivery_outcome_v1",1), ("commit_core_inbound_v1",1),
    ("compare_and_swap_embodiment_profile_v1",1), ("compile_core_host_request_v1",1),
    ("create_embodiment_persona_if_missing_v1",1), ("embodiment_clock_status_v1",1),
    ("ensure_genesis",1), ("flush_and_close",1), ("get_embodiment_persona_v1",1),
    ("health",1), ("inspect",1), ("list_embodiment_personas_v1",1), ("open",1),
    ("read_embodiment_profile_v1",1), ("settle_semantic_appraisal_v1",1),
    ("verify_replay",1), ("version",1),
];
pub const RETIRED_SURFACE_MANIFEST_V1_SHA256: &str = "6a28d0e925be34a108b636c47b4ea8ff1a69b763d7de14ba91d4a44127dc291a";
pub const CORE_PUBLIC_METHOD_MANIFEST_V1_SHA256: &str = "800e4dccb2a29b6edbaa2ac7cc46c34f466ece682e0ccf1a272577aacb3790c4";

/// Only exact bounded ASCII names are recognized. No allocation or body parsing.
pub fn classify_retired_operation_v1(name: &str) -> &'static str {
    if name.len() <= 64 && name.is_ascii() && (RETIRED_OPERATION_NAME_MANIFEST_V1.contains(&name)
        || RETIRED_STORE_RUNTIME_SYMBOL_MANIFEST_V1.contains(&name)) {
        "UNSUPPORTED_CORE_BOUNDARY"
    } else {
        "UNKNOWN_OPERATION"
    }
}
/// The caller supplies only the fixed envelope kind byte, never an event body.
pub fn classify_retired_event_kind_v1(kind: u8) -> &'static str {
    if RETIRED_EVENT_KIND_MANIFEST_V1.iter().any(|row| row.0 == kind) {
        "UNSUPPORTED_CORE_BOUNDARY"
    } else {
        "UNKNOWN_OPERATION"
    }
}
fn lp16(out: &mut Vec<u8>, value: &str) {
    out.extend(u16::try_from(value.len()).expect("manifest name length").to_le_bytes());
    out.extend(value.as_bytes());
}
pub fn core_public_method_manifest_v1_bytes() -> Vec<u8> {
    assert_eq!(CORE_PUBLIC_METHOD_MANIFEST_V1.len(),19);
    let mut out=19u16.to_le_bytes().to_vec();
    for (name, version) in CORE_PUBLIC_METHOD_MANIFEST_V1 {
        lp16(&mut out,name); out.extend(version.to_le_bytes());
    }
    out
}
pub fn retired_surface_manifest_v1_bytes() -> Vec<u8> {
    assert_eq!(RETIRED_OPERATION_NAME_MANIFEST_V1.len(),35);
    assert_eq!(RETIRED_EVENT_KIND_MANIFEST_V1.len(),11);
    assert_eq!(RETIRED_HOST_IDENTIFIER_MANIFEST_V1.len(),14);
    let mut out=35u16.to_le_bytes().to_vec();
    for name in RETIRED_OPERATION_NAME_MANIFEST_V1 {lp16(&mut out,name);}
    out.extend(11u16.to_le_bytes());
    for (kind,name,route) in RETIRED_EVENT_KIND_MANIFEST_V1 {out.push(*kind);lp16(&mut out,name);out.push(*route);}
    out.extend(14u16.to_le_bytes());
    for name in RETIRED_HOST_IDENTIFIER_MANIFEST_V1 {lp16(&mut out,name);}
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_retired_names_and_kinds_are_bounded_and_closed() {
        for name in RETIRED_OPERATION_NAME_MANIFEST_V1 {
            assert_eq!(classify_retired_operation_v1(name),"UNSUPPORTED_CORE_BOUNDARY");
            assert_eq!(classify_retired_operation_v1(&format!("{name}x")),"UNKNOWN_OPERATION");
        }
        for kind in 0..=255 {
            assert_eq!(classify_retired_event_kind_v1(kind),if (1..=11).contains(&kind) {"UNSUPPORTED_CORE_BOUNDARY"} else {"UNKNOWN_OPERATION"});
        }
        for name in ["", "APPLY_EVENT", "apply_event\0", "apply_évènt", &"x".repeat(65)] {
            assert_eq!(classify_retired_operation_v1(name),"UNKNOWN_OPERATION");
        }
    }
    #[test]
    fn manifest_bytes_match_independent_goldens() {
        use sha2::{Digest,Sha256};
        assert_eq!(crate::hex::encode32(&Sha256::digest(retired_surface_manifest_v1_bytes()).into()),RETIRED_SURFACE_MANIFEST_V1_SHA256);
        assert_eq!(crate::hex::encode32(&Sha256::digest(core_public_method_manifest_v1_bytes()).into()),CORE_PUBLIC_METHOD_MANIFEST_V1_SHA256);
    }
}
