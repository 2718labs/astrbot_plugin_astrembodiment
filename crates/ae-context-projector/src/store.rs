use std::fmt;
use std::path::Path;

use hmac::{Hmac, Mac};
use rusqlite::{params, Connection};
use sha2::Sha256;

const DIMENSION_COUNT: usize = 15;
const MAX_TURNS_PER_RELATION: i64 = 32;
const MAX_RELATIONS: i64 = 8;
const RELATION_HMAC_KEY: [u8; 32] = [
    0x6d, 0x18, 0x4a, 0xf3, 0x82, 0x97, 0x51, 0x0c, 0x34, 0xbe, 0x76, 0x29, 0xd1, 0x45, 0xa8, 0x63,
    0x9b, 0x27, 0xce, 0x40, 0x75, 0x1f, 0xe2, 0x5a, 0xb4, 0x08, 0x9d, 0x36, 0xc1, 0x7e, 0x52, 0xfa,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DeliveryOutcome {
    Pending = 0,
    Delivered = 1,
    Failed = 2,
}

impl DeliveryOutcome {
    fn from_code(code: i64) -> Result<Self, StoreError> {
        match code {
            0 => Ok(Self::Pending),
            1 => Ok(Self::Delivered),
            2 => Ok(Self::Failed),
            _ => Err(StoreError::CorruptPayload),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContextSummaryV1 {
    pub summary_revision: u32,
    pub source_continuum_revision: u64,
    pub dimensions_ema_fxp6: [i64; DIMENSION_COUNT],
    pub unresolved_boundary: bool,
    pub unresolved_repair: bool,
    pub repetition_count: u64,
    pub delivery_outcome: DeliveryOutcome,
    pub summary_digest: [u8; 32],
}

impl ContextSummaryV1 {
    pub const SCHEMA_VERSION: u32 = 1;
    pub const CANONICAL_BYTES_LEN: usize = 8 + 4 + 4 + 8 + (DIMENSION_COUNT * 8) + 1 + 1 + 8 + 1;

    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(Self::CANONICAL_BYTES_LEN);
        bytes.extend_from_slice(b"AECSUMV1");
        bytes.extend_from_slice(&Self::SCHEMA_VERSION.to_le_bytes());
        bytes.extend_from_slice(&self.summary_revision.to_le_bytes());
        bytes.extend_from_slice(&self.source_continuum_revision.to_le_bytes());
        for value in self.dimensions_ema_fxp6 {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(u8::from(self.unresolved_boundary));
        bytes.push(u8::from(self.unresolved_repair));
        bytes.extend_from_slice(&self.repetition_count.to_le_bytes());
        bytes.push(self.delivery_outcome as u8);
        bytes
    }

    pub fn digest_of(canonical_bytes: &[u8]) -> [u8; 32] {
        *blake3::hash(canonical_bytes).as_bytes()
    }

    fn new(
        summary_revision: u32,
        source_continuum_revision: u64,
        dimensions_ema_fxp6: [i64; DIMENSION_COUNT],
        unresolved_boundary: bool,
        unresolved_repair: bool,
        repetition_count: u64,
        delivery_outcome: DeliveryOutcome,
    ) -> Self {
        let mut summary = Self {
            summary_revision,
            source_continuum_revision,
            dimensions_ema_fxp6,
            unresolved_boundary,
            unresolved_repair,
            repetition_count,
            delivery_outcome,
            summary_digest: [0; 32],
        };
        summary.summary_digest = Self::digest_of(&summary.canonical_bytes());
        summary
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptCommitStatus {
    Pending,
    Committed,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReceiptEnvelopeV1 {
    pub commit_status: ReceiptCommitStatus,
    pub event_id: [u8; 16],
    pub relation_token: [u8; 16],
    pub source_continuum_revision: u64,
    pub dimensions_fxp6: [i64; DIMENSION_COUNT],
    pub unresolved_boundary: bool,
    pub unresolved_repair: bool,
    pub repetition_increment: u64,
    pub delivery_outcome: DeliveryOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedCommittedReceiptV1 {
    event_id: [u8; 16],
    relation_token: [u8; 16],
    source_continuum_revision: u64,
    dimensions_fxp6: [i64; DIMENSION_COUNT],
    unresolved_boundary: bool,
    unresolved_repair: bool,
    repetition_increment: u64,
    delivery_outcome: DeliveryOutcome,
}

impl ValidatedCommittedReceiptV1 {
    pub const MAX_ABS_DIMENSION_FXP6: i64 = 1_000_000_000_000;
    pub const MAX_REPETITION_INCREMENT: u64 = 4_096;

    pub fn try_from_envelope(envelope: ReceiptEnvelopeV1) -> Result<Self, ReceiptValidationError> {
        if envelope.commit_status != ReceiptCommitStatus::Committed {
            return Err(ReceiptValidationError::NotCommitted);
        }
        if envelope.event_id == [0; 16] {
            return Err(ReceiptValidationError::InvalidEventId);
        }
        if envelope.relation_token == [0; 16] {
            return Err(ReceiptValidationError::InvalidRelationToken);
        }
        if envelope.source_continuum_revision == 0 {
            return Err(ReceiptValidationError::InvalidSourceRevision);
        }
        if envelope
            .dimensions_fxp6
            .iter()
            .any(|dimension| dimension.unsigned_abs() > Self::MAX_ABS_DIMENSION_FXP6 as u64)
        {
            return Err(ReceiptValidationError::DimensionOutOfRange);
        }
        if envelope.repetition_increment == 0
            || envelope.repetition_increment > Self::MAX_REPETITION_INCREMENT
        {
            return Err(ReceiptValidationError::InvalidRepetitionIncrement);
        }
        Ok(Self {
            event_id: envelope.event_id,
            relation_token: envelope.relation_token,
            source_continuum_revision: envelope.source_continuum_revision,
            dimensions_fxp6: envelope.dimensions_fxp6,
            unresolved_boundary: envelope.unresolved_boundary,
            unresolved_repair: envelope.unresolved_repair,
            repetition_increment: envelope.repetition_increment,
            delivery_outcome: envelope.delivery_outcome,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReceiptValidationError {
    NotCommitted,
    InvalidEventId,
    InvalidRelationToken,
    InvalidSourceRevision,
    DimensionOutOfRange,
    InvalidRepetitionIncrement,
}

impl fmt::Display for ReceiptValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::NotCommitted => "receipt is not committed",
            Self::InvalidEventId => "receipt event id is invalid",
            Self::InvalidRelationToken => "receipt relation token is invalid",
            Self::InvalidSourceRevision => "receipt source revision is invalid",
            Self::DimensionOutOfRange => "receipt dimension is outside the fixed range",
            Self::InvalidRepetitionIncrement => "receipt repetition increment is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for ReceiptValidationError {}

#[derive(Debug)]
pub enum StoreError {
    Sqlite(rusqlite::Error),
    CorruptPayload,
    SummaryRevisionOverflow,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "sqlite error: {error}"),
            Self::CorruptPayload => formatter.write_str("stored context summary is corrupt"),
            Self::SummaryRevisionOverflow => {
                formatter.write_str("context summary revision overflow")
            }
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

pub struct ContextSummaryStore {
    connection: Connection,
}

impl ContextSummaryStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS relation_summaries (
                relation_hmac BLOB PRIMARY KEY NOT NULL CHECK(length(relation_hmac) = 32),
                summary_revision INTEGER NOT NULL,
                source_continuum_revision BLOB NOT NULL CHECK(length(source_continuum_revision) = 8),
                dimensions_ema_fxp6 BLOB NOT NULL CHECK(length(dimensions_ema_fxp6) = 120),
                unresolved_boundary INTEGER NOT NULL,
                unresolved_repair INTEGER NOT NULL,
                repetition_count INTEGER NOT NULL,
                delivery_outcome INTEGER NOT NULL,
                summary_digest BLOB NOT NULL CHECK(length(summary_digest) = 32)
            );
            CREATE TABLE IF NOT EXISTS relation_turns (
                relation_hmac BLOB NOT NULL CHECK(length(relation_hmac) = 32),
                event_id BLOB NOT NULL CHECK(length(event_id) = 16),
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                source_continuum_revision BLOB NOT NULL CHECK(length(source_continuum_revision) = 8),
                dimensions_fxp6 BLOB NOT NULL CHECK(length(dimensions_fxp6) = 120),
                unresolved_boundary INTEGER NOT NULL,
                unresolved_repair INTEGER NOT NULL,
                repetition_increment INTEGER NOT NULL,
                delivery_outcome INTEGER NOT NULL,
                UNIQUE(relation_hmac, event_id)
            );
            CREATE INDEX IF NOT EXISTS relation_turns_by_relation_sequence
                ON relation_turns(relation_hmac, sequence);
            ",
        )?;
        Ok(Self { connection })
    }

    pub fn apply_committed_receipt(
        &mut self,
        receipt: &ValidatedCommittedReceiptV1,
    ) -> Result<ContextSummaryV1, StoreError> {
        let relation_hmac = relation_hmac(receipt.relation_token);
        if self.event_exists(&relation_hmac, &receipt.event_id)? {
            return summary_for_hmac(&self.connection, &relation_hmac)?
                .ok_or(StoreError::CorruptPayload);
        }

        let transaction = self.connection.transaction()?;
        let previous_summary = summary_for_hmac(&*transaction, &relation_hmac)?;
        let relation_exists = previous_summary.is_some();
        if !relation_exists && relation_count(&*transaction)? >= MAX_RELATIONS {
            let evicted: Vec<u8> = transaction.query_row(
                "SELECT relation_hmac FROM relation_summaries ORDER BY relation_hmac ASC LIMIT 1",
                [],
                |row| row.get(0),
            )?;
            transaction.execute(
                "DELETE FROM relation_turns WHERE relation_hmac = ?1",
                params![evicted],
            )?;
            transaction.execute(
                "DELETE FROM relation_summaries WHERE relation_hmac = ?1",
                params![evicted],
            )?;
        }

        let next_revision = match previous_summary {
            Some(summary) => summary
                .summary_revision
                .checked_add(1)
                .ok_or(StoreError::SummaryRevisionOverflow)?,
            None => 1,
        };

        transaction.execute(
            "INSERT INTO relation_turns (
                relation_hmac, event_id, source_continuum_revision, dimensions_fxp6,
                unresolved_boundary, unresolved_repair, repetition_increment, delivery_outcome
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                &relation_hmac[..],
                &receipt.event_id[..],
                receipt.source_continuum_revision.to_le_bytes().to_vec(),
                encode_dimensions(&receipt.dimensions_fxp6),
                i64::from(receipt.unresolved_boundary),
                i64::from(receipt.unresolved_repair),
                i64::try_from(receipt.repetition_increment)
                    .map_err(|_| StoreError::CorruptPayload)?,
                receipt.delivery_outcome as i64,
            ],
        )?;
        transaction.execute(
            "DELETE FROM relation_turns
             WHERE relation_hmac = ?1
               AND sequence NOT IN (
                    SELECT sequence FROM relation_turns
                    WHERE relation_hmac = ?1
                    ORDER BY sequence DESC
                    LIMIT ?2
               )",
            params![&relation_hmac[..], MAX_TURNS_PER_RELATION],
        )?;

        let summary = summary_from_turns(&*transaction, &relation_hmac, next_revision)?;
        transaction.execute(
            "INSERT INTO relation_summaries (
                relation_hmac, summary_revision, source_continuum_revision, dimensions_ema_fxp6,
                unresolved_boundary, unresolved_repair, repetition_count, delivery_outcome, summary_digest
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(relation_hmac) DO UPDATE SET
                summary_revision = excluded.summary_revision,
                source_continuum_revision = excluded.source_continuum_revision,
                dimensions_ema_fxp6 = excluded.dimensions_ema_fxp6,
                unresolved_boundary = excluded.unresolved_boundary,
                unresolved_repair = excluded.unresolved_repair,
                repetition_count = excluded.repetition_count,
                delivery_outcome = excluded.delivery_outcome,
                summary_digest = excluded.summary_digest",
            params![
                &relation_hmac[..],
                i64::from(summary.summary_revision),
                summary.source_continuum_revision.to_le_bytes().to_vec(),
                encode_dimensions(&summary.dimensions_ema_fxp6),
                i64::from(summary.unresolved_boundary),
                i64::from(summary.unresolved_repair),
                i64::try_from(summary.repetition_count).map_err(|_| StoreError::CorruptPayload)?,
                summary.delivery_outcome as i64,
                &summary.summary_digest[..],
            ],
        )?;
        transaction.commit()?;
        Ok(summary)
    }

    pub fn summary_for_relation(
        &self,
        relation_scope: [u8; 16],
    ) -> Result<Option<ContextSummaryV1>, StoreError> {
        summary_for_hmac(&self.connection, &relation_hmac(relation_scope))
    }

    pub fn active_relation_count(&self) -> Result<usize, StoreError> {
        usize::try_from(relation_count(&self.connection)?).map_err(|_| StoreError::CorruptPayload)
    }

    pub fn turn_count_for_relation(&self, relation_scope: [u8; 16]) -> Result<usize, StoreError> {
        let relation_hmac = relation_hmac(relation_scope);
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM relation_turns WHERE relation_hmac = ?1",
            params![&relation_hmac[..]],
            |row| row.get(0),
        )?;
        usize::try_from(count).map_err(|_| StoreError::CorruptPayload)
    }

    fn event_exists(
        &self,
        relation_hmac: &[u8; 32],
        event_id: &[u8; 16],
    ) -> Result<bool, StoreError> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(
                SELECT 1 FROM relation_turns WHERE relation_hmac = ?1 AND event_id = ?2
             )",
            params![&relation_hmac[..], &event_id[..]],
            |row| row.get(0),
        )?)
    }
}

fn relation_hmac(relation_scope: [u8; 16]) -> [u8; 32] {
    let mut mac = Hmac::<Sha256>::new_from_slice(&RELATION_HMAC_KEY)
        .expect("the fixed HMAC key has a valid length");
    mac.update(&relation_scope);
    let mut output = [0; 32];
    output.copy_from_slice(&mac.finalize().into_bytes());
    output
}

fn relation_count(connection: &Connection) -> Result<i64, StoreError> {
    Ok(
        connection.query_row("SELECT COUNT(*) FROM relation_summaries", [], |row| {
            row.get(0)
        })?,
    )
}

fn summary_for_hmac(
    connection: &Connection,
    relation_hmac: &[u8; 32],
) -> Result<Option<ContextSummaryV1>, StoreError> {
    let mut statement = connection.prepare(
        "SELECT summary_revision, source_continuum_revision, dimensions_ema_fxp6,
                unresolved_boundary, unresolved_repair, repetition_count, delivery_outcome,
                summary_digest
         FROM relation_summaries WHERE relation_hmac = ?1",
    )?;
    let mut rows = statement.query(params![&relation_hmac[..]])?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };

    let summary_revision: i64 = row.get(0)?;
    let source_continuum_revision: Vec<u8> = row.get(1)?;
    let dimensions_ema_fxp6: Vec<u8> = row.get(2)?;
    let unresolved_boundary: i64 = row.get(3)?;
    let unresolved_repair: i64 = row.get(4)?;
    let repetition_count: i64 = row.get(5)?;
    let delivery_outcome: i64 = row.get(6)?;
    let summary_digest: Vec<u8> = row.get(7)?;

    if summary_revision <= 0 || repetition_count < 0 {
        return Err(StoreError::CorruptPayload);
    }
    let summary = ContextSummaryV1 {
        summary_revision: u32::try_from(summary_revision)
            .map_err(|_| StoreError::CorruptPayload)?,
        source_continuum_revision: decode_u64(&source_continuum_revision)?,
        dimensions_ema_fxp6: decode_dimensions(&dimensions_ema_fxp6)?,
        unresolved_boundary: unresolved_boundary != 0,
        unresolved_repair: unresolved_repair != 0,
        repetition_count: u64::try_from(repetition_count)
            .map_err(|_| StoreError::CorruptPayload)?,
        delivery_outcome: DeliveryOutcome::from_code(delivery_outcome)?,
        summary_digest: decode_32(&summary_digest)?,
    };
    if summary.summary_digest != ContextSummaryV1::digest_of(&summary.canonical_bytes()) {
        return Err(StoreError::CorruptPayload);
    }
    Ok(Some(summary))
}

fn summary_from_turns(
    connection: &Connection,
    relation_hmac: &[u8; 32],
    summary_revision: u32,
) -> Result<ContextSummaryV1, StoreError> {
    let mut statement = connection.prepare(
        "SELECT source_continuum_revision, dimensions_fxp6, unresolved_boundary,
                unresolved_repair, repetition_increment, delivery_outcome
         FROM relation_turns WHERE relation_hmac = ?1 ORDER BY sequence ASC",
    )?;
    let mut rows = statement.query(params![&relation_hmac[..]])?;
    let mut dimensions_ema_fxp6 = [0; DIMENSION_COUNT];
    let mut source_continuum_revision = 0;
    let mut unresolved_boundary = false;
    let mut unresolved_repair = false;
    let mut repetition_count = 0_u64;
    let mut delivery_outcome = DeliveryOutcome::Pending;
    let mut turn_index = 0_u32;

    while let Some(row) = rows.next()? {
        let source_revision: Vec<u8> = row.get(0)?;
        let dimensions: Vec<u8> = row.get(1)?;
        let row_unresolved_boundary: i64 = row.get(2)?;
        let row_unresolved_repair: i64 = row.get(3)?;
        let row_repetition_increment: i64 = row.get(4)?;
        let row_delivery_outcome: i64 = row.get(5)?;
        if row_repetition_increment < 0 {
            return Err(StoreError::CorruptPayload);
        }

        let dimensions = decode_dimensions(&dimensions)?;
        if turn_index == 0 {
            dimensions_ema_fxp6 = dimensions;
        } else {
            for (ema, sample) in dimensions_ema_fxp6.iter_mut().zip(dimensions) {
                let next = ((*ema as i128) * 7 + (sample as i128)) / 8;
                *ema = next.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
            }
        }
        source_continuum_revision = decode_u64(&source_revision)?;
        unresolved_boundary |= row_unresolved_boundary != 0;
        unresolved_repair |= row_unresolved_repair != 0;
        repetition_count = repetition_count.saturating_add(row_repetition_increment as u64);
        delivery_outcome = DeliveryOutcome::from_code(row_delivery_outcome)?;
        turn_index += 1;
    }

    if turn_index == 0 {
        return Err(StoreError::CorruptPayload);
    }
    Ok(ContextSummaryV1::new(
        summary_revision,
        source_continuum_revision,
        dimensions_ema_fxp6,
        unresolved_boundary,
        unresolved_repair,
        repetition_count,
        delivery_outcome,
    ))
}

fn encode_dimensions(dimensions: &[i64; DIMENSION_COUNT]) -> Vec<u8> {
    let mut encoded = Vec::with_capacity(DIMENSION_COUNT * 8);
    for dimension in dimensions {
        encoded.extend_from_slice(&dimension.to_le_bytes());
    }
    encoded
}

fn decode_dimensions(encoded: &[u8]) -> Result<[i64; DIMENSION_COUNT], StoreError> {
    if encoded.len() != DIMENSION_COUNT * 8 {
        return Err(StoreError::CorruptPayload);
    }
    let mut dimensions = [0; DIMENSION_COUNT];
    for (dimension, bytes) in dimensions.iter_mut().zip(encoded.chunks_exact(8)) {
        let bytes: [u8; 8] = bytes.try_into().map_err(|_| StoreError::CorruptPayload)?;
        *dimension = i64::from_le_bytes(bytes);
    }
    Ok(dimensions)
}

fn decode_u64(encoded: &[u8]) -> Result<u64, StoreError> {
    let bytes: [u8; 8] = encoded.try_into().map_err(|_| StoreError::CorruptPayload)?;
    Ok(u64::from_le_bytes(bytes))
}

fn decode_32(encoded: &[u8]) -> Result<[u8; 32], StoreError> {
    encoded.try_into().map_err(|_| StoreError::CorruptPayload)
}
