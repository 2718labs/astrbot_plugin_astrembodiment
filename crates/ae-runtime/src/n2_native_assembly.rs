#![allow(dead_code)] // dormant until real P2/P3/P4 owner constructors exist

use ae_contracts::{wire, Digest, Id128};
use ae_store::{CommittedN1AuthorityContextV1, N1IdentityBindingV1};
use thiserror::Error;

const ZERO_DIGEST: Digest = [0; 32];
const ZERO_ID: Id128 = [0; 16];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum N2CapabilityKind {
    Kv,
    Turn,
    Soma,
    Morph,
    Estimate,
    Policy,
    Provenance,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct N2SourceTupleV1 {
    identity: N1IdentityBindingV1,
    state_digest: Digest,
    writer_scope_digest: Digest,
    turn_scope_digest: Digest,
    turn_id: Id128,
    n1_revision: u64,
    provenance_digest: Digest,
}

/// Internal representation of an owner-issued capability.  Raw bytes and
/// caller-selected classifications are intentionally not part of this type.
#[derive(Clone, Debug, PartialEq, Eq)]
struct N2VerifiedCapabilityV1 {
    kind: N2CapabilityKind,
    tuple: N2SourceTupleV1,
    source_digest: Digest,
    kv_stream_revision: Option<u64>,
}

/// Opaque owner capabilities for the preparation seam.  There is deliberately
/// no production constructor in this slice: until native P2/P3/P4 owners are
/// available, every live call must remain fail-closed at `SourceUnavailable`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct N2VerifiedSourceSetV1 {
    kv: Option<N2VerifiedCapabilityV1>,
    turn: Option<N2VerifiedCapabilityV1>,
    soma: Option<N2VerifiedCapabilityV1>,
    morph: Option<N2VerifiedCapabilityV1>,
    estimate: Option<N2VerifiedCapabilityV1>,
    policy: Option<N2VerifiedCapabilityV1>,
    provenance: Option<N2VerifiedCapabilityV1>,
}

impl N2VerifiedSourceSetV1 {
    pub(crate) fn empty() -> Self {
        Self::default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedN1NativeSemanticCommitV1 {
    pub(crate) base_revision: u64,
    pub(crate) next_revision: u64,
    pub(crate) identity: N1IdentityBindingV1,
    pub(crate) writer_scope_digest: Digest,
    pub(crate) turn_scope_digest: Digest,
    pub(crate) turn_id: Id128,
    pub(crate) provenance_digest: Digest,
    pub(crate) kv_stream_revision: u64,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub(crate) enum N2AssemblyError {
    #[error("N2 source capability is unavailable: {0}")]
    SourceUnavailable(&'static str),
    #[error("N2 source capability is invalid: {0}")]
    InvalidCapability(&'static str),
    #[error("N2 source binding mismatch: {0}")]
    BindingMismatch(&'static str),
    #[error("N2 Store context is invalid: {0}")]
    InvalidContext(&'static str),
}

pub(crate) fn prepare_n2_no_action_v1(
    context: &CommittedN1AuthorityContextV1,
    sources: N2VerifiedSourceSetV1,
) -> Result<PreparedN1NativeSemanticCommitV1, N2AssemblyError> {
    validate_context(context)?;

    let required = [
        ("kv", sources.kv.as_ref(), N2CapabilityKind::Kv),
        ("turn", sources.turn.as_ref(), N2CapabilityKind::Turn),
        ("soma", sources.soma.as_ref(), N2CapabilityKind::Soma),
        ("morph", sources.morph.as_ref(), N2CapabilityKind::Morph),
        (
            "estimate",
            sources.estimate.as_ref(),
            N2CapabilityKind::Estimate,
        ),
        ("policy", sources.policy.as_ref(), N2CapabilityKind::Policy),
        (
            "provenance",
            sources.provenance.as_ref(),
            N2CapabilityKind::Provenance,
        ),
    ];

    let mut baseline: Option<&N2SourceTupleV1> = None;
    for (name, capability, expected_kind) in required {
        let capability = capability.ok_or(N2AssemblyError::SourceUnavailable(name))?;
        if capability.kind != expected_kind || capability.source_digest == ZERO_DIGEST {
            return Err(N2AssemblyError::InvalidCapability(name));
        }
        if expected_kind == N2CapabilityKind::Provenance
            && capability.source_digest != capability.tuple.provenance_digest
        {
            return Err(N2AssemblyError::BindingMismatch("provenance"));
        }
        validate_tuple(context, &capability.tuple)?;
        if let Some(baseline) = baseline {
            if baseline != &capability.tuple {
                return Err(N2AssemblyError::BindingMismatch(name));
            }
        } else {
            baseline = Some(&capability.tuple);
        }
    }

    let kv = sources
        .kv
        .as_ref()
        .ok_or(N2AssemblyError::SourceUnavailable("kv"))?;
    let kv_stream_revision = kv
        .kv_stream_revision
        .ok_or(N2AssemblyError::InvalidCapability("kv"))?;
    let tuple = baseline.ok_or(N2AssemblyError::SourceUnavailable("kv"))?;

    Ok(PreparedN1NativeSemanticCommitV1 {
        base_revision: context.current_revision,
        next_revision: context
            .current_revision
            .checked_add(1)
            .ok_or(N2AssemblyError::InvalidContext("revision overflow"))?,
        identity: context.identity.clone(),
        writer_scope_digest: context.writer_scope_digest,
        turn_scope_digest: tuple.turn_scope_digest,
        turn_id: tuple.turn_id,
        provenance_digest: tuple.provenance_digest,
        kv_stream_revision,
    })
}

fn validate_context(context: &CommittedN1AuthorityContextV1) -> Result<(), N2AssemblyError> {
    if context.state_bytes.is_empty() {
        return Err(N2AssemblyError::InvalidContext("empty state"));
    }
    if context.scope.bot_token == ZERO_ID
        || context.scope.persona_token == ZERO_ID
        || context.scope.session_token == ZERO_ID
        || context.scope.relation_token == Some(ZERO_ID)
    {
        return Err(N2AssemblyError::InvalidContext("zero scope token"));
    }
    if context.writer_scope_digest == ZERO_DIGEST || context.state_digest == ZERO_DIGEST {
        return Err(N2AssemblyError::InvalidContext("zero context digest"));
    }
    let expected_writer_scope = wire::persona_scope_digest(
        &context.scope.bot_token,
        &context.scope.persona_token,
        context.scope.relation_token.as_ref(),
    );
    if context.writer_scope_digest != expected_writer_scope {
        return Err(N2AssemblyError::InvalidContext("writer scope digest"));
    }
    if context.identity.incarnation_id == ZERO_DIGEST
        || context.identity.manifest_digest == ZERO_DIGEST
        || context.identity.seed_code_digest == ZERO_DIGEST
        || context.identity.formula_digest == ZERO_DIGEST
        || context.identity.constitution_digest == ZERO_DIGEST
        || context.identity.genesis_receipt_digest == ZERO_DIGEST
    {
        return Err(N2AssemblyError::InvalidContext("zero identity binding"));
    }
    Ok(())
}

fn validate_tuple(
    context: &CommittedN1AuthorityContextV1,
    tuple: &N2SourceTupleV1,
) -> Result<(), N2AssemblyError> {
    if tuple.identity != context.identity {
        return Err(N2AssemblyError::BindingMismatch("identity"));
    }
    if tuple.state_digest != context.state_digest {
        return Err(N2AssemblyError::BindingMismatch("state"));
    }
    if tuple.writer_scope_digest != context.writer_scope_digest {
        return Err(N2AssemblyError::BindingMismatch("writer scope"));
    }
    if tuple.turn_scope_digest != wire::scope_digest(&context.scope) {
        return Err(N2AssemblyError::BindingMismatch("turn scope"));
    }
    if tuple.turn_id == ZERO_ID || tuple.turn_scope_digest == ZERO_DIGEST {
        return Err(N2AssemblyError::InvalidCapability("turn"));
    }
    if tuple.provenance_digest == ZERO_DIGEST {
        return Err(N2AssemblyError::InvalidCapability("provenance"));
    }
    if tuple.n1_revision != context.current_revision {
        return Err(N2AssemblyError::BindingMismatch("N1 revision"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use ae_contracts::{wire, ScopeRef};
    use ae_store::{CommittedN1AuthorityContextV1, N1IdentityBindingV1};

    use super::{
        prepare_n2_no_action_v1, N2AssemblyError, N2CapabilityKind, N2SourceTupleV1,
        N2VerifiedCapabilityV1, N2VerifiedSourceSetV1, ZERO_DIGEST, ZERO_ID,
    };

    fn context() -> CommittedN1AuthorityContextV1 {
        let id = [1u8; 16];
        let digest = [2u8; 32];
        let scope = ScopeRef {
            bot_token: id,
            persona_token: [3u8; 16],
            relation_token: None,
            session_token: [4u8; 16],
        };
        let writer_scope_digest =
            wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None);
        CommittedN1AuthorityContextV1 {
            scope,
            writer_scope_digest,
            identity: N1IdentityBindingV1 {
                incarnation_id: digest,
                manifest_digest: [6u8; 32],
                seed_code_digest: [7u8; 32],
                formula_digest: [8u8; 32],
                constitution_digest: [9u8; 32],
                genesis_receipt_digest: [10u8; 32],
            },
            current_revision: 0,
            state_bytes: vec![11],
            state_digest: [12u8; 32],
            graph_digest: [13u8; 32],
        }
    }

    fn tuple(context: &CommittedN1AuthorityContextV1) -> N2SourceTupleV1 {
        N2SourceTupleV1 {
            identity: context.identity.clone(),
            state_digest: context.state_digest,
            writer_scope_digest: context.writer_scope_digest,
            turn_scope_digest: wire::scope_digest(&context.scope),
            turn_id: [15u8; 16],
            n1_revision: context.current_revision,
            provenance_digest: [16u8; 32],
        }
    }

    fn capability(
        kind: N2CapabilityKind,
        tuple: &N2SourceTupleV1,
        source_byte: u8,
        kv_stream_revision: Option<u64>,
    ) -> N2VerifiedCapabilityV1 {
        N2VerifiedCapabilityV1 {
            kind,
            tuple: tuple.clone(),
            source_digest: [source_byte; 32],
            kv_stream_revision,
        }
    }

    fn complete_sources(context: &CommittedN1AuthorityContextV1) -> N2VerifiedSourceSetV1 {
        let tuple = tuple(context);
        N2VerifiedSourceSetV1 {
            kv: Some(capability(N2CapabilityKind::Kv, &tuple, 20, Some(77))),
            turn: Some(capability(N2CapabilityKind::Turn, &tuple, 21, None)),
            soma: Some(capability(N2CapabilityKind::Soma, &tuple, 22, None)),
            morph: Some(capability(N2CapabilityKind::Morph, &tuple, 23, None)),
            estimate: Some(capability(N2CapabilityKind::Estimate, &tuple, 24, None)),
            policy: Some(capability(N2CapabilityKind::Policy, &tuple, 25, None)),
            provenance: Some(capability(N2CapabilityKind::Provenance, &tuple, 16, None)),
        }
    }

    #[test]
    fn missing_sources_fail_closed_before_any_prepared_commit() {
        let error = prepare_n2_no_action_v1(&context(), N2VerifiedSourceSetV1::empty())
            .expect_err("missing owner capabilities must not prepare a commit");
        assert_eq!(error, N2AssemblyError::SourceUnavailable("kv"));
    }

    #[test]
    fn complete_verified_sources_prepare_no_action_without_store_mutation() {
        let context = context();
        let prepared = prepare_n2_no_action_v1(&context, complete_sources(&context))
            .expect("all verified sources should prepare a no-action descriptor");
        assert_eq!(prepared.base_revision, 0);
        assert_eq!(prepared.next_revision, 1);
        assert_eq!(prepared.kv_stream_revision, 77);
        assert_eq!(prepared.identity, context.identity);
        assert_eq!(prepared.writer_scope_digest, context.writer_scope_digest);
    }

    #[test]
    fn each_missing_owner_capability_is_rejected() {
        let names = [
            "kv",
            "turn",
            "soma",
            "morph",
            "estimate",
            "policy",
            "provenance",
        ];
        for name in names {
            let context = context();
            let mut sources = complete_sources(&context);
            match name {
                "kv" => sources.kv = None,
                "turn" => sources.turn = None,
                "soma" => sources.soma = None,
                "morph" => sources.morph = None,
                "estimate" => sources.estimate = None,
                "policy" => sources.policy = None,
                "provenance" => sources.provenance = None,
                _ => unreachable!(),
            }
            let error = prepare_n2_no_action_v1(&context, sources).unwrap_err();
            assert_eq!(error, N2AssemblyError::SourceUnavailable(name));
        }
    }

    #[test]
    fn mismatched_source_tuple_is_rejected_before_preparation() {
        let context = context();
        let mut sources = complete_sources(&context);
        sources.soma.as_mut().unwrap().tuple.state_digest[0] ^= 1;
        let error = prepare_n2_no_action_v1(&context, sources).unwrap_err();
        assert_eq!(error, N2AssemblyError::BindingMismatch("state"));
    }

    #[test]
    fn turn_scope_digest_mismatch_is_rejected_before_preparation() {
        let context = context();
        let mut sources = complete_sources(&context);
        sources.turn.as_mut().unwrap().tuple.turn_scope_digest[0] ^= 1;
        let error = prepare_n2_no_action_v1(&context, sources).unwrap_err();
        assert_eq!(error, N2AssemblyError::BindingMismatch("turn scope"));
    }

    #[test]
    fn kv_stream_revision_never_becomes_n1_revision() {
        let mut context = context();
        context.current_revision = 7;
        let prepared = prepare_n2_no_action_v1(&context, complete_sources(&context))
            .expect("KV stream revision is independent from N1 revision");
        assert_eq!(prepared.base_revision, 7);
        assert_eq!(prepared.next_revision, 8);
        assert_eq!(prepared.kv_stream_revision, 77);
    }

    #[test]
    fn zero_owner_attestation_is_rejected_before_preparation() {
        let context = context();
        let mut sources = complete_sources(&context);
        sources.policy.as_mut().unwrap().source_digest = ZERO_DIGEST;
        let error = prepare_n2_no_action_v1(&context, sources).unwrap_err();
        assert_eq!(error, N2AssemblyError::InvalidCapability("policy"));
    }

    #[test]
    fn mismatched_n1_revision_is_rejected_before_preparation() {
        let context = context();
        let mut sources = complete_sources(&context);
        sources.turn.as_mut().unwrap().tuple.n1_revision += 1;
        let error = prepare_n2_no_action_v1(&context, sources).unwrap_err();
        assert_eq!(error, N2AssemblyError::BindingMismatch("N1 revision"));
    }

    #[test]
    fn provenance_attestation_must_match_the_bound_provenance_digest() {
        let context = context();
        let mut sources = complete_sources(&context);
        sources.provenance.as_mut().unwrap().source_digest[0] ^= 1;
        let error = prepare_n2_no_action_v1(&context, sources).unwrap_err();
        assert_eq!(error, N2AssemblyError::BindingMismatch("provenance"));
    }

    #[test]
    fn revision_overflow_is_rejected_without_a_prepared_commit() {
        let mut context = context();
        context.current_revision = u64::MAX;
        let error = prepare_n2_no_action_v1(&context, complete_sources(&context)).unwrap_err();
        assert_eq!(error, N2AssemblyError::InvalidContext("revision overflow"));
    }

    #[test]
    fn zero_scope_token_is_rejected_before_preparation() {
        let mut context = context();
        context.scope.bot_token = ZERO_ID;
        let error = prepare_n2_no_action_v1(&context, complete_sources(&context)).unwrap_err();
        assert_eq!(error, N2AssemblyError::InvalidContext("zero scope token"));
    }
}
