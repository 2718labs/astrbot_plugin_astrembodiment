#![forbid(unsafe_code)]

use ae_contracts::{wire, CanonicalEvent, Digest, SettlementKind, SourceAuthority};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResidualCoordinate {
    Bond,
    Reciprocity,
    FrictionRepetition,
    FrictionInstability,
    BoundaryViolation,
    Scar,
    Repair,
    Fallibility,
    FairCorrection,
    Humiliation,
    FalseAccusation,
    OutreachRejection,
}

impl ResidualCoordinate {
    pub const ALL: [ResidualCoordinate; 12] = [
        ResidualCoordinate::Bond,
        ResidualCoordinate::Reciprocity,
        ResidualCoordinate::FrictionRepetition,
        ResidualCoordinate::FrictionInstability,
        ResidualCoordinate::BoundaryViolation,
        ResidualCoordinate::Scar,
        ResidualCoordinate::Repair,
        ResidualCoordinate::Fallibility,
        ResidualCoordinate::FairCorrection,
        ResidualCoordinate::Humiliation,
        ResidualCoordinate::FalseAccusation,
        ResidualCoordinate::OutreachRejection,
    ];

    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|coordinate| *coordinate == self)
            .expect("coordinate in ALL")
    }
}

/// Authority is the intersection of source authority and validated settlement
/// semantics. A raw source by itself never grants irreversible write authority.
pub struct AuthorityProjection;

impl AuthorityProjection {
    pub fn allows(
        source: SourceAuthority,
        settlement: SettlementKind,
        coordinate: ResidualCoordinate,
    ) -> bool {
        use ResidualCoordinate::*;
        use SettlementKind::*;
        use SourceAuthority::*;

        if matches!(
            source,
            SelfAction | SelfCritique | PlatformObserved | TimeAdvance | PersonaConfig
        ) {
            return false;
        }
        // Administrative authority owns lifecycle transactions (migration,
        // reincarnation, deletion), not arbitrary affective history. It therefore
        // has no direct residual coordinate in the ordinary plasticity path.
        if source == AdminAction {
            return false;
        }

        match settlement {
            ExplicitAcceptance if source == ExplicitFeedback => {
                matches!(coordinate, Bond | Reciprocity)
            }
            ExplicitRejection if source == ExplicitFeedback => {
                matches!(coordinate, BoundaryViolation | OutreachRejection)
            }
            RepairAcknowledged if source == ExplicitFeedback => coordinate == Repair,
            ConfirmedSelfError if source == VerifierResult => {
                matches!(coordinate, Fallibility | FairCorrection)
            }
            RejectedChallenge if source == VerifierResult => coordinate == FalseAccusation,
            VerifiedBoundaryViolation if matches!(source, VerifierResult | ExplicitFeedback) => {
                matches!(coordinate, BoundaryViolation | Scar | Humiliation)
            }
            ConfirmedFrictionPattern if matches!(source, VerifierResult | ExplicitFeedback) => {
                matches!(coordinate, FrictionRepetition | FrictionInstability)
            }
            StrongContinuation if source == ExplicitFeedback => coordinate == Reciprocity,
            ToolResult | DeliveryTerminal | AmbiguousObservation => false,
            _ => false,
        }
    }

    /// Residual-coordinate allowance bitmap for a (source, settlement) pair.
    /// Bit i corresponds to ResidualCoordinate::ALL[i].
    pub fn allowance_bitmap(source: SourceAuthority, settlement: SettlementKind) -> [u8; 2] {
        let mut bitmap = 0u16;
        for coordinate in ResidualCoordinate::ALL {
            if Self::allows(source, settlement, coordinate) {
                bitmap |= 1 << coordinate.index();
            }
        }
        bitmap.to_le_bytes()
    }
}

/// Canonical authority digest for an event: the source authority plus the
/// residual allowance bitmap the lattice would grant. SELF_ACTION,
/// SELF_CRITIQUE, PLATFORM_OBSERVED, TIME_ADVANCE, PERSONA_CONFIG and
/// AdminAction always produce an all-zero bitmap.
pub fn authority_projection_digest(event: &CanonicalEvent) -> Digest {
    let source = event.authority();
    let bitmap = match event {
        CanonicalEvent::SettlementEvidence(evidence) => {
            AuthorityProjection::allowance_bitmap(evidence.source, evidence.kind)
        }
        _ => [0u8; 2],
    };
    wire::domain_hash(
        wire::AUTHORITY_DOMAIN,
        &[&[wire::source_authority_code(source)], &bitmap],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ae_contracts::{
        CausalRef, DeliveryOutcome, ScopeRef, SelfActionCandidate, SettlementEvidence, TimeAdvance,
    };
    use ae_fixed::Fixed;

    fn scope() -> ScopeRef {
        ScopeRef {
            bot_token: [1; 16],
            persona_token: [2; 16],
            relation_token: None,
            session_token: [3; 16],
        }
    }

    fn causal() -> CausalRef {
        CausalRef {
            turn_id: [4; 16],
            action_id: None,
            delivery_id: None,
            claim_id: None,
            base_revision: 0,
        }
    }

    #[test]
    fn self_action_has_zero_residual_authority_for_every_coordinate() {
        for coordinate in ResidualCoordinate::ALL {
            for settlement in [
                SettlementKind::ExplicitAcceptance,
                SettlementKind::ExplicitRejection,
                SettlementKind::RepairAcknowledged,
                SettlementKind::ConfirmedSelfError,
                SettlementKind::RejectedChallenge,
                SettlementKind::VerifiedBoundaryViolation,
                SettlementKind::ConfirmedFrictionPattern,
                SettlementKind::ToolResult,
                SettlementKind::DeliveryTerminal,
                SettlementKind::StrongContinuation,
                SettlementKind::AmbiguousObservation,
            ] {
                assert!(
                    !AuthorityProjection::allows(
                        SourceAuthority::SelfAction,
                        settlement,
                        coordinate
                    ),
                    "{settlement:?} {coordinate:?}"
                );
            }
        }
    }

    #[test]
    fn self_critique_platform_time_and_admin_have_zero_authority() {
        for source in [
            SourceAuthority::SelfCritique,
            SourceAuthority::PlatformObserved,
            SourceAuthority::TimeAdvance,
            SourceAuthority::AdminAction,
            SourceAuthority::PersonaConfig,
        ] {
            for coordinate in ResidualCoordinate::ALL {
                for settlement in [
                    SettlementKind::ExplicitAcceptance,
                    SettlementKind::ToolResult,
                ] {
                    assert!(!AuthorityProjection::allows(source, settlement, coordinate));
                }
            }
        }
    }

    #[test]
    fn allowed_lattice_matches_the_matrix() {
        assert!(AuthorityProjection::allows(
            SourceAuthority::ExplicitFeedback,
            SettlementKind::ExplicitAcceptance,
            ResidualCoordinate::Bond
        ));
        assert!(AuthorityProjection::allows(
            SourceAuthority::ExplicitFeedback,
            SettlementKind::ExplicitAcceptance,
            ResidualCoordinate::Reciprocity
        ));
        assert!(!AuthorityProjection::allows(
            SourceAuthority::ExplicitFeedback,
            SettlementKind::ExplicitAcceptance,
            ResidualCoordinate::Scar
        ));
        assert!(AuthorityProjection::allows(
            SourceAuthority::VerifierResult,
            SettlementKind::ConfirmedSelfError,
            ResidualCoordinate::Fallibility
        ));
        assert!(!AuthorityProjection::allows(
            SourceAuthority::VerifierResult,
            SettlementKind::ConfirmedSelfError,
            ResidualCoordinate::Bond
        ));
        assert!(!AuthorityProjection::allows(
            SourceAuthority::PlatformObserved,
            SettlementKind::DeliveryTerminal,
            ResidualCoordinate::Bond
        ));
    }

    #[test]
    fn authority_digest_is_zero_bitmap_for_non_settlement_events() {
        let stimulus = CanonicalEvent::DeliveryOutcome(DeliveryOutcome {
            event_id: [5; 16],
            scope: scope(),
            causal: causal(),
            delivered: true,
            visible_action_digest: [6; 32],
            delivered_at_ms: 1,
        });
        let digest = authority_projection_digest(&stimulus);
        assert_ne!(digest, [0; 32]);

        let settlement = CanonicalEvent::SettlementEvidence(SettlementEvidence {
            settlement_id: [7; 16],
            scope: scope(),
            causal: causal(),
            kind: SettlementKind::ExplicitAcceptance,
            source: SourceAuthority::ExplicitFeedback,
            confidence: Fixed::ONE,
            evidence_level: 1,
            evidence_digest: [8; 32],
            observed_at_ms: 1,
        });
        let settlement_digest = authority_projection_digest(&settlement);
        assert_ne!(settlement_digest, digest);
    }

    #[test]
    fn self_action_event_digest_is_stable() {
        let candidate = CanonicalEvent::SelfActionCandidate(SelfActionCandidate {
            event_id: [9; 16],
            scope: scope(),
            causal: causal(),
            visible_action_digest: [10; 32],
            claims: vec![],
        });
        let first = authority_projection_digest(&candidate);
        let second = authority_projection_digest(&candidate);
        assert_eq!(first, second);

        let advance = CanonicalEvent::TimeAdvance(TimeAdvance {
            event_id: [11; 16],
            scope: scope(),
            elapsed_ms: 5,
        });
        let advance_digest = authority_projection_digest(&advance);
        assert_ne!(advance_digest, first);
        assert_eq!(advance_digest, authority_projection_digest(&advance));
    }
}
