use ae_contracts::{wire, DurableIntentionV1, IntentionStateV1};
use ae_fixed::Fixed;
use ae_renorm::WorkspaceWinnerV1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndogenousSignalsV1 {
    pub affiliation_need: Fixed,
    pub unfinished_topic_salience: Fixed,
    pub social_energy: Fixed,
    pub repetition_penalty: Fixed,
}

pub fn form_endogenous_candidate(
    winner: &WorkspaceWinnerV1,
    signals: &EndogenousSignalsV1,
    now_utc_ms: u64,
) -> Option<DurableIntentionV1> {
    const THRESHOLD_RAW: i128 = 550_000;
    let weighted = i128::from(signals.affiliation_need.raw()) * 45
        + i128::from(signals.unfinished_topic_salience.raw()) * 35
        + i128::from(signals.social_energy.raw()) * 20;
    let score_raw = weighted / 100 - i128::from(signals.repetition_penalty.raw());
    if score_raw < THRESHOLD_RAW {
        return None;
    }
    let score = Fixed::from_raw(i64::try_from(score_raw).unwrap_or(i64::MAX))
        .clamp(Fixed::ZERO, Fixed::ONE);
    let semantic = wire::domain_hash(
        b"ae.endogenous.relationship-connection.v1",
        &[
            &[winner.token_index],
            &signals.affiliation_need.encode(),
            &signals.unfinished_topic_salience.encode(),
            &signals.social_energy.encode(),
            &now_utc_ms.to_le_bytes(),
        ],
    );
    Some(DurableIntentionV1 {
        schema_version: 1,
        intention_id: semantic[..16].try_into().expect("digest prefix"),
        persona_scope: [0; 32],
        relation_scope: [0; 32],
        state: IntentionStateV1::Ready,
        action_class: "relationship_connection".into(),
        salience: score,
        urgency: signals.affiliation_need.clamp(Fixed::ZERO, Fixed::ONE),
        confidence: winner.score.clamp(Fixed::ZERO, Fixed::ONE),
        created_at_utc_ms: now_utc_ms,
        not_before_utc_ms: now_utc_ms,
        expires_at_utc_ms: now_utc_ms.saturating_add(86_400_000),
        externalization_attempts: 0,
        semantic_idempotency_digest: semantic,
        workspace_mapping_digest: [0; 32],
        workspace_residual: Fixed::ZERO,
        source_event_ids: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationship_connection_candidate_is_deterministic_and_non_random() {
        let winner = WorkspaceWinnerV1 {
            token_index: 3,
            score: Fixed::from_raw(700_000),
            token: [Fixed::ZERO; 8],
        };
        let signals = EndogenousSignalsV1 {
            affiliation_need: Fixed::from_raw(900_000),
            unfinished_topic_salience: Fixed::from_raw(700_000),
            social_energy: Fixed::from_raw(800_000),
            repetition_penalty: Fixed::ZERO,
        };
        let first = form_endogenous_candidate(&winner, &signals, 1_000).unwrap();
        let second = form_endogenous_candidate(&winner, &signals, 1_000).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.action_class, "relationship_connection");
        assert_eq!(first.expires_at_utc_ms, 86_401_000);
    }

    #[test]
    fn weak_or_repetitive_signal_forms_no_intention() {
        let winner = WorkspaceWinnerV1 {
            token_index: 0,
            score: Fixed::ZERO,
            token: [Fixed::ZERO; 8],
        };
        let signals = EndogenousSignalsV1 {
            affiliation_need: Fixed::from_raw(100_000),
            unfinished_topic_salience: Fixed::ZERO,
            social_energy: Fixed::from_raw(500_000),
            repetition_penalty: Fixed::from_raw(500_000),
        };
        assert!(form_endogenous_candidate(&winner, &signals, 1_000).is_none());
    }
}
