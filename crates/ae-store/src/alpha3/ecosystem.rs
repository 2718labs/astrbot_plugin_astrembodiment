#![allow(dead_code)]

use crate::{Store, StoreError};
use ae_contracts::{
    wire, CanonicalEvent, CapabilityGrantStateV1, Digest, EcosystemCapabilityGrantV1,
    EcosystemCapabilityV1, EcosystemProposalKindV1,
    EcosystemProposalV1, Id128, WorldLayerV1,
};
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest as ShaDigest, Sha256};

const MAX_BODY_BYTES: usize = 256 * 1024;
const OBSERVATION_TTL_MS: u64 = 5 * 60 * 1_000;

fn conflict(code: &str, detail: &str) -> StoreError {
    StoreError::AutonomyConflict(format!("ALPHA3_{code}::{detail}"))
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, StoreError> {
    let body = serde_json::to_string(value)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    if body.len() > MAX_BODY_BYTES {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "ecosystem body exceeds 256 KiB",
        ));
    }
    Ok(body)
}

fn parse<T: serde::de::DeserializeOwned>(body: Option<String>) -> Result<T, StoreError> {
    let body = body.ok_or_else(|| {
        conflict(
            "PROJECTION_INCOMPLETE",
            "ecosystem body exceeds its storage bound",
        )
    })?;
    serde_json::from_str(&body).map_err(|error| StoreError::AutonomyConflict(error.to_string()))
}

fn sql_u64(value: u64, field: &str) -> Result<i64, StoreError> {
    value.try_into().map_err(|_| {
        conflict(
            "PROJECTION_INCOMPLETE",
            &format!("{field} exceeds SQLite integer range"),
        )
    })
}

fn capability_code(value: EcosystemCapabilityV1) -> &'static [u8] {
    match value {
        EcosystemCapabilityV1::ObserveLivedState => b"observe_lived_state",
        EcosystemCapabilityV1::ProposeExternalObservation => b"propose_external_observation",
        EcosystemCapabilityV1::ProposeActivity => b"propose_activity",
        EcosystemCapabilityV1::ProposeGoalProgress => b"propose_goal_progress",
    }
}

fn proposal_kind_code(value: EcosystemProposalKindV1) -> &'static str {
    match value {
        EcosystemProposalKindV1::ExternalObservation => "external_observation",
        EcosystemProposalKindV1::ActivityOffer => "activity_offer",
        EcosystemProposalKindV1::GoalProgressEvidence => "goal_progress_evidence",
    }
}

fn world_layer_code(value: WorldLayerV1) -> &'static str {
    match value {
        WorldLayerV1::ExternalObserved => "external_observed",
        WorldLayerV1::PersonaNearReal => "persona_near_real",
        WorldLayerV1::DeclaredFantasy => "declared_fantasy",
    }
}

fn grant_state_code(value: CapabilityGrantStateV1) -> &'static str {
    match value {
        CapabilityGrantStateV1::Active => "active",
        CapabilityGrantStateV1::Revoked => "revoked",
        CapabilityGrantStateV1::Expired => "expired",
    }
}

fn sha256_framed(domain: &[u8], fields: &[&[u8]]) -> Digest {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update([0]);
    for field in fields {
        hasher.update((field.len() as u64).to_le_bytes());
        hasher.update(field);
    }
    hasher.finalize().into()
}

pub(crate) fn ecosystem_capability_digest_v1(grant: &EcosystemCapabilityGrantV1) -> Digest {
    let mut capabilities = grant.capabilities.iter().copied().collect::<Vec<_>>();
    capabilities.sort();
    let encoded = capabilities
        .into_iter()
        .flat_map(|capability| {
            let value = capability_code(capability);
            let mut framed = (value.len() as u64).to_le_bytes().to_vec();
            framed.extend_from_slice(value);
            framed
        })
        .collect::<Vec<_>>();
    sha256_framed(
        b"ae.ecosystem.capability.v1",
        &[
            &grant.plugin_instance_digest,
            &grant.plugin_name_digest,
            &grant.plugin_version_digest,
            &grant.manifest_digest,
            &encoded,
        ],
    )
}

pub(crate) fn ecosystem_observation_id_v1(
    persona_scope: &Digest,
    plugin_instance_digest: &Digest,
    capability_digest: &Digest,
    canonical_revision: u64,
) -> Id128 {
    let revision = canonical_revision.to_le_bytes();
    wire::domain_hash(
        b"ae.ecosystem.observation-id.v1",
        &[
            persona_scope,
            plugin_instance_digest,
            capability_digest,
            &revision,
        ],
    )[..16]
        .try_into()
        .expect("digest prefix")
}

pub(crate) fn ecosystem_proposal_source_digest_v1(
    proposal: &EcosystemProposalV1,
) -> Result<Digest, StoreError> {
    let payload = serde_json::to_vec(&proposal.payload)
        .map_err(|error| StoreError::AutonomyConflict(error.to_string()))?;
    let valid_from = proposal.valid_from_utc_ms.to_le_bytes();
    let expires = proposal.expires_at_utc_ms.to_le_bytes();
    Ok(sha256_framed(
        b"ae.ecosystem.proposal-source.v1",
        &[
            proposal_kind_code(proposal.kind).as_bytes(),
            world_layer_code(proposal.world_layer).as_bytes(),
            &valid_from,
            &expires,
            &payload,
        ],
    ))
}

pub(crate) fn ecosystem_proposal_semantic_digest_v1(
    proposal: &EcosystemProposalV1,
) -> Result<Digest, StoreError> {
    let revision = proposal.expected_canonical_revision.to_le_bytes();
    Ok(sha256_framed(
        b"ae.ecosystem.proposal-semantic.v1",
        &[
            &proposal.plugin_instance_digest,
            &proposal.capability_digest,
            &proposal.observation_id,
            &revision,
            proposal_kind_code(proposal.kind).as_bytes(),
            world_layer_code(proposal.world_layer).as_bytes(),
            &proposal.source_digest,
        ],
    ))
}

fn latest_grant(
    conn: &Connection,
    plugin_instance_digest: &Digest,
) -> Result<Option<EcosystemCapabilityGrantV1>, StoreError> {
    conn.query_row(
        "SELECT CASE WHEN length(CAST(body_json AS BLOB))<=?2 THEN body_json END
         FROM ecosystem_capability_grant
         WHERE plugin_instance_digest=?1 ORDER BY revision DESC LIMIT 1",
        params![plugin_instance_digest.to_vec(), MAX_BODY_BYTES as i64],
        |row| row.get::<_, Option<String>>(0),
    )
    .optional()?
    .map(parse)
    .transpose()
}

fn live_grant(
    conn: &Connection,
    plugin_instance_digest: &Digest,
    capability_digest: &Digest,
    at_utc_ms: u64,
) -> Result<EcosystemCapabilityGrantV1, StoreError> {
    let grant = latest_grant(conn, plugin_instance_digest)?
        .ok_or_else(|| conflict("CAPABILITY_DENIED", "ecosystem grant is unavailable"))?;
    if grant.capability_digest != *capability_digest
        || grant.state != CapabilityGrantStateV1::Active
        || grant
            .valid_until_utc_ms
            .is_some_and(|expires| at_utc_ms >= expires)
    {
        return Err(conflict(
            "CAPABILITY_DENIED",
            "ecosystem grant is inactive or expired",
        ));
    }
    Ok(grant)
}

fn proposal_capability(kind: EcosystemProposalKindV1) -> EcosystemCapabilityV1 {
    match kind {
        EcosystemProposalKindV1::ExternalObservation => {
            EcosystemCapabilityV1::ProposeExternalObservation
        }
        EcosystemProposalKindV1::ActivityOffer => EcosystemCapabilityV1::ProposeActivity,
        EcosystemProposalKindV1::GoalProgressEvidence => EcosystemCapabilityV1::ProposeGoalProgress,
    }
}

fn locate_observation_persona(
    conn: &Connection,
    proposal: &EcosystemProposalV1,
) -> Result<Digest, StoreError> {
    let mut statement =
        conn.prepare("SELECT persona_scope FROM lived_day_state ORDER BY persona_scope")?;
    let scopes = statement
        .query_map([], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for raw in scopes {
        let Ok(persona_scope) = <Vec<u8> as TryInto<Digest>>::try_into(raw) else {
            return Err(conflict(
                "PROJECTION_INCOMPLETE",
                "invalid lived persona scope",
            ));
        };
        if ecosystem_observation_id_v1(
            &persona_scope,
            &proposal.plugin_instance_digest,
            &proposal.capability_digest,
            proposal.expected_canonical_revision,
        ) == proposal.observation_id
        {
            return Ok(persona_scope);
        }
    }
    Err(conflict(
        "SCOPE_MISMATCH",
        "observation does not bind a lived persona",
    ))
}

fn persona_local_minute_at(
    conn: &Connection,
    persona_scope: &Digest,
    observed_at_utc_ms: u64,
) -> Result<u16, StoreError> {
    let event_bytes = conn
        .query_row(
            "SELECT event_bytes FROM journal
             WHERE scope_digest=?1 AND event_kind='time_advance'
             ORDER BY logical_revision DESC LIMIT 1",
            params![persona_scope.to_vec()],
            |row| row.get::<_, Vec<u8>>(0),
        )
        .optional()?
        .ok_or_else(|| conflict("PROJECTION_INCOMPLETE", "frozen persona time unavailable"))?;
    let event = wire::decode_event(&event_bytes)
        .map_err(|_| conflict("PROJECTION_INCOMPLETE", "frozen persona time is invalid"))?;
    let CanonicalEvent::TimeAdvance(event) = event else {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "frozen persona time source is not a time advance",
        ));
    };
    if observed_at_utc_ms < event.frozen.effective_now_utc_ms
        || event
            .frozen
            .next_timezone_transition_utc_ms
            .is_some_and(|transition| observed_at_utc_ms >= transition)
    {
        return Err(conflict(
            "PROJECTION_INCOMPLETE",
            "observation is outside the frozen timezone interval",
        ));
    }
    let elapsed_minutes =
        observed_at_utc_ms.saturating_sub(event.frozen.effective_now_utc_ms) / 60_000;
    Ok(((u64::from(event.frozen.persona_local_minute) + elapsed_minutes) % 1_440) as u16)
}

impl Store {

}
