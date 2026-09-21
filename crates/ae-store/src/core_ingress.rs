//! V9 persona-only ingress. Idempotency precedes journal-head and claim work.
use crate::{blob, Store, StoreError};
use ae_contracts::*;
use ae_neurofield::{graph_digest, state_digest};
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

pub(crate) fn invalid(code: &'static str) -> StoreError {
    StoreError::SemanticInvalid(code)
}
pub(crate) fn encode<T: serde::Serialize>(v: &T) -> Result<Vec<u8>, StoreError> {
    core_encode(v).map_err(|_| invalid("CORE_WIRE_INVALID"))
}
pub(crate) fn decode<T: serde::de::DeserializeOwned + serde::Serialize>(
    v: &[u8],
) -> Result<T, StoreError> {
    core_decode(v).map_err(|_| invalid("CORE_WIRE_INVALID"))
}
pub(crate) fn digest(domain: &[u8], v: &[u8]) -> Digest {
    wire::domain_hash(domain, &[v])
}
pub(crate) fn check_core(conn: &Connection, scope: &PersonaScopeRef) -> Result<(), StoreError> {
    if !core_scope_valid(scope) {
        return Err(invalid("INVALID_PERSONA_SCOPE"));
    }
    if crate::core_boundary_v9::preflight(conn)? != crate::core_boundary_v9::OpenRoute::V9 {
        return Err(invalid("V9_REQUIRED"));
    }
    let schema = crate::core_boundary_v9::render_schema()?;
    crate::core_boundary_v9::verify_schema_catalog(
        &crate::core_boundary_v9::bounded_catalog(conn)?,
        &schema,
    )?;
    let applied:bool=conn.query_row("SELECT COUNT(*)=1 FROM core_boundary_control_v1 AS c JOIN schema_migrations AS m ON m.version=9 WHERE c.singleton=1 AND c.state='applied' AND c.externalization_disabled=1 AND c.boundary_revision=1 AND c.schema_digest=?1 AND m.digest=?2 AND m.completed_at_ms=c.authoritative_now_utc_ms",params![blob(digest(b"ae.autonomy.db.v9.schema-sql.v1",schema.sql.as_bytes())),crate::core_boundary_v9::MIGRATION_DIGEST],|r|r.get(0))?;
    if !applied {
        return Err(invalid("CORE_BOUNDARY_NOT_APPLIED"));
    }
    let bound: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM active_bindings WHERE bot_token=?1 AND persona_token=?2)",
        params![blob(scope.bot_token), blob(scope.persona_token)],
        |r| r.get(0),
    )?;
    if !bound {
        return Err(StoreError::GenesisNotFound);
    }
    Ok(())
}
pub(crate) fn journal_scope(scope: &PersonaScopeRef, turn_id: Id128) -> ScopeRef {
    ScopeRef {
        bot_token: scope.bot_token,
        persona_token: scope.persona_token,
        relation_token: None,
        session_token: turn_id,
    }
}

/// Authenticate the v9 persona-only kind-10 lane during legacy journal audit.
/// No empty-delta exemption exists without its immutable core receipt.
pub(crate) fn verify_core_inbound_journal(conn: &Connection, event_bytes: &[u8]) -> Result<bool, StoreError> {
    let installed: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name='core_inbound_receipt_v1')", [], |r|r.get(0))?;
    if !installed { return Ok(false); }
    let event=wire::decode_event(event_bytes).map_err(|_|invalid("CORE_EVENT_INVALID"))?;
    let CanonicalEvent::InteractionFactBatch(batch)=&event else { return Ok(false); };
    let p=wire::persona_scope_digest(&batch.scope.bot_token,&batch.scope.persona_token,None);
    let ed=wire::event_digest(&event);
    let row:Option<(Vec<u8>,Vec<u8>)>=conn.query_row("SELECT CASE WHEN length(initial_receipt_bytes)<=65536 THEN initial_receipt_bytes ELSE zeroblob(0) END,initial_receipt_digest FROM core_inbound_receipt_v1 WHERE persona_scope=?1 AND event_digest=?2",params![blob(p),blob(ed)],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    let Some((bytes,hash))=row else { return Ok(false); };
    if digest(b"ae.core-inbound.initial-receipt.v1",&bytes).as_slice()!=hash { return Err(invalid("CORE_RECEIPT_INVALID")); }
    let initial:CoreInboundInitialReceiptV1=decode(&bytes)?;let r=&initial.event;
    check_core(conn,&r.scope)?;
    if r.schema_version!=1 || core_persona_digest(&r.scope)!=p || r.event_bytes!=event_bytes || r.event_id!=batch.event_id || r.transition.event_digest!=ed || batch.scope.relation_token.is_some() || batch.facts.len()!=1 || r.transition.base_revision!=batch.causal.base_revision || r.transition.base_revision.checked_add(1)!=Some(r.transition.next_revision) { return Err(invalid("CORE_RECEIPT_INVALID")); }
    let bound:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM core_inbound_receipt_v1 AS c JOIN journal AS j ON j.scope_digest=c.persona_scope AND j.logical_revision=c.inbound_revision JOIN applied_events AS a ON a.scope_digest=j.scope_digest AND a.event_digest=j.event_digest AND a.revision=j.logical_revision WHERE c.persona_scope=?1 AND c.event_digest=?2 AND c.operation_id=?3 AND c.turn_id=?4 AND c.request_digest=?5 AND c.event_id=?6 AND c.event_bytes=?7 AND c.inbound_revision=?8 AND c.clock_head_digest=?9 AND j.event_bytes=c.event_bytes AND j.event_digest=c.event_digest AND j.receipt_bytes=?10 AND length(j.delta_bytes)=0 AND j.event_kind='interaction_fact_batch' AND j.base_revision=?11)",params![blob(p),blob(ed),blob(r.operation_id),blob(r.turn_id),blob(r.request_digest),blob(r.event_id),event_bytes,r.transition.next_revision,blob(r.clock_head_digest),wire::encode_transition_receipt(&r.transition),r.transition.base_revision],|r|r.get(0))?;
    if !bound { return Err(invalid("CORE_JOURNAL_RECEIPT_INVALID")); }
    let f=&batch.facts[0];
    let fact_body:String=conn.query_row("SELECT CASE WHEN length(CAST(body_json AS BLOB))<=262144 THEN body_json ELSE '' END FROM interaction_fact WHERE fact_id=?1 AND event_id=?2 AND persona_scope=?3 AND relation_scope=?3 AND revision=?4 AND observed_at_utc_ms=?5 AND source_digest=?6",params![blob(f.fact_id),blob(batch.event_id),blob(p),r.transition.next_revision,f.observed_at_utc_ms,blob(f.source_digest)],|r|r.get(0))?;
    if serde_json::from_str::<InteractionFactV1>(&fact_body).map_err(|_|invalid("CORE_FACT_INVALID"))?!=*f { return Err(invalid("CORE_FACT_INVALID")); }
    let inner=InnerEventV1 { schema_version:1,event_id:core_id(b"ae.core-inbound.inner-event.v1",&[&p,&ed]),persona_scope:p,kind:InnerEventKindV1::HomeostasisChanged,committed_at_utc_ms:f.observed_at_utc_ms,summary_code:"inbound_observed".into(),value_before:None,value_after:None,source_event_ids:vec![f.fact_id],tombstoned:false };
    let inner_body:String=conn.query_row("SELECT CASE WHEN length(CAST(body_json AS BLOB))<=262144 THEN body_json ELSE '' END FROM inner_event WHERE event_id=?1 AND persona_scope=?2 AND journal_revision=?3 AND committed_at_utc_ms=?4 AND kind=?5 AND tombstoned=0",params![blob(inner.event_id),blob(p),r.transition.next_revision,inner.committed_at_utc_ms,format!("{:?}",inner.kind)],|r|r.get(0))?;
    if serde_json::from_str::<InnerEventV1>(&inner_body).map_err(|_|invalid("CORE_INNER_INVALID"))?!=inner { return Err(invalid("CORE_INNER_INVALID")); }
    let closure:bool=conn.query_row("SELECT (SELECT COUNT(*) FROM interaction_fact WHERE event_id=?1)=1 AND (SELECT COUNT(*) FROM inner_event WHERE persona_scope=?2 AND journal_revision=?3)=1 AND EXISTS(SELECT 1 FROM inner_event_manifest WHERE persona_scope=?2 AND journal_revision=?3 AND event_count=1 AND event_digest=?4)",params![blob(batch.event_id),blob(p),r.transition.next_revision,blob(crate::autonomy::inner_event_manifest_digest(&[inner])?)],|r|r.get(0))?;
    if !closure { return Err(invalid("CORE_INNER_MANIFEST_INVALID")); }
    Ok(true)
}

fn append(
    tx: &Transaction<'_>,
    scope: &PersonaScopeRef,
    operation_id: Id128,
    turn_id: Id128,
    request_digest: Digest,
    make: impl FnOnce(u64) -> CanonicalEvent,
) -> Result<CoreEventReceiptV1, StoreError> {
    let p = core_persona_digest(scope);
    let identity =
        crate::semantic::active_identity_for_scope_tx(tx, &journal_scope(scope, turn_id))?;
    let (base, chain) =
        crate::semantic::attest_persona_journal_tx(tx, p, identity.initial_snapshot_digest)?;
    if base >= crate::MAX_JOURNAL_ROWS_PER_SCOPE {
        return Err(invalid("CORE_HISTORY_CAPACITY"));
    }
    let formula = phase0_canonical_formula_digest_v1(&identity.formula_digest);
    let (field, graph) = if let Some(origin) = crate::semantic::stored_origin(tx, p)? {
        let (_, f, g) =
            crate::semantic::semantic_state_for_derivation_tx(tx, p, &origin, &identity)?;
        (f, g)
    } else {
        (identity.baseline_field, identity.baseline_graph)
    };
    // New operations consume the authenticated rolling closure and projected
    // field. Exact idempotent replay returned before entering this function.
    let (clock_head_digest, field, graph) = match crate::embodiment_clock::core_clock_projection(tx, scope)? {
        Some(projection) => projection,
        None => (digest(b"ae.embodiment.clock-missing.v1", &p), field, graph),
    };
    let event = make(base);
    let event_bytes = wire::encode_event(&event);
    let event_digest = wire::event_digest(&event);
    let event_id = match &event {
        CanonicalEvent::InteractionFactBatch(b) => b.event_id,
        CanonicalEvent::DeliveryOutcome(d) => d.event_id,
        _ => return Err(invalid("CORE_EVENT_KIND")),
    };
    let transition = TransitionReceipt {
        schema_version: wire::WIRE_SCHEMA_VERSION,
        formula_digest: formula,
        scope_digest: p,
        event_digest,
        authority_digest: ae_authority::authority_projection_digest(&event),
        base_revision: base,
        next_revision: base
            .checked_add(1)
            .ok_or(invalid("CORE_REVISION_OVERFLOW"))?,
        state_before: state_digest(&field, &formula),
        state_after: state_digest(&field, &formula),
        graph_after: graph_digest(&graph),
        action_contract: None,
        active_nodes: 0,
        active_edges: 0,
        residuals: InvariantResiduals::default(),
        status: CommitStatus::Committed,
    };
    let receipt_bytes = wire::encode_transition_receipt(&transition);
    let chain = ae_continuum::chain_link(&chain, &event_bytes, &receipt_bytes);
    tx.execute("INSERT INTO journal(logical_revision,scope_digest,base_revision,event_kind,event_bytes,event_digest,receipt_bytes,delta_bytes,chain_digest,committed_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![transition.next_revision,blob(p),base,wire::event_kind_name(&event),&event_bytes,blob(event_digest),receipt_bytes,Vec::<u8>::new(),blob(chain),crate::now_ms()])?;
    tx.execute(
        "INSERT INTO applied_events(scope_digest,event_digest,revision) VALUES(?1,?2,?3)",
        params![blob(p), blob(event_digest), transition.next_revision],
    )?;
    if let CanonicalEvent::InteractionFactBatch(batch) = &event {
        let f = &batch.facts[0];
        tx.execute("INSERT INTO interaction_fact(fact_id,event_id,persona_scope,relation_scope,observed_at_utc_ms,source_digest,revision,body_json) VALUES(?1,?2,?3,?3,?4,?5,?6,?7)",params![blob(f.fact_id),blob(event_id),blob(p),f.observed_at_utc_ms,blob(f.source_digest),transition.next_revision,serde_json::to_string(f).map_err(|_|invalid("CORE_FACT_ENCODE"))?])?;
        let inner = InnerEventV1 {
            schema_version: 1,
            event_id: core_id(b"ae.core-inbound.inner-event.v1", &[&p, &event_digest]),
            persona_scope: p,
            kind: InnerEventKindV1::HomeostasisChanged,
            committed_at_utc_ms: f.observed_at_utc_ms,
            summary_code: "inbound_observed".into(),
            value_before: None,
            value_after: None,
            source_event_ids: vec![f.fact_id],
            tombstoned: false,
        };
        let inner_digest = crate::autonomy::inner_event_manifest_digest(&[inner.clone()])?;
        tx.execute("INSERT INTO inner_event_manifest(persona_scope,journal_revision,event_count,event_digest) VALUES(?1,?2,1,?3)",params![blob(p),transition.next_revision,blob(inner_digest)])?;
        tx.execute("INSERT INTO inner_event(event_id,persona_scope,journal_revision,committed_at_utc_ms,kind,tombstoned,body_json) VALUES(?1,?2,?3,?4,?5,0,?6)",params![blob(inner.event_id),blob(p),transition.next_revision,inner.committed_at_utc_ms,format!("{:?}",inner.kind),serde_json::to_string(&inner).map_err(|_|invalid("CORE_INNER_ENCODE"))?])?;
    }
    Ok(CoreEventReceiptV1 {
        schema_version: 1,
        scope: scope.clone(),
        operation_id,
        turn_id,
        request_digest,
        event_id,
        event_bytes,
        transition,
        clock_head_digest,
    })
}

impl Store {
    pub fn commit_core_inbound_v1(
        &mut self,
        request: &CommitCoreInboundV1,
    ) -> Result<CoreInboundCommitOutcomeV1, StoreError> {
        request.validate_v1().map_err(invalid)?;
        let o = &request.observation;
        let p = core_persona_digest(&o.scope);
        let request_digest = digest(b"ae.core-inbound.request.v1", &encode(request)?);
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_core(&tx, &o.scope)?;
        let existing:Option<(Vec<u8>,Vec<u8>,Vec<u8>)>=tx.query_row("SELECT request_digest,initial_receipt_bytes,initial_receipt_digest FROM core_inbound_receipt_v1 WHERE persona_scope=?1 AND operation_id=?2",params![blob(p),blob(o.operation_id)],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((d, b, h)) = existing {
            if d != request_digest {
                return Err(invalid("IDEMPOTENCY_CONFLICT"));
            }
            if b.len() > 65536 || digest(b"ae.core-inbound.initial-receipt.v1", &b).as_slice() != h
            {
                return Err(invalid("CORE_RECEIPT_INVALID"));
            }
            let initial_receipt: CoreInboundInitialReceiptV1 = decode(&b)?;
            if initial_receipt.event.scope != o.scope
                || initial_receipt.event.operation_id != o.operation_id
                || initial_receipt.event.request_digest != request_digest
            {
                return Err(invalid("CORE_RECEIPT_INVALID"));
            }
            return Ok(CoreInboundCommitOutcomeV1 {
                commit_status: CoreCommitStatusV1::Existing,
                provider_authorized_now: false,
                initial_receipt,
            });
        }
        let collision:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM core_inbound_receipt_v1 WHERE persona_scope=?1 AND turn_id=?2)",params![blob(p),blob(o.turn_id)],|r|r.get(0))?;
        if collision {
            return Err(invalid("IDEMPOTENCY_CONFLICT"));
        }
        let fact_id = core_id(b"ae.core-inbound.fact-id.v1", &[&request_digest]);
        let event_id = core_id(
            b"ae.core-inbound.event-id.v1",
            &[&p, &o.operation_id, &request_digest],
        );
        let observation_bytes = encode(o)?;
        let observation_digest = wire::domain_hash(
            b"ae.core-inbound.observation.v1",
            &[&observation_bytes, &fact_id, &event_id],
        );
        let event = append(
            &tx,
            &o.scope,
            o.operation_id,
            o.turn_id,
            request_digest,
            |base| {
                CanonicalEvent::InteractionFactBatch(InteractionFactBatchV1 {
                    schema_version: 1,
                    event_id,
                    scope: journal_scope(&o.scope, o.turn_id),
                    causal: CausalRef {
                        turn_id: o.turn_id,
                        action_id: None,
                        delivery_id: None,
                        claim_id: None,
                        base_revision: base,
                    },
                    facts: vec![InteractionFactV1 {
                        fact_id,
                        kind: InteractionFactKindV1::InboundObserved,
                        observed_at_utc_ms: o.observed_at_utc_ms,
                        source_authority: InteractionSourceAuthorityV1::AstrbotMetadata,
                        source_digest: observation_digest,
                        extractor_digest: o.extractor_digest,
                        confidence: o.confidence,
                        value_code: None,
                        subject_public_ref: None,
                        consent_terms: None,
                        scheduled_at_utc_ms: None,
                        expires_at_utc_ms: None,
                    }],
                })
            },
        )?;
        let claim = if let Some(a) = &request.appraisal {
            crate::semantic::begin_semantic_appraisal_claim_tx_v1(
                &tx,
                event.transition.event_digest,
                a.daily_token_limit,
                a.reserved_tokens,
                a.provider_digest,
                crate::now_ms(),
                false,
            )?
        } else {
            None
        };
        let disposition = match claim.as_ref().map(|c| c.status) {
            Some(SemanticAppraisalBeginStatusV1::Claimed) => {
                CoreInitialAppraisalDispositionV1::Claimed
            }
            Some(SemanticAppraisalBeginStatusV1::BudgetExhausted) => {
                CoreInitialAppraisalDispositionV1::BudgetExhausted
            }
            Some(SemanticAppraisalBeginStatusV1::CapacityDeferred) => {
                CoreInitialAppraisalDispositionV1::CapacityDeferred
            }
            None if request.appraisal.is_some() => {
                CoreInitialAppraisalDispositionV1::RetryExpiredOrUnknown
            }
            None => CoreInitialAppraisalDispositionV1::NotRequested,
        };
        let authorized = disposition == CoreInitialAppraisalDispositionV1::Claimed;
        let initial = CoreInboundInitialReceiptV1 {
            event,
            disposition,
            provider_authority_granted_initial: authorized,
            challenge: claim.as_ref().and_then(|c| c.challenge.clone()),
            capacity_reason: claim.as_ref().and_then(|c| c.capacity_reason),
            budget: claim.as_ref().and_then(|c| c.budget.clone()),
            reply_affect: claim.and_then(|c| c.reply_affect),
        };
        let bytes = encode(&initial)?;
        if bytes.len() > 65536 {
            return Err(invalid("CORE_RECEIPT_LIMIT"));
        }
        let budget = encode(&initial.budget)?;
        let disposition = match initial.disposition {
            CoreInitialAppraisalDispositionV1::NotRequested => "not_requested",
            CoreInitialAppraisalDispositionV1::Claimed => "claimed",
            CoreInitialAppraisalDispositionV1::BudgetExhausted => "budget_exhausted",
            CoreInitialAppraisalDispositionV1::CapacityDeferred => "capacity_deferred",
            CoreInitialAppraisalDispositionV1::RetryExpiredOrUnknown => "retry_expired_or_unknown",
        };
        tx.execute("INSERT INTO core_inbound_receipt_v1(persona_scope,operation_id,turn_id,request_digest,observation_bytes,observation_digest,fact_id,event_id,inbound_revision,clock_head_digest,event_bytes,event_digest,initial_disposition,budget_receipt_bytes,settlement_challenge_digest,provider_authority_granted_initial,initial_receipt_bytes,initial_receipt_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",params![blob(p),blob(o.operation_id),blob(o.turn_id),blob(request_digest),observation_bytes,blob(observation_digest),blob(fact_id),blob(event_id),initial.event.transition.next_revision,blob(initial.event.clock_head_digest),&initial.event.event_bytes,blob(initial.event.transition.event_digest),disposition,budget,initial.challenge.as_ref().map(|c|c.request_nonce_digest.to_vec()),authorized,&bytes,blob(digest(b"ae.core-inbound.initial-receipt.v1",&bytes))])?;
        tx.commit()?;
        Ok(CoreInboundCommitOutcomeV1 {
            commit_status: CoreCommitStatusV1::Committed,
            provider_authorized_now: authorized,
            initial_receipt: initial,
        })
    }

    pub fn commit_core_delivery_outcome_v1(
        &mut self,
        r: &CommitCoreDeliveryOutcomeV1,
    ) -> Result<CoreDeliveryCommitOutcomeV1, StoreError> {
        r.validate_v1().map_err(invalid)?;
        let p = core_persona_digest(&r.scope);
        let rd = digest(b"ae.core-delivery.request.v1", &encode(r)?);
        let tx = self
            .conn
            .as_mut()
            .ok_or(StoreError::Closed)?
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        check_core(&tx, &r.scope)?;
        let existing:Option<(Vec<u8>,Vec<u8>,Vec<u8>)>=tx.query_row("SELECT request_digest,receipt_bytes,receipt_digest FROM core_delivery_receipt_v1 WHERE persona_scope=?1 AND operation_id=?2",params![blob(p),blob(r.operation_id)],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional()?;
        if let Some((d, b, h)) = existing {
            if d != rd {
                return Err(invalid("IDEMPOTENCY_CONFLICT"));
            }
            if b.len() > 65536 || digest(b"ae.core-delivery.receipt.v1", &b).as_slice() != h {
                return Err(invalid("CORE_RECEIPT_INVALID"));
            }
            let receipt: CoreEventReceiptV1 = decode(&b)?;
            if receipt.scope != r.scope
                || receipt.operation_id != r.operation_id
                || receipt.request_digest != rd
            {
                return Err(invalid("CORE_RECEIPT_INVALID"));
            }
            return Ok(CoreDeliveryCommitOutcomeV1 {
                commit_status: CoreCommitStatusV1::Existing,
                receipt,
            });
        }
        let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM core_inbound_receipt_v1 WHERE persona_scope=?1 AND operation_id=?2 AND turn_id=?3)",params![blob(p),blob(r.inbound_operation_id),blob(r.turn_id)],|row|row.get(0))?;
        if !linked {
            return Err(invalid("INBOUND_RECEIPT_NOT_FOUND"));
        }
        let event_id = core_id(b"ae.core-delivery.event-id.v1", &[&p, &r.operation_id, &rd]);
        let receipt = append(&tx, &r.scope, r.operation_id, r.turn_id, rd, |base| {
            CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
                event_id,
                scope: journal_scope(&r.scope, r.turn_id),
                causal: CausalRef {
                    turn_id: r.turn_id,
                    action_id: None,
                    delivery_id: None,
                    claim_id: None,
                    base_revision: base,
                },
                delivered: r.delivered,
                visible_action_digest: r.visible_action_digest,
                delivered_at_ms: r.observed_at_utc_ms,
            })
        })?;
        let bytes = encode(&receipt)?;
        if bytes.len() > 65536 {
            return Err(invalid("CORE_RECEIPT_LIMIT"));
        }
        tx.execute("INSERT INTO core_delivery_receipt_v1(persona_scope,operation_id,turn_id,inbound_operation_id,request_digest,event_id,committed_revision,clock_head_digest,event_bytes,event_digest,receipt_bytes,receipt_digest) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",params![blob(p),blob(r.operation_id),blob(r.turn_id),blob(r.inbound_operation_id),blob(rd),blob(event_id),receipt.transition.next_revision,blob(receipt.clock_head_digest),&receipt.event_bytes,blob(receipt.transition.event_digest),&bytes,blob(digest(b"ae.core-delivery.receipt.v1",&bytes))])?;
        tx.commit()?;
        Ok(CoreDeliveryCommitOutcomeV1 {
            commit_status: CoreCommitStatusV1::Committed,
            receipt,
        })
    }
}
