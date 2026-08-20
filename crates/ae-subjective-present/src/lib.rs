#![forbid(unsafe_code)]

//! Closed, canonical R7 subjective-present projection values.
//!
//! This crate accepts only the bounded R7 `subjective_present` item shape and derives every
//! digest itself. It deliberately has no fields for organism arrays, Continuum-KV banks,
//! free-form emotional narratives, persistence, or provider transport.

use ae_contracts::{wire, Digest};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const SUBJECTIVE_PRESENT_MAX_ITEMS_V1: usize = 32;
pub const SUBJECTIVE_AXIS_MAX_CHARS_V1: usize = 64;
pub const SUBJECTIVE_BEHAVIORAL_EFFECT_MAX_CHARS_V1: usize = 256;
pub const SUBJECTIVE_CAUSE_REF_MAX_CHARS_V1: usize = 128;
pub const SUBJECTIVE_PRESENT_ITEM_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/subjective-present-item-v1";
pub const SUBJECTIVE_PRESENT_PROJECTION_DOMAIN_V1: &[u8] =
    b"astr-embodiment/r7/subjective-present-projection-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectiveBandV1 {
    VeryLow,
    Low,
    Moderate,
    High,
    VeryHigh,
}

impl SubjectiveBandV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::VeryLow => "very_low",
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
            Self::VeryHigh => "very_high",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectiveTrendV1 {
    FallingFast,
    Falling,
    Stable,
    Rising,
    RisingFast,
}

impl SubjectiveTrendV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::FallingFast => "falling_fast",
            Self::Falling => "falling",
            Self::Stable => "stable",
            Self::Rising => "rising",
            Self::RisingFast => "rising_fast",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DisclosureV1 {
    PrivateControl,
    BehavioralOnly,
    ImplicitAllowed,
    ExplicitAllowed,
    Required,
}

impl DisclosureV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::PrivateControl => "PRIVATE_CONTROL",
            Self::BehavioralOnly => "BEHAVIORAL_ONLY",
            Self::ImplicitAllowed => "IMPLICIT_ALLOWED",
            Self::ExplicitAllowed => "EXPLICIT_ALLOWED",
            Self::Required => "REQUIRED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceV1 {
    Low,
    Moderate,
    High,
}

impl ConfidenceV1 {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectivePresentInputV1 {
    pub axis: String,
    pub band: SubjectiveBandV1,
    pub trend: SubjectiveTrendV1,
    pub behavioral_effect: String,
    pub disclosure: DisclosureV1,
    pub confidence: ConfidenceV1,
    pub cause_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SubjectivePresentErrorV1 {
    #[error("invalid subjective-present JSON shape")]
    InvalidJson { reason: &'static str },
    #[error("{field} must not be empty")]
    EmptyToken { field: &'static str },
    #[error("{field} exceeds its character bound ({actual_chars} > {max_chars})")]
    TokenTooLong {
        field: &'static str,
        max_chars: usize,
        actual_chars: usize,
    },
    #[error("{field} is not a canonical token")]
    NonCanonicalToken { field: &'static str },
    #[error("subjective_present has {actual_items} items, above {max_items}")]
    TooManyItems {
        max_items: usize,
        actual_items: usize,
    },
    #[error("subjective_present has a duplicate axis at index {index}")]
    DuplicateAxis { index: usize },
    #[error("subjective_present axis order is noncanonical at index {index}")]
    NonCanonicalAxisOrder { index: usize },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSubjectivePresentV1 {
    axis: String,
    band: SubjectiveBandV1,
    trend: SubjectiveTrendV1,
    behavioral_effect: String,
    disclosure: DisclosureV1,
    confidence: ConfidenceV1,
    #[serde(default)]
    cause_ref: Option<String>,
}

impl From<RawSubjectivePresentV1> for SubjectivePresentInputV1 {
    fn from(raw: RawSubjectivePresentV1) -> Self {
        Self {
            axis: raw.axis,
            band: raw.band,
            trend: raw.trend,
            behavioral_effect: raw.behavioral_effect,
            disclosure: raw.disclosure,
            confidence: raw.confidence,
            cause_ref: raw.cause_ref,
        }
    }
}

/// One bounded subjective item that can be projected to the R7 schema shape.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectivePresentV1 {
    axis: String,
    band: SubjectiveBandV1,
    trend: SubjectiveTrendV1,
    behavioral_effect: String,
    disclosure: DisclosureV1,
    confidence: ConfidenceV1,
    cause_ref: Option<String>,
    identity_digest: Digest,
}

impl SubjectivePresentV1 {
    pub fn from_json(value: &str) -> Result<Self, SubjectivePresentErrorV1> {
        let raw = serde_json::from_str::<RawSubjectivePresentV1>(value).map_err(|_| {
            SubjectivePresentErrorV1::InvalidJson {
                reason: "item shape",
            }
        })?;
        Self::try_from_input(raw.into())
    }

    pub fn try_from_input(
        input: SubjectivePresentInputV1,
    ) -> Result<Self, SubjectivePresentErrorV1> {
        require_canonical_token("axis", &input.axis, SUBJECTIVE_AXIS_MAX_CHARS_V1)?;
        require_canonical_token(
            "behavioral_effect",
            &input.behavioral_effect,
            SUBJECTIVE_BEHAVIORAL_EFFECT_MAX_CHARS_V1,
        )?;
        if let Some(cause_ref) = &input.cause_ref {
            require_canonical_token("cause_ref", cause_ref, SUBJECTIVE_CAUSE_REF_MAX_CHARS_V1)?;
        }

        let identity_digest = item_digest(
            &input.axis,
            input.band,
            input.trend,
            &input.behavioral_effect,
            input.disclosure,
            input.confidence,
            input.cause_ref.as_deref(),
        );
        Ok(Self {
            axis: input.axis,
            band: input.band,
            trend: input.trend,
            behavioral_effect: input.behavioral_effect,
            disclosure: input.disclosure,
            confidence: input.confidence,
            cause_ref: input.cause_ref,
            identity_digest,
        })
    }

    pub fn axis(&self) -> &str {
        &self.axis
    }

    pub fn identity_digest(&self) -> &Digest {
        &self.identity_digest
    }

    pub fn to_canonical_json(&self) -> String {
        format!(
            r#"{{"axis":"{}","band":"{}","trend":"{}","behavioral_effect":"{}","disclosure":"{}","confidence":"{}","cause_ref":{}}}"#,
            self.axis,
            self.band.as_str(),
            self.trend.as_str(),
            self.behavioral_effect,
            self.disclosure.as_str(),
            self.confidence.as_str(),
            self.cause_ref
                .as_deref()
                .map_or_else(|| "null".to_owned(), |value| format!(r#""{value}""#)),
        )
    }
}

/// A bounded, ordered collection of subjective items for future SOMA/ASTER producers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectivePresentProjectionV1 {
    items: Vec<SubjectivePresentV1>,
    identity_digest: Digest,
}

impl SubjectivePresentProjectionV1 {
    pub fn from_json(value: &str) -> Result<Self, SubjectivePresentErrorV1> {
        let raw = serde_json::from_str::<Vec<RawSubjectivePresentV1>>(value).map_err(|_| {
            SubjectivePresentErrorV1::InvalidJson {
                reason: "collection shape",
            }
        })?;
        let items = raw
            .into_iter()
            .map(|item| SubjectivePresentV1::try_from_input(item.into()))
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(items)
    }

    pub fn new(items: Vec<SubjectivePresentV1>) -> Result<Self, SubjectivePresentErrorV1> {
        if items.len() > SUBJECTIVE_PRESENT_MAX_ITEMS_V1 {
            return Err(SubjectivePresentErrorV1::TooManyItems {
                max_items: SUBJECTIVE_PRESENT_MAX_ITEMS_V1,
                actual_items: items.len(),
            });
        }
        for (offset, pair) in items.windows(2).enumerate() {
            if pair[0].axis == pair[1].axis {
                return Err(SubjectivePresentErrorV1::DuplicateAxis { index: offset + 1 });
            }
            if pair[0].axis > pair[1].axis {
                return Err(SubjectivePresentErrorV1::NonCanonicalAxisOrder { index: offset + 1 });
            }
        }

        let item_count = u64::try_from(items.len())
            .expect("subjective-present collection length fits u64")
            .to_be_bytes();
        let mut fields = Vec::with_capacity(items.len() + 1);
        fields.push(item_count.as_slice());
        fields.extend(items.iter().map(|item| item.identity_digest.as_slice()));
        let identity_digest = wire::domain_hash(SUBJECTIVE_PRESENT_PROJECTION_DOMAIN_V1, &fields);
        Ok(Self {
            items,
            identity_digest,
        })
    }

    pub fn items(&self) -> &[SubjectivePresentV1] {
        &self.items
    }

    pub fn identity_digest(&self) -> &Digest {
        &self.identity_digest
    }

    pub fn to_canonical_json(&self) -> String {
        let items = self
            .items
            .iter()
            .map(SubjectivePresentV1::to_canonical_json)
            .collect::<Vec<_>>();
        format!("[{}]", items.join(","))
    }
}

fn item_digest(
    axis: &str,
    band: SubjectiveBandV1,
    trend: SubjectiveTrendV1,
    behavioral_effect: &str,
    disclosure: DisclosureV1,
    confidence: ConfidenceV1,
    cause_ref: Option<&str>,
) -> Digest {
    wire::domain_hash(
        SUBJECTIVE_PRESENT_ITEM_DOMAIN_V1,
        &[
            axis.as_bytes(),
            band.as_str().as_bytes(),
            trend.as_str().as_bytes(),
            behavioral_effect.as_bytes(),
            disclosure.as_str().as_bytes(),
            confidence.as_str().as_bytes(),
            cause_ref.unwrap_or("").as_bytes(),
        ],
    )
}

fn require_canonical_token(
    field: &'static str,
    value: &str,
    max_chars: usize,
) -> Result<(), SubjectivePresentErrorV1> {
    if value.is_empty() {
        return Err(SubjectivePresentErrorV1::EmptyToken { field });
    }
    let actual_chars = value.chars().count();
    if actual_chars > max_chars {
        return Err(SubjectivePresentErrorV1::TokenTooLong {
            field,
            max_chars,
            actual_chars,
        });
    }
    if !is_canonical_token(value) {
        return Err(SubjectivePresentErrorV1::NonCanonicalToken { field });
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
