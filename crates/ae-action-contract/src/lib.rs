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

/// Fixed, inert wire codec limits for `ActionContractV1`.
pub const ACTION_CONTRACT_CODEC_MAGIC_V1: [u8; 8] = *b"AEACTV1\0";
pub const ACTION_CONTRACT_CODEC_VERSION_V1: u16 = 1;
pub const ACTION_CONTRACT_CODEC_HEADER_BYTES_V1: usize = 14;
pub const MAX_ACTION_CONTRACT_BYTES: usize = 65_536;
pub const MAX_CODEC_TOKEN_BYTES: usize = 128;
pub const MAX_SPEECH_ACT_BYTES: usize = 64;
pub const MAX_REQUIREMENT_ITEMS: u16 = 64;

const ACTION_CONTRACT_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/action-contract-v1";
const ACTION_REQUIREMENTS_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/action-requirements-v1";
const ACTION_REALIZATION_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/action-realization-v1";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ActionCodecErrorV1 {
    #[error("codec input is shorter than the fixed header")]
    HeaderTruncated,
    #[error("codec magic is not recognized")]
    InvalidMagic,
    #[error("codec version {0} is not supported")]
    UnsupportedVersion(u16),
    #[error("codec body length {declared} does not match input body length {actual}")]
    BodyLengthMismatch { declared: usize, actual: usize },
    #[error("codec body exceeds the fixed maximum of {max} bytes")]
    BodyTooLong { max: usize },
    #[error("codec input is truncated while reading {field}")]
    Truncated { field: &'static str },
    #[error("codec contains trailing bytes")]
    TrailingBytes,
    #[error("codec length arithmetic overflow")]
    Overflow,
    #[error("codec {field} exceeds the fixed bound ({actual} > {max})")]
    Bounds {
        field: &'static str,
        actual: usize,
        max: usize,
    },
    #[error("codec {field} is not valid UTF-8")]
    InvalidUtf8 { field: &'static str },
    #[error("codec disposition code {0} is unknown")]
    UnknownDisposition(u8),
    #[error("codec contract digest does not match semantic fields")]
    DigestMismatch,
    #[error("codec bytes are not canonical after decode/re-encode")]
    NonCanonical,
    #[error(transparent)]
    Core(#[from] ActionCoreErrorV1),
}

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

    pub fn must(&self) -> &CanonicalTokenSetV1 {
        &self.must
    }

    pub fn should(&self) -> &CanonicalTokenSetV1 {
        &self.should
    }

    pub fn may(&self) -> &CanonicalTokenSetV1 {
        &self.may
    }

    pub fn must_not(&self) -> &CanonicalTokenSetV1 {
        &self.must_not
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

        let contract_digest = compute_contract_digest(
            &action_id,
            &turn_binding,
            base_revision,
            &source_state_digest,
            &identity_constitution_digest,
            disposition,
            &speech_act,
            &requirements,
            &allowed_tools,
            &allowed_disclosures,
            confidence_ceiling,
            expires_at_ms,
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

    pub fn disposition(&self) -> ActionDispositionV1 {
        self.disposition
    }

    pub fn requirements(&self) -> &ActionRequirementsV1 {
        &self.requirements
    }

    pub fn allowed_tools(&self) -> &CanonicalTokenSetV1 {
        &self.allowed_tools
    }

    pub fn allowed_disclosures(&self) -> &CanonicalTokenSetV1 {
        &self.allowed_disclosures
    }

    pub fn confidence_ceiling(&self) -> UnitIntervalV1 {
        self.confidence_ceiling
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

#[allow(clippy::too_many_arguments)]
fn compute_contract_digest(
    action_id: &Id128,
    turn_binding: &Digest,
    base_revision: u64,
    source_state_digest: &Digest,
    identity_constitution_digest: &Digest,
    disposition: ActionDispositionV1,
    speech_act: &CanonicalTokenV1,
    requirements: &ActionRequirementsV1,
    allowed_tools: &CanonicalTokenSetV1,
    allowed_disclosures: &CanonicalTokenSetV1,
    confidence_ceiling: UnitIntervalV1,
    expires_at_ms: u64,
) -> Digest {
    let revision = base_revision.to_be_bytes();
    let requirements_digest = requirements.content_digest();
    let allowed_tools_digest =
        allowed_tools.content_digest(b"astr-embodiment/r7/action-contract/allowed-tools-v1");
    let allowed_disclosures_digest = allowed_disclosures
        .content_digest(b"astr-embodiment/r7/action-contract/allowed-disclosures-v1");
    let confidence = confidence_ceiling.parts_per_million().to_be_bytes();
    let expiry = expires_at_ms.to_be_bytes();
    wire::domain_hash(
        ACTION_CONTRACT_DOMAIN_V1,
        &[
            action_id,
            turn_binding,
            &revision,
            source_state_digest,
            identity_constitution_digest,
            disposition.name(),
            speech_act.as_str().as_bytes(),
            &requirements_digest,
            &allowed_tools_digest,
            &allowed_disclosures_digest,
            &confidence,
            &expiry,
        ],
    )
}

fn validate_fixed_codec_contract(contract: &ActionContractV1) -> Result<(), ActionCodecErrorV1> {
    require_id(contract.action_id(), "action_id")?;
    require_digest(contract.turn_binding(), "turn_binding")?;
    require_digest(contract.source_state_digest(), "source_state_digest")?;
    require_digest(
        contract.identity_constitution_digest(),
        "identity_constitution_digest",
    )?;
    if contract.speech_act().as_str().len() > MAX_SPEECH_ACT_BYTES {
        return Err(ActionCodecErrorV1::Bounds {
            field: "speech_act",
            actual: contract.speech_act().as_str().len(),
            max: MAX_SPEECH_ACT_BYTES,
        });
    }
    validate_fixed_token(contract.speech_act(), "speech_act")?;
    validate_fixed_set(
        contract.requirements().must(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.must",
    )?;
    validate_fixed_set(
        contract.requirements().should(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.should",
    )?;
    validate_fixed_set(
        contract.requirements().may(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.may",
    )?;
    validate_fixed_set(
        contract.requirements().must_not(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.must_not",
    )?;
    validate_fixed_set(
        contract.allowed_tools(),
        MAX_PROPOSED_TOOLS,
        "allowed_tools",
    )?;
    validate_fixed_set(
        contract.allowed_disclosures(),
        MAX_DISCLOSURES_USED,
        "allowed_disclosures",
    )?;
    if contract.expires_at_ms() == 0 {
        return Err(ActionCodecErrorV1::Core(ActionCoreErrorV1::MissingExpiry));
    }
    let expected = compute_contract_digest(
        contract.action_id(),
        contract.turn_binding(),
        contract.base_revision(),
        contract.source_state_digest(),
        contract.identity_constitution_digest(),
        contract.disposition(),
        contract.speech_act(),
        contract.requirements(),
        contract.allowed_tools(),
        contract.allowed_disclosures(),
        contract.confidence_ceiling(),
        contract.expires_at_ms(),
    );
    if expected != *contract.contract_digest() {
        return Err(ActionCodecErrorV1::DigestMismatch);
    }
    Ok(())
}

fn validate_fixed_token(
    token: &CanonicalTokenV1,
    field: &'static str,
) -> Result<(), ActionCodecErrorV1> {
    if token.as_str().len() > MAX_CODEC_TOKEN_BYTES {
        return Err(ActionCodecErrorV1::Bounds {
            field,
            actual: token.as_str().len(),
            max: MAX_CODEC_TOKEN_BYTES,
        });
    }
    if !is_canonical_token(token.as_str()) {
        return Err(ActionCodecErrorV1::Core(
            ActionCoreErrorV1::NonCanonicalToken,
        ));
    }
    Ok(())
}

fn validate_fixed_set(
    set: &CanonicalTokenSetV1,
    max_items: u16,
    field: &'static str,
) -> Result<(), ActionCodecErrorV1> {
    if set.as_slice().len() > usize::from(max_items) {
        return Err(ActionCodecErrorV1::Bounds {
            field,
            actual: set.as_slice().len(),
            max: usize::from(max_items),
        });
    }
    for token in set.as_slice() {
        validate_fixed_token(token, field)?;
    }
    Ok(())
}

struct CodecReader<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> CodecReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn remaining(&self) -> usize {
        self.input.len().saturating_sub(self.offset)
    }

    fn take(&mut self, count: usize, field: &'static str) -> Result<&'a [u8], ActionCodecErrorV1> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or(ActionCodecErrorV1::Overflow)?;
        if end > self.input.len() {
            return Err(ActionCodecErrorV1::Truncated { field });
        }
        let result = &self.input[self.offset..end];
        self.offset = end;
        Ok(result)
    }

    fn read_u8(&mut self, field: &'static str) -> Result<u8, ActionCodecErrorV1> {
        Ok(self.take(1, field)?[0])
    }

    fn read_u16(&mut self, field: &'static str) -> Result<u16, ActionCodecErrorV1> {
        Ok(u16::from_le_bytes(
            self.take(2, field)?
                .try_into()
                .map_err(|_| ActionCodecErrorV1::Overflow)?,
        ))
    }

    fn read_u32(&mut self, field: &'static str) -> Result<u32, ActionCodecErrorV1> {
        Ok(u32::from_le_bytes(
            self.take(4, field)?
                .try_into()
                .map_err(|_| ActionCodecErrorV1::Overflow)?,
        ))
    }

    fn read_u64(&mut self, field: &'static str) -> Result<u64, ActionCodecErrorV1> {
        Ok(u64::from_le_bytes(
            self.take(8, field)?
                .try_into()
                .map_err(|_| ActionCodecErrorV1::Overflow)?,
        ))
    }

    fn read_array<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], ActionCodecErrorV1> {
        self.take(N, field)?
            .try_into()
            .map_err(|_| ActionCodecErrorV1::Overflow)
    }
}

fn encode_token(
    output: &mut Vec<u8>,
    token: &CanonicalTokenV1,
    field: &'static str,
) -> Result<(), ActionCodecErrorV1> {
    validate_fixed_token(token, field)?;
    let length = u16::try_from(token.as_str().len()).map_err(|_| ActionCodecErrorV1::Overflow)?;
    output.extend_from_slice(&length.to_le_bytes());
    output.extend_from_slice(token.as_str().as_bytes());
    Ok(())
}

fn encode_set(
    output: &mut Vec<u8>,
    set: &CanonicalTokenSetV1,
    max_items: u16,
    field: &'static str,
) -> Result<(), ActionCodecErrorV1> {
    validate_fixed_set(set, max_items, field)?;
    let count = u16::try_from(set.as_slice().len()).map_err(|_| ActionCodecErrorV1::Overflow)?;
    output.extend_from_slice(&count.to_le_bytes());
    for token in set.as_slice() {
        encode_token(output, token, field)?;
    }
    Ok(())
}

/// Encode an `ActionContractV1` using the fixed canonical little-endian wire format.
pub fn encode_action_contract_v1(
    contract: &ActionContractV1,
) -> Result<Vec<u8>, ActionCodecErrorV1> {
    validate_fixed_codec_contract(contract)?;
    let mut body = Vec::new();
    body.extend_from_slice(contract.action_id());
    body.extend_from_slice(contract.turn_binding());
    body.extend_from_slice(&contract.base_revision().to_le_bytes());
    body.extend_from_slice(contract.source_state_digest());
    body.extend_from_slice(contract.identity_constitution_digest());
    body.push(match contract.disposition() {
        ActionDispositionV1::Silence => 0,
        ActionDispositionV1::Speech => 1,
        ActionDispositionV1::ToolPlan => 2,
        ActionDispositionV1::SpeechAndToolPlan => 3,
    });
    encode_token(&mut body, contract.speech_act(), "speech_act")?;
    encode_set(
        &mut body,
        contract.requirements().must(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.must",
    )?;
    encode_set(
        &mut body,
        contract.requirements().should(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.should",
    )?;
    encode_set(
        &mut body,
        contract.requirements().may(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.may",
    )?;
    encode_set(
        &mut body,
        contract.requirements().must_not(),
        MAX_REQUIREMENT_ITEMS,
        "requirements.must_not",
    )?;
    encode_set(
        &mut body,
        contract.allowed_tools(),
        MAX_PROPOSED_TOOLS,
        "allowed_tools",
    )?;
    encode_set(
        &mut body,
        contract.allowed_disclosures(),
        MAX_DISCLOSURES_USED,
        "allowed_disclosures",
    )?;
    body.extend_from_slice(
        &contract
            .confidence_ceiling()
            .parts_per_million()
            .to_le_bytes(),
    );
    body.extend_from_slice(&contract.expires_at_ms().to_le_bytes());
    body.extend_from_slice(contract.contract_digest());

    if body.len() > MAX_ACTION_CONTRACT_BYTES - ACTION_CONTRACT_CODEC_HEADER_BYTES_V1 {
        return Err(ActionCodecErrorV1::BodyTooLong {
            max: MAX_ACTION_CONTRACT_BYTES - ACTION_CONTRACT_CODEC_HEADER_BYTES_V1,
        });
    }
    let body_len = u32::try_from(body.len()).map_err(|_| ActionCodecErrorV1::Overflow)?;
    let mut output = Vec::with_capacity(ACTION_CONTRACT_CODEC_HEADER_BYTES_V1 + body.len());
    output.extend_from_slice(&ACTION_CONTRACT_CODEC_MAGIC_V1);
    output.extend_from_slice(&ACTION_CONTRACT_CODEC_VERSION_V1.to_le_bytes());
    output.extend_from_slice(&body_len.to_le_bytes());
    output.extend_from_slice(&body);
    Ok(output)
}

fn decode_token(
    reader: &mut CodecReader<'_>,
    field: &'static str,
    max_bytes: usize,
) -> Result<CanonicalTokenV1, ActionCodecErrorV1> {
    let length = usize::from(reader.read_u16(field)?);
    if length == 0 {
        return Err(ActionCodecErrorV1::Core(ActionCoreErrorV1::EmptyToken));
    }
    if length > max_bytes {
        return Err(ActionCodecErrorV1::Bounds {
            field,
            actual: length,
            max: max_bytes,
        });
    }
    let bytes = reader.take(length, field)?;
    let value = std::str::from_utf8(bytes)
        .map_err(|_| ActionCodecErrorV1::InvalidUtf8 { field })?
        .to_owned();
    CanonicalTokenV1::new(value, MAX_CODEC_TOKEN_BYTES as u16).map_err(ActionCodecErrorV1::Core)
}

fn decode_set(
    reader: &mut CodecReader<'_>,
    field: &'static str,
    max_items: u16,
) -> Result<CanonicalTokenSetV1, ActionCodecErrorV1> {
    let count = usize::from(reader.read_u16(field)?);
    if count > usize::from(max_items) {
        return Err(ActionCodecErrorV1::Bounds {
            field,
            actual: count,
            max: usize::from(max_items),
        });
    }
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(decode_token(reader, field, MAX_CODEC_TOKEN_BYTES)?);
    }
    CanonicalTokenSetV1::new(values, max_items).map_err(ActionCodecErrorV1::Core)
}

/// Decode and strictly validate a fixed canonical `ActionContractV1` byte string.
pub fn decode_action_contract_v1(input: &[u8]) -> Result<ActionContractV1, ActionCodecErrorV1> {
    if input.len() < ACTION_CONTRACT_CODEC_HEADER_BYTES_V1 {
        return Err(ActionCodecErrorV1::HeaderTruncated);
    }
    if input[..8] != ACTION_CONTRACT_CODEC_MAGIC_V1 {
        return Err(ActionCodecErrorV1::InvalidMagic);
    }
    let version = u16::from_le_bytes([input[8], input[9]]);
    if version != ACTION_CONTRACT_CODEC_VERSION_V1 {
        return Err(ActionCodecErrorV1::UnsupportedVersion(version));
    }
    let body_len = usize::try_from(u32::from_le_bytes([
        input[10], input[11], input[12], input[13],
    ]))
    .map_err(|_| ActionCodecErrorV1::Overflow)?;
    let max_body = MAX_ACTION_CONTRACT_BYTES - ACTION_CONTRACT_CODEC_HEADER_BYTES_V1;
    if body_len > max_body {
        return Err(ActionCodecErrorV1::BodyTooLong { max: max_body });
    }
    let actual_body_len = input
        .len()
        .checked_sub(ACTION_CONTRACT_CODEC_HEADER_BYTES_V1)
        .ok_or(ActionCodecErrorV1::Overflow)?;
    if body_len != actual_body_len {
        return Err(ActionCodecErrorV1::BodyLengthMismatch {
            declared: body_len,
            actual: actual_body_len,
        });
    }

    let mut reader = CodecReader::new(&input[ACTION_CONTRACT_CODEC_HEADER_BYTES_V1..]);
    let action_id = reader.read_array::<16>("action_id")?;
    let turn_binding = reader.read_array::<32>("turn_binding")?;
    let base_revision = reader.read_u64("base_revision")?;
    let source_state_digest = reader.read_array::<32>("source_state_digest")?;
    let identity_constitution_digest = reader.read_array::<32>("identity_constitution_digest")?;
    let disposition = match reader.read_u8("disposition")? {
        0 => ActionDispositionV1::Silence,
        1 => ActionDispositionV1::Speech,
        2 => ActionDispositionV1::ToolPlan,
        3 => ActionDispositionV1::SpeechAndToolPlan,
        value => return Err(ActionCodecErrorV1::UnknownDisposition(value)),
    };
    let speech_act = decode_token(&mut reader, "speech_act", MAX_SPEECH_ACT_BYTES)?;
    let requirements = ActionRequirementsV1::new(
        decode_set(&mut reader, "requirements.must", MAX_REQUIREMENT_ITEMS)?,
        decode_set(&mut reader, "requirements.should", MAX_REQUIREMENT_ITEMS)?,
        decode_set(&mut reader, "requirements.may", MAX_REQUIREMENT_ITEMS)?,
        decode_set(&mut reader, "requirements.must_not", MAX_REQUIREMENT_ITEMS)?,
    );
    let allowed_tools = decode_set(&mut reader, "allowed_tools", MAX_PROPOSED_TOOLS)?;
    let allowed_disclosures = decode_set(&mut reader, "allowed_disclosures", MAX_DISCLOSURES_USED)?;
    let confidence = reader.read_u32("confidence_ceiling")?;
    let confidence_ceiling =
        UnitIntervalV1::from_parts_per_million(confidence).map_err(ActionCodecErrorV1::Core)?;
    let expires_at_ms = reader.read_u64("expires_at_ms")?;
    let contract_digest = reader.read_array::<32>("contract_digest")?;
    if reader.remaining() != 0 {
        return Err(ActionCodecErrorV1::TrailingBytes);
    }

    let contract = ActionContractV1::from_evaluation(
        action_id,
        turn_binding,
        base_revision,
        source_state_digest,
        identity_constitution_digest,
        disposition,
        speech_act,
        requirements,
        allowed_tools,
        allowed_disclosures,
        confidence_ceiling,
        expires_at_ms,
    )
    .map_err(ActionCodecErrorV1::Core)?;
    if contract_digest != *contract.contract_digest() {
        return Err(ActionCodecErrorV1::DigestMismatch);
    }
    let canonical = encode_action_contract_v1(&contract)?;
    if canonical != input {
        return Err(ActionCodecErrorV1::NonCanonical);
    }
    Ok(contract)
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
