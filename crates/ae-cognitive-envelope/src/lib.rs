#![forbid(unsafe_code)]

//! Fail-closed contract assembly for the R7 `CognitiveEnvelopeV1` boundary.
//!
//! This crate is deliberately not an organism-state projector, provider renderer, or
//! `ActionRealization` producer. Callers must supply nine bounded source capsules, typed R7
//! source products, and explicit projection preconditions. The core only validates their
//! bindings and assembles the typed envelope and certificate deterministically.
//!
//! The JSON schema makes `relation` and `exact_anchors` optional, while A49 makes both
//! mandatory. This runtime contract follows the stricter A49 rule: both fields are
//! non-optional members of [`CognitiveEnvelopeV1`] and their source capsules are required
//! members of [`ProjectionInput`].
//!
//! Raw 16K organism state, neural arrays, raw Continuum-KV banks, Persona/user text, and raw
//! provider payloads are intentionally not accepted by the typed source seams. A typed
//! `ActionRealizationV1` enters only as the post-output efference copy; it is never collapsed
//! into the pre-output action contract.

use ae_action_contract::{ActionContractV1, ActionRealizationV1, ACTION_CONTRACT_SCHEMA_V1};
use ae_contracts::{wire, Digest, Id128};
use ae_efference_copy::EfferenceCopyV1;
use ae_epistemic_state::EpistemicProjectionV1;
use ae_genesis::IdentityConstitutionV1;
use ae_soma::{
    compile_subjective_present_v1, SomaClassificationIngressV1, SomaErrorV1, SomaStateV1,
};
use ae_subjective_present::{SubjectivePresentProjectionV1, SubjectivePresentV1};
use serde::{Serialize, Serializer};
use serde_json::Value;
use thiserror::Error;

pub const COGNITIVE_ENVELOPE_SCHEMA_V1: &str = "astrembodiment.cognitive-envelope.v1";
/// Closed typed envelope supplied before provider output exists. It carries an
/// evaluated R7 action contract, never a post-output realization or efference
/// copy.
pub const PRE_OUTPUT_COGNITIVE_ENVELOPE_SCHEMA_V1: &str =
    "astrembodiment.cognitive-envelope.pre-output.v1";
pub const MAX_PROJECTION_TOKENS: u32 = 3_200;
pub const MAX_SUBJECTIVE_PRESENT_ITEMS: u16 = 32;
pub const MAX_EXACT_ANCHOR_ITEMS: u16 = 64;
pub const MAX_AFFORDANCE_ITEMS: u16 = 64;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProjectionErrorV1 {
    #[error("{field} must not be empty")]
    EmptyText { field: &'static str },
    #[error("{field} exceeds its character bound ({actual_chars} > {max_chars})")]
    TextTooLong {
        field: &'static str,
        max_chars: u32,
        actual_chars: u32,
    },
    #[error("bounded projection text must be a canonical token, not raw payload text")]
    NonCanonicalText,
    #[error("{field} exceeds its item bound ({actual_items} > {max_items})")]
    TooManyItems {
        field: &'static str,
        max_items: u16,
        actual_items: usize,
    },
    #[error("missing required projected content: {field}")]
    MissingRequiredContent { field: &'static str },
    #[error("zero digest is not valid for {field}")]
    ZeroDigest { field: &'static str },
    #[error("zero identifier is not valid for {field}")]
    ZeroId { field: &'static str },
    #[error("wrong source kind for {field}: expected {expected:?}, got {actual:?}")]
    WrongSourceKind {
        field: &'static str,
        expected: ProjectionSourceKindV1,
        actual: ProjectionSourceKindV1,
    },
    #[error("unknown projection source kind")]
    UnknownSourceKind,
    #[error("identity capsule digest does not match the typed constitution")]
    IdentityConstitutionDigestMismatch,
    #[error("action contract capsule digest does not match the typed contract")]
    ActionContractDigestMismatch,
    #[error("typed action contract encoding is not the expected closed shape")]
    ActionContractEncodingInvalid,
    #[error("action contract turn binding does not match the organism snapshot")]
    ActionTurnBindingMismatch,
    #[error("action contract base revision does not match the organism snapshot revision")]
    ActionBaseRevisionMismatch,
    #[error("action contract source-state digest does not match the organism snapshot")]
    ActionSourceStateDigestMismatch,
    #[error("action contract identity digest does not match the typed constitution")]
    ActionIdentityConstitutionDigestMismatch,
    #[error("SOMA capsule digest does not match the typed SOMA state")]
    SomaStateDigestMismatch,
    #[error("SOMA state digest does not match the organism snapshot")]
    SomaSourceStateDigestMismatch,
    #[error("SOMA state does not declare a committed organism source-state binding")]
    SomaSourceStateBindingMissing,
    #[error("SOMA revision does not match the organism snapshot revision")]
    SomaSourceRevisionMismatch,
    #[error("SOMA identity digest does not match the typed constitution")]
    SomaIdentityConstitutionDigestMismatch,
    #[error("SOMA subjective ingress state digest does not match the typed SOMA state")]
    SomaSubjectiveStateMismatch,
    #[error("SOMA subjective ingress revision does not match the typed SOMA state")]
    SomaSubjectiveRevisionMismatch,
    #[error("SOMA subjective ingress identity does not match the typed SOMA state")]
    SomaSubjectiveIdentityMismatch,
    #[error("SOMA subjective projection was rejected")]
    SomaSubjectiveProjectionRejected,
    #[error("epistemic turn does not match the organism snapshot turn")]
    EpistemicTurnBindingMismatch,
    #[error("epistemic state digest does not match the organism snapshot")]
    EpistemicSourceStateDigestMismatch,
    #[error("epistemic revision does not match the organism snapshot revision")]
    EpistemicRevisionMismatch,
    #[error("epistemic identity digest does not match the typed constitution")]
    EpistemicIdentityConstitutionDigestMismatch,
    #[error("epistemic projection digest is invalid")]
    EpistemicProjectionDigestInvalid,
    #[error("{field} scope does not match the organism turn scope")]
    ScopeBindingMismatch { field: &'static str },
    #[error("action realization id does not match the selected action contract")]
    ActionRealizationActionIdMismatch,
    #[error("action realization contract digest does not match the selected action contract")]
    ActionRealizationContractDigestMismatch,
    #[error("efference capsule digest does not match the typed action realization")]
    ActionRealizationDigestMismatch,
    #[error("efference copy action id does not match the selected action contract")]
    EfferenceCopyActionIdMismatch,
    #[error("efference copy contract digest does not match the selected action contract")]
    EfferenceCopyContractDigestMismatch,
    #[error("efference copy realization digest does not match the direct action realization")]
    EfferenceCopyRealizationDigestMismatch,
    #[error("selected action contract is expired at projection time")]
    ActionContractExpired {
        projected_at_ms: u64,
        expires_at_ms: u64,
    },
    #[error("exact-anchor residual must be zero, got {residual}")]
    ExactAnchorResidualNonzero { residual: u64 },
    #[error("scope residual must be zero, got {residual}")]
    ScopeResidualNonzero { residual: u64 },
    #[error("disclosure residual must be zero, got {residual}")]
    DisclosureResidualNonzero { residual: u64 },
    #[error("action-sensitivity residual {residual} exceeds caller bound {bound}")]
    ActionSensitivityResidualExceeded { residual: u64, bound: u64 },
    #[error("projection token use {used} exceeds hard ceiling {hard_ceiling}")]
    TokenBudgetHardCeilingExceeded { used: u32, hard_ceiling: u32 },
    #[error("provider token limit {limit} exceeds hard ceiling {hard_ceiling}")]
    ProviderTokenLimitHardCeilingExceeded { limit: u32, hard_ceiling: u32 },
    #[error("projection token use {used} exceeds provider limit {provider_limit}")]
    ProviderTokenBudgetExceeded { used: u32, provider_limit: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedTextV1 {
    value: String,
    max_chars: u32,
}

impl BoundedTextV1 {
    pub fn new(value: String, max_chars: u32) -> Result<Self, ProjectionErrorV1> {
        let actual_chars = value.chars().count();
        if actual_chars > max_chars as usize {
            return Err(ProjectionErrorV1::TextTooLong {
                field: "bounded_text",
                max_chars,
                actual_chars: saturating_u32(actual_chars),
            });
        }
        if !value.is_empty() && !is_canonical_token(&value) {
            return Err(ProjectionErrorV1::NonCanonicalText);
        }
        Ok(Self { value, max_chars })
    }

    pub fn as_str(&self) -> &str {
        &self.value
    }

    pub fn max_chars(&self) -> u32 {
        self.max_chars
    }

    fn require_non_empty(&self, field: &'static str) -> Result<(), ProjectionErrorV1> {
        if self.value.is_empty() {
            return Err(ProjectionErrorV1::EmptyText { field });
        }
        Ok(())
    }

    fn require_schema_max(
        &self,
        field: &'static str,
        max_chars: u32,
    ) -> Result<(), ProjectionErrorV1> {
        let actual_chars = self.value.chars().count();
        if actual_chars > max_chars as usize {
            return Err(ProjectionErrorV1::TextTooLong {
                field,
                max_chars,
                actual_chars: saturating_u32(actual_chars),
            });
        }
        Ok(())
    }
}

impl Serialize for BoundedTextV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.value)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedListV1<T> {
    items: Vec<T>,
    max_items: u16,
}

impl<T> BoundedListV1<T> {
    pub fn new(items: Vec<T>, max_items: u16) -> Result<Self, ProjectionErrorV1> {
        if items.len() > usize::from(max_items) {
            return Err(ProjectionErrorV1::TooManyItems {
                field: "bounded_list",
                max_items,
                actual_items: items.len(),
            });
        }
        Ok(Self { items, max_items })
    }

    pub fn as_slice(&self) -> &[T] {
        &self.items
    }

    pub fn max_items(&self) -> u16 {
        self.max_items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T: Serialize> Serialize for BoundedListV1<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.items.serialize(serializer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapsuleFieldV1 {
    name: BoundedTextV1,
    value: BoundedTextV1,
    source_digest: Digest,
}

impl CapsuleFieldV1 {
    pub fn new(
        name: BoundedTextV1,
        value: BoundedTextV1,
        source_digest: Digest,
    ) -> Result<Self, ProjectionErrorV1> {
        name.require_non_empty("capsule_field.name")?;
        value.require_non_empty("capsule_field.value")?;
        require_digest(&source_digest, "capsule_field.source_digest")?;
        Ok(Self {
            name,
            value,
            source_digest,
        })
    }
}

macro_rules! required_field_object {
    ($name:ident, $field:literal) => {
        #[derive(Clone, Debug, PartialEq, Eq, Serialize)]
        pub struct $name {
            fields: BoundedListV1<CapsuleFieldV1>,
        }

        impl $name {
            pub fn new(fields: BoundedListV1<CapsuleFieldV1>) -> Result<Self, ProjectionErrorV1> {
                if fields.is_empty() {
                    return Err(ProjectionErrorV1::MissingRequiredContent { field: $field });
                }
                Ok(Self { fields })
            }

            pub fn fields(&self) -> &[CapsuleFieldV1] {
                self.fields.as_slice()
            }
        }
    };
}

required_field_object!(EpistemicsV1, "epistemics");
required_field_object!(PraxisV1, "praxis");
required_field_object!(RelationV1, "relation");

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TurnV1 {
    turn_ref: BoundedTextV1,
    incarnation_ref: BoundedTextV1,
    scope_ref: BoundedTextV1,
}

impl TurnV1 {
    pub fn new(
        turn_ref: BoundedTextV1,
        incarnation_ref: BoundedTextV1,
        scope_ref: BoundedTextV1,
    ) -> Result<Self, ProjectionErrorV1> {
        validate_required_schema_text(&turn_ref, "turn.turn_ref", 128)?;
        validate_required_schema_text(&incarnation_ref, "turn.incarnation_ref", 128)?;
        validate_required_schema_text(&scope_ref, "turn.scope_ref", 128)?;
        Ok(Self {
            turn_ref,
            incarnation_ref,
            scope_ref,
        })
    }

    pub fn scope_ref(&self) -> &str {
        self.scope_ref.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrganismSnapshotRefV1 {
    snapshot_ref: BoundedTextV1,
    state_digest: Digest,
    turn_binding: Digest,
    turn_id: Id128,
    turn: TurnV1,
}

impl OrganismSnapshotRefV1 {
    pub fn new(
        snapshot_ref: BoundedTextV1,
        state_digest: Digest,
        turn_binding: Digest,
        turn_id: Id128,
        turn: TurnV1,
    ) -> Result<Self, ProjectionErrorV1> {
        snapshot_ref.require_non_empty("organism_snapshot.snapshot_ref")?;
        require_digest(&state_digest, "organism_snapshot.state_digest")?;
        require_digest(&turn_binding, "organism_snapshot.turn_binding")?;
        require_id(&turn_id, "organism_snapshot.turn_id")?;
        Ok(Self {
            snapshot_ref,
            state_digest,
            turn_binding,
            turn_id,
            turn,
        })
    }

    pub fn turn(&self) -> &TurnV1 {
        &self.turn
    }

    pub fn state_digest(&self) -> &Digest {
        &self.state_digest
    }

    pub fn turn_binding(&self) -> &Digest {
        &self.turn_binding
    }

    pub fn turn_id(&self) -> &Id128 {
        &self.turn_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CognitiveKvViewV1 {
    view_ref: BoundedTextV1,
    kv_snapshot_digest: Digest,
    subjective_present: SubjectivePresentProjectionV1,
    epistemics: EpistemicsV1,
    praxis: PraxisV1,
}

impl CognitiveKvViewV1 {
    pub fn new(
        view_ref: BoundedTextV1,
        kv_snapshot_digest: Digest,
        subjective_present: SubjectivePresentProjectionV1,
        epistemics: EpistemicsV1,
        praxis: PraxisV1,
    ) -> Result<Self, ProjectionErrorV1> {
        view_ref.require_non_empty("cognitive_kv_view.view_ref")?;
        require_digest(&kv_snapshot_digest, "cognitive_kv_view.kv_snapshot_digest")?;
        Ok(Self {
            view_ref,
            kv_snapshot_digest,
            subjective_present,
            epistemics,
            praxis,
        })
    }

    pub fn epistemics(&self) -> &EpistemicsV1 {
        &self.epistemics
    }

    pub fn praxis(&self) -> &PraxisV1 {
        &self.praxis
    }

    pub fn kv_snapshot_digest(&self) -> &Digest {
        &self.kv_snapshot_digest
    }

    pub fn subjective_present(&self) -> &SubjectivePresentProjectionV1 {
        &self.subjective_present
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactAnchorKindV1 {
    ConfirmedErrorOrCorrectedFact,
    ExplicitUserBoundaryOrConsent,
    ActiveSafetyRequirement,
    ActiveCommitmentOrCompletionObligation,
    ChallengedClaimOrAction,
    RequiredToolOrDeliveryFact,
    MustState,
    MustNotState,
    IncarnationOrPersonaBinding,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExactAnchorV1 {
    kind: ExactAnchorKindV1,
    anchor_ref: BoundedTextV1,
    exact_content: BoundedTextV1,
    source_digest: Digest,
}

impl ExactAnchorV1 {
    pub fn new(
        kind: ExactAnchorKindV1,
        anchor_ref: BoundedTextV1,
        exact_content: BoundedTextV1,
        source_digest: Digest,
    ) -> Result<Self, ProjectionErrorV1> {
        anchor_ref.require_non_empty("exact_anchor.anchor_ref")?;
        exact_content.require_non_empty("exact_anchor.exact_content")?;
        require_digest(&source_digest, "exact_anchor.source_digest")?;
        Ok(Self {
            kind,
            anchor_ref,
            exact_content,
            source_digest,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExactTurnAnchorsV1 {
    anchors: BoundedListV1<ExactAnchorV1>,
}

impl ExactTurnAnchorsV1 {
    pub fn new(anchors: BoundedListV1<ExactAnchorV1>) -> Result<Self, ProjectionErrorV1> {
        require_list_schema_max(
            &anchors,
            "exact_turn_anchors.anchors",
            MAX_EXACT_ANCHOR_ITEMS,
        )?;
        Ok(Self { anchors })
    }

    pub fn anchors(&self) -> &BoundedListV1<ExactAnchorV1> {
        &self.anchors
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationScopeV1 {
    scope_ref: BoundedTextV1,
    relation: RelationV1,
}

impl RelationScopeV1 {
    pub fn new(scope_ref: BoundedTextV1, relation: RelationV1) -> Result<Self, ProjectionErrorV1> {
        scope_ref.require_non_empty("relation_scope.scope_ref")?;
        Ok(Self {
            scope_ref,
            relation,
        })
    }

    pub fn relation(&self) -> &RelationV1 {
        &self.relation
    }

    pub fn scope_ref(&self) -> &str {
        self.scope_ref.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AffordanceV1 {
    affordance_id: BoundedTextV1,
    description: BoundedTextV1,
    authority_evidence_digest: Digest,
    policy_evidence_digest: Digest,
}

impl AffordanceV1 {
    pub fn new(
        affordance_id: BoundedTextV1,
        description: BoundedTextV1,
        authority_evidence_digest: Digest,
        policy_evidence_digest: Digest,
    ) -> Result<Self, ProjectionErrorV1> {
        affordance_id.require_non_empty("affordance.affordance_id")?;
        description.require_non_empty("affordance.description")?;
        require_digest(
            &authority_evidence_digest,
            "affordance.authority_evidence_digest",
        )?;
        require_digest(&policy_evidence_digest, "affordance.policy_evidence_digest")?;
        Ok(Self {
            affordance_id,
            description,
            authority_evidence_digest,
            policy_evidence_digest,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AffordanceCatalogV1 {
    scope_ref: BoundedTextV1,
    affordances: BoundedListV1<AffordanceV1>,
}

impl AffordanceCatalogV1 {
    pub fn new(
        scope_ref: BoundedTextV1,
        affordances: BoundedListV1<AffordanceV1>,
    ) -> Result<Self, ProjectionErrorV1> {
        scope_ref.require_non_empty("affordance_catalog.scope_ref")?;
        require_list_schema_max(
            &affordances,
            "affordance_catalog.affordances",
            MAX_AFFORDANCE_ITEMS,
        )?;
        Ok(Self {
            scope_ref,
            affordances,
        })
    }

    pub fn affordances(&self) -> &BoundedListV1<AffordanceV1> {
        &self.affordances
    }

    pub fn scope_ref(&self) -> &str {
        self.scope_ref.as_str()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderProfileV1 {
    profile_ref: BoundedTextV1,
    projection_token_limit: u32,
}

impl ProviderProfileV1 {
    pub fn new(
        profile_ref: BoundedTextV1,
        projection_token_limit: u32,
    ) -> Result<Self, ProjectionErrorV1> {
        profile_ref.require_non_empty("provider_profile.profile_ref")?;
        if projection_token_limit > MAX_PROJECTION_TOKENS {
            return Err(ProjectionErrorV1::ProviderTokenLimitHardCeilingExceeded {
                limit: projection_token_limit,
                hard_ceiling: MAX_PROJECTION_TOKENS,
            });
        }
        Ok(Self {
            profile_ref,
            projection_token_limit,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionSourceKindV1 {
    OrganismSnapshot,
    CognitiveKvView,
    ExactTurnAnchors,
    IdentityConstitution,
    RelationScope,
    ActionContract,
    AffordanceCatalog,
    ProviderProfile,
    SomaState,
}

impl ProjectionSourceKindV1 {
    pub fn parse(value: &str) -> Result<Self, ProjectionErrorV1> {
        match value {
            "organism_snapshot" => Ok(Self::OrganismSnapshot),
            "cognitive_kv_view" => Ok(Self::CognitiveKvView),
            "exact_turn_anchors" => Ok(Self::ExactTurnAnchors),
            "identity_constitution" => Ok(Self::IdentityConstitution),
            "relation_scope" => Ok(Self::RelationScope),
            "action_contract" => Ok(Self::ActionContract),
            "affordance_catalog" => Ok(Self::AffordanceCatalog),
            "provider_profile" => Ok(Self::ProviderProfile),
            "soma_state" => Ok(Self::SomaState),
            _ => Err(ProjectionErrorV1::UnknownSourceKind),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceProvenanceV1 {
    source_kind: ProjectionSourceKindV1,
    source_ref: BoundedTextV1,
    source_revision: u64,
    certification_digest: Digest,
}

impl SourceProvenanceV1 {
    pub fn new(
        source_kind: ProjectionSourceKindV1,
        source_ref: BoundedTextV1,
        source_revision: u64,
        certification_digest: Digest,
    ) -> Result<Self, ProjectionErrorV1> {
        source_ref.require_non_empty("source_provenance.source_ref")?;
        require_digest(&certification_digest, "certification_digest")?;
        Ok(Self {
            source_kind,
            source_ref,
            source_revision,
            certification_digest,
        })
    }

    pub fn source_kind(&self) -> ProjectionSourceKindV1 {
        self.source_kind
    }

    pub fn source_revision(&self) -> u64 {
        self.source_revision
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceCapsuleV1<T> {
    provenance: SourceProvenanceV1,
    content_digest: Digest,
    capsule_digest: Digest,
    value: T,
}

impl<T> SourceCapsuleV1<T> {
    pub fn new(
        provenance: SourceProvenanceV1,
        content_digest: Digest,
        value: T,
    ) -> Result<Self, ProjectionErrorV1> {
        require_digest(&content_digest, "content_digest")?;
        let revision = provenance.source_revision.to_be_bytes();
        let capsule_digest = wire::domain_hash(
            b"astr-embodiment/r7-projection-source-capsule-v1",
            &[
                source_kind_name(provenance.source_kind),
                provenance.source_ref.as_str().as_bytes(),
                &revision,
                &provenance.certification_digest,
                &content_digest,
            ],
        );
        Ok(Self {
            provenance,
            content_digest,
            capsule_digest,
            value,
        })
    }

    pub fn provenance(&self) -> &SourceProvenanceV1 {
        &self.provenance
    }

    pub fn content_digest(&self) -> &Digest {
        &self.content_digest
    }

    pub fn capsule_digest(&self) -> &Digest {
        &self.capsule_digest
    }

    pub fn value(&self) -> &T {
        &self.value
    }
}

/// The nine A49 source capsules plus direct typed R7 source products.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionInput {
    organism_snapshot: SourceCapsuleV1<OrganismSnapshotRefV1>,
    cognitive_kv_view: SourceCapsuleV1<CognitiveKvViewV1>,
    exact_turn_anchors: SourceCapsuleV1<ExactTurnAnchorsV1>,
    identity_constitution: SourceCapsuleV1<IdentityConstitutionV1>,
    relation_scope: SourceCapsuleV1<RelationScopeV1>,
    action_contract: SourceCapsuleV1<ActionContractV1>,
    affordance_catalog: SourceCapsuleV1<AffordanceCatalogV1>,
    provider_profile: SourceCapsuleV1<ProviderProfileV1>,
    soma_state: SourceCapsuleV1<SomaStateV1>,
    soma_classification_ingress: SomaClassificationIngressV1,
    epistemic_projection: EpistemicProjectionV1,
    action_realization: ActionRealizationV1,
    efference_copy: EfferenceCopyV1,
}

impl ProjectionInput {
    pub const FIELD_COUNT: usize = 9;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        organism_snapshot: SourceCapsuleV1<OrganismSnapshotRefV1>,
        cognitive_kv_view: SourceCapsuleV1<CognitiveKvViewV1>,
        exact_turn_anchors: SourceCapsuleV1<ExactTurnAnchorsV1>,
        identity_constitution: SourceCapsuleV1<IdentityConstitutionV1>,
        relation_scope: SourceCapsuleV1<RelationScopeV1>,
        action_contract: SourceCapsuleV1<ActionContractV1>,
        affordance_catalog: SourceCapsuleV1<AffordanceCatalogV1>,
        provider_profile: SourceCapsuleV1<ProviderProfileV1>,
        soma_state: SourceCapsuleV1<SomaStateV1>,
        soma_classification_ingress: SomaClassificationIngressV1,
        epistemic_projection: EpistemicProjectionV1,
        action_realization: ActionRealizationV1,
        efference_copy: EfferenceCopyV1,
    ) -> Self {
        Self {
            organism_snapshot,
            cognitive_kv_view,
            exact_turn_anchors,
            identity_constitution,
            relation_scope,
            action_contract,
            affordance_catalog,
            provider_profile,
            soma_state,
            soma_classification_ingress,
            epistemic_projection,
            action_realization,
            efference_copy,
        }
    }

    pub fn organism_snapshot(&self) -> &SourceCapsuleV1<OrganismSnapshotRefV1> {
        &self.organism_snapshot
    }

    pub fn cognitive_kv_view(&self) -> &SourceCapsuleV1<CognitiveKvViewV1> {
        &self.cognitive_kv_view
    }

    pub fn exact_turn_anchors(&self) -> &SourceCapsuleV1<ExactTurnAnchorsV1> {
        &self.exact_turn_anchors
    }

    pub fn identity_constitution(&self) -> &SourceCapsuleV1<IdentityConstitutionV1> {
        &self.identity_constitution
    }

    pub fn relation_scope(&self) -> &SourceCapsuleV1<RelationScopeV1> {
        &self.relation_scope
    }

    pub fn action_contract(&self) -> &SourceCapsuleV1<ActionContractV1> {
        &self.action_contract
    }

    pub fn affordance_catalog(&self) -> &SourceCapsuleV1<AffordanceCatalogV1> {
        &self.affordance_catalog
    }

    pub fn provider_profile(&self) -> &SourceCapsuleV1<ProviderProfileV1> {
        &self.provider_profile
    }

    pub fn soma_state(&self) -> &SourceCapsuleV1<SomaStateV1> {
        &self.soma_state
    }

    pub fn soma_classification_ingress(&self) -> &SomaClassificationIngressV1 {
        &self.soma_classification_ingress
    }

    pub fn epistemic_projection(&self) -> &EpistemicProjectionV1 {
        &self.epistemic_projection
    }

    pub fn action_realization(&self) -> &ActionRealizationV1 {
        &self.action_realization
    }

    pub fn efference_copy(&self) -> &EfferenceCopyV1 {
        &self.efference_copy
    }

    pub fn source_capsule_digests(&self) -> [Digest; Self::FIELD_COUNT] {
        [
            *self.organism_snapshot.capsule_digest(),
            *self.cognitive_kv_view.capsule_digest(),
            *self.exact_turn_anchors.capsule_digest(),
            *self.identity_constitution.capsule_digest(),
            *self.relation_scope.capsule_digest(),
            *self.action_contract.capsule_digest(),
            *self.affordance_catalog.capsule_digest(),
            *self.provider_profile.capsule_digest(),
            *self.soma_state.capsule_digest(),
        ]
    }
}

/// The nine A49 source capsules plus direct typed sources available before
/// output realization. This is intentionally separate from [`ProjectionInput`]
/// so callers cannot satisfy a pre-output path with fixture realization data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreOutputProjectionInputV1 {
    organism_snapshot: SourceCapsuleV1<OrganismSnapshotRefV1>,
    cognitive_kv_view: SourceCapsuleV1<CognitiveKvViewV1>,
    exact_turn_anchors: SourceCapsuleV1<ExactTurnAnchorsV1>,
    identity_constitution: SourceCapsuleV1<IdentityConstitutionV1>,
    relation_scope: SourceCapsuleV1<RelationScopeV1>,
    action_contract: SourceCapsuleV1<ActionContractV1>,
    affordance_catalog: SourceCapsuleV1<AffordanceCatalogV1>,
    provider_profile: SourceCapsuleV1<ProviderProfileV1>,
    soma_state: SourceCapsuleV1<SomaStateV1>,
    soma_classification_ingress: SomaClassificationIngressV1,
    epistemic_projection: EpistemicProjectionV1,
}

impl PreOutputProjectionInputV1 {
    pub const FIELD_COUNT: usize = ProjectionInput::FIELD_COUNT;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        organism_snapshot: SourceCapsuleV1<OrganismSnapshotRefV1>,
        cognitive_kv_view: SourceCapsuleV1<CognitiveKvViewV1>,
        exact_turn_anchors: SourceCapsuleV1<ExactTurnAnchorsV1>,
        identity_constitution: SourceCapsuleV1<IdentityConstitutionV1>,
        relation_scope: SourceCapsuleV1<RelationScopeV1>,
        action_contract: SourceCapsuleV1<ActionContractV1>,
        affordance_catalog: SourceCapsuleV1<AffordanceCatalogV1>,
        provider_profile: SourceCapsuleV1<ProviderProfileV1>,
        soma_state: SourceCapsuleV1<SomaStateV1>,
        soma_classification_ingress: SomaClassificationIngressV1,
        epistemic_projection: EpistemicProjectionV1,
    ) -> Self {
        Self {
            organism_snapshot,
            cognitive_kv_view,
            exact_turn_anchors,
            identity_constitution,
            relation_scope,
            action_contract,
            affordance_catalog,
            provider_profile,
            soma_state,
            soma_classification_ingress,
            epistemic_projection,
        }
    }

    pub fn source_capsule_digests(&self) -> [Digest; Self::FIELD_COUNT] {
        [
            *self.organism_snapshot.capsule_digest(),
            *self.cognitive_kv_view.capsule_digest(),
            *self.exact_turn_anchors.capsule_digest(),
            *self.identity_constitution.capsule_digest(),
            *self.relation_scope.capsule_digest(),
            *self.action_contract.capsule_digest(),
            *self.affordance_catalog.capsule_digest(),
            *self.provider_profile.capsule_digest(),
            *self.soma_state.capsule_digest(),
        ]
    }
}

/// Explicit caller-supplied certification measurements and limits.
///
/// R7 does not choose a numeric action-sensitivity threshold, so both the measured residual
/// and its bound are mandatory inputs. No hidden epsilon is introduced here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectionPreconditionsV1 {
    projected_at_ms: u64,
    exact_anchor_residual: u64,
    scope_residual: u64,
    action_sensitivity_residual: u64,
    action_sensitivity_bound: u64,
    disclosure_residual: u64,
    token_budget_used: u32,
}

impl ProjectionPreconditionsV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        projected_at_ms: u64,
        exact_anchor_residual: u64,
        scope_residual: u64,
        action_sensitivity_residual: u64,
        action_sensitivity_bound: u64,
        disclosure_residual: u64,
        token_budget_used: u32,
    ) -> Self {
        Self {
            projected_at_ms,
            exact_anchor_residual,
            scope_residual,
            action_sensitivity_residual,
            action_sensitivity_bound,
            disclosure_residual,
            token_budget_used,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ProjectionCertificateV1 {
    source_state_digest: Digest,
    soma_state_digest: Digest,
    subjective_present_digest: Digest,
    kv_snapshot_digest: Digest,
    epistemic_projection_digest: Digest,
    action_contract_digest: Digest,
    action_realization_digest: Digest,
    efference_copy_digest: Digest,
    provider_profile_digest: Digest,
    included_capsule_digests: [Digest; ProjectionInput::FIELD_COUNT],
    exact_anchor_residual: u64,
    scope_residual: u64,
    action_sensitivity_residual: u64,
    action_sensitivity_bound: u64,
    disclosure_residual: u64,
    token_budget_used: u32,
}

impl ProjectionCertificateV1 {
    pub fn source_state_digest(&self) -> &Digest {
        &self.source_state_digest
    }

    pub fn soma_state_digest(&self) -> &Digest {
        &self.soma_state_digest
    }

    pub fn subjective_present_digest(&self) -> &Digest {
        &self.subjective_present_digest
    }

    pub fn kv_snapshot_digest(&self) -> &Digest {
        &self.kv_snapshot_digest
    }

    pub fn epistemic_projection_digest(&self) -> &Digest {
        &self.epistemic_projection_digest
    }

    pub fn action_contract_digest(&self) -> &Digest {
        &self.action_contract_digest
    }

    pub fn action_realization_digest(&self) -> &Digest {
        &self.action_realization_digest
    }

    pub fn efference_copy_digest(&self) -> &Digest {
        &self.efference_copy_digest
    }

    pub fn provider_profile_digest(&self) -> &Digest {
        &self.provider_profile_digest
    }

    pub fn included_capsule_digests(&self) -> &[Digest; ProjectionInput::FIELD_COUNT] {
        &self.included_capsule_digests
    }

    pub fn action_sensitivity_bound(&self) -> u64 {
        self.action_sensitivity_bound
    }

    pub fn token_budget_used(&self) -> u32 {
        self.token_budget_used
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AgencyV1 {
    action_contract: ActionContractV1,
    action_contract_digest: Digest,
    efference_copy_digest: Digest,
}

/// Canonical provider-independent projection. Provider rendering happens later.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CognitiveEnvelopeV1 {
    schema: &'static str,
    turn: TurnV1,
    identity: IdentityConstitutionV1,
    subjective_present: SubjectivePresentProjectionV1,
    relation: RelationV1,
    epistemics: EpistemicProjectionV1,
    praxis: PraxisV1,
    affordances: BoundedListV1<AffordanceV1>,
    agency: AgencyV1,
    realization: ActionRealizationV1,
    exact_anchors: BoundedListV1<ExactAnchorV1>,
    projection_certificate: ProjectionCertificateV1,
    envelope_digest: Digest,
}

impl CognitiveEnvelopeV1 {
    pub fn schema(&self) -> &str {
        self.schema
    }

    pub fn identity(&self) -> &IdentityConstitutionV1 {
        &self.identity
    }

    pub fn subjective_present(&self) -> &[SubjectivePresentV1] {
        self.subjective_present.items()
    }

    pub fn action_contract(&self) -> &ActionContractV1 {
        &self.agency.action_contract
    }

    pub fn relation(&self) -> &RelationV1 {
        &self.relation
    }

    pub fn epistemics(&self) -> &EpistemicProjectionV1 {
        &self.epistemics
    }

    pub fn praxis(&self) -> &PraxisV1 {
        &self.praxis
    }

    pub fn affordances(&self) -> &[AffordanceV1] {
        self.affordances.as_slice()
    }

    pub fn realization(&self) -> &ActionRealizationV1 {
        &self.realization
    }

    pub fn exact_anchors(&self) -> &[ExactAnchorV1] {
        self.exact_anchors.as_slice()
    }

    pub fn projection_certificate(&self) -> &ProjectionCertificateV1 {
        &self.projection_certificate
    }

    pub fn envelope_digest(&self) -> &Digest {
        &self.envelope_digest
    }
}

impl Serialize for CognitiveEnvelopeV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct EpistemicWire {
            turn_id: String,
            state_digest: String,
            revision: u64,
            identity_digest: String,
            claim_under_challenge: Option<String>,
            source_estimate_digest: String,
            classification_is_caller_provided: bool,
            projection_digest: String,
        }

        #[derive(Serialize)]
        struct Wire<'a> {
            schema: &'static str,
            turn: &'a TurnV1,
            identity: &'a IdentityConstitutionV1,
            subjective_present: Vec<Value>,
            relation: &'a RelationV1,
            epistemics: EpistemicWire,
            praxis: &'a PraxisV1,
            affordances: &'a BoundedListV1<AffordanceV1>,
            agency: &'a AgencyV1,
            realization: &'a ActionRealizationV1,
            exact_anchors: &'a BoundedListV1<ExactAnchorV1>,
            projection_certificate: &'a ProjectionCertificateV1,
        }

        let subjective_present = self
            .subjective_present
            .items()
            .iter()
            .map(|item| {
                serde_json::from_str(&item.to_canonical_json())
                    .map_err(<S::Error as serde::ser::Error>::custom)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let epistemics = EpistemicWire {
            turn_id: encode_hex(self.epistemics.turn_id()),
            state_digest: encode_hex(self.epistemics.state_digest()),
            revision: self.epistemics.revision(),
            identity_digest: encode_hex(self.epistemics.identity_digest()),
            claim_under_challenge: self
                .epistemics
                .claim_under_challenge()
                .map(|claim| encode_hex(claim)),
            source_estimate_digest: encode_hex(self.epistemics.source_estimate_digest()),
            classification_is_caller_provided: self.epistemics.classification_is_caller_provided(),
            projection_digest: encode_hex(self.epistemics.projection_digest()),
        };
        Wire {
            schema: self.schema,
            turn: &self.turn,
            identity: &self.identity,
            subjective_present,
            relation: &self.relation,
            epistemics,
            praxis: &self.praxis,
            affordances: &self.affordances,
            agency: &self.agency,
            realization: &self.realization,
            exact_anchors: &self.exact_anchors,
            projection_certificate: &self.projection_certificate,
        }
        .serialize(serializer)
    }
}

/// Certificate for a pre-output projection. It intentionally has no
/// `ActionRealizationV1` or `EfferenceCopyV1` digest because neither exists
/// until after a provider output is formed and observed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreOutputProjectionCertificateV1 {
    source_state_digest: Digest,
    soma_state_digest: Digest,
    subjective_present_digest: Digest,
    kv_snapshot_digest: Digest,
    epistemic_projection_digest: Digest,
    action_contract_digest: Digest,
    provider_profile_digest: Digest,
    included_capsule_digests: [Digest; PreOutputProjectionInputV1::FIELD_COUNT],
    exact_anchor_residual: u64,
    scope_residual: u64,
    action_sensitivity_residual: u64,
    action_sensitivity_bound: u64,
    disclosure_residual: u64,
    token_budget_used: u32,
}

impl PreOutputProjectionCertificateV1 {
    pub fn source_state_digest(&self) -> &Digest {
        &self.source_state_digest
    }

    pub fn soma_state_digest(&self) -> &Digest {
        &self.soma_state_digest
    }

    pub fn epistemic_projection_digest(&self) -> &Digest {
        &self.epistemic_projection_digest
    }

    pub fn action_contract_digest(&self) -> &Digest {
        &self.action_contract_digest
    }

    pub fn included_capsule_digests(&self) -> &[Digest; PreOutputProjectionInputV1::FIELD_COUNT] {
        &self.included_capsule_digests
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PreOutputAgencyV1 {
    action_contract: ActionContractV1,
    action_contract_digest: Digest,
}

/// Canonical pre-output envelope. Provider rendering and the later
/// realization/efference sidecars are deliberately outside this type.
/// `ExpressionBasisV1` remains part of the immutable identity constitution and
/// thus this envelope's identity/digest binding only; this compiler does not
/// claim or implement an expression-basis transition dynamic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreOutputCognitiveEnvelopeV1 {
    schema: &'static str,
    turn: TurnV1,
    identity: IdentityConstitutionV1,
    subjective_present: SubjectivePresentProjectionV1,
    relation: RelationV1,
    epistemics: EpistemicProjectionV1,
    praxis: PraxisV1,
    affordances: BoundedListV1<AffordanceV1>,
    agency: PreOutputAgencyV1,
    exact_anchors: BoundedListV1<ExactAnchorV1>,
    projection_certificate: PreOutputProjectionCertificateV1,
    envelope_digest: Digest,
}

impl PreOutputCognitiveEnvelopeV1 {
    pub fn identity(&self) -> &IdentityConstitutionV1 {
        &self.identity
    }

    pub fn epistemics(&self) -> &EpistemicProjectionV1 {
        &self.epistemics
    }

    pub fn action_contract(&self) -> &ActionContractV1 {
        &self.agency.action_contract
    }

    pub fn projection_certificate(&self) -> &PreOutputProjectionCertificateV1 {
        &self.projection_certificate
    }

    pub fn envelope_digest(&self) -> &Digest {
        &self.envelope_digest
    }
}

impl Serialize for PreOutputCognitiveEnvelopeV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        struct EpistemicWire {
            turn_id: String,
            state_digest: String,
            revision: u64,
            identity_digest: String,
            claim_under_challenge: Option<String>,
            source_estimate_digest: String,
            classification_is_caller_provided: bool,
            projection_digest: String,
        }

        #[derive(Serialize)]
        struct Wire<'a> {
            schema: &'static str,
            turn: &'a TurnV1,
            identity: &'a IdentityConstitutionV1,
            subjective_present: Vec<Value>,
            relation: &'a RelationV1,
            epistemics: EpistemicWire,
            praxis: &'a PraxisV1,
            affordances: &'a BoundedListV1<AffordanceV1>,
            agency: &'a PreOutputAgencyV1,
            exact_anchors: &'a BoundedListV1<ExactAnchorV1>,
            projection_certificate: &'a PreOutputProjectionCertificateV1,
        }

        let subjective_present = self
            .subjective_present
            .items()
            .iter()
            .map(|item| {
                serde_json::from_str(&item.to_canonical_json())
                    .map_err(<S::Error as serde::ser::Error>::custom)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let epistemics = EpistemicWire {
            turn_id: encode_hex(self.epistemics.turn_id()),
            state_digest: encode_hex(self.epistemics.state_digest()),
            revision: self.epistemics.revision(),
            identity_digest: encode_hex(self.epistemics.identity_digest()),
            claim_under_challenge: self
                .epistemics
                .claim_under_challenge()
                .map(|claim| encode_hex(claim)),
            source_estimate_digest: encode_hex(self.epistemics.source_estimate_digest()),
            classification_is_caller_provided: self.epistemics.classification_is_caller_provided(),
            projection_digest: encode_hex(self.epistemics.projection_digest()),
        };
        Wire {
            schema: self.schema,
            turn: &self.turn,
            identity: &self.identity,
            subjective_present,
            relation: &self.relation,
            epistemics,
            praxis: &self.praxis,
            affordances: &self.affordances,
            agency: &self.agency,
            exact_anchors: &self.exact_anchors,
            projection_certificate: &self.projection_certificate,
        }
        .serialize(serializer)
    }
}

struct ActionBindingViewV1 {
    turn_binding: String,
    base_revision: u64,
    source_state_digest: String,
    identity_constitution_digest: String,
    expires_at_ms: u64,
}

fn extract_action_binding(
    contract: &ActionContractV1,
) -> Result<ActionBindingViewV1, ProjectionErrorV1> {
    let value = serde_json::to_value(contract)
        .map_err(|_| ProjectionErrorV1::ActionContractEncodingInvalid)?;
    let schema = value
        .get("schema")
        .and_then(Value::as_str)
        .ok_or(ProjectionErrorV1::ActionContractEncodingInvalid)?;
    let action_id = value
        .get("action_id")
        .and_then(Value::as_str)
        .ok_or(ProjectionErrorV1::ActionContractEncodingInvalid)?;
    let contract_digest = value
        .get("contract_digest")
        .and_then(Value::as_str)
        .ok_or(ProjectionErrorV1::ActionContractEncodingInvalid)?;
    let speech_act = value
        .get("speech_act")
        .and_then(Value::as_str)
        .ok_or(ProjectionErrorV1::ActionContractEncodingInvalid)?;
    if schema != ACTION_CONTRACT_SCHEMA_V1
        || action_id != encode_hex(contract.action_id())
        || contract_digest != encode_hex(contract.contract_digest())
        || speech_act != contract.speech_act().as_str()
    {
        return Err(ProjectionErrorV1::ActionContractEncodingInvalid);
    }
    Ok(ActionBindingViewV1 {
        turn_binding: required_wire_string(&value, "turn_binding")?,
        base_revision: required_wire_u64(&value, "base_revision")?,
        source_state_digest: required_wire_string(&value, "source_state_digest")?,
        identity_constitution_digest: required_wire_string(&value, "identity_constitution_digest")?,
        expires_at_ms: required_wire_u64(&value, "expires_at_ms")?,
    })
}

fn required_wire_string(value: &Value, field: &str) -> Result<String, ProjectionErrorV1> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ProjectionErrorV1::ActionContractEncodingInvalid)
}

fn required_wire_u64(value: &Value, field: &str) -> Result<u64, ProjectionErrorV1> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or(ProjectionErrorV1::ActionContractEncodingInvalid)
}

pub fn compile_projection_v1(
    input: &ProjectionInput,
    preconditions: &ProjectionPreconditionsV1,
) -> Result<(CognitiveEnvelopeV1, ProjectionCertificateV1), ProjectionErrorV1> {
    validate_source_kind(
        "organism_snapshot",
        input.organism_snapshot.provenance(),
        ProjectionSourceKindV1::OrganismSnapshot,
    )?;
    validate_source_kind(
        "cognitive_kv_view",
        input.cognitive_kv_view.provenance(),
        ProjectionSourceKindV1::CognitiveKvView,
    )?;
    validate_source_kind(
        "exact_turn_anchors",
        input.exact_turn_anchors.provenance(),
        ProjectionSourceKindV1::ExactTurnAnchors,
    )?;
    validate_source_kind(
        "identity_constitution",
        input.identity_constitution.provenance(),
        ProjectionSourceKindV1::IdentityConstitution,
    )?;
    validate_source_kind(
        "relation_scope",
        input.relation_scope.provenance(),
        ProjectionSourceKindV1::RelationScope,
    )?;
    validate_source_kind(
        "action_contract",
        input.action_contract.provenance(),
        ProjectionSourceKindV1::ActionContract,
    )?;
    validate_source_kind(
        "affordance_catalog",
        input.affordance_catalog.provenance(),
        ProjectionSourceKindV1::AffordanceCatalog,
    )?;
    validate_source_kind(
        "provider_profile",
        input.provider_profile.provenance(),
        ProjectionSourceKindV1::ProviderProfile,
    )?;
    validate_source_kind(
        "soma_state",
        input.soma_state.provenance(),
        ProjectionSourceKindV1::SomaState,
    )?;

    let organism = input.organism_snapshot.value();
    let cognitive = input.cognitive_kv_view.value();
    let identity_constitution = input.identity_constitution.value();
    let relation_scope = input.relation_scope.value();
    let action_contract = input.action_contract.value();
    let soma_state = input.soma_state.value();
    let subjective_present =
        compile_subjective_present_v1(soma_state, input.soma_classification_ingress())
            .map_err(map_soma_subjective_error)?;
    let epistemic_projection = input.epistemic_projection();
    let action_realization = input.action_realization();
    let efference_copy = input.efference_copy();
    let affordance_catalog = input.affordance_catalog.value();
    let provider_profile = input.provider_profile.value();

    if identity_constitution.constitution_digest() != input.identity_constitution.content_digest() {
        return Err(ProjectionErrorV1::IdentityConstitutionDigestMismatch);
    }

    if soma_state.state_digest() != input.soma_state.content_digest() {
        return Err(ProjectionErrorV1::SomaStateDigestMismatch);
    }
    let soma_source_state = soma_state
        .source_state_digest()
        .ok_or(ProjectionErrorV1::SomaSourceStateBindingMissing)?;
    if soma_source_state != organism.state_digest() {
        return Err(ProjectionErrorV1::SomaSourceStateDigestMismatch);
    }
    if soma_state.revision() != input.organism_snapshot.provenance().source_revision()
        || soma_state.revision() != input.soma_state.provenance().source_revision()
    {
        return Err(ProjectionErrorV1::SomaSourceRevisionMismatch);
    }
    if soma_state.identity_constitution_digest() != identity_constitution.constitution_digest() {
        return Err(ProjectionErrorV1::SomaIdentityConstitutionDigestMismatch);
    }
    if epistemic_projection.turn_id() != organism.turn_id() {
        return Err(ProjectionErrorV1::EpistemicTurnBindingMismatch);
    }
    if epistemic_projection.state_digest() != organism.state_digest() {
        return Err(ProjectionErrorV1::EpistemicSourceStateDigestMismatch);
    }
    if epistemic_projection.revision() != input.organism_snapshot.provenance().source_revision() {
        return Err(ProjectionErrorV1::EpistemicRevisionMismatch);
    }
    if epistemic_projection.identity_digest() != identity_constitution.constitution_digest() {
        return Err(ProjectionErrorV1::EpistemicIdentityConstitutionDigestMismatch);
    }
    require_digest(
        epistemic_projection.projection_digest(),
        "epistemic_projection.projection_digest",
    )
    .map_err(|_| ProjectionErrorV1::EpistemicProjectionDigestInvalid)?;

    if action_contract.contract_digest() != input.action_contract.content_digest() {
        return Err(ProjectionErrorV1::ActionContractDigestMismatch);
    }
    let action_binding = extract_action_binding(action_contract)?;
    if action_binding.turn_binding != encode_hex(organism.turn_binding()) {
        return Err(ProjectionErrorV1::ActionTurnBindingMismatch);
    }
    if action_binding.base_revision != input.organism_snapshot.provenance().source_revision()
        || action_binding.base_revision != input.action_contract.provenance().source_revision()
    {
        return Err(ProjectionErrorV1::ActionBaseRevisionMismatch);
    }
    if action_binding.source_state_digest != encode_hex(organism.state_digest()) {
        return Err(ProjectionErrorV1::ActionSourceStateDigestMismatch);
    }
    if action_binding.identity_constitution_digest
        != encode_hex(identity_constitution.constitution_digest())
    {
        return Err(ProjectionErrorV1::ActionIdentityConstitutionDigestMismatch);
    }
    if relation_scope.scope_ref.as_str() != organism.turn.scope_ref() {
        return Err(ProjectionErrorV1::ScopeBindingMismatch {
            field: "relation_scope",
        });
    }
    if affordance_catalog.scope_ref.as_str() != organism.turn.scope_ref() {
        return Err(ProjectionErrorV1::ScopeBindingMismatch {
            field: "affordance_catalog",
        });
    }
    if action_realization.action_id() != action_contract.action_id() {
        return Err(ProjectionErrorV1::ActionRealizationActionIdMismatch);
    }
    if action_realization.contract_digest() != action_contract.contract_digest() {
        return Err(ProjectionErrorV1::ActionRealizationContractDigestMismatch);
    }
    if efference_copy.action_id() != action_contract.action_id() {
        return Err(ProjectionErrorV1::EfferenceCopyActionIdMismatch);
    }
    if efference_copy.contract_digest() != action_contract.contract_digest() {
        return Err(ProjectionErrorV1::EfferenceCopyContractDigestMismatch);
    }
    if efference_copy.realization_digest() != action_realization.realization_digest() {
        return Err(ProjectionErrorV1::EfferenceCopyRealizationDigestMismatch);
    }
    if action_binding.expires_at_ms <= preconditions.projected_at_ms {
        return Err(ProjectionErrorV1::ActionContractExpired {
            projected_at_ms: preconditions.projected_at_ms,
            expires_at_ms: action_binding.expires_at_ms,
        });
    }

    if preconditions.exact_anchor_residual != 0 {
        return Err(ProjectionErrorV1::ExactAnchorResidualNonzero {
            residual: preconditions.exact_anchor_residual,
        });
    }
    if preconditions.scope_residual != 0 {
        return Err(ProjectionErrorV1::ScopeResidualNonzero {
            residual: preconditions.scope_residual,
        });
    }
    if preconditions.disclosure_residual != 0 {
        return Err(ProjectionErrorV1::DisclosureResidualNonzero {
            residual: preconditions.disclosure_residual,
        });
    }
    if preconditions.action_sensitivity_residual > preconditions.action_sensitivity_bound {
        return Err(ProjectionErrorV1::ActionSensitivityResidualExceeded {
            residual: preconditions.action_sensitivity_residual,
            bound: preconditions.action_sensitivity_bound,
        });
    }
    if preconditions.token_budget_used > MAX_PROJECTION_TOKENS {
        return Err(ProjectionErrorV1::TokenBudgetHardCeilingExceeded {
            used: preconditions.token_budget_used,
            hard_ceiling: MAX_PROJECTION_TOKENS,
        });
    }
    if preconditions.token_budget_used > provider_profile.projection_token_limit {
        return Err(ProjectionErrorV1::ProviderTokenBudgetExceeded {
            used: preconditions.token_budget_used,
            provider_limit: provider_profile.projection_token_limit,
        });
    }

    let certificate = ProjectionCertificateV1 {
        source_state_digest: *organism.state_digest(),
        soma_state_digest: *soma_state.state_digest(),
        subjective_present_digest: *subjective_present.identity_digest(),
        kv_snapshot_digest: *cognitive.kv_snapshot_digest(),
        epistemic_projection_digest: *epistemic_projection.projection_digest(),
        action_contract_digest: *action_contract.contract_digest(),
        action_realization_digest: *action_realization.realization_digest(),
        efference_copy_digest: *efference_copy.copy_digest(),
        provider_profile_digest: *input.provider_profile.content_digest(),
        included_capsule_digests: input.source_capsule_digests(),
        exact_anchor_residual: preconditions.exact_anchor_residual,
        scope_residual: preconditions.scope_residual,
        action_sensitivity_residual: preconditions.action_sensitivity_residual,
        action_sensitivity_bound: preconditions.action_sensitivity_bound,
        disclosure_residual: preconditions.disclosure_residual,
        token_budget_used: preconditions.token_budget_used,
    };

    let envelope_digest = compute_envelope_digest(
        input,
        preconditions,
        EnvelopeDigestBindingsV1 {
            identity_digest: identity_constitution.constitution_digest(),
            subjective_present_digest: subjective_present.identity_digest(),
            epistemic_projection_digest: epistemic_projection.projection_digest(),
            action_contract_digest: action_contract.contract_digest(),
            action_realization_digest: action_realization.realization_digest(),
            efference_copy_digest: efference_copy.copy_digest(),
        },
    );
    let envelope = CognitiveEnvelopeV1 {
        schema: COGNITIVE_ENVELOPE_SCHEMA_V1,
        turn: organism.turn.clone(),
        identity: identity_constitution.clone(),
        subjective_present,
        relation: relation_scope.relation.clone(),
        epistemics: epistemic_projection.clone(),
        praxis: cognitive.praxis.clone(),
        affordances: affordance_catalog.affordances.clone(),
        agency: AgencyV1 {
            action_contract: action_contract.clone(),
            action_contract_digest: *input.action_contract.content_digest(),
            efference_copy_digest: *efference_copy.copy_digest(),
        },
        realization: action_realization.clone(),
        exact_anchors: input.exact_turn_anchors.value().anchors.clone(),
        projection_certificate: certificate.clone(),
        envelope_digest,
    };

    Ok((envelope, certificate))
}

/// Compiles the closed native envelope that exists before provider output.
/// A current-turn realization/efference pair is intentionally not accepted,
/// because it cannot truthfully exist at this boundary.
pub fn compile_pre_output_projection_v1(
    input: &PreOutputProjectionInputV1,
    preconditions: &ProjectionPreconditionsV1,
) -> Result<
    (
        PreOutputCognitiveEnvelopeV1,
        PreOutputProjectionCertificateV1,
    ),
    ProjectionErrorV1,
> {
    for (field, provenance, expected) in [
        (
            "organism_snapshot",
            input.organism_snapshot.provenance(),
            ProjectionSourceKindV1::OrganismSnapshot,
        ),
        (
            "cognitive_kv_view",
            input.cognitive_kv_view.provenance(),
            ProjectionSourceKindV1::CognitiveKvView,
        ),
        (
            "exact_turn_anchors",
            input.exact_turn_anchors.provenance(),
            ProjectionSourceKindV1::ExactTurnAnchors,
        ),
        (
            "identity_constitution",
            input.identity_constitution.provenance(),
            ProjectionSourceKindV1::IdentityConstitution,
        ),
        (
            "relation_scope",
            input.relation_scope.provenance(),
            ProjectionSourceKindV1::RelationScope,
        ),
        (
            "action_contract",
            input.action_contract.provenance(),
            ProjectionSourceKindV1::ActionContract,
        ),
        (
            "affordance_catalog",
            input.affordance_catalog.provenance(),
            ProjectionSourceKindV1::AffordanceCatalog,
        ),
        (
            "provider_profile",
            input.provider_profile.provenance(),
            ProjectionSourceKindV1::ProviderProfile,
        ),
        (
            "soma_state",
            input.soma_state.provenance(),
            ProjectionSourceKindV1::SomaState,
        ),
    ] {
        validate_source_kind(field, provenance, expected)?;
    }

    let organism = input.organism_snapshot.value();
    let cognitive = input.cognitive_kv_view.value();
    let identity_constitution = input.identity_constitution.value();
    let relation_scope = input.relation_scope.value();
    let action_contract = input.action_contract.value();
    let soma_state = input.soma_state.value();
    let subjective_present =
        compile_subjective_present_v1(soma_state, &input.soma_classification_ingress)
            .map_err(map_soma_subjective_error)?;
    let epistemic_projection = &input.epistemic_projection;
    let affordance_catalog = input.affordance_catalog.value();
    let provider_profile = input.provider_profile.value();

    if identity_constitution.constitution_digest() != input.identity_constitution.content_digest() {
        return Err(ProjectionErrorV1::IdentityConstitutionDigestMismatch);
    }
    if soma_state.state_digest() != input.soma_state.content_digest() {
        return Err(ProjectionErrorV1::SomaStateDigestMismatch);
    }
    let soma_source_state = soma_state
        .source_state_digest()
        .ok_or(ProjectionErrorV1::SomaSourceStateBindingMissing)?;
    if soma_source_state != organism.state_digest() {
        return Err(ProjectionErrorV1::SomaSourceStateDigestMismatch);
    }
    if soma_state.revision() != input.organism_snapshot.provenance().source_revision()
        || soma_state.revision() != input.soma_state.provenance().source_revision()
    {
        return Err(ProjectionErrorV1::SomaSourceRevisionMismatch);
    }
    if soma_state.identity_constitution_digest() != identity_constitution.constitution_digest() {
        return Err(ProjectionErrorV1::SomaIdentityConstitutionDigestMismatch);
    }
    if epistemic_projection.turn_id() != organism.turn_id() {
        return Err(ProjectionErrorV1::EpistemicTurnBindingMismatch);
    }
    if epistemic_projection.state_digest() != organism.state_digest() {
        return Err(ProjectionErrorV1::EpistemicSourceStateDigestMismatch);
    }
    if epistemic_projection.revision() != input.organism_snapshot.provenance().source_revision() {
        return Err(ProjectionErrorV1::EpistemicRevisionMismatch);
    }
    if epistemic_projection.identity_digest() != identity_constitution.constitution_digest() {
        return Err(ProjectionErrorV1::EpistemicIdentityConstitutionDigestMismatch);
    }
    require_digest(
        epistemic_projection.projection_digest(),
        "epistemic_projection.projection_digest",
    )
    .map_err(|_| ProjectionErrorV1::EpistemicProjectionDigestInvalid)?;

    if action_contract.contract_digest() != input.action_contract.content_digest() {
        return Err(ProjectionErrorV1::ActionContractDigestMismatch);
    }
    let action_binding = extract_action_binding(action_contract)?;
    if action_binding.turn_binding != encode_hex(organism.turn_binding()) {
        return Err(ProjectionErrorV1::ActionTurnBindingMismatch);
    }
    if action_binding.base_revision != input.organism_snapshot.provenance().source_revision()
        || action_binding.base_revision != input.action_contract.provenance().source_revision()
    {
        return Err(ProjectionErrorV1::ActionBaseRevisionMismatch);
    }
    if action_binding.source_state_digest != encode_hex(organism.state_digest()) {
        return Err(ProjectionErrorV1::ActionSourceStateDigestMismatch);
    }
    if action_binding.identity_constitution_digest
        != encode_hex(identity_constitution.constitution_digest())
    {
        return Err(ProjectionErrorV1::ActionIdentityConstitutionDigestMismatch);
    }
    if relation_scope.scope_ref.as_str() != organism.turn.scope_ref() {
        return Err(ProjectionErrorV1::ScopeBindingMismatch {
            field: "relation_scope",
        });
    }
    if affordance_catalog.scope_ref.as_str() != organism.turn.scope_ref() {
        return Err(ProjectionErrorV1::ScopeBindingMismatch {
            field: "affordance_catalog",
        });
    }
    if action_binding.expires_at_ms <= preconditions.projected_at_ms {
        return Err(ProjectionErrorV1::ActionContractExpired {
            projected_at_ms: preconditions.projected_at_ms,
            expires_at_ms: action_binding.expires_at_ms,
        });
    }
    validate_projection_preconditions_v1(preconditions, provider_profile)?;

    let certificate = PreOutputProjectionCertificateV1 {
        source_state_digest: *organism.state_digest(),
        soma_state_digest: *soma_state.state_digest(),
        subjective_present_digest: *subjective_present.identity_digest(),
        kv_snapshot_digest: *cognitive.kv_snapshot_digest(),
        epistemic_projection_digest: *epistemic_projection.projection_digest(),
        action_contract_digest: *action_contract.contract_digest(),
        provider_profile_digest: *input.provider_profile.content_digest(),
        included_capsule_digests: input.source_capsule_digests(),
        exact_anchor_residual: preconditions.exact_anchor_residual,
        scope_residual: preconditions.scope_residual,
        action_sensitivity_residual: preconditions.action_sensitivity_residual,
        action_sensitivity_bound: preconditions.action_sensitivity_bound,
        disclosure_residual: preconditions.disclosure_residual,
        token_budget_used: preconditions.token_budget_used,
    };
    let envelope_digest = compute_pre_output_envelope_digest_v1(
        input,
        preconditions,
        identity_constitution.constitution_digest(),
        subjective_present.identity_digest(),
        epistemic_projection.projection_digest(),
        action_contract.contract_digest(),
    );
    let envelope = PreOutputCognitiveEnvelopeV1 {
        schema: PRE_OUTPUT_COGNITIVE_ENVELOPE_SCHEMA_V1,
        turn: organism.turn.clone(),
        identity: identity_constitution.clone(),
        subjective_present,
        relation: relation_scope.relation.clone(),
        epistemics: epistemic_projection.clone(),
        praxis: cognitive.praxis.clone(),
        affordances: affordance_catalog.affordances.clone(),
        agency: PreOutputAgencyV1 {
            action_contract: action_contract.clone(),
            action_contract_digest: *input.action_contract.content_digest(),
        },
        exact_anchors: input.exact_turn_anchors.value().anchors.clone(),
        projection_certificate: certificate.clone(),
        envelope_digest,
    };
    Ok((envelope, certificate))
}

fn validate_projection_preconditions_v1(
    preconditions: &ProjectionPreconditionsV1,
    provider_profile: &ProviderProfileV1,
) -> Result<(), ProjectionErrorV1> {
    if preconditions.exact_anchor_residual != 0 {
        return Err(ProjectionErrorV1::ExactAnchorResidualNonzero {
            residual: preconditions.exact_anchor_residual,
        });
    }
    if preconditions.scope_residual != 0 {
        return Err(ProjectionErrorV1::ScopeResidualNonzero {
            residual: preconditions.scope_residual,
        });
    }
    if preconditions.disclosure_residual != 0 {
        return Err(ProjectionErrorV1::DisclosureResidualNonzero {
            residual: preconditions.disclosure_residual,
        });
    }
    if preconditions.action_sensitivity_residual > preconditions.action_sensitivity_bound {
        return Err(ProjectionErrorV1::ActionSensitivityResidualExceeded {
            residual: preconditions.action_sensitivity_residual,
            bound: preconditions.action_sensitivity_bound,
        });
    }
    if preconditions.token_budget_used > MAX_PROJECTION_TOKENS {
        return Err(ProjectionErrorV1::TokenBudgetHardCeilingExceeded {
            used: preconditions.token_budget_used,
            hard_ceiling: MAX_PROJECTION_TOKENS,
        });
    }
    if preconditions.token_budget_used > provider_profile.projection_token_limit {
        return Err(ProjectionErrorV1::ProviderTokenBudgetExceeded {
            used: preconditions.token_budget_used,
            provider_limit: provider_profile.projection_token_limit,
        });
    }
    Ok(())
}

fn validate_source_kind(
    field: &'static str,
    provenance: &SourceProvenanceV1,
    expected: ProjectionSourceKindV1,
) -> Result<(), ProjectionErrorV1> {
    let actual = provenance.source_kind();
    if actual != expected {
        return Err(ProjectionErrorV1::WrongSourceKind {
            field,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_required_schema_text(
    value: &BoundedTextV1,
    field: &'static str,
    max_chars: u32,
) -> Result<(), ProjectionErrorV1> {
    value.require_non_empty(field)?;
    value.require_schema_max(field, max_chars)
}

fn require_list_schema_max<T>(
    list: &BoundedListV1<T>,
    field: &'static str,
    max_items: u16,
) -> Result<(), ProjectionErrorV1> {
    if list.max_items() > max_items || list.as_slice().len() > usize::from(max_items) {
        return Err(ProjectionErrorV1::TooManyItems {
            field,
            max_items,
            actual_items: list.as_slice().len(),
        });
    }
    Ok(())
}

fn require_digest(digest: &Digest, field: &'static str) -> Result<(), ProjectionErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(ProjectionErrorV1::ZeroDigest { field });
    }
    Ok(())
}

fn require_id(id: &Id128, field: &'static str) -> Result<(), ProjectionErrorV1> {
    if id.iter().all(|byte| *byte == 0) {
        return Err(ProjectionErrorV1::ZeroId { field });
    }
    Ok(())
}

fn map_soma_subjective_error(error: SomaErrorV1) -> ProjectionErrorV1 {
    match error {
        SomaErrorV1::StateDigestMismatch => ProjectionErrorV1::SomaSubjectiveStateMismatch,
        SomaErrorV1::RevisionMismatch => ProjectionErrorV1::SomaSubjectiveRevisionMismatch,
        SomaErrorV1::IdentityBindingMismatch => ProjectionErrorV1::SomaSubjectiveIdentityMismatch,
        _ => ProjectionErrorV1::SomaSubjectiveProjectionRejected,
    }
}

struct EnvelopeDigestBindingsV1<'a> {
    identity_digest: &'a Digest,
    subjective_present_digest: &'a Digest,
    epistemic_projection_digest: &'a Digest,
    action_contract_digest: &'a Digest,
    action_realization_digest: &'a Digest,
    efference_copy_digest: &'a Digest,
}

fn compute_pre_output_envelope_digest_v1(
    input: &PreOutputProjectionInputV1,
    preconditions: &ProjectionPreconditionsV1,
    identity_digest: &Digest,
    subjective_present_digest: &Digest,
    epistemic_projection_digest: &Digest,
    action_contract_digest: &Digest,
) -> Digest {
    let mut fields = Vec::with_capacity(14 + PreOutputProjectionInputV1::FIELD_COUNT);
    fields.push(b"pre-output-v1".to_vec());
    fields.extend(
        input
            .source_capsule_digests()
            .iter()
            .map(|digest| digest.to_vec()),
    );
    fields.push(identity_digest.to_vec());
    fields.push(subjective_present_digest.to_vec());
    fields.push(epistemic_projection_digest.to_vec());
    fields.push(action_contract_digest.to_vec());
    fields.push(preconditions.projected_at_ms.to_be_bytes().to_vec());
    fields.push(preconditions.exact_anchor_residual.to_be_bytes().to_vec());
    fields.push(preconditions.scope_residual.to_be_bytes().to_vec());
    fields.push(
        preconditions
            .action_sensitivity_residual
            .to_be_bytes()
            .to_vec(),
    );
    fields.push(
        preconditions
            .action_sensitivity_bound
            .to_be_bytes()
            .to_vec(),
    );
    fields.push(preconditions.disclosure_residual.to_be_bytes().to_vec());
    fields.push(preconditions.token_budget_used.to_be_bytes().to_vec());
    wire::domain_hash(
        b"astr-embodiment/r7/cognitive-envelope/pre-output-digest-v1",
        &fields.iter().map(Vec::as_slice).collect::<Vec<_>>(),
    )
}

fn compute_envelope_digest(
    input: &ProjectionInput,
    preconditions: &ProjectionPreconditionsV1,
    bindings: EnvelopeDigestBindingsV1<'_>,
) -> Digest {
    let mut fields = vec![COGNITIVE_ENVELOPE_SCHEMA_V1.as_bytes().to_vec()];
    fields.extend(
        input
            .source_capsule_digests()
            .into_iter()
            .map(|digest| digest.to_vec()),
    );
    fields.push(bindings.identity_digest.to_vec());
    fields.push(bindings.subjective_present_digest.to_vec());
    fields.push(bindings.epistemic_projection_digest.to_vec());
    fields.push(bindings.action_contract_digest.to_vec());
    fields.push(bindings.action_realization_digest.to_vec());
    fields.push(bindings.efference_copy_digest.to_vec());
    fields.push(preconditions.projected_at_ms.to_be_bytes().to_vec());
    fields.push(preconditions.exact_anchor_residual.to_be_bytes().to_vec());
    fields.push(preconditions.scope_residual.to_be_bytes().to_vec());
    fields.push(
        preconditions
            .action_sensitivity_residual
            .to_be_bytes()
            .to_vec(),
    );
    fields.push(
        preconditions
            .action_sensitivity_bound
            .to_be_bytes()
            .to_vec(),
    );
    fields.push(preconditions.disclosure_residual.to_be_bytes().to_vec());
    fields.push(preconditions.token_budget_used.to_be_bytes().to_vec());
    let field_refs = fields.iter().map(Vec::as_slice).collect::<Vec<_>>();
    wire::domain_hash(
        b"astr-embodiment/r7/cognitive-envelope-typed-assembly-v1",
        &field_refs,
    )
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn is_canonical_token(value: &str) -> bool {
    let mut previous_was_separator = true;
    for byte in value.bytes() {
        if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            previous_was_separator = false;
        } else if matches!(byte, b'_' | b'-' | b'.' | b':') && !previous_was_separator {
            previous_was_separator = true;
        } else {
            return false;
        }
    }
    !previous_was_separator
}

fn source_kind_name(kind: ProjectionSourceKindV1) -> &'static [u8] {
    match kind {
        ProjectionSourceKindV1::OrganismSnapshot => b"organism_snapshot",
        ProjectionSourceKindV1::CognitiveKvView => b"cognitive_kv_view",
        ProjectionSourceKindV1::ExactTurnAnchors => b"exact_turn_anchors",
        ProjectionSourceKindV1::IdentityConstitution => b"identity_constitution",
        ProjectionSourceKindV1::RelationScope => b"relation_scope",
        ProjectionSourceKindV1::ActionContract => b"action_contract",
        ProjectionSourceKindV1::AffordanceCatalog => b"affordance_catalog",
        ProjectionSourceKindV1::ProviderProfile => b"provider_profile",
        ProjectionSourceKindV1::SomaState => b"soma_state",
    }
}

fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}
