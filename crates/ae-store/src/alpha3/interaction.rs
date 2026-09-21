use super::super::{Store, StoreError};
use ae_contracts::{
    wire, CanonicalEvent, ConsentTermsV1, Digest, InnerEventKindV1, InnerEventV1,
    InteractionFactBatchV1, InteractionFactKindV1, InteractionFactV1, RelationConsentStateV1,
    RelationConsentV1,
};

// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const MAX_BODY_BYTES: usize = 256 * 1024;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const MAX_EVENT_BYTES: usize = 256 * 1024;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const MAX_DELTA_BYTES: usize = 1024 * 1024;
// Retained historical verification data; no active executor is restored.
#[allow(dead_code)]
const REPETITION_SUBMISSION_STEP_RAW: i64 = 250_000;

fn relation_scope(batch: &InteractionFactBatchV1) -> Result<Digest, StoreError> {
    let relation_token = batch
        .scope
        .relation_token
        .ok_or_else(|| StoreError::AutonomyConflict("relation scope is required".into()))?;
    Ok(wire::persona_scope_digest(
        &batch.scope.bot_token,
        &batch.scope.persona_token,
        Some(&relation_token),
    ))
}

fn persona_scope(batch: &InteractionFactBatchV1) -> Digest {
    wire::persona_scope_digest(&batch.scope.bot_token, &batch.scope.persona_token, None)
}

pub fn interaction_inner_event_v1(batch: &InteractionFactBatchV1) -> InnerEventV1 {
    let relation_scope = relation_scope(batch).expect("validated relation scope");
    let event_digest = wire::event_digest(&CanonicalEvent::InteractionFactBatch(batch.clone()));
    let summary_code = if batch
        .facts
        .iter()
        .any(|fact| fact.kind == InteractionFactKindV1::RelationEnded)
    {
        "relation_ended"
    } else if batch.facts.iter().any(|fact| {
        matches!(
            fact.kind,
            InteractionFactKindV1::BoundarySet | InteractionFactKindV1::ContactPaused
        )
    }) {
        "contact_paused"
    } else if batch
        .facts
        .iter()
        .any(|fact| fact.kind == InteractionFactKindV1::ContactGranted)
    {
        "contact_granted"
    } else if batch
        .facts
        .iter()
        .any(|fact| fact.kind == InteractionFactKindV1::FollowUpRequested)
    {
        "follow_up_requested"
    } else if batch
        .facts
        .iter()
        .any(|fact| fact.kind == InteractionFactKindV1::FollowUpResolved)
    {
        "follow_up_resolved"
    } else {
        "inbound_observed"
    };
    let mut sources = batch
        .facts
        .iter()
        .filter(|fact| fact.kind != InteractionFactKindV1::InboundObserved)
        .map(|fact| fact.fact_id)
        .take(8)
        .collect::<Vec<_>>();
    if sources.is_empty() {
        sources.extend(batch.facts.iter().map(|fact| fact.fact_id).take(1));
    }
    InnerEventV1 {
        schema_version: 1,
        event_id: wire::domain_hash(
            b"ae.inner-event.interaction.v1",
            &[&relation_scope, &event_digest],
        )[..16]
            .try_into()
            .expect("digest prefix"),
        persona_scope: persona_scope(batch),
        kind: InnerEventKindV1::HomeostasisChanged,
        committed_at_utc_ms: batch
            .facts
            .iter()
            .map(|fact| fact.observed_at_utc_ms)
            .max()
            .unwrap_or(0),
        summary_code: summary_code.into(),
        value_before: None,
        value_after: None,
        source_event_ids: sources,
        tombstoned: false,
    }
}

fn policy_digest(
    relation_scope: &Digest,
    fact: &InteractionFactV1,
    terms: Option<&ConsentTermsV1>,
) -> Result<Digest, StoreError> {
    let terms = serde_json::to_vec(&terms)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    Ok(wire::domain_hash(
        b"ae.relation-consent.policy.v1",
        &[relation_scope, &fact.source_digest, &terms],
    ))
}

pub(crate) fn reduce_consent_transition_v1(
    current: &RelationConsentV1,
    fact: &InteractionFactV1,
) -> Result<Option<RelationConsentV1>, StoreError> {
    let mut next = current.clone();
    let changed = match fact.kind {
        InteractionFactKindV1::ContactGranted => {
            let terms = fact.consent_terms.as_ref().ok_or_else(|| {
                StoreError::AutonomyConflict("contact grant omitted terms".into())
            })?;
            if terms.pause_until_utc_ms.is_some() {
                return Err(StoreError::AutonomyConflict(
                    "a granted epoch cannot begin paused".into(),
                ));
            }
            match current.state {
                RelationConsentStateV1::Disabled
                | RelationConsentStateV1::PendingReconfirmation => {}
                RelationConsentStateV1::Ended => {
                    next.consent_epoch = current.consent_epoch.saturating_add(1);
                }
                _ => {
                    return Err(StoreError::AutonomyConflict(
                        "contact grant is invalid for current consent state".into(),
                    ))
                }
            }
            next.state = RelationConsentStateV1::Granted;
            next.purposes = terms.purposes.clone();
            next.channels = terms.channels.clone();
            next.valid_from_utc_ms = fact.observed_at_utc_ms;
            next.valid_until_utc_ms = terms.valid_until_utc_ms;
            next.pause_until_utc_ms = None;
            true
        }
        InteractionFactKindV1::ContactPaused => {
            if current.state != RelationConsentStateV1::Granted {
                return Err(StoreError::AutonomyConflict(
                    "contact pause requires a granted epoch".into(),
                ));
            }
            next.state = RelationConsentStateV1::Paused;
            next.pause_until_utc_ms = None;
            true
        }
        InteractionFactKindV1::BoundarySet => {
            if current.state == RelationConsentStateV1::Granted {
                next.state = RelationConsentStateV1::Paused;
                next.pause_until_utc_ms = None;
                true
            } else {
                false
            }
        }
        InteractionFactKindV1::ContactResumed => {
            if current.state != RelationConsentStateV1::Paused
                || current
                    .valid_until_utc_ms
                    .is_some_and(|until| fact.observed_at_utc_ms >= until)
            {
                return Err(StoreError::AutonomyConflict(
                    "contact resume requires unexpired same-epoch terms".into(),
                ));
            }
            next.state = RelationConsentStateV1::Granted;
            next.pause_until_utc_ms = None;
            true
        }
        InteractionFactKindV1::RelationEnded => {
            if current.state == RelationConsentStateV1::Ended {
                false
            } else {
                next.state = RelationConsentStateV1::Ended;
                next.pause_until_utc_ms = None;
                true
            }
        }
        InteractionFactKindV1::InboundObserved
        | InteractionFactKindV1::FollowUpRequested
        | InteractionFactKindV1::FollowUpResolved
        | InteractionFactKindV1::ExplicitOutcomeReported => false,
    };
    if !changed {
        return Ok(None);
    }
    next.revision = current.revision.saturating_add(1);
    next.source_event_id = fact.fact_id;
    next.policy_digest = policy_digest(&next.relation_scope, fact, fact.consent_terms.as_ref())?;
    next.validate()
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    Ok(Some(next))
}

impl Store {}
