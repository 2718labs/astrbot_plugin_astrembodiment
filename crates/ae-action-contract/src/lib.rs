#![forbid(unsafe_code)]

//! Pure, deterministic R7 action evaluation and realization contracts.
//!
//! The R7 pack has an authoritative JSON schema for `ActionRealizationV1`, but no JSON
//! schema for `ActionContractV1`. The realization type therefore serializes only schema
//! fields. Prose-only fields such as provider-profile digest, visible-text digest, source
//! basis, and contract adherence are deliberately absent because the schema has
//! `additionalProperties: false`.
//!
//! Raw utterances, tool arguments, provider payloads, world state, and delivery evidence
//! are not accepted. Revision and state bindings live in the embodiment-side action
//! contract and are transitively bound into realization through its contract digest.

use ae_contracts::r7::{wire, Digest, Id128};
use serde::{Serialize, Serializer};
use std::cmp::Ordering;
use thiserror::Error;

pub const ACTION_CONTRACT_SCHEMA_V1: &str = "astrembodiment.action-contract.v1";
pub const ACTION_REALIZATION_SCHEMA_V1: &str = "astrembodiment.action-realization.v1";
pub const MAX_OWNED_CLAIMS: u16 = 64;
pub const MAX_PROPOSED_TOOLS: u16 = 32;
pub const MAX_DISCLOSURES_USED: u16 = 32;
pub const UNIT_INTERVAL_SCALE: u32 = 1_000_000;

const ACTION_CONTRACT_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/action-contract-v1";
const ACTION_REQUIREMENTS_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/action-requirements-v1";
const ACTION_REALIZATION_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/action-realization-v1";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ActionCoreErrorV1 {
    #[error("{field} bound must be nonzero")]
    ZeroBound { field: &'static str },
    #[error("canonical token must not be empty")]
    EmptyToken,
    #[error("canonical token exceeds {max_bytes} bytes")]
    TokenTooLong { max_bytes: u16, actual_bytes: usize },
    #[error("value is not a canonical token")]
    NonCanonicalToken,
    #[error("{field} has {actual_items} items, above {max_items}")]
    TooManyItems {
        field: &'static str,
        max_items: u16,
        actual_items: usize,
    },
    #[error("duplicate canonical token at {index}")]
    DuplicateToken { index: usize },
    #[error("canonical tokens are not ordered at {index}")]
    NonCanonicalTokenOrder { index: usize },
    #[error("unknown action disposition")]
    UnknownDisposition,
    #[error("unit interval value exceeds one")]
    UnitIntervalOutOfRange { parts_per_million: u32 },
    #[error("zero identifier is not valid for {field}")]
    ZeroId { field: &'static str },
    #[error("zero digest is not valid for {field}")]
    ZeroDigest { field: &'static str },
    #[error("speech_act exceeds the schema maximum")]
    SpeechActTooLong,
    #[error("action contract requires a nonzero expiry")]
    MissingExpiry,
    #[error("fixed disposition is inconsistent with allowed tools or speech act")]
    InvalidDispositionShape,
    #[error("owned claim span_ref exceeds the schema maximum")]
    SpanRefTooLong,
    #[error("duplicate owned claim at {index}")]
    DuplicateOwnedClaim { index: usize },
    #[error("owned claims are not ordered at {index}")]
    NonCanonicalOwnedClaimOrder { index: usize },
    #[error("duplicate proposed tool at {index}")]
    DuplicateProposedTool { index: usize },
    #[error("proposed tools are not ordered at {index}")]
    NonCanonicalProposedToolOrder { index: usize },
    #[error("duplicate disclosure at {index}")]
    DuplicateDisclosure { index: usize },
    #[error("disclosures are not ordered at {index}")]
    NonCanonicalDisclosureOrder { index: usize },
    #[error("proposed tool at {index} is not allowed by the fixed contract")]
    ToolNotAllowed { index: usize },
    #[error("disclosure at {index} is not allowed by the fixed contract")]
    DisclosureNotAllowed { index: usize },
    #[error("realization shape is inconsistent with the fixed disposition")]
    InvalidRealizationShape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CanonicalTokenV1(String);

impl CanonicalTokenV1 {
    pub fn new(value: String, max_bytes: u16) -> Result<Self, ActionCoreErrorV1> {
        if max_bytes == 0 {
            return Err(ActionCoreErrorV1::ZeroBound { field: "max_bytes" });
        }
        if value.is_empty() {
            return Err(ActionCoreErrorV1::EmptyToken);
        }
        if value.len() > usize::from(max_bytes) {
            return Err(ActionCoreErrorV1::TokenTooLong {
                max_bytes,
                actual_bytes: value.len(),
            });
        }
        if !is_canonical_token(&value) {
            return Err(ActionCoreErrorV1::NonCanonicalToken);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for CanonicalTokenV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CanonicalTokenSetV1(Vec<CanonicalTokenV1>);

impl CanonicalTokenSetV1 {
    pub fn new(values: Vec<CanonicalTokenV1>, max_items: u16) -> Result<Self, ActionCoreErrorV1> {
        if values.len() > usize::from(max_items) {
            return Err(ActionCoreErrorV1::TooManyItems {
                field: "canonical_token_set",
                max_items,
                actual_items: values.len(),
            });
        }
        for (offset, pair) in values.windows(2).enumerate() {
            match pair[0].as_str().cmp(pair[1].as_str()) {
                Ordering::Equal => {
                    return Err(ActionCoreErrorV1::DuplicateToken { index: offset + 1 });
                }
                Ordering::Greater => {
                    return Err(ActionCoreErrorV1::NonCanonicalTokenOrder { index: offset + 1 });
                }
                Ordering::Less => {}
            }
        }
        Ok(Self(values))
    }

    pub fn as_slice(&self) -> &[CanonicalTokenV1] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn contains(&self, value: &CanonicalTokenV1) -> bool {
        self.0
            .binary_search_by(|candidate| candidate.as_str().cmp(value.as_str()))
            .is_ok()
    }

    fn content_digest(&self, domain: &[u8]) -> Digest {
        let fields: Vec<&[u8]> = self
            .0
            .iter()
            .map(|value| value.as_str().as_bytes())
            .collect();
        wire::domain_hash(domain, &fields)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionDispositionV1 {
    Silence,
    Speech,
    ToolPlan,
    SpeechAndToolPlan,
}

impl ActionDispositionV1 {
    pub fn parse(value: &str) -> Result<Self, ActionCoreErrorV1> {
        match value {
            "silence" => Ok(Self::Silence),
            "speech" => Ok(Self::Speech),
            "tool_plan" => Ok(Self::ToolPlan),
            "speech_and_tool_plan" => Ok(Self::SpeechAndToolPlan),
            _ => Err(ActionCoreErrorV1::UnknownDisposition),
        }
    }

    fn name(self) -> &'static [u8] {
        match self {
            Self::Silence => b"silence",
            Self::Speech => b"speech",
            Self::ToolPlan => b"tool_plan",
            Self::SpeechAndToolPlan => b"speech_and_tool_plan",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnitIntervalV1(u32);

impl UnitIntervalV1 {
    pub fn from_parts_per_million(parts_per_million: u32) -> Result<Self, ActionCoreErrorV1> {
        if parts_per_million > UNIT_INTERVAL_SCALE {
            return Err(ActionCoreErrorV1::UnitIntervalOutOfRange { parts_per_million });
        }
        Ok(Self(parts_per_million))
    }

    pub fn parts_per_million(self) -> u32 {
        self.0
    }
}

impl Serialize for UnitIntervalV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_f64(f64::from(self.0) / f64::from(UNIT_INTERVAL_SCALE))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionRequirementsV1 {
    must: CanonicalTokenSetV1,
    should: CanonicalTokenSetV1,
    may: CanonicalTokenSetV1,
    must_not: CanonicalTokenSetV1,
}

impl ActionRequirementsV1 {
    pub fn new(
        must: CanonicalTokenSetV1,
        should: CanonicalTokenSetV1,
        may: CanonicalTokenSetV1,
        must_not: CanonicalTokenSetV1,
    ) -> Self {
        Self {
            must,
            should,
            may,
            must_not,
        }
    }

    fn content_digest(&self) -> Digest {
        let must = self
            .must
            .content_digest(b"astr-embodiment/r7/action-requirements/must-v1");
        let should = self
            .should
            .content_digest(b"astr-embodiment/r7/action-requirements/should-v1");
        let may = self
            .may
            .content_digest(b"astr-embodiment/r7/action-requirements/may-v1");
        let must_not = self
            .must_not
            .content_digest(b"astr-embodiment/r7/action-requirements/must-not-v1");
        wire::domain_hash(
            ACTION_REQUIREMENTS_DOMAIN_V1,
            &[&must, &should, &may, &must_not],
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DigestHexV1(Digest);

impl DigestHexV1 {
    fn bytes(&self) -> &Digest {
        &self.0
    }
}

impl Serialize for DigestHexV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_hex(&self.0))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct IdHexV1(Id128);

impl IdHexV1 {
    fn bytes(&self) -> &Id128 {
        &self.0
    }
}

impl Serialize for IdHexV1 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&encode_hex(&self.0))
    }
}

/// Embodiment-side evaluation result with a fixed realization disposition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionContractV1 {
    schema: &'static str,
    action_id: IdHexV1,
    turn_binding: DigestHexV1,
    base_revision: u64,
    source_state_digest: DigestHexV1,
    identity_constitution_digest: DigestHexV1,
    disposition: ActionDispositionV1,
    speech_act: CanonicalTokenV1,
    requirements: ActionRequirementsV1,
    allowed_tools: CanonicalTokenSetV1,
    allowed_disclosures: CanonicalTokenSetV1,
    confidence_ceiling: UnitIntervalV1,
    expires_at_ms: u64,
    contract_digest: DigestHexV1,
}

impl ActionContractV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn from_evaluation(
        action_id: Id128,
        turn_binding: Digest,
        base_revision: u64,
        source_state_digest: Digest,
        identity_constitution_digest: Digest,
        disposition: ActionDispositionV1,
        speech_act: CanonicalTokenV1,
        requirements: ActionRequirementsV1,
        allowed_tools: CanonicalTokenSetV1,
        allowed_disclosures: CanonicalTokenSetV1,
        confidence_ceiling: UnitIntervalV1,
        expires_at_ms: u64,
    ) -> Result<Self, ActionCoreErrorV1> {
        require_id(&action_id, "action_id")?;
        require_digest(&turn_binding, "turn_binding")?;
        require_digest(&source_state_digest, "source_state_digest")?;
        require_digest(
            &identity_constitution_digest,
            "identity_constitution_digest",
        )?;
        if speech_act.as_str().len() > 64 {
            return Err(ActionCoreErrorV1::SpeechActTooLong);
        }
        if expires_at_ms == 0 {
            return Err(ActionCoreErrorV1::MissingExpiry);
        }
        let disposition_is_valid = match disposition {
            ActionDispositionV1::Silence => {
                speech_act.as_str() == "silence"
                    && allowed_tools.is_empty()
                    && allowed_disclosures.is_empty()
            }
            ActionDispositionV1::Speech => allowed_tools.is_empty(),
            ActionDispositionV1::ToolPlan | ActionDispositionV1::SpeechAndToolPlan => {
                !allowed_tools.is_empty()
            }
        };
        if !disposition_is_valid {
            return Err(ActionCoreErrorV1::InvalidDispositionShape);
        }

        let revision = base_revision.to_be_bytes();
        let requirements_digest = requirements.content_digest();
        let allowed_tools_digest =
            allowed_tools.content_digest(b"astr-embodiment/r7/action-contract/allowed-tools-v1");
        let allowed_disclosures_digest = allowed_disclosures
            .content_digest(b"astr-embodiment/r7/action-contract/allowed-disclosures-v1");
        let confidence = confidence_ceiling.parts_per_million().to_be_bytes();
        let expiry = expires_at_ms.to_be_bytes();
        let contract_digest = wire::domain_hash(
            ACTION_CONTRACT_DOMAIN_V1,
            &[
                &action_id,
                &turn_binding,
                &revision,
                &source_state_digest,
                &identity_constitution_digest,
                disposition.name(),
                speech_act.as_str().as_bytes(),
                &requirements_digest,
                &allowed_tools_digest,
                &allowed_disclosures_digest,
                &confidence,
                &expiry,
            ],
        );
        Ok(Self {
            schema: ACTION_CONTRACT_SCHEMA_V1,
            action_id: IdHexV1(action_id),
            turn_binding: DigestHexV1(turn_binding),
            base_revision,
            source_state_digest: DigestHexV1(source_state_digest),
            identity_constitution_digest: DigestHexV1(identity_constitution_digest),
            disposition,
            speech_act,
            requirements,
            allowed_tools,
            allowed_disclosures,
            confidence_ceiling,
            expires_at_ms,
            contract_digest: DigestHexV1(contract_digest),
        })
    }

    pub fn action_id(&self) -> &Id128 {
        self.action_id.bytes()
    }

    /// Exact turn selected by the typed evaluator. This remains a closed
    /// digest-only binding; it is not a provider or user-text channel.
    pub fn turn_binding(&self) -> &Digest {
        self.turn_binding.bytes()
    }

    pub fn base_revision(&self) -> u64 {
        self.base_revision
    }

    pub fn source_state_digest(&self) -> &Digest {
        self.source_state_digest.bytes()
    }

    pub fn identity_constitution_digest(&self) -> &Digest {
        self.identity_constitution_digest.bytes()
    }

    pub fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    pub fn contract_digest(&self) -> &Digest {
        self.contract_digest.bytes()
    }

    pub fn speech_act(&self) -> &CanonicalTokenV1 {
        &self.speech_act
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnedClaimV1 {
    claim_digest: DigestHexV1,
    span_ref: Option<CanonicalTokenV1>,
    confidence: UnitIntervalV1,
    assertiveness: UnitIntervalV1,
    stakes: UnitIntervalV1,
    verifiable: bool,
}

impl OwnedClaimV1 {
    pub fn new(
        claim_digest: Digest,
        span_ref: Option<CanonicalTokenV1>,
        confidence: UnitIntervalV1,
        assertiveness: UnitIntervalV1,
        stakes: UnitIntervalV1,
        verifiable: bool,
    ) -> Result<Self, ActionCoreErrorV1> {
        require_digest(&claim_digest, "owned_claim.claim_digest")?;
        if span_ref
            .as_ref()
            .is_some_and(|reference| reference.as_str().len() > 128)
        {
            return Err(ActionCoreErrorV1::SpanRefTooLong);
        }
        Ok(Self {
            claim_digest: DigestHexV1(claim_digest),
            span_ref,
            confidence,
            assertiveness,
            stakes,
            verifiable,
        })
    }

    fn identity_digest(&self) -> Digest {
        let confidence = self.confidence.parts_per_million().to_be_bytes();
        let assertiveness = self.assertiveness.parts_per_million().to_be_bytes();
        let stakes = self.stakes.parts_per_million().to_be_bytes();
        let verifiable = [u8::from(self.verifiable)];
        let span_ref = self
            .span_ref
            .as_ref()
            .map_or(b"".as_slice(), |reference| reference.as_str().as_bytes());
        wire::domain_hash(
            b"astr-embodiment/r7/owned-claim-v1",
            &[
                self.claim_digest.bytes(),
                span_ref,
                &confidence,
                &assertiveness,
                &stakes,
                &verifiable,
            ],
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ToolProposalV1 {
    tool_id: CanonicalTokenV1,
    arguments_digest: DigestHexV1,
}

impl ToolProposalV1 {
    pub fn new(
        tool_id: CanonicalTokenV1,
        arguments_digest: Digest,
    ) -> Result<Self, ActionCoreErrorV1> {
        require_digest(&arguments_digest, "tool_proposal.arguments_digest")?;
        Ok(Self {
            tool_id,
            arguments_digest: DigestHexV1(arguments_digest),
        })
    }

    fn identity_digest(&self) -> Digest {
        wire::domain_hash(
            b"astr-embodiment/r7/tool-proposal-v1",
            &[
                self.tool_id.as_str().as_bytes(),
                self.arguments_digest.bytes(),
            ],
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DisclosureUseV1 {
    disclosure_id: CanonicalTokenV1,
    source_digest: DigestHexV1,
}

impl DisclosureUseV1 {
    pub fn new(
        disclosure_id: CanonicalTokenV1,
        source_digest: Digest,
    ) -> Result<Self, ActionCoreErrorV1> {
        require_digest(&source_digest, "disclosure_use.source_digest")?;
        Ok(Self {
            disclosure_id,
            source_digest: DigestHexV1(source_digest),
        })
    }

    fn identity_digest(&self) -> Digest {
        wire::domain_hash(
            b"astr-embodiment/r7/disclosure-use-v1",
            &[
                self.disclosure_id.as_str().as_bytes(),
                self.source_digest.bytes(),
            ],
        )
    }
}

/// Schema-closed realization sidecar. The internal identity is not serialized because the
/// authoritative schema forbids additional properties.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionRealizationV1 {
    schema: &'static str,
    action_id: IdHexV1,
    contract_digest: DigestHexV1,
    speech_act: CanonicalTokenV1,
    owned_claims: Vec<OwnedClaimV1>,
    proposed_tools: Vec<ToolProposalV1>,
    disclosures_used: Vec<DisclosureUseV1>,
    manifest_confidence: UnitIntervalV1,
    #[serde(skip)]
    realization_digest: Digest,
}

impl ActionRealizationV1 {
    pub fn for_contract(
        contract: &ActionContractV1,
        owned_claims: Vec<OwnedClaimV1>,
        proposed_tools: Vec<ToolProposalV1>,
        disclosures_used: Vec<DisclosureUseV1>,
        manifest_confidence: UnitIntervalV1,
    ) -> Result<Self, ActionCoreErrorV1> {
        validate_owned_claims(&owned_claims)?;
        validate_proposed_tools(&proposed_tools)?;
        validate_disclosures(&disclosures_used)?;

        for (index, proposal) in proposed_tools.iter().enumerate() {
            if !contract.allowed_tools.contains(&proposal.tool_id) {
                return Err(ActionCoreErrorV1::ToolNotAllowed { index });
            }
        }
        for (index, disclosure) in disclosures_used.iter().enumerate() {
            if !contract
                .allowed_disclosures
                .contains(&disclosure.disclosure_id)
            {
                return Err(ActionCoreErrorV1::DisclosureNotAllowed { index });
            }
        }
        let shape_is_valid = match contract.disposition {
            ActionDispositionV1::Silence => {
                owned_claims.is_empty() && proposed_tools.is_empty() && disclosures_used.is_empty()
            }
            ActionDispositionV1::Speech => proposed_tools.is_empty(),
            ActionDispositionV1::ToolPlan | ActionDispositionV1::SpeechAndToolPlan => {
                !proposed_tools.is_empty()
            }
        };
        if !shape_is_valid {
            return Err(ActionCoreErrorV1::InvalidRealizationShape);
        }

        let claims_digest = list_digest(
            b"astr-embodiment/r7/action-realization/owned-claims-v1",
            owned_claims.iter().map(OwnedClaimV1::identity_digest),
        );
        let tools_digest = list_digest(
            b"astr-embodiment/r7/action-realization/proposed-tools-v1",
            proposed_tools.iter().map(ToolProposalV1::identity_digest),
        );
        let disclosures_digest = list_digest(
            b"astr-embodiment/r7/action-realization/disclosures-used-v1",
            disclosures_used
                .iter()
                .map(DisclosureUseV1::identity_digest),
        );
        let confidence = manifest_confidence.parts_per_million().to_be_bytes();
        let realization_digest = wire::domain_hash(
            ACTION_REALIZATION_DOMAIN_V1,
            &[
                contract.action_id.bytes(),
                contract.contract_digest.bytes(),
                contract.speech_act.as_str().as_bytes(),
                &claims_digest,
                &tools_digest,
                &disclosures_digest,
                &confidence,
            ],
        );
        Ok(Self {
            schema: ACTION_REALIZATION_SCHEMA_V1,
            action_id: contract.action_id,
            contract_digest: contract.contract_digest,
            speech_act: contract.speech_act.clone(),
            owned_claims,
            proposed_tools,
            disclosures_used,
            manifest_confidence,
            realization_digest,
        })
    }

    pub fn action_id(&self) -> &Id128 {
        self.action_id.bytes()
    }

    pub fn contract_digest(&self) -> &Digest {
        self.contract_digest.bytes()
    }

    pub fn speech_act(&self) -> &CanonicalTokenV1 {
        &self.speech_act
    }

    pub fn realization_digest(&self) -> &Digest {
        &self.realization_digest
    }
}

fn validate_owned_claims(claims: &[OwnedClaimV1]) -> Result<(), ActionCoreErrorV1> {
    if claims.len() > usize::from(MAX_OWNED_CLAIMS) {
        return Err(ActionCoreErrorV1::TooManyItems {
            field: "owned_claims",
            max_items: MAX_OWNED_CLAIMS,
            actual_items: claims.len(),
        });
    }
    for (offset, pair) in claims.windows(2).enumerate() {
        match pair[0]
            .claim_digest
            .bytes()
            .cmp(pair[1].claim_digest.bytes())
        {
            Ordering::Equal => {
                return Err(ActionCoreErrorV1::DuplicateOwnedClaim { index: offset + 1 });
            }
            Ordering::Greater => {
                return Err(ActionCoreErrorV1::NonCanonicalOwnedClaimOrder { index: offset + 1 });
            }
            Ordering::Less => {}
        }
    }
    Ok(())
}

fn validate_proposed_tools(tools: &[ToolProposalV1]) -> Result<(), ActionCoreErrorV1> {
    if tools.len() > usize::from(MAX_PROPOSED_TOOLS) {
        return Err(ActionCoreErrorV1::TooManyItems {
            field: "proposed_tools",
            max_items: MAX_PROPOSED_TOOLS,
            actual_items: tools.len(),
        });
    }
    for (offset, pair) in tools.windows(2).enumerate() {
        match pair[0].tool_id.as_str().cmp(pair[1].tool_id.as_str()) {
            Ordering::Equal => {
                return Err(ActionCoreErrorV1::DuplicateProposedTool { index: offset + 1 });
            }
            Ordering::Greater => {
                return Err(ActionCoreErrorV1::NonCanonicalProposedToolOrder { index: offset + 1 });
            }
            Ordering::Less => {}
        }
    }
    Ok(())
}

fn validate_disclosures(disclosures: &[DisclosureUseV1]) -> Result<(), ActionCoreErrorV1> {
    if disclosures.len() > usize::from(MAX_DISCLOSURES_USED) {
        return Err(ActionCoreErrorV1::TooManyItems {
            field: "disclosures_used",
            max_items: MAX_DISCLOSURES_USED,
            actual_items: disclosures.len(),
        });
    }
    for (offset, pair) in disclosures.windows(2).enumerate() {
        match pair[0]
            .disclosure_id
            .as_str()
            .cmp(pair[1].disclosure_id.as_str())
        {
            Ordering::Equal => {
                return Err(ActionCoreErrorV1::DuplicateDisclosure { index: offset + 1 });
            }
            Ordering::Greater => {
                return Err(ActionCoreErrorV1::NonCanonicalDisclosureOrder { index: offset + 1 });
            }
            Ordering::Less => {}
        }
    }
    Ok(())
}

fn list_digest(domain: &[u8], digests: impl Iterator<Item = Digest>) -> Digest {
    let digests: Vec<Digest> = digests.collect();
    let fields: Vec<&[u8]> = digests.iter().map(|digest| digest.as_slice()).collect();
    wire::domain_hash(domain, &fields)
}

fn require_digest(digest: &Digest, field: &'static str) -> Result<(), ActionCoreErrorV1> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(ActionCoreErrorV1::ZeroDigest { field });
    }
    Ok(())
}

fn require_id(id: &Id128, field: &'static str) -> Result<(), ActionCoreErrorV1> {
    if id.iter().all(|byte| *byte == 0) {
        return Err(ActionCoreErrorV1::ZeroId { field });
    }
    Ok(())
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

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}
