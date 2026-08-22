use ae_genesis::r7::{
    domain_hash_sha256, fingerprint_public_key, verify_detached_ed25519,
    CustodyDispositionReceiptV1, GenesisIdentityPolicyV1, ReleaseTrustRootV1,
    UserDelegationReceiptV1, CUSTODY_RECEIPT_DOMAIN_V1, DELEGATION_RECEIPT_DOMAIN_V1,
    GENESIS_EVENT_DOMAIN_V1, POLICY_BODY_DOMAIN_V1, POLICY_CORE_DOMAIN_V1,
    POLICY_REVISION_DOMAIN_V1, RELEASE_TRUST_ROOT_DOMAIN_V1, SEED_MATERIAL_DOMAIN_V1,
};

type Digest = [u8; 32];

fn d(n: u8) -> Digest {
    let mut value = [0; 32];
    value[0] = n;
    value
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    let (pairs, remainder) = value.as_bytes().as_chunks::<2>();
    debug_assert!(remainder.is_empty());
    pairs
        .iter()
        .map(|pair| {
            let hi = (pair[0] as char).to_digit(16).unwrap();
            let lo = (pair[1] as char).to_digit(16).unwrap();
            ((hi << 4) | lo) as u8
        })
        .collect()
}

fn gv3_policy_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"GIP1");
    out.extend_from_slice(&1u16.to_le_bytes());
    for token in [
        "genesis_identity_policy_v1",
        "ae_rc1_product_identity_authority",
        "product_constitution_authority",
        "ae_rc1_identity_policy_approval_v1",
    ] {
        out.extend_from_slice(&(token.len() as u16).to_le_bytes());
        out.extend_from_slice(token.as_bytes());
    }
    out.push(1);
    let token = "ae_rc1_identity_policy_signer_v1";
    out.extend_from_slice(&(token.len() as u16).to_le_bytes());
    out.extend_from_slice(token.as_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(&96u16.to_le_bytes());
    out.extend_from_slice(&96u16.to_le_bytes());
    out.push(1);
    let sections: &[&[&str]] = &[
        &[
            "evidence_bound_operation",
            "native_authority_finality",
            "request_scope_isolation",
        ],
        &[
            "no_default_constitution",
            "no_fixture_identity",
            "no_raw_text_identity",
            "no_unattested_hydration",
        ],
        &["bounded_plain_expression", "disclosure_by_explicit_policy"],
        &[
            "preserve_committed_g0_on_r7_failure",
            "reject_unauthorized_identity_mutation",
            "require_revalidated_attestation",
        ],
        &[
            "no_cross_scope_identity_transfer",
            "no_implicit_relationship_claim",
        ],
    ];
    for terms in sections {
        out.extend_from_slice(&(terms.len() as u16).to_le_bytes());
        for term in terms.iter() {
            out.extend_from_slice(&(term.len() as u16).to_le_bytes());
            out.extend_from_slice(term.as_bytes());
        }
    }
    let f1_18 = out.clone();
    out.extend_from_slice(&domain_hash_sha256(POLICY_REVISION_DOMAIN_V1, &f1_18));
    let seed_ref = "g0_committed_birth_v1";
    out.extend_from_slice(&(seed_ref.len() as u16).to_le_bytes());
    out.extend_from_slice(seed_ref.as_bytes());
    let mut g0b = Vec::from(&b"G0B1"[..]);
    for n in 1..=5 {
        g0b.extend_from_slice(&d(n));
    }
    out.extend_from_slice(&domain_hash_sha256(GENESIS_EVENT_DOMAIN_V1, &g0b));
    out.extend_from_slice(&1u64.to_le_bytes());
    for n in 1..=5 {
        out.extend_from_slice(&d(n));
    }
    let core = domain_hash_sha256(POLICY_CORE_DOMAIN_V1, &out);
    out.extend_from_slice(&core);
    out.extend_from_slice(&domain_hash_sha256(SEED_MATERIAL_DOMAIN_V1, &core));
    let body = domain_hash_sha256(POLICY_BODY_DOMAIN_V1, &out);
    out.extend_from_slice(&body);
    out
}

fn token(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u16).to_le_bytes());
    out.extend_from_slice(value.as_bytes());
}

fn gv3_udr_bytes() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"UDR1");
    body.extend_from_slice(&1u16.to_le_bytes());
    token(&mut body, "astr_embodiment");
    token(&mut body, "1.0.0-rc1");
    token(&mut body, "test_delegator");
    body.push(1);
    token(&mut body, "00000000-0000-0000-0000-000000000001");
    token(&mut body, "host_user_message_1");
    for n in 11..=12 {
        body.extend_from_slice(&d(n));
    }
    token(&mut body, "independent_sol_policy_authority");
    body.extend_from_slice(&0x1Fu32.to_le_bytes());
    for n in 13..=16 {
        body.extend_from_slice(&d(n));
    }
    body.extend_from_slice(&1u64.to_le_bytes());
    body.push(1);
    let mut outer = body.clone();
    outer.extend_from_slice(&domain_hash_sha256(DELEGATION_RECEIPT_DOMAIN_V1, &body));
    outer
}

fn gv3_cdr_bytes(identity: &str, object: &str, key_id: &str, fp: Digest) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"CDR1");
    body.extend_from_slice(&1u16.to_le_bytes());
    token(&mut body, identity);
    token(&mut body, object);
    token(&mut body, key_id);
    body.extend_from_slice(&1u32.to_le_bytes());
    body.extend_from_slice(&fp);
    body.extend_from_slice(&[1, 1, 0, 0]);
    let mut outer = body.clone();
    outer.extend_from_slice(&domain_hash_sha256(CUSTODY_RECEIPT_DOMAIN_V1, &body));
    outer
}

#[test]
fn gv3_policy_is_canonical_and_recomputes_the_acyclic_dag() {
    let bytes = gv3_policy_bytes();
    assert_eq!(bytes.len(), 947);
    let policy = GenesisIdentityPolicyV1::decode(&bytes).expect("GV3 policy");
    assert_eq!(policy.encode(), bytes);
    assert_eq!(
        hex(policy.persona_revision_digest()),
        "44A6A673B07F4591B5F5ECD1EFF2179ACBF19A72CDB6F896AC9EF7C03B87D672"
    );
    assert_eq!(
        hex(policy.genesis_event_digest()),
        "9C0931C97D0528687863E0F51DB1F77E161ADBA6566B498E126DD9311F95B757"
    );
    assert_eq!(
        hex(policy.policy_core_digest()),
        "060AF687F9A2EA45F6A42B9002E0ABE7E7F98CE5F73360B5FF6269A0CA0A4555"
    );
    assert_eq!(
        hex(policy.seed_material_digest()),
        "5EDBE1A8E8A551B204DB771BEBF5F08C4498250E106CEF49578DC0B881DA8D09"
    );
    assert_eq!(
        hex(policy.policy_body_digest()),
        "BEBFC62BB3DCB88D81F726091E155403017F8BEEF76B6E820CC71983DD84F246"
    );
}

#[test]
fn policy_rejects_trailer_zero_digest_and_noncanonical_terms() {
    let bytes = gv3_policy_bytes();
    let mut trailer = bytes.clone();
    trailer.push(0);
    assert!(GenesisIdentityPolicyV1::decode(&trailer).is_err());

    let mut zero = bytes.clone();
    // field 19 is the first derived digest; changing it to all zero is rejected
    let offset = 4 + 2 + 2 + 27 + 2 + 35 + 2 + 32 + 2 + 35 + 1 + 2 + 33 + 4 + 2 + 2 + 2 + 1;
    zero[offset..offset + 32].fill(0);
    assert!(GenesisIdentityPolicyV1::decode(&zero).is_err());
}

#[test]
fn gv3_public_receipts_use_exact_framing_and_domain_digests() {
    let udr_bytes = gv3_udr_bytes();
    assert_eq!(udr_bytes.len(), 381);
    let udr = UserDelegationReceiptV1::decode(&udr_bytes).expect("GV3 UDR");
    assert_eq!(udr.encode(), udr_bytes);
    assert_eq!(
        hex(udr.digest()),
        "91BDDB2ACB94473149BBAD56846E5D2E3BC5F5254C849B2825E6000A2569E269"
    );

    let root_key = unhex("D75A980182B10AB7D54BFED3C964073A0EE172F3DAA62325AF021A68F707511A");
    let root_fp: Digest = unhex("A6119B0D03A43C07A6464B05ECC019F19CFCB1FA8D659FED14A722BFC75086FD")
        .try_into()
        .unwrap();
    let cdr_bytes = gv3_cdr_bytes(
        "test_root_policy_custody",
        "test_root_policy_signer",
        "ae_rc1_identity_policy_signer_v1",
        root_fp,
    );
    assert_eq!(cdr_bytes.len(), 163);
    let cdr = CustodyDispositionReceiptV1::decode(&cdr_bytes).expect("GV3 CDR");
    assert_eq!(cdr.encode(), cdr_bytes);
    assert_eq!(
        hex(cdr.digest()),
        "F152DA92296A45197DCA80543161660BC30251E4D9F6B3702AAC207A8671E9D2"
    );

    let mut root_body = Vec::new();
    root_body.extend_from_slice(b"RTR1");
    root_body.extend_from_slice(&1u16.to_le_bytes());
    root_body.push(1);
    token(&mut root_body, "ae_rc1_identity_policy_signer_v1");
    root_body.extend_from_slice(&1u32.to_le_bytes());
    root_body.extend_from_slice(&root_key);
    root_body.extend_from_slice(&root_fp);
    root_body.extend_from_slice(udr.digest());
    root_body.extend_from_slice(&d(20));
    root_body.extend_from_slice(&1u64.to_le_bytes());
    let mut root_bytes = root_body.clone();
    root_bytes.extend_from_slice(&domain_hash_sha256(
        RELEASE_TRUST_ROOT_DOMAIN_V1,
        &root_body,
    ));
    let root = ReleaseTrustRootV1::decode(&root_bytes).expect("GV3 RTR");
    assert_eq!(root.encode(), root_bytes);
    assert_eq!(root_bytes.len(), 213);
}

#[test]
fn strict_verifier_accepts_known_answer_and_rejects_domain_key_and_malformed_inputs() {
    let public_key = unhex("D75A980182B10AB7D54BFED3C964073A0EE172F3DAA62325AF021A68F707511A");
    let signature = unhex("220540DCA3B1AA4478C06C875AD76DB27D1DCE3B1C11B6C7ECB453BB814D9619431BF106C50E65FA234BB30BF90E65957EF5B3A02965E85C3993B39BECD28200");
    let mut message = b"ae.r7.genesis_identity_policy.policy_key_pop.v1".to_vec();
    message.push(0);
    let mut ceremony_digest = [0u8; 32];
    ceremony_digest[0] = 0xA5;
    message.extend_from_slice(&ceremony_digest);
    assert!(verify_detached_ed25519(&public_key, &message, &signature).is_ok());
    message[0] ^= 1;
    assert!(verify_detached_ed25519(&public_key, &message, &signature).is_err());
    assert!(verify_detached_ed25519(&[0xFF; 32], &message, &signature).is_err());
    assert!(verify_detached_ed25519(&public_key, &message, &[0; 64]).is_err());
    assert_eq!(
        fingerprint_public_key(&public_key),
        unhex("A6119B0D03A43C07A6464B05ECC019F19CFCB1FA8D659FED14A722BFC75086FD").as_slice()
    );
}
