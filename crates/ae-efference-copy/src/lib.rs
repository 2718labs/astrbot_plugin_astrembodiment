#![forbid(unsafe_code)]

//! Pure native R7 efference-copy source.
//!
//! R7 defines the action-realization schema and the separation between motor
//! intention, candidate realization, and actual effect, but it does not define a
//! standalone efference-copy JSON schema or a comparison formula. This crate
//! therefore exposes only typed native inputs. Expected and observed dispositions
//! and every effect classification are caller-provided closed enums; no text,
//! provider payload, delivery receipt, user content, neural state, or KV value is
//! accepted or inferred.

use ae_action_contract::{ActionContractV1, ActionRealizationV1};
use ae_contracts::r7::{wire, Digest, Id128};
use std::collections::BTreeSet;
use thiserror::Error;

pub const MAX_EFFECT_DIGEST_RECORDS: u16 = 64;

const EFFECT_RECORD_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/efference-effect-record-v1";
const EFFECT_RECORD_LIST_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/efference-effect-record-list-v1";
const EFFERENCE_COPY_DOMAIN_V1: &[u8] = b"astr-embodiment/r7/efference-copy-v1";

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum EfferenceCopyErrorV1 {
    #[error("action identity does not match the typed realization")]
    ActionIdMismatch,
    #[error("action contract digest does not match the typed realization")]
    ContractDigestMismatch,
    #[error("speech act does not match the typed realization")]
    SpeechActMismatch,
    #[error("typed realization digest must be nonzero")]
    ZeroRealizationDigest,
    #[error("effect digest must be nonzero")]
    ZeroEffectDigest,
    #[error("effect record count {actual} exceeds {max}")]
    TooManyEffectRecords { max: u16, actual: usize },
    #[error("effect record {index} has ordinal {actual}, expected {expected}")]
    NonContiguousEffectOrdinal {
        index: usize,
        expected: u16,
        actual: u16,
    },
    #[error("action feedback has already been formed")]
    ReplayedAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpectedDispositionV1 {
    Silence,
    Speech,
    ToolPlan,
    SpeechAndToolPlan,
}

impl ExpectedDispositionV1 {
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
pub enum ObservedDispositionV1 {
    NoEffect,
    Speech,
    ToolEffect,
    SpeechAndToolEffect,
}

impl ObservedDispositionV1 {
    fn name(self) -> &'static [u8] {
        match self {
            Self::NoEffect => b"no_effect",
            Self::Speech => b"speech",
            Self::ToolEffect => b"tool_effect",
            Self::SpeechAndToolEffect => b"speech_and_tool_effect",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectPhaseV1 {
    Expected,
    Observed,
}

impl EffectPhaseV1 {
    fn name(self) -> &'static [u8] {
        match self {
            Self::Expected => b"expected",
            Self::Observed => b"observed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectClassV1 {
    VisibleOutput,
    ToolEffect,
    Disclosure,
}

impl EffectClassV1 {
    fn name(self) -> &'static [u8] {
        match self {
            Self::VisibleOutput => b"visible_output",
            Self::ToolEffect => b"tool_effect",
            Self::Disclosure => b"disclosure",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectDigestRecordV1 {
    ordinal: u16,
    phase: EffectPhaseV1,
    class: EffectClassV1,
    effect_digest: Digest,
}

impl EffectDigestRecordV1 {
    pub fn new(
        ordinal: u16,
        phase: EffectPhaseV1,
        class: EffectClassV1,
        effect_digest: Digest,
    ) -> Result<Self, EfferenceCopyErrorV1> {
        if effect_digest.iter().all(|byte| *byte == 0) {
            return Err(EfferenceCopyErrorV1::ZeroEffectDigest);
        }
        Ok(Self {
            ordinal,
            phase,
            class,
            effect_digest,
        })
    }

    pub fn ordinal(&self) -> u16 {
        self.ordinal
    }

    pub fn phase(&self) -> EffectPhaseV1 {
        self.phase
    }

    pub fn class(&self) -> EffectClassV1 {
        self.class
    }

    pub fn effect_digest(&self) -> &Digest {
        &self.effect_digest
    }

    fn record_digest(&self) -> Digest {
        let ordinal = self.ordinal.to_be_bytes();
        wire::domain_hash(
            EFFECT_RECORD_DOMAIN_V1,
            &[
                &ordinal,
                self.phase.name(),
                self.class.name(),
                &self.effect_digest,
            ],
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EfferenceCopyV1 {
    action_id: Id128,
    contract_digest: Digest,
    realization_digest: Digest,
    expected_disposition: ExpectedDispositionV1,
    observed_disposition: ObservedDispositionV1,
    effect_records: Vec<EffectDigestRecordV1>,
    copy_digest: Digest,
}

impl EfferenceCopyV1 {
    pub fn action_id(&self) -> &Id128 {
        &self.action_id
    }

    pub fn contract_digest(&self) -> &Digest {
        &self.contract_digest
    }

    pub fn realization_digest(&self) -> &Digest {
        &self.realization_digest
    }

    pub fn expected_disposition(&self) -> ExpectedDispositionV1 {
        self.expected_disposition
    }

    pub fn observed_disposition(&self) -> ObservedDispositionV1 {
        self.observed_disposition
    }

    pub fn effect_records(&self) -> &[EffectDigestRecordV1] {
        &self.effect_records
    }

    pub fn copy_digest(&self) -> &Digest {
        &self.copy_digest
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ActionReplayKeyV1 {
    action_id: Id128,
    contract_digest: Digest,
}

#[derive(Debug, Default)]
pub struct EfferenceCopySourceV1 {
    consumed_actions: BTreeSet<ActionReplayKeyV1>,
}

impl EfferenceCopySourceV1 {
    pub fn form(
        &mut self,
        contract: &ActionContractV1,
        realization: &ActionRealizationV1,
        expected_disposition: ExpectedDispositionV1,
        observed_disposition: ObservedDispositionV1,
        effect_records: Vec<EffectDigestRecordV1>,
    ) -> Result<EfferenceCopyV1, EfferenceCopyErrorV1> {
        if contract.action_id() != realization.action_id() {
            return Err(EfferenceCopyErrorV1::ActionIdMismatch);
        }
        if contract.contract_digest() != realization.contract_digest() {
            return Err(EfferenceCopyErrorV1::ContractDigestMismatch);
        }
        if contract.speech_act() != realization.speech_act() {
            return Err(EfferenceCopyErrorV1::SpeechActMismatch);
        }
        if realization
            .realization_digest()
            .iter()
            .all(|byte| *byte == 0)
        {
            return Err(EfferenceCopyErrorV1::ZeroRealizationDigest);
        }
        if effect_records.len() > usize::from(MAX_EFFECT_DIGEST_RECORDS) {
            return Err(EfferenceCopyErrorV1::TooManyEffectRecords {
                max: MAX_EFFECT_DIGEST_RECORDS,
                actual: effect_records.len(),
            });
        }
        for (index, record) in effect_records.iter().enumerate() {
            let expected = u16::try_from(index).expect("bounded effect index fits u16");
            if record.ordinal != expected {
                return Err(EfferenceCopyErrorV1::NonContiguousEffectOrdinal {
                    index,
                    expected,
                    actual: record.ordinal,
                });
            }
        }

        let replay_key = ActionReplayKeyV1 {
            action_id: *contract.action_id(),
            contract_digest: *contract.contract_digest(),
        };
        if self.consumed_actions.contains(&replay_key) {
            return Err(EfferenceCopyErrorV1::ReplayedAction);
        }

        let effect_digests: Vec<Digest> = effect_records
            .iter()
            .map(EffectDigestRecordV1::record_digest)
            .collect();
        let effect_fields: Vec<&[u8]> = effect_digests
            .iter()
            .map(|digest| digest.as_slice())
            .collect();
        let effect_records_digest = wire::domain_hash(EFFECT_RECORD_LIST_DOMAIN_V1, &effect_fields);
        let copy_digest = wire::domain_hash(
            EFFERENCE_COPY_DOMAIN_V1,
            &[
                contract.action_id(),
                contract.contract_digest(),
                realization.realization_digest(),
                expected_disposition.name(),
                observed_disposition.name(),
                &effect_records_digest,
            ],
        );
        let copy = EfferenceCopyV1 {
            action_id: *contract.action_id(),
            contract_digest: *contract.contract_digest(),
            realization_digest: *realization.realization_digest(),
            expected_disposition,
            observed_disposition,
            effect_records,
            copy_digest,
        };
        let inserted = self.consumed_actions.insert(replay_key);
        debug_assert!(inserted);
        Ok(copy)
    }
}
