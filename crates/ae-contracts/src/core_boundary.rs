//! Closed persona-only core APIs. No request accepts a journal revision.
use crate::*;
use serde::{Deserialize, Serialize};

pub fn core_persona_digest(scope: &PersonaScopeRef) -> Digest {
    wire::persona_scope_digest(&scope.bot_token, &scope.persona_token, None)
}
pub fn core_id(domain: &[u8], pieces: &[&[u8]]) -> Id128 {
    let hash = wire::domain_hash(domain, pieces);
    hash[..16].try_into().expect("digest width")
}
pub fn core_scope_valid(scope: &PersonaScopeRef) -> bool {
    scope.bot_token != [0; 16] && scope.persona_token != [0; 16]
}

/// Versioned, bounded structural binary codec. Maps are lexically sorted, all
/// lengths are U64LE, integers retain signedness, and JSON text is never hashed.
/// Re-encoding after decode rejects alternate orderings and duplicate keys.
pub fn core_encode<T: Serialize>(value: &T) -> Result<Vec<u8>, String> {
    fn put(v: &serde_json::Value, out: &mut Vec<u8>, depth: usize) -> Result<(), String> {
        if depth > 24 {
            return Err("CORE_WIRE_DEPTH".into());
        }
        use serde_json::Value::*;
        match v {
            Null => out.push(0),
            Bool(v) => out.extend([1, u8::from(*v)]),
            Number(v) => {
                if let Some(n) = v.as_u64() {
                    out.push(2);
                    out.extend(n.to_le_bytes());
                } else if let Some(n) = v.as_i64() {
                    out.push(3);
                    out.extend(n.to_le_bytes());
                } else {
                    return Err("CORE_WIRE_FLOAT".into());
                }
            }
            String(v) => {
                out.push(4);
                out.extend((v.len() as u64).to_le_bytes());
                out.extend(v.as_bytes());
            }
            Array(v) => {
                out.push(5);
                out.extend((v.len() as u64).to_le_bytes());
                for x in v {
                    put(x, out, depth + 1)?;
                }
            }
            Object(v) => {
                out.push(6);
                out.extend((v.len() as u64).to_le_bytes());
                let ordered: std::collections::BTreeMap<_, _> = v.iter().collect();
                for (k, v) in ordered {
                    put(&String(k.clone()), out, depth + 1)?;
                    put(v, out, depth + 1)?;
                }
            }
        }
        if out.len() > 262144 {
            return Err("CORE_WIRE_LIMIT".into());
        }
        Ok(())
    }
    let mut out = b"AEC1".to_vec();
    put(
        &serde_json::to_value(value).map_err(|e| e.to_string())?,
        &mut out,
        0,
    )?;
    Ok(out)
}
pub fn core_decode<T: serde::de::DeserializeOwned + Serialize>(bytes: &[u8]) -> Result<T, String> {
    fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Result<&'a [u8], String> {
        if n > bytes.len() {
            return Err("CORE_WIRE_TRUNCATED".into());
        }
        let (a, b) = bytes.split_at(n);
        *bytes = b;
        Ok(a)
    }
    fn len(bytes: &mut &[u8]) -> Result<usize, String> {
        let n = u64::from_le_bytes(take(bytes, 8)?.try_into().unwrap());
        if n > 262144 {
            return Err("CORE_WIRE_LIMIT".into());
        }
        Ok(n as usize)
    }
    fn get(bytes: &mut &[u8], depth: usize) -> Result<serde_json::Value, String> {
        if depth > 24 {
            return Err("CORE_WIRE_DEPTH".into());
        }
        use serde_json::Value;
        Ok(match take(bytes, 1)?[0] {
            0 => Value::Null,
            1 => match take(bytes, 1)?[0] {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                _ => return Err("CORE_WIRE_BOOL".into()),
            },
            2 => Value::from(u64::from_le_bytes(take(bytes, 8)?.try_into().unwrap())),
            3 => Value::from(i64::from_le_bytes(take(bytes, 8)?.try_into().unwrap())),
            4 => {
                let n = len(bytes)?;
                Value::String(
                    std::str::from_utf8(take(bytes, n)?)
                        .map_err(|_| "CORE_WIRE_UTF8")?
                        .into(),
                )
            }
            5 => {
                let n = len(bytes)?;
                if n > bytes.len() {
                    return Err("CORE_WIRE_LIMIT".into());
                }
                let mut v = Vec::new();
                for _ in 0..n {
                    v.push(get(bytes, depth + 1)?);
                }
                Value::Array(v)
            }
            6 => {
                let n = len(bytes)?;
                if n > bytes.len() / 2 {
                    return Err("CORE_WIRE_LIMIT".into());
                }
                let mut v = serde_json::Map::new();
                for _ in 0..n {
                    let Value::String(k) = get(bytes, depth + 1)? else {
                        return Err("CORE_WIRE_KEY".into());
                    };
                    if v.insert(k, get(bytes, depth + 1)?).is_some() {
                        return Err("CORE_WIRE_DUPLICATE".into());
                    }
                }
                Value::Object(v)
            }
            _ => return Err("CORE_WIRE_TAG".into()),
        })
    }
    if bytes.len() > 262144 || !bytes.starts_with(b"AEC1") {
        return Err("CORE_WIRE_VERSION".into());
    }
    let mut body = &bytes[4..];
    let value = get(&mut body, 0)?;
    if !body.is_empty() {
        return Err("CORE_WIRE_TRAILING".into());
    }
    let value: T = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if core_encode(&value)? != bytes {
        return Err("CORE_WIRE_NONCANONICAL".into());
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInboundObservationV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d16")]
    pub turn_id: Id128,
    pub observed_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub message_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub astrbot_event_identity_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub astrbot_source_digest: Digest,
    #[serde(with = "crate::hex::d32")]
    pub extractor_digest: Digest,
    pub confidence: Fixed,
    #[serde(with = "crate::hex::d32_opt")]
    pub relation_evidence_ref: Option<Digest>,
    #[serde(with = "crate::hex::d32_opt")]
    pub session_evidence_ref: Option<Digest>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInboundAppraisalReservationV1 {
    pub daily_token_limit: u32,
    pub reserved_tokens: u32,
    #[serde(with = "crate::hex::d32")]
    pub provider_digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitCoreInboundV1 {
    pub schema_version: u16,
    pub observation: CoreInboundObservationV1,
    pub appraisal: Option<CoreInboundAppraisalReservationV1>,
}
impl CommitCoreInboundV1 {
    pub fn validate_v1(&self) -> Result<(), &'static str> {
        let o = &self.observation;
        let p = core_persona_digest(&o.scope);
        if self.schema_version != 1
            || o.schema_version != 1
            || !core_scope_valid(&o.scope)
            || o.observed_at_utc_ms == 0
            || o.observed_at_utc_ms > i64::MAX as u64
            || [
                o.message_digest,
                o.astrbot_event_identity_digest,
                o.astrbot_source_digest,
                o.extractor_digest,
            ]
            .contains(&[0; 32])
            || !(0..=1_000_000).contains(&o.confidence.raw())
        {
            return Err("INVALID_CORE_INBOUND");
        }
        if o.turn_id
            != core_id(
                b"ae.core-inbound.turn-id.v1",
                &[&p, &o.astrbot_event_identity_digest],
            )
        {
            return Err("INVALID_TURN_ID");
        }
        if o.operation_id != core_id(b"ae.core-inbound.operation-id.v1", &[&p, &o.turn_id]) {
            return Err("INVALID_OPERATION_ID");
        }
        if self.appraisal.as_ref().is_some_and(|a| {
            a.reserved_tokens < 768
                || a.reserved_tokens > SEMANTIC_APPRAISAL_DAILY_TOKEN_MAX_V1
                || a.daily_token_limit > SEMANTIC_APPRAISAL_DAILY_TOKEN_MAX_V1
                || a.provider_digest == [0; 32]
        }) {
            return Err("INVALID_APPRAISAL_RESERVATION");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitCoreDeliveryOutcomeV1 {
    pub schema_version: u16,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d16")]
    pub turn_id: Id128,
    #[serde(with = "crate::hex::d16")]
    pub inbound_operation_id: Id128,
    pub delivered: bool,
    pub observed_at_utc_ms: u64,
    #[serde(with = "crate::hex::d32")]
    pub visible_action_digest: Digest,
}
impl CommitCoreDeliveryOutcomeV1 {
    pub fn validate_v1(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || !core_scope_valid(&self.scope)
            || self.turn_id == [0; 16]
            || self.inbound_operation_id == [0; 16]
            || self.observed_at_utc_ms == 0
            || self.observed_at_utc_ms > i64::MAX as u64
            || self.visible_action_digest == [0; 32]
        {
            return Err("INVALID_CORE_DELIVERY");
        }
        if self.operation_id
            != core_id(
                b"ae.core-delivery.operation-id.v1",
                &[
                    &core_persona_digest(&self.scope),
                    &self.turn_id,
                    &self.inbound_operation_id,
                ],
            )
        {
            return Err("INVALID_OPERATION_ID");
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreEventReceiptV1 {
    pub schema_version: u16,
    pub scope: PersonaScopeRef,
    #[serde(with = "crate::hex::d16")]
    pub operation_id: Id128,
    #[serde(with = "crate::hex::d16")]
    pub turn_id: Id128,
    #[serde(with = "crate::hex::d32")]
    pub request_digest: Digest,
    #[serde(with = "crate::hex::d16")]
    pub event_id: Id128,
    pub event_bytes: Vec<u8>,
    pub transition: TransitionReceipt,
    #[serde(with = "crate::hex::d32")]
    pub clock_head_digest: Digest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreInitialAppraisalDispositionV1 {
    NotRequested,
    Claimed,
    BudgetExhausted,
    CapacityDeferred,
    RetryExpiredOrUnknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInboundInitialReceiptV1 {
    pub event: CoreEventReceiptV1,
    pub disposition: CoreInitialAppraisalDispositionV1,
    pub provider_authority_granted_initial: bool,
    pub challenge: Option<PerceptionChallengeV1>,
    pub capacity_reason: Option<SemanticAppraisalCapacityReasonV1>,
    pub budget: Option<SemanticAppraisalBudgetReceiptV1>,
    pub reply_affect: Option<ReplyAffectV1>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInboundCommitOutcomeV1 {
    pub commit_status: CoreCommitStatusV1,
    pub provider_authorized_now: bool,
    pub initial_receipt: CoreInboundInitialReceiptV1,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreDeliveryCommitOutcomeV1 {
    pub commit_status: CoreCommitStatusV1,
    pub receipt: CoreEventReceiptV1,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoreCommitStatusV1 {
    Committed,
    Existing,
}
