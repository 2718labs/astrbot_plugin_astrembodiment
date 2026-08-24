use crate::continuity_migration::ContinuityAuthority;
use crate::{ClaimOutcome, Store};
use ae_contracts::{wire, Digest};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;

const OWNER_METADATA_MAX_BYTES: u64 = 4096;
const CURRENT_METADATA_MAX_BYTES: u64 = 4096;
const GENERATIONS_DIRECTORY: &str = "generations";
const AUTHORITY_DATABASE: &str = "authority.sqlite";
const LOCATOR_DATABASE: &str = "continuity_locator.sqlite";
const REBIRTH_LEDGER_DATABASE: &str = "rebirth_lifecycle.sqlite";
const LOCATOR_SLOT: i64 = 1;
const CURRENT_SLOT: i64 = 1;
static NEXT_CONTROL_WRITE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VaultMode {
    Ready,
    Migrating,
    RecoveryRequired,
    ReadOnlyRecovery,
    WriteRefusedIncompatible,
    Unborn,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VaultLocation {
    pub root: PathBuf,
    pub generation_id: String,
    pub store_uuid: [u8; 16],
    pub mode: VaultMode,
    pub incarnation_id: Digest,
    pub revision: u64,
    pub genesis_authorized: bool,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VaultLocateError {
    #[error("vault root must be an absolute path")]
    RootNotAbsolute,
    #[error("vault root must name a directory")]
    RootNotDirectory,
    #[error("vault root cannot be canonicalized: {0}")]
    Canonicalization(String),
    #[error("vault root is inside a plugin package")]
    PluginPackagePath,
    #[error("vault owner metadata is invalid: {0}")]
    InvalidOwner(String),
    #[error("vault current metadata is invalid: {0}")]
    InvalidCurrent(String),
    #[error("vault current metadata exists without owner metadata")]
    CurrentWithoutOwner,
    #[error("vault directory could not be inspected: {0}")]
    Inspection(String),
}

#[derive(Debug)]
struct OwnerMetadata {
    generation_id: String,
    store_uuid: [u8; 16],
}

#[derive(Debug)]
struct CurrentMetadata {
    generation_id: String,
    incarnation_id: Digest,
    revision: u64,
    mode: VaultMode,
}

#[derive(Clone, Copy)]
enum ControlMetadataKind {
    Owner,
    Current,
}

impl ControlMetadataKind {
    fn max_bytes(self) -> u64 {
        match self {
            ControlMetadataKind::Owner => OWNER_METADATA_MAX_BYTES,
            ControlMetadataKind::Current => CURRENT_METADATA_MAX_BYTES,
        }
    }

    fn too_large_error(self) -> VaultLocateError {
        match self {
            ControlMetadataKind::Owner => {
                VaultLocateError::InvalidOwner("owner.cbor exceeds 4096 bytes".into())
            }
            ControlMetadataKind::Current => {
                VaultLocateError::InvalidCurrent("current exceeds 4096 bytes".into())
            }
        }
    }
}

/// Locate only the durable, package-external continuity vault.  This routine
/// neither creates nor migrates anything: incomplete or invalid continuity
/// metadata is reported as recovery/error state and never authorizes Genesis.
pub fn locate_vault(path: impl AsRef<Path>) -> Result<VaultLocation, VaultLocateError> {
    let root = canonical_vault_path(path.as_ref())?;
    reject_plugin_package_path(&root)?;

    match fs::metadata(&root) {
        Ok(metadata) if !metadata.is_dir() => return Err(VaultLocateError::RootNotDirectory),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(unborn(root));
        }
        Err(error) => return Err(VaultLocateError::Inspection(error.to_string())),
    }

    let owner_path = root.join("owner.cbor");
    let owner_bytes = match read_control_metadata(&owner_path, ControlMetadataKind::Owner)? {
        Some(bytes) => bytes,
        None => {
            match fs::metadata(root.join("current")) {
                Ok(_) => return Err(VaultLocateError::CurrentWithoutOwner),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(VaultLocateError::Inspection(error.to_string())),
            }
            if directory_has_entries(&root)? {
                return Ok(recovery_required(root));
            }
            return Ok(unborn(root));
        }
    };
    let owner = parse_owner(&owner_bytes)?;

    let current_path = root.join("current");
    let current_bytes = match read_control_metadata(&current_path, ControlMetadataKind::Current)? {
        Some(bytes) => bytes,
        None => {
            return Ok(VaultLocation {
                root,
                generation_id: owner.generation_id,
                store_uuid: owner.store_uuid,
                mode: VaultMode::RecoveryRequired,
                incarnation_id: [0; 32],
                revision: 0,
                genesis_authorized: false,
            });
        }
    };
    let current = parse_current(&current_bytes)?;
    if current.generation_id != owner.generation_id {
        return Err(VaultLocateError::InvalidCurrent(
            "generation_id does not match owner metadata".into(),
        ));
    }
    if current.incarnation_id.iter().all(|byte| *byte == 0) {
        return Err(VaultLocateError::InvalidCurrent(
            "incarnation_id must not be all zero".into(),
        ));
    }

    let (generation_id, incarnation_id, revision) =
        read_rebirth_current_overlay(&root)?.unwrap_or((
            owner.generation_id,
            current.incarnation_id,
            current.revision,
        ));
    Ok(VaultLocation {
        root,
        generation_id,
        store_uuid: owner.store_uuid,
        mode: current.mode,
        incarnation_id,
        revision,
        genesis_authorized: false,
    })
}

fn unborn(root: PathBuf) -> VaultLocation {
    VaultLocation {
        root,
        generation_id: String::new(),
        store_uuid: [0; 16],
        mode: VaultMode::Unborn,
        incarnation_id: [0; 32],
        revision: 0,
        genesis_authorized: false,
    }
}

fn recovery_required(root: PathBuf) -> VaultLocation {
    VaultLocation {
        root,
        generation_id: String::new(),
        store_uuid: [0; 16],
        mode: VaultMode::RecoveryRequired,
        incarnation_id: [0; 32],
        revision: 0,
        genesis_authorized: false,
    }
}

fn canonical_vault_path(path: &Path) -> Result<PathBuf, VaultLocateError> {
    if !path.is_absolute() {
        return Err(VaultLocateError::RootNotAbsolute);
    }
    match fs::canonicalize(path) {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path.file_name().ok_or_else(|| {
                VaultLocateError::Canonicalization("missing vault directory name".into())
            })?;
            let parent = path.parent().ok_or_else(|| {
                VaultLocateError::Canonicalization("missing vault parent directory".into())
            })?;
            let canonical_parent = fs::canonicalize(parent)
                .map_err(|error| VaultLocateError::Canonicalization(error.to_string()))?;
            Ok(canonical_parent.join(name))
        }
        Err(error) => Err(VaultLocateError::Canonicalization(error.to_string())),
    }
}

fn reject_plugin_package_path(root: &Path) -> Result<(), VaultLocateError> {
    for ancestor in root.ancestors() {
        let metadata = match fs::metadata(ancestor) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(VaultLocateError::Inspection(error.to_string())),
        };
        if metadata.is_dir()
            && (ancestor.join("plugin.toml").is_file()
                || ancestor.join("plugin.json").is_file()
                || ancestor.join(".codex-plugin").join("plugin.json").is_file()
                || (ancestor.join("metadata.yaml").is_file() && ancestor.join("main.py").is_file()))
        {
            return Err(VaultLocateError::PluginPackagePath);
        }
    }
    Ok(())
}

fn directory_has_entries(path: &Path) -> Result<bool, VaultLocateError> {
    let mut entries =
        fs::read_dir(path).map_err(|error| VaultLocateError::Inspection(error.to_string()))?;
    match entries.next() {
        Some(Ok(_)) => Ok(true),
        Some(Err(error)) => Err(VaultLocateError::Inspection(error.to_string())),
        None => Ok(false),
    }
}

fn read_control_metadata(
    path: &Path,
    kind: ControlMetadataKind,
) -> Result<Option<Vec<u8>>, VaultLocateError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(VaultLocateError::Inspection(error.to_string())),
    };
    if metadata.len() > kind.max_bytes() {
        return Err(kind.too_large_error());
    }

    let file =
        fs::File::open(path).map_err(|error| VaultLocateError::Inspection(error.to_string()))?;
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take(kind.max_bytes() + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| VaultLocateError::Inspection(error.to_string()))?;
    if bytes.len() > kind.max_bytes() as usize {
        return Err(kind.too_large_error());
    }
    Ok(Some(bytes))
}

fn parse_owner(bytes: &[u8]) -> Result<OwnerMetadata, VaultLocateError> {
    let mut reader = CborReader::new(bytes);
    if reader.map_len()? != 2 {
        return Err(VaultLocateError::InvalidOwner(
            "owner.cbor must be a two-field map".into(),
        ));
    }

    let mut generation_id = None;
    let mut store_uuid = None;
    for _ in 0..2 {
        match reader.text()? {
            "generation_id" if generation_id.is_none() => {
                generation_id = Some(validate_owner_generation_id(reader.text()?)?);
            }
            "store_uuid" if store_uuid.is_none() => {
                let bytes = reader.bytes()?;
                let uuid: [u8; 16] = bytes.try_into().map_err(|_| {
                    VaultLocateError::InvalidOwner("store_uuid must contain 16 bytes".into())
                })?;
                if uuid.iter().all(|byte| *byte == 0) {
                    return Err(VaultLocateError::InvalidOwner(
                        "store_uuid must not be all zero".into(),
                    ));
                }
                store_uuid = Some(uuid);
            }
            key => {
                return Err(VaultLocateError::InvalidOwner(format!(
                    "unexpected or duplicate owner field: {key}"
                )));
            }
        }
    }
    if !reader.is_finished() {
        return Err(VaultLocateError::InvalidOwner(
            "owner.cbor has trailing bytes".into(),
        ));
    }

    Ok(OwnerMetadata {
        generation_id: generation_id.ok_or_else(|| {
            VaultLocateError::InvalidOwner("owner.cbor is missing generation_id".into())
        })?,
        store_uuid: store_uuid.ok_or_else(|| {
            VaultLocateError::InvalidOwner("owner.cbor is missing store_uuid".into())
        })?,
    })
}

fn valid_generation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_owner_generation_id(value: &str) -> Result<String, VaultLocateError> {
    if !valid_generation_id(value) {
        return Err(VaultLocateError::InvalidOwner(
            "generation_id must be a bounded portable identifier".into(),
        ));
    }
    Ok(value.to_owned())
}

fn validate_current_generation_id(value: &str) -> Result<String, VaultLocateError> {
    if !valid_generation_id(value) {
        return Err(VaultLocateError::InvalidCurrent(
            "generation_id must be a bounded portable identifier".into(),
        ));
    }
    Ok(value.to_owned())
}

fn parse_current(bytes: &[u8]) -> Result<CurrentMetadata, VaultLocateError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| VaultLocateError::InvalidCurrent("current is not UTF-8".into()))?;
    let mut generation_id = None;
    let mut incarnation_id = None;
    let mut revision = None;
    let mut mode = None;

    for line in text.lines() {
        let (key, value) = line.split_once('=').ok_or_else(|| {
            VaultLocateError::InvalidCurrent("current contains a malformed line".into())
        })?;
        let duplicate = match key {
            "generation_id" => generation_id
                .replace(validate_current_generation_id(value)?)
                .is_some(),
            "incarnation_id" => incarnation_id.replace(parse_digest(value)?).is_some(),
            "revision" => revision
                .replace(value.parse().map_err(|_| {
                    VaultLocateError::InvalidCurrent("revision is not an unsigned integer".into())
                })?)
                .is_some(),
            "mode" => mode.replace(parse_mode(value)?).is_some(),
            _ => {
                return Err(VaultLocateError::InvalidCurrent(format!(
                    "current contains an unknown field: {key}"
                )));
            }
        };
        if duplicate {
            return Err(VaultLocateError::InvalidCurrent(format!(
                "current contains a duplicate field: {key}"
            )));
        }
    }

    Ok(CurrentMetadata {
        generation_id: generation_id.ok_or_else(|| {
            VaultLocateError::InvalidCurrent("current is missing generation_id".into())
        })?,
        incarnation_id: incarnation_id.ok_or_else(|| {
            VaultLocateError::InvalidCurrent("current is missing incarnation_id".into())
        })?,
        revision: revision.ok_or_else(|| {
            VaultLocateError::InvalidCurrent("current is missing revision".into())
        })?,
        mode: mode
            .ok_or_else(|| VaultLocateError::InvalidCurrent("current is missing mode".into()))?,
    })
}

/// The legacy control file remains the recovery baseline.  Once an explicit
/// rebirth atomically switches the SQLite locator, this sidecar mirrors the
/// selected complete child authority without altering the locator's strict
/// D1 schema.  A malformed sidecar fails closed instead of falling back to
/// old identity after a committed rebirth.
fn read_rebirth_current_overlay(
    root: &Path,
) -> Result<Option<(String, Digest, u64)>, VaultLocateError> {
    let path = root.join(REBIRTH_LEDGER_DATABASE);
    if !path.is_file() {
        return Ok(None);
    }
    let connection =
        Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|_| {
            VaultLocateError::InvalidCurrent("rebirth lifecycle ledger is unreadable".into())
        })?;
    let row: Option<(String, Vec<u8>, i64)> = connection
        .query_row(
            "SELECT generation_id, incarnation_id, revision FROM rebirth_current_v1 WHERE slot = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|_| {
            VaultLocateError::InvalidCurrent("rebirth lifecycle ledger is invalid".into())
        })?;
    let Some((generation_id, incarnation, revision)) = row else {
        return Ok(None);
    };
    let generation_id = validate_current_generation_id(&generation_id)?;
    let ledger_incarnation_id: Digest = incarnation.try_into().map_err(|_| {
        VaultLocateError::InvalidCurrent(
            "rebirth lifecycle incarnation must contain 32 bytes".into(),
        )
    })?;
    if ledger_incarnation_id.iter().all(|byte| *byte == 0) {
        return Err(VaultLocateError::InvalidCurrent(
            "rebirth lifecycle incarnation must not be all zero".into(),
        ));
    }
    let _ledger_revision = u64::try_from(revision).map_err(|_| {
        VaultLocateError::InvalidCurrent("rebirth lifecycle revision is invalid".into())
    })?;
    let authority_path = root
        .join(GENERATIONS_DIRECTORY)
        .join(&generation_id)
        .join(AUTHORITY_DATABASE);
    let authority = Connection::open_with_flags(&authority_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| {
            VaultLocateError::InvalidCurrent("rebirth child authority is unreadable".into())
        })?;
    let mut statement = authority
        .prepare(
            "SELECT incarnation_id, revision FROM active_bindings
             ORDER BY bot_token ASC, persona_token ASC",
        )
        .map_err(|_| {
            VaultLocateError::InvalidCurrent("rebirth child authority is invalid".into())
        })?;
    let mut rows = statement.query([]).map_err(|_| {
        VaultLocateError::InvalidCurrent("rebirth child authority is invalid".into())
    })?;
    let first = rows
        .next()
        .map_err(|_| VaultLocateError::InvalidCurrent("rebirth child authority is invalid".into()))?
        .ok_or_else(|| {
            VaultLocateError::InvalidCurrent("rebirth child authority is missing".into())
        })?;
    let incarnation: Vec<u8> = first.get(0).map_err(|_| {
        VaultLocateError::InvalidCurrent("rebirth child authority is invalid".into())
    })?;
    let incarnation_id: Digest = incarnation.try_into().map_err(|_| {
        VaultLocateError::InvalidCurrent("rebirth child incarnation must contain 32 bytes".into())
    })?;
    let revision: i64 = first.get(1).map_err(|_| {
        VaultLocateError::InvalidCurrent("rebirth child authority is invalid".into())
    })?;
    if rows
        .next()
        .map_err(|_| VaultLocateError::InvalidCurrent("rebirth child authority is invalid".into()))?
        .is_some()
    {
        return Err(VaultLocateError::InvalidCurrent(
            "rebirth child authority is ambiguous".into(),
        ));
    }
    if incarnation_id.iter().all(|byte| *byte == 0) {
        return Err(VaultLocateError::InvalidCurrent(
            "rebirth child incarnation must not be all zero".into(),
        ));
    }
    let revision = u64::try_from(revision).map_err(|_| {
        VaultLocateError::InvalidCurrent("rebirth child revision is invalid".into())
    })?;
    Ok(Some((generation_id, incarnation_id, revision)))
}

fn parse_digest(value: &str) -> Result<Digest, VaultLocateError> {
    if value.len() != 64 {
        return Err(VaultLocateError::InvalidCurrent(
            "incarnation_id must be 32-byte hexadecimal".into(),
        ));
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).map_err(|_| {
            VaultLocateError::InvalidCurrent("incarnation_id must be hexadecimal".into())
        })?;
    }
    Ok(digest)
}

fn parse_mode(value: &str) -> Result<VaultMode, VaultLocateError> {
    match value {
        "ready" => Ok(VaultMode::Ready),
        "migrating" => Ok(VaultMode::Migrating),
        "recovery_required" => Ok(VaultMode::RecoveryRequired),
        "read_only_recovery" => Ok(VaultMode::ReadOnlyRecovery),
        "write_refused_incompatible" => Ok(VaultMode::WriteRefusedIncompatible),
        "unborn" => Err(VaultLocateError::InvalidCurrent(
            "owner-bearing vault cannot authorize unborn mode".into(),
        )),
        _ => Err(VaultLocateError::InvalidCurrent(
            "current contains an unsupported mode".into(),
        )),
    }
}

struct CborReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> CborReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }

    fn map_len(&mut self) -> Result<usize, VaultLocateError> {
        self.length(5)
    }

    fn text(&mut self) -> Result<&'a str, VaultLocateError> {
        let length = self.length(3)?;
        let bytes = self.take(length)?;
        std::str::from_utf8(bytes)
            .map_err(|_| VaultLocateError::InvalidOwner("owner.cbor contains invalid UTF-8".into()))
    }

    fn bytes(&mut self) -> Result<&'a [u8], VaultLocateError> {
        let length = self.length(2)?;
        self.take(length)
    }

    fn length(&mut self, expected_major: u8) -> Result<usize, VaultLocateError> {
        let head = *self
            .bytes
            .get(self.offset)
            .ok_or_else(|| VaultLocateError::InvalidOwner("owner.cbor is truncated".into()))?;
        self.offset += 1;
        if head >> 5 != expected_major {
            return Err(VaultLocateError::InvalidOwner(
                "owner.cbor has an unexpected value type".into(),
            ));
        }
        let additional = head & 0x1f;
        let length = match additional {
            value @ 0..=23 => value as u64,
            24 => self.take(1)?[0] as u64,
            25 => u16::from_be_bytes(self.take(2)?.try_into().unwrap()) as u64,
            26 => u32::from_be_bytes(self.take(4)?.try_into().unwrap()) as u64,
            27 => u64::from_be_bytes(self.take(8)?.try_into().unwrap()),
            _ => {
                return Err(VaultLocateError::InvalidOwner(
                    "owner.cbor must use definite-length values".into(),
                ));
            }
        };
        usize::try_from(length)
            .map_err(|_| VaultLocateError::InvalidOwner("owner.cbor length is too large".into()))
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], VaultLocateError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| VaultLocateError::InvalidOwner("owner.cbor length overflows".into()))?;
        let bytes = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| VaultLocateError::InvalidOwner("owner.cbor is truncated".into()))?;
        self.offset = end;
        Ok(bytes)
    }
}

/// The only destructive continuity actions that may enter the durable rebirth
/// state machine.  Normal startup, migration and recovery never construct one
/// of these values on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebirthActionV1 {
    Rebirth,
    ClearActiveState,
}

impl RebirthActionV1 {
    fn as_str(self) -> &'static str {
        match self {
            RebirthActionV1::Rebirth => "REBIRTH",
            RebirthActionV1::ClearActiveState => "CLEAR_ACTIVE_STATE",
        }
    }

    fn tag(self) -> [u8; 1] {
        match self {
            RebirthActionV1::Rebirth => [1],
            RebirthActionV1::ClearActiveState => [2],
        }
    }

    fn parse(value: &str) -> Result<Self, RebirthLifecycleError> {
        match value {
            "REBIRTH" => Ok(RebirthActionV1::Rebirth),
            "CLEAR_ACTIVE_STATE" => Ok(RebirthActionV1::ClearActiveState),
            _ => Err(RebirthLifecycleError::Durability),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebirthResponseStateV1 {
    ConfirmationPending,
    Committed,
    Replayed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebirthOutcomeV1 {
    Committed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RebirthFaultV1 {
    BeforeLocatorCommit,
    AfterLocatorCommit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthPrepareRequestV1 {
    pub scope_token: Digest,
    pub expected_incarnation_id: Digest,
    pub expected_revision: u64,
    pub action: RebirthActionV1,
}

/// The raw nonce is intentionally exposed only by this first response.  Do
/// not add `Debug`: accidental tracing of this type would violate the durable
/// privacy boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct RebirthPrepareResponseV1 {
    pub state: RebirthResponseStateV1,
    pub request_nonce: [u8; 32],
    pub request_nonce_digest: Digest,
    pub binding_digest: Digest,
}

/// The second, independently authorized user action.  As with the prepare
/// response, a raw nonce-bearing request deliberately has no `Debug` impl.
#[derive(Clone, PartialEq, Eq)]
pub struct UserAuthorizedRebirthV1 {
    pub scope_token: Digest,
    pub expected_incarnation_id: Digest,
    pub expected_revision: u64,
    pub request_nonce: [u8; 32],
    pub action: RebirthActionV1,
    pub confirmed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthChallengeV1 {
    pub request_nonce_digest: Digest,
    pub binding_digest: Digest,
    pub scope_token: Digest,
    pub expected_incarnation_id: Digest,
    pub expected_revision: u64,
    pub action: RebirthActionV1,
}

impl RebirthChallengeV1 {
    /// Challenges persist only a one-way nonce digest.
    pub fn contains_raw_nonce(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthAuditReceiptV1 {
    pub receipt_id: Digest,
    pub action: RebirthActionV1,
    pub scope_token_short: String,
    pub request_nonce_digest: Digest,
    pub parent_incarnation_short: String,
    pub child_incarnation_short: String,
    pub before_revision: u64,
    pub after_revision: u64,
    pub outcome: RebirthOutcomeV1,
    pub audit_time_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthResponseEnvelopeV1 {
    pub state: RebirthResponseStateV1,
    pub receipt: Option<RebirthAuditReceiptV1>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthCurrentV1 {
    pub generation_id: String,
    pub authority: ContinuityAuthority,
}

/// A permit is obtained only after the second user authorization has passed
/// all durable challenge checks.  `stage_epoch` is intentionally private so a
/// caller cannot mint or revive a stale staging lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthCommitPermitV1 {
    pub scope_token: Digest,
    pub expected_incarnation_id: Digest,
    pub expected_revision: u64,
    pub action: RebirthActionV1,
    pub request_nonce_digest: Digest,
    pub binding_digest: Digest,
    pub parent_generation_id: String,
    pub parent_authority: ContinuityAuthority,
    stage_epoch: u64,
}

/// Runtime supplies a normal, validated Genesis transaction; Store owns the
/// candidate generation location, durable close and parent lineage update.
#[derive(Clone, Debug)]
pub struct RebirthChildStageRequestV1 {
    pub genesis: crate::GenesisCommit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RebirthStagedChildV1 {
    pub scope_token: Digest,
    pub action: RebirthActionV1,
    pub parent_generation_id: String,
    pub parent_authority: ContinuityAuthority,
    pub child_generation_id: String,
    pub child_authority: ContinuityAuthority,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebirthPreflightV1 {
    Stage(RebirthCommitPermitV1),
    Replayed(RebirthResponseEnvelopeV1),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RebirthLifecycleError {
    #[error("REBIRTH_CONFIRMATION_REQUIRED")]
    ConfirmationRequired,
    #[error("REBIRTH_FENCE_STALE")]
    FenceStale,
    #[error("REBIRTH_NONCE_CONFLICT")]
    NonceConflict,
    #[error("REBIRTH_IN_FLIGHT")]
    InFlight,
    #[error("REBIRTH_CHILD_INVALID")]
    ChildInvalid,
    #[error("REBIRTH_LOCATOR_INVALID")]
    LocatorInvalid,
    #[error("REBIRTH_DURABILITY_FAILURE")]
    Durability,
    #[error("REBIRTH_INJECTED_FAULT")]
    InjectedFault(RebirthFaultV1),
    #[error("REBIRTH_BOOTSTRAP_CONFLICT")]
    BootstrapConflict,
}

impl RebirthLifecycleError {
    pub fn code(&self) -> &'static str {
        match self {
            RebirthLifecycleError::ConfirmationRequired => "REBIRTH_CONFIRMATION_REQUIRED",
            RebirthLifecycleError::FenceStale => "REBIRTH_FENCE_STALE",
            RebirthLifecycleError::NonceConflict => "REBIRTH_NONCE_CONFLICT",
            RebirthLifecycleError::InFlight => "REBIRTH_IN_FLIGHT",
            RebirthLifecycleError::ChildInvalid => "REBIRTH_CHILD_INVALID",
            RebirthLifecycleError::LocatorInvalid => "REBIRTH_LOCATOR_INVALID",
            RebirthLifecycleError::Durability => "REBIRTH_DURABILITY_FAILURE",
            RebirthLifecycleError::InjectedFault(_) => "REBIRTH_INJECTED_FAULT",
            RebirthLifecycleError::BootstrapConflict => "REBIRTH_BOOTSTRAP_CONFLICT",
        }
    }
}

impl From<VaultLocateError> for RebirthLifecycleError {
    fn from(_: VaultLocateError) -> Self {
        RebirthLifecycleError::LocatorInvalid
    }
}

/// Durable lifecycle owner for a package-external Continuity Vault.  It does
/// not own the live Store connection and therefore cannot accidentally turn a
/// regular open, upgrade or recovery path into Genesis.
#[derive(Clone, Debug)]
pub struct VaultLifecycle {
    root: PathBuf,
}

impl VaultLifecycle {
    pub fn open(vault_root: impl AsRef<Path>) -> Result<Self, RebirthLifecycleError> {
        let location = locate_vault(vault_root)?;
        Ok(Self {
            root: location.root,
        })
    }

    pub fn vault_mode_v1(&self) -> Result<VaultMode, RebirthLifecycleError> {
        Ok(locate_vault(&self.root)?.mode)
    }

    /// One-time import for a pre-vault Store.  It snapshots and verifies an
    /// already committed authority; it never invokes Genesis or fabricates
    /// identity.  A different legacy authority against an initialized vault
    /// is a fail-closed conflict.
    pub fn bootstrap_legacy_store_v1(
        &self,
        legacy_authority_database: impl AsRef<Path>,
    ) -> Result<RebirthCurrentV1, RebirthLifecycleError> {
        let legacy = fs::canonicalize(legacy_authority_database.as_ref())
            .map_err(|_| RebirthLifecycleError::Durability)?;
        if !legacy.is_file() {
            return Err(RebirthLifecycleError::ChildInvalid);
        }
        let authority = capture_any_authority(&legacy)?;
        let generation_id = legacy_generation_id(&authority.incarnation_id);
        let existing = locate_vault(&self.root)?;
        match existing.mode {
            VaultMode::Ready => {
                let current = self.current_authority_v1()?;
                if current.authority == authority {
                    return Ok(current);
                }
                return Err(RebirthLifecycleError::BootstrapConflict);
            }
            VaultMode::Unborn => {}
            _ => return Err(RebirthLifecycleError::BootstrapConflict),
        }

        fs::create_dir_all(self.root.join(GENERATIONS_DIRECTORY))
            .map_err(|_| RebirthLifecycleError::Durability)?;
        let target_directory = self.root.join(GENERATIONS_DIRECTORY).join(&generation_id);
        let target_database = target_directory.join(AUTHORITY_DATABASE);
        if target_directory.exists() {
            if !target_database.is_file() || capture_any_authority(&target_database)? != authority {
                return Err(RebirthLifecycleError::BootstrapConflict);
            }
        } else {
            fs::create_dir(&target_directory).map_err(|_| RebirthLifecycleError::Durability)?;
            sqlite_shadow_backup(&legacy, &target_database)?;
            sync_sqlite_database(&target_database)?;
        }

        let store_uuid = bootstrap_store_uuid(&self.root, &authority.incarnation_id);
        write_control_file(
            &self.root.join("owner.cbor"),
            &owner_cbor(&generation_id, store_uuid),
        )?;
        write_control_file(
            &self.root.join("current"),
            format!(
                "generation_id={generation_id}\nincarnation_id={}\nrevision={}\nmode=ready\n",
                digest_hex(&authority.incarnation_id),
                authority.revision,
            )
            .as_bytes(),
        )?;
        ensure_locator_current(&self.root, &generation_id)?;
        let current = self.current_authority_v1()?;
        if current.generation_id != generation_id || current.authority != authority {
            return Err(RebirthLifecycleError::BootstrapConflict);
        }
        Ok(current)
    }

    pub fn current_authority_v1(&self) -> Result<RebirthCurrentV1, RebirthLifecycleError> {
        let location = locate_vault(&self.root)?;
        if location.mode != VaultMode::Ready {
            return Err(RebirthLifecycleError::LocatorInvalid);
        }
        let generation_id = read_authoritative_generation(&self.root, &location.generation_id)?;
        let authority = capture_any_authority(
            &self
                .root
                .join(GENERATIONS_DIRECTORY)
                .join(&generation_id)
                .join(AUTHORITY_DATABASE),
        )?;
        Ok(RebirthCurrentV1 {
            generation_id,
            authority,
        })
    }

    /// The sole supported way for runtime to reopen the selected Store.  It
    /// derives the path from the authoritative lifecycle current and never
    /// permits runtime to read or mutate locator SQLite directly.
    pub fn current_authority_database_path(&self) -> Result<PathBuf, RebirthLifecycleError> {
        let current = self.current_authority_v1()?;
        self.child_authority_database_path(&current.generation_id)
    }

    pub fn current_fence(
        &self,
        scope_token: Digest,
    ) -> Result<(Digest, u64), RebirthLifecycleError> {
        let (current, current_scope) = self.current_with_scope()?;
        if current_scope != scope_token {
            return Err(RebirthLifecycleError::FenceStale);
        }
        Ok((current.authority.incarnation_id, current.authority.revision))
    }

    pub fn prepare_rebirth(
        &self,
        request: RebirthPrepareRequestV1,
    ) -> Result<RebirthPrepareResponseV1, RebirthLifecycleError> {
        let (current, current_scope) = self.current_with_scope()?;
        if current_scope != request.scope_token
            || current.authority.incarnation_id != request.expected_incarnation_id
            || current.authority.revision != request.expected_revision
        {
            return Err(RebirthLifecycleError::FenceStale);
        }
        let binding_digest = rebirth_binding_digest(&request);
        let mut connection = self.open_ledger()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| RebirthLifecycleError::Durability)?;
        initialize_ledger_schema(&transaction)?;
        ensure_ledger_current(&transaction, &current)?;
        let raw_nonce: Vec<u8> = transaction
            .query_row("SELECT randomblob(32)", [], |row| row.get(0))
            .map_err(|_| RebirthLifecycleError::Durability)?;
        let request_nonce: [u8; 32] = raw_nonce
            .try_into()
            .map_err(|_| RebirthLifecycleError::Durability)?;
        let request_nonce_digest =
            wire::domain_hash(b"astr-embodiment/rebirth-nonce-v1", &[&request_nonce]);
        let now = crate::now_ms();
        transaction
            .execute(
                "INSERT INTO rebirth_challenge_v1 (
                     request_nonce_digest, binding_digest, scope_token,
                     expected_incarnation_id, expected_revision, action,
                     status, stage_epoch, created_at_ms, updated_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?7)",
                params![
                    request_nonce_digest.to_vec(),
                    binding_digest.to_vec(),
                    request.scope_token.to_vec(),
                    request.expected_incarnation_id.to_vec(),
                    revision_to_sql(request.expected_revision)?,
                    request.action.as_str(),
                    revision_to_sql(now)?,
                ],
            )
            .map_err(|_| RebirthLifecycleError::Durability)?;
        transaction
            .commit()
            .map_err(|_| RebirthLifecycleError::Durability)?;
        Ok(RebirthPrepareResponseV1 {
            state: RebirthResponseStateV1::ConfirmationPending,
            request_nonce,
            request_nonce_digest,
            binding_digest,
        })
    }

    pub fn challenge_by_nonce_digest(
        &self,
        request_nonce_digest: Digest,
    ) -> Result<Option<RebirthChallengeV1>, RebirthLifecycleError> {
        let path = self.ledger_path();
        if !path.is_file() {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| RebirthLifecycleError::Durability)?;
        connection
            .query_row(
                "SELECT binding_digest, scope_token, expected_incarnation_id, expected_revision, action
                 FROM rebirth_challenge_v1 WHERE request_nonce_digest = ?1",
                params![request_nonce_digest.to_vec()],
                |row| {
                    let revision: i64 = row.get(3)?;
                    Ok((
                        digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error)?,
                        digest_from_blob(row.get(1)?).map_err(sqlite_conversion_error)?,
                        digest_from_blob(row.get(2)?).map_err(sqlite_conversion_error)?,
                        revision,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| RebirthLifecycleError::Durability)?
            .map(|(binding_digest, scope_token, expected_incarnation_id, revision, action)| {
                Ok(RebirthChallengeV1 {
                    request_nonce_digest,
                    binding_digest,
                    scope_token,
                    expected_incarnation_id,
                    expected_revision: revision_from_sql(revision)?,
                    action: RebirthActionV1::parse(&action)?,
                })
            })
            .transpose()
    }

    pub fn preflight_rebirth_confirmation(
        &self,
        request: &UserAuthorizedRebirthV1,
    ) -> Result<RebirthPreflightV1, RebirthLifecycleError> {
        if !request.confirmed {
            return Err(RebirthLifecycleError::ConfirmationRequired);
        }
        let request_nonce_digest = rebirth_nonce_digest(&request.request_nonce);
        let binding_digest = rebirth_authorized_binding_digest(request);
        let mut connection = self.open_ledger()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| RebirthLifecycleError::Durability)?;
        initialize_ledger_schema(&transaction)?;
        let stored = load_stored_challenge(&transaction, request_nonce_digest)?
            .ok_or(RebirthLifecycleError::NonceConflict)?;
        if !challenge_matches_request(&stored.challenge, request, binding_digest) {
            return Err(RebirthLifecycleError::NonceConflict);
        }
        if let Some(receipt) = load_receipt(&transaction, request_nonce_digest)? {
            transaction
                .commit()
                .map_err(|_| RebirthLifecycleError::Durability)?;
            return Ok(RebirthPreflightV1::Replayed(RebirthResponseEnvelopeV1 {
                state: RebirthResponseStateV1::Replayed,
                receipt: Some(receipt),
            }));
        }
        let (current, current_scope) = self.current_with_scope()?;
        if current_scope != request.scope_token
            || current.authority.incarnation_id != request.expected_incarnation_id
            || current.authority.revision != request.expected_revision
        {
            return Err(RebirthLifecycleError::FenceStale);
        }
        ensure_ledger_current(&transaction, &current)?;
        if stored.status == "staging" {
            return Err(RebirthLifecycleError::InFlight);
        }
        if stored.status != "pending" {
            return Err(RebirthLifecycleError::Durability);
        }
        let next_epoch = stored
            .stage_epoch
            .checked_add(1)
            .ok_or(RebirthLifecycleError::Durability)?;
        let updated = transaction
            .execute(
                "UPDATE rebirth_challenge_v1
                 SET status = 'staging', stage_epoch = ?2, updated_at_ms = ?3
                 WHERE request_nonce_digest = ?1 AND status = 'pending' AND stage_epoch = ?4",
                params![
                    request_nonce_digest.to_vec(),
                    revision_to_sql(next_epoch)?,
                    revision_to_sql(crate::now_ms())?,
                    revision_to_sql(stored.stage_epoch)?,
                ],
            )
            .map_err(|_| RebirthLifecycleError::Durability)?;
        if updated != 1 {
            return Err(RebirthLifecycleError::InFlight);
        }
        transaction
            .commit()
            .map_err(|_| RebirthLifecycleError::Durability)?;
        Ok(RebirthPreflightV1::Stage(RebirthCommitPermitV1 {
            scope_token: request.scope_token,
            expected_incarnation_id: request.expected_incarnation_id,
            expected_revision: request.expected_revision,
            action: request.action,
            request_nonce_digest,
            binding_digest,
            parent_generation_id: current.generation_id,
            parent_authority: current.authority,
            stage_epoch: next_epoch,
        }))
    }

    /// Read a completed receipt without changing a pending challenge.  Runtime
    /// normally uses `preflight_rebirth_confirmation`; this helper is useful
    /// for restart recovery paths that must not allocate a second stage lease.
    pub fn replay_rebirth(
        &self,
        request: &UserAuthorizedRebirthV1,
    ) -> Result<Option<RebirthResponseEnvelopeV1>, RebirthLifecycleError> {
        if !request.confirmed {
            return Err(RebirthLifecycleError::ConfirmationRequired);
        }
        let path = self.ledger_path();
        if !path.is_file() {
            return Ok(None);
        }
        let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|_| RebirthLifecycleError::Durability)?;
        let request_nonce_digest = rebirth_nonce_digest(&request.request_nonce);
        let stored = load_stored_challenge(&connection, request_nonce_digest)?;
        let Some(stored) = stored else {
            return Ok(None);
        };
        if !challenge_matches_request(
            &stored.challenge,
            request,
            rebirth_authorized_binding_digest(request),
        ) {
            return Err(RebirthLifecycleError::NonceConflict);
        }
        Ok(
            load_receipt(&connection, request_nonce_digest)?.map(|receipt| {
                RebirthResponseEnvelopeV1 {
                    state: RebirthResponseStateV1::Replayed,
                    receipt: Some(receipt),
                }
            }),
        )
    }

    pub fn commit_rebirth(
        &self,
        permit: &RebirthCommitPermitV1,
        child: &RebirthStagedChildV1,
    ) -> Result<RebirthResponseEnvelopeV1, RebirthLifecycleError> {
        self.commit_rebirth_with_fault(permit, child, None)
    }

    pub fn commit_rebirth_with_fault(
        &self,
        permit: &RebirthCommitPermitV1,
        child: &RebirthStagedChildV1,
        fault: Option<RebirthFaultV1>,
    ) -> Result<RebirthResponseEnvelopeV1, RebirthLifecycleError> {
        let (current, current_scope) = self.current_with_scope()?;
        if current_scope != permit.scope_token
            || current.generation_id != permit.parent_generation_id
            || current.authority != permit.parent_authority
            || current.authority.incarnation_id != permit.expected_incarnation_id
            || current.authority.revision != permit.expected_revision
        {
            return Err(RebirthLifecycleError::FenceStale);
        }
        validate_staged_child(self, permit, child)?;
        let locator_path = self.root.join(LOCATOR_DATABASE);
        if !locator_path.is_file() {
            return Err(RebirthLifecycleError::LocatorInvalid);
        }

        let mut connection = self.open_ledger()?;
        let locator_literal = sqlite_string_literal(&locator_path)?;
        connection
            .execute_batch(&format!(
                "ATTACH DATABASE {locator_literal} AS rebirth_locator;\
                 PRAGMA rebirth_locator.journal_mode = DELETE;\
                 PRAGMA rebirth_locator.synchronous = FULL;"
            ))
            .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| RebirthLifecycleError::Durability)?;
        initialize_ledger_schema(&transaction)?;
        ensure_ledger_current(&transaction, &current)?;
        let stored = load_stored_challenge(&transaction, permit.request_nonce_digest)?
            .ok_or(RebirthLifecycleError::NonceConflict)?;
        if !challenge_matches_permit(&stored, permit) {
            return Err(RebirthLifecycleError::NonceConflict);
        }
        if let Some(receipt) = load_receipt(&transaction, permit.request_nonce_digest)? {
            transaction
                .commit()
                .map_err(|_| RebirthLifecycleError::Durability)?;
            return Ok(RebirthResponseEnvelopeV1 {
                state: RebirthResponseStateV1::Replayed,
                receipt: Some(receipt),
            });
        }
        if stored.status != "staging" || stored.stage_epoch != permit.stage_epoch {
            return Err(RebirthLifecycleError::FenceStale);
        }
        validate_attached_locator(&transaction)?;
        let locator_generation: String = transaction
            .query_row(
                "SELECT generation_id FROM rebirth_locator.continuity_generation_locator WHERE slot = ?1",
                params![LOCATOR_SLOT],
                |row| row.get(0),
            )
            .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
        if locator_generation != permit.parent_generation_id {
            return Err(RebirthLifecycleError::FenceStale);
        }
        if fault == Some(RebirthFaultV1::BeforeLocatorCommit) {
            return Err(RebirthLifecycleError::InjectedFault(
                RebirthFaultV1::BeforeLocatorCommit,
            ));
        }
        let audit_time_ms = crate::now_ms();
        let receipt = RebirthAuditReceiptV1 {
            receipt_id: rebirth_receipt_id(permit, child),
            action: permit.action,
            scope_token_short: short_digest(&permit.scope_token),
            request_nonce_digest: permit.request_nonce_digest,
            parent_incarnation_short: short_digest(&permit.parent_authority.incarnation_id),
            child_incarnation_short: short_digest(&child.child_authority.incarnation_id),
            before_revision: permit.parent_authority.revision,
            after_revision: child.child_authority.revision,
            outcome: RebirthOutcomeV1::Committed,
            audit_time_ms,
        };
        let changed = transaction
            .execute(
                "UPDATE rebirth_locator.continuity_generation_locator
                 SET generation_id = ?2 WHERE slot = ?1 AND generation_id = ?3",
                params![
                    LOCATOR_SLOT,
                    child.child_generation_id,
                    permit.parent_generation_id,
                ],
            )
            .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
        if changed != 1 {
            return Err(RebirthLifecycleError::FenceStale);
        }
        transaction
            .execute(
                "UPDATE rebirth_current_v1 SET generation_id = ?2, incarnation_id = ?3,
                 revision = ?4, state_digest = ?5, graph_digest = ?6, history_digest = ?7
                 WHERE slot = ?1",
                params![
                    CURRENT_SLOT,
                    child.child_generation_id,
                    child.child_authority.incarnation_id.to_vec(),
                    revision_to_sql(child.child_authority.revision)?,
                    child.child_authority.state_digest.to_vec(),
                    child.child_authority.graph_digest.to_vec(),
                    child.child_authority.history_digest.to_vec(),
                ],
            )
            .map_err(|_| RebirthLifecycleError::Durability)?;
        transaction
            .execute(
                "INSERT INTO rebirth_receipt_v1 (
                     request_nonce_digest, receipt_id, action, scope_token,
                     parent_incarnation_id, child_incarnation_id,
                     before_revision, after_revision, audit_time_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    permit.request_nonce_digest.to_vec(),
                    receipt.receipt_id.to_vec(),
                    permit.action.as_str(),
                    permit.scope_token.to_vec(),
                    permit.parent_authority.incarnation_id.to_vec(),
                    child.child_authority.incarnation_id.to_vec(),
                    revision_to_sql(receipt.before_revision)?,
                    revision_to_sql(receipt.after_revision)?,
                    revision_to_sql(receipt.audit_time_ms)?,
                ],
            )
            .map_err(|_| RebirthLifecycleError::Durability)?;
        let challenge_updated = transaction
            .execute(
                "UPDATE rebirth_challenge_v1 SET status = 'committed', updated_at_ms = ?2
                 WHERE request_nonce_digest = ?1 AND status = 'staging' AND stage_epoch = ?3",
                params![
                    permit.request_nonce_digest.to_vec(),
                    revision_to_sql(audit_time_ms)?,
                    revision_to_sql(permit.stage_epoch)?,
                ],
            )
            .map_err(|_| RebirthLifecycleError::Durability)?;
        if challenge_updated != 1 {
            return Err(RebirthLifecycleError::FenceStale);
        }
        transaction
            .commit()
            .map_err(|_| RebirthLifecycleError::Durability)?;
        if fault == Some(RebirthFaultV1::AfterLocatorCommit) {
            return Err(RebirthLifecycleError::InjectedFault(
                RebirthFaultV1::AfterLocatorCommit,
            ));
        }
        Ok(RebirthResponseEnvelopeV1 {
            state: RebirthResponseStateV1::Committed,
            receipt: Some(receipt),
        })
    }

    /// The deterministic Genesis nonce binds a permitted rebirth to a retry
    /// without leaking the raw user-confirmation nonce.  Runtime combines this
    /// nonce with its validated source/formula/parent through ae-genesis to
    /// derive the actual child identity.
    pub fn child_genesis_nonce_digest_for_permit(permit: &RebirthCommitPermitV1) -> Digest {
        wire::domain_hash(
            b"astr-embodiment/rebirth-child-genesis-nonce-v1",
            &[&permit.binding_digest, &permit.request_nonce_digest],
        )
    }

    /// Stage a full Store child outside the authoritative locator.  The
    /// runtime gives Store a normal validated GenesisCommit, but cannot pick a
    /// path, modify the locator or write parent lineage itself.
    pub fn stage_rebirth_child_v1(
        &self,
        permit: &RebirthCommitPermitV1,
        request: RebirthChildStageRequestV1,
    ) -> Result<RebirthStagedChildV1, RebirthLifecycleError> {
        let expected_nonce = Self::child_genesis_nonce_digest_for_permit(permit);
        let source_scope = wire::persona_scope_digest(
            &request.genesis.source.scope.bot_token,
            &request.genesis.source.scope.persona_token,
            None,
        );
        if request.genesis.nonce_digest != expected_nonce
            || source_scope != permit.scope_token
            || request.genesis.incarnation_id == permit.parent_authority.incarnation_id
            || request.genesis.receipt.incarnation_id != request.genesis.incarnation_id
        {
            return Err(RebirthLifecycleError::ChildInvalid);
        }
        let expected_incarnation = request.genesis.incarnation_id;
        let generation_id = Self::child_generation_id_for(&expected_incarnation);
        let database = self.child_authority_database_path(&generation_id)?;
        let directory = database.parent().ok_or(RebirthLifecycleError::Durability)?;
        let existing = if directory.exists() {
            Some(stage_existing_child(
                &database,
                &request.genesis,
                &permit.parent_authority.incarnation_id,
            )?)
        } else {
            None
        };
        let child_authority = match existing {
            Some(authority) => authority,
            None => {
                let generations = directory
                    .parent()
                    .ok_or(RebirthLifecycleError::Durability)?;
                fs::create_dir_all(generations).map_err(|_| RebirthLifecycleError::Durability)?;
                let stage_sequence = NEXT_CONTROL_WRITE.fetch_add(1, Ordering::Relaxed);
                let temporary = generations.join(format!(
                    ".rebirth-stage-{generation_id}-{}-{stage_sequence}",
                    std::process::id()
                ));
                fs::create_dir(&temporary).map_err(|_| RebirthLifecycleError::Durability)?;
                let temporary_database = temporary.join(AUTHORITY_DATABASE);
                let mut store = Store::open(&temporary_database)
                    .map_err(|_| RebirthLifecycleError::Durability)?;
                let mut genesis = request.genesis;
                match store
                    .claim_lease(&genesis.scope_key, Some(genesis.nonce_digest))
                    .map_err(|_| RebirthLifecycleError::Durability)?
                {
                    ClaimOutcome::Claimed { lease_epoch, nonce }
                        if nonce == genesis.nonce_digest =>
                    {
                        genesis.lease_epoch = lease_epoch;
                    }
                    ClaimOutcome::InFlight => return Err(RebirthLifecycleError::InFlight),
                    ClaimOutcome::Committed => return Err(RebirthLifecycleError::ChildInvalid),
                    ClaimOutcome::Claimed { .. } => {
                        return Err(RebirthLifecycleError::ChildInvalid)
                    }
                }
                store
                    .commit_genesis(&genesis)
                    .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
                store
                    .close()
                    .map_err(|_| RebirthLifecycleError::Durability)?;
                install_child_lineage(
                    &temporary_database,
                    &genesis.source.scope.bot_token,
                    &genesis.source.scope.persona_token,
                    &genesis.incarnation_id,
                    &permit.parent_authority.incarnation_id,
                )?;
                sync_sqlite_database(&temporary_database)?;
                match fs::rename(&temporary, directory) {
                    Ok(()) => stage_existing_child(
                        &database,
                        &genesis,
                        &permit.parent_authority.incarnation_id,
                    )?,
                    Err(_) if directory.exists() => stage_existing_child(
                        &database,
                        &genesis,
                        &permit.parent_authority.incarnation_id,
                    )?,
                    Err(_) => return Err(RebirthLifecycleError::Durability),
                }
            }
        };
        if child_authority.revision != 0 || child_authority.incarnation_id != expected_incarnation {
            return Err(RebirthLifecycleError::ChildInvalid);
        }
        Ok(RebirthStagedChildV1 {
            scope_token: permit.scope_token,
            action: permit.action,
            parent_generation_id: permit.parent_generation_id.clone(),
            parent_authority: permit.parent_authority.clone(),
            child_generation_id: generation_id,
            child_authority,
        })
    }

    pub fn child_generation_id_for(incarnation_id: &Digest) -> String {
        format!("rebirth-{}", short_digest(incarnation_id))
    }

    pub fn child_authority_database_path(
        &self,
        generation_id: &str,
    ) -> Result<PathBuf, RebirthLifecycleError> {
        validate_generation_id(generation_id)?;
        Ok(self
            .root
            .join(GENERATIONS_DIRECTORY)
            .join(generation_id)
            .join(AUTHORITY_DATABASE))
    }

    fn current_with_scope(&self) -> Result<(RebirthCurrentV1, Digest), RebirthLifecycleError> {
        let current = self.current_authority_v1()?;
        let database = self.child_authority_database_path(&current.generation_id)?;
        let (bot, persona, incarnation_id, revision) = read_single_binding(&database)?;
        if incarnation_id != current.authority.incarnation_id
            || revision != current.authority.revision
        {
            return Err(RebirthLifecycleError::LocatorInvalid);
        }
        Ok((current, wire::persona_scope_digest(&bot, &persona, None)))
    }

    fn ledger_path(&self) -> PathBuf {
        self.root.join(REBIRTH_LEDGER_DATABASE)
    }

    fn open_ledger(&self) -> Result<Connection, RebirthLifecycleError> {
        if !self.root.is_dir() {
            return Err(RebirthLifecycleError::LocatorInvalid);
        }
        let connection =
            Connection::open(self.ledger_path()).map_err(|_| RebirthLifecycleError::Durability)?;
        connection
            .pragma_update(None, "journal_mode", "DELETE")
            .map_err(|_| RebirthLifecycleError::Durability)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(|_| RebirthLifecycleError::Durability)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(|_| RebirthLifecycleError::Durability)?;
        Ok(connection)
    }
}

fn initialize_ledger_schema(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), RebirthLifecycleError> {
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS rebirth_current_v1 (
                 slot INTEGER PRIMARY KEY CHECK (slot = 1),
                 generation_id TEXT NOT NULL,
                 incarnation_id BLOB NOT NULL CHECK (length(incarnation_id) = 32),
                 revision INTEGER NOT NULL,
                 state_digest BLOB NOT NULL CHECK (length(state_digest) = 32),
                 graph_digest BLOB NOT NULL CHECK (length(graph_digest) = 32),
                 history_digest BLOB NOT NULL CHECK (length(history_digest) = 32)
             );
             CREATE TABLE IF NOT EXISTS rebirth_challenge_v1 (
                 request_nonce_digest BLOB PRIMARY KEY CHECK (length(request_nonce_digest) = 32),
                 binding_digest BLOB NOT NULL CHECK (length(binding_digest) = 32),
                 scope_token BLOB NOT NULL CHECK (length(scope_token) = 32),
                 expected_incarnation_id BLOB NOT NULL CHECK (length(expected_incarnation_id) = 32),
                 expected_revision INTEGER NOT NULL,
                 action TEXT NOT NULL,
                 status TEXT NOT NULL,
                 stage_epoch INTEGER NOT NULL,
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS rebirth_receipt_v1 (
                 request_nonce_digest BLOB PRIMARY KEY CHECK (length(request_nonce_digest) = 32),
                 receipt_id BLOB NOT NULL CHECK (length(receipt_id) = 32),
                 action TEXT NOT NULL,
                 scope_token BLOB NOT NULL CHECK (length(scope_token) = 32),
                 parent_incarnation_id BLOB NOT NULL CHECK (length(parent_incarnation_id) = 32),
                 child_incarnation_id BLOB NOT NULL CHECK (length(child_incarnation_id) = 32),
                 before_revision INTEGER NOT NULL,
                 after_revision INTEGER NOT NULL,
                 audit_time_ms INTEGER NOT NULL
             );",
        )
        .map_err(|_| RebirthLifecycleError::Durability)
}

fn ensure_ledger_current(
    transaction: &rusqlite::Transaction<'_>,
    current: &RebirthCurrentV1,
) -> Result<(), RebirthLifecycleError> {
    let stored = transaction
        .query_row(
            "SELECT generation_id, incarnation_id, revision, state_digest, graph_digest, history_digest
             FROM rebirth_current_v1 WHERE slot = 1",
            [],
            |row| {
                let revision: i64 = row.get(2)?;
                Ok((
                    row.get::<_, String>(0)?,
                    ContinuityAuthority {
                        incarnation_id: digest_from_blob(row.get(1)?).map_err(sqlite_conversion_error)?,
                        revision: revision_from_sql(revision).map_err(sqlite_conversion_error)?,
                        state_digest: digest_from_blob(row.get(3)?).map_err(sqlite_conversion_error)?,
                        graph_digest: digest_from_blob(row.get(4)?).map_err(sqlite_conversion_error)?,
                        history_digest: digest_from_blob(row.get(5)?).map_err(sqlite_conversion_error)?,
                    },
                ))
            },
        )
        .optional()
        .map_err(|_| RebirthLifecycleError::Durability)?;
    match stored {
        None => {
            transaction
                .execute(
                    "INSERT INTO rebirth_current_v1 (
                         slot, generation_id, incarnation_id, revision, state_digest, graph_digest, history_digest
                     ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        current.generation_id,
                        current.authority.incarnation_id.to_vec(),
                        revision_to_sql(current.authority.revision)?,
                        current.authority.state_digest.to_vec(),
                        current.authority.graph_digest.to_vec(),
                        current.authority.history_digest.to_vec(),
                    ],
                )
                .map_err(|_| RebirthLifecycleError::Durability)?;
            Ok(())
        }
        Some((generation_id, authority))
            if generation_id == current.generation_id && authority == current.authority =>
        {
            Ok(())
        }
        Some(_) => {
            transaction
                .execute(
                    "UPDATE rebirth_current_v1 SET generation_id = ?2, incarnation_id = ?3,
                     revision = ?4, state_digest = ?5, graph_digest = ?6, history_digest = ?7
                     WHERE slot = ?1",
                    params![
                        CURRENT_SLOT,
                        current.generation_id,
                        current.authority.incarnation_id.to_vec(),
                        revision_to_sql(current.authority.revision)?,
                        current.authority.state_digest.to_vec(),
                        current.authority.graph_digest.to_vec(),
                        current.authority.history_digest.to_vec(),
                    ],
                )
                .map_err(|_| RebirthLifecycleError::Durability)?;
            Ok(())
        }
    }
}

#[derive(Clone, Debug)]
struct StoredChallenge {
    challenge: RebirthChallengeV1,
    status: String,
    stage_epoch: u64,
}

fn load_stored_challenge(
    connection: &Connection,
    request_nonce_digest: Digest,
) -> Result<Option<StoredChallenge>, RebirthLifecycleError> {
    let row: Option<(Vec<u8>, Vec<u8>, Vec<u8>, i64, String, String, i64)> = connection
        .query_row(
            "SELECT binding_digest, scope_token, expected_incarnation_id, expected_revision,
                    action, status, stage_epoch
             FROM rebirth_challenge_v1 WHERE request_nonce_digest = ?1",
            params![request_nonce_digest.to_vec()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(|_| RebirthLifecycleError::Durability)?;
    row.map(
        |(
            binding_digest,
            scope_token,
            expected_incarnation_id,
            expected_revision,
            action,
            status,
            stage_epoch,
        )| {
            Ok(StoredChallenge {
                challenge: RebirthChallengeV1 {
                    request_nonce_digest,
                    binding_digest: digest_from_blob(binding_digest)?,
                    scope_token: digest_from_blob(scope_token)?,
                    expected_incarnation_id: digest_from_blob(expected_incarnation_id)?,
                    expected_revision: revision_from_sql(expected_revision)?,
                    action: RebirthActionV1::parse(&action)?,
                },
                status,
                stage_epoch: revision_from_sql(stage_epoch)?,
            })
        },
    )
    .transpose()
}

fn load_receipt(
    connection: &Connection,
    request_nonce_digest: Digest,
) -> Result<Option<RebirthAuditReceiptV1>, RebirthLifecycleError> {
    let row: Option<(Vec<u8>, String, Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64)> = connection
        .query_row(
            "SELECT receipt_id, action, scope_token, parent_incarnation_id,
                    child_incarnation_id, before_revision, after_revision, audit_time_ms
             FROM rebirth_receipt_v1 WHERE request_nonce_digest = ?1",
            params![request_nonce_digest.to_vec()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .optional()
        .map_err(|_| RebirthLifecycleError::Durability)?;
    row.map(
        |(
            receipt_id,
            action,
            scope_token,
            parent_incarnation_id,
            child_incarnation_id,
            before_revision,
            after_revision,
            audit_time_ms,
        )| {
            let scope_token = digest_from_blob(scope_token)?;
            let parent_incarnation_id = digest_from_blob(parent_incarnation_id)?;
            let child_incarnation_id = digest_from_blob(child_incarnation_id)?;
            Ok(RebirthAuditReceiptV1 {
                receipt_id: digest_from_blob(receipt_id)?,
                action: RebirthActionV1::parse(&action)?,
                scope_token_short: short_digest(&scope_token),
                request_nonce_digest,
                parent_incarnation_short: short_digest(&parent_incarnation_id),
                child_incarnation_short: short_digest(&child_incarnation_id),
                before_revision: revision_from_sql(before_revision)?,
                after_revision: revision_from_sql(after_revision)?,
                outcome: RebirthOutcomeV1::Committed,
                audit_time_ms: revision_from_sql(audit_time_ms)?,
            })
        },
    )
    .transpose()
}

fn challenge_matches_request(
    challenge: &RebirthChallengeV1,
    request: &UserAuthorizedRebirthV1,
    binding_digest: Digest,
) -> bool {
    challenge.binding_digest == binding_digest
        && challenge.scope_token == request.scope_token
        && challenge.expected_incarnation_id == request.expected_incarnation_id
        && challenge.expected_revision == request.expected_revision
        && challenge.action == request.action
}

fn challenge_matches_permit(stored: &StoredChallenge, permit: &RebirthCommitPermitV1) -> bool {
    stored.challenge.binding_digest == permit.binding_digest
        && stored.challenge.scope_token == permit.scope_token
        && stored.challenge.expected_incarnation_id == permit.expected_incarnation_id
        && stored.challenge.expected_revision == permit.expected_revision
        && stored.challenge.action == permit.action
        && stored.stage_epoch == permit.stage_epoch
}

fn validate_staged_child(
    lifecycle: &VaultLifecycle,
    permit: &RebirthCommitPermitV1,
    child: &RebirthStagedChildV1,
) -> Result<(), RebirthLifecycleError> {
    if child.scope_token != permit.scope_token
        || child.action != permit.action
        || child.parent_generation_id != permit.parent_generation_id
        || child.parent_authority != permit.parent_authority
        || child.child_authority.revision != 0
        || child.child_authority.incarnation_id == permit.parent_authority.incarnation_id
        || child.child_generation_id
            != VaultLifecycle::child_generation_id_for(&child.child_authority.incarnation_id)
    {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    let database = lifecycle.child_authority_database_path(&child.child_generation_id)?;
    if !database.is_file() {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    sync_sqlite_database(&database)?;
    let (bot, persona, _, _) = read_single_binding(&database)?;
    if wire::persona_scope_digest(&bot, &persona, None) != permit.scope_token {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    if capture_any_authority(&database)? != child.child_authority {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    Ok(())
}

fn install_child_lineage(
    database: &Path,
    bot_token: &[u8; 16],
    persona_token: &[u8; 16],
    child_incarnation_id: &Digest,
    parent_incarnation_id: &Digest,
) -> Result<(), RebirthLifecycleError> {
    let mut connection =
        Connection::open(database).map_err(|_| RebirthLifecycleError::Durability)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|_| RebirthLifecycleError::Durability)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| RebirthLifecycleError::Durability)?;
    let incarnation_updated = transaction
        .execute(
            "UPDATE incarnations SET parent_incarnation_id = ?2
             WHERE incarnation_id = ?1 AND parent_incarnation_id IS NULL",
            params![
                child_incarnation_id.to_vec(),
                parent_incarnation_id.to_vec(),
            ],
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    if incarnation_updated != 1 {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    let binding_updated = transaction
        .execute(
            "UPDATE active_bindings SET revision = 0
             WHERE bot_token = ?1 AND persona_token = ?2 AND incarnation_id = ?3",
            params![
                bot_token.to_vec(),
                persona_token.to_vec(),
                child_incarnation_id.to_vec(),
            ],
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    if binding_updated != 1 {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    transaction
        .commit()
        .map_err(|_| RebirthLifecycleError::Durability)
}

fn stage_existing_child(
    database: &Path,
    genesis: &crate::GenesisCommit,
    parent_incarnation_id: &Digest,
) -> Result<ContinuityAuthority, RebirthLifecycleError> {
    if !database.is_file() {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    let authority = capture_any_authority(database)?;
    if authority.incarnation_id != genesis.incarnation_id || authority.revision != 0 {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let stored_parent: Option<Vec<u8>> = connection
        .query_row(
            "SELECT parent_incarnation_id FROM incarnations WHERE incarnation_id = ?1",
            params![genesis.incarnation_id.to_vec()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    if stored_parent.as_deref() != Some(parent_incarnation_id.as_slice()) {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    let (bot, persona, _, _) = read_single_binding(database)?;
    if bot != genesis.source.scope.bot_token || persona != genesis.source.scope.persona_token {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    Ok(authority)
}

fn validate_attached_locator(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), RebirthLifecycleError> {
    let objects: Vec<(String, String, String, Option<String>)> = {
        let mut statement = transaction
            .prepare(
                "SELECT type, name, tbl_name, sql FROM rebirth_locator.sqlite_schema
                 ORDER BY type, name",
            )
            .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|_| RebirthLifecycleError::LocatorInvalid)?
    };
    let expected_index = objects.iter().any(|(kind, name, table, sql)| {
        kind == "index"
            && name == "sqlite_autoindex_continuity_migration_receipts_1"
            && table == "continuity_migration_receipts"
            && sql.is_none()
    });
    let expected_tables = objects.iter().filter(|(kind, name, table, _)| {
        kind == "table"
            && ((*name == "continuity_generation_locator"
                && *table == "continuity_generation_locator")
                || (*name == "continuity_migration_receipts"
                    && *table == "continuity_migration_receipts"))
    });
    if objects.len() != 3 || expected_tables.count() != 2 || !expected_index {
        return Err(RebirthLifecycleError::LocatorInvalid);
    }
    Ok(())
}

fn rebirth_nonce_digest(request_nonce: &[u8; 32]) -> Digest {
    wire::domain_hash(b"astr-embodiment/rebirth-nonce-v1", &[request_nonce])
}

fn rebirth_authorized_binding_digest(request: &UserAuthorizedRebirthV1) -> Digest {
    rebirth_binding_digest_parts(
        request.scope_token,
        request.expected_incarnation_id,
        request.expected_revision,
        request.action,
    )
}

fn rebirth_receipt_id(permit: &RebirthCommitPermitV1, child: &RebirthStagedChildV1) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/rebirth-receipt-v1",
        &[
            &permit.request_nonce_digest,
            &permit.binding_digest,
            &permit.parent_authority.incarnation_id,
            &child.child_authority.incarnation_id,
            &child.child_authority.state_digest,
            &child.child_authority.graph_digest,
        ],
    )
}

fn capture_any_authority(database: &Path) -> Result<ContinuityAuthority, RebirthLifecycleError> {
    let (bot, persona, incarnation_id, revision) = read_single_binding(database)?;
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let scope = wire::persona_scope_digest(&bot, &persona, None);
    let revision_sql = revision_to_sql(revision)?;
    let graph_digest = connection
        .query_row(
            "SELECT graph_digest FROM incarnations WHERE incarnation_id = ?1",
            params![incarnation_id.to_vec()],
            |row| digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error),
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let state_digest = connection
        .query_row(
            "SELECT state_digest FROM snapshots WHERE scope_digest = ?1 AND revision = ?2",
            params![scope.to_vec(), revision_sql],
            |row| digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error),
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let history_digest = if revision == 0 {
        wire::domain_hash(
            b"astr-embodiment/continuity-empty-history-v1",
            &[&incarnation_id, &revision.to_le_bytes()],
        )
    } else {
        connection
            .query_row(
                "SELECT chain_digest FROM journal WHERE scope_digest = ?1 AND logical_revision = ?2",
                params![scope.to_vec(), revision_sql],
                |row| digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error),
            )
            .map_err(|_| RebirthLifecycleError::ChildInvalid)?
    };
    Ok(ContinuityAuthority {
        incarnation_id,
        revision,
        state_digest,
        graph_digest,
        history_digest,
    })
}

fn read_single_binding(
    database: &Path,
) -> Result<([u8; 16], [u8; 16], Digest, u64), RebirthLifecycleError> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let mut statement = connection
        .prepare(
            "SELECT bot_token, persona_token, incarnation_id, revision
             FROM active_bindings ORDER BY bot_token ASC, persona_token ASC",
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let mut rows = statement
        .query([])
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let row = rows
        .next()
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?
        .ok_or(RebirthLifecycleError::ChildInvalid)?;
    let bot: [u8; 16] = row
        .get::<_, Vec<u8>>(0)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?
        .try_into()
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let persona: [u8; 16] = row
        .get::<_, Vec<u8>>(1)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?
        .try_into()
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let incarnation_id = digest_from_blob(
        row.get::<_, Vec<u8>>(2)
            .map_err(|_| RebirthLifecycleError::ChildInvalid)?,
    )?;
    let revision = revision_from_sql(
        row.get::<_, i64>(3)
            .map_err(|_| RebirthLifecycleError::ChildInvalid)?,
    )?;
    if rows
        .next()
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?
        .is_some()
    {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    Ok((bot, persona, incarnation_id, revision))
}

fn ensure_locator_current(root: &Path, generation_id: &str) -> Result<(), RebirthLifecycleError> {
    let path = root.join(LOCATOR_DATABASE);
    let mut connection = Connection::open(path).map_err(|_| RebirthLifecycleError::Durability)?;
    connection
        .pragma_update(None, "journal_mode", "DELETE")
        .map_err(|_| RebirthLifecycleError::Durability)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|_| RebirthLifecycleError::Durability)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|_| RebirthLifecycleError::Durability)?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS continuity_generation_locator (
                 slot INTEGER PRIMARY KEY CHECK (slot = 1),
                 generation_id TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS continuity_migration_receipts (
                 source_generation TEXT NOT NULL,
                 target_generation TEXT NOT NULL,
                 before_incarnation_id BLOB NOT NULL,
                 before_revision INTEGER NOT NULL,
                 before_state_digest BLOB NOT NULL,
                 before_graph_digest BLOB NOT NULL,
                 before_history_digest BLOB NOT NULL,
                 after_incarnation_id BLOB NOT NULL,
                 after_revision INTEGER NOT NULL,
                 after_state_digest BLOB NOT NULL,
                 after_graph_digest BLOB NOT NULL,
                 after_history_digest BLOB NOT NULL,
                 replay INTEGER NOT NULL,
                 decision TEXT NOT NULL,
                 PRIMARY KEY (source_generation, target_generation)
             );",
        )
        .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO continuity_generation_locator (slot, generation_id) VALUES (?1, ?2)",
            params![LOCATOR_SLOT, generation_id],
        )
        .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
    let selected: String = transaction
        .query_row(
            "SELECT generation_id FROM continuity_generation_locator WHERE slot = ?1",
            params![LOCATOR_SLOT],
            |row| row.get(0),
        )
        .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
    if selected != generation_id {
        return Err(RebirthLifecycleError::BootstrapConflict);
    }
    transaction
        .commit()
        .map_err(|_| RebirthLifecycleError::Durability)
}

fn read_authoritative_generation(
    root: &Path,
    fallback_generation: &str,
) -> Result<String, RebirthLifecycleError> {
    let path = root.join(LOCATOR_DATABASE);
    if !path.is_file() {
        validate_generation_id(fallback_generation)?;
        return Ok(fallback_generation.to_owned());
    }
    let connection = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
    let generation: Option<String> = connection
        .query_row(
            "SELECT generation_id FROM continuity_generation_locator WHERE slot = ?1",
            params![LOCATOR_SLOT],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| RebirthLifecycleError::LocatorInvalid)?;
    let generation = generation.ok_or(RebirthLifecycleError::LocatorInvalid)?;
    validate_generation_id(&generation)?;
    Ok(generation)
}

fn sqlite_shadow_backup(source: &Path, destination: &Path) -> Result<(), RebirthLifecycleError> {
    let connection = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::Durability)?;
    let destination = sqlite_string_literal(destination)?;
    connection
        .execute_batch(&format!("VACUUM INTO {destination};"))
        .map_err(|_| RebirthLifecycleError::Durability)
}

fn sync_sqlite_database(path: &Path) -> Result<(), RebirthLifecycleError> {
    let connection = Connection::open(path).map_err(|_| RebirthLifecycleError::Durability)?;
    connection
        .pragma_update(None, "synchronous", "FULL")
        .map_err(|_| RebirthLifecycleError::Durability)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| RebirthLifecycleError::Durability)?;
    if integrity != "ok" {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    Ok(())
}

fn sqlite_string_literal(path: &Path) -> Result<String, RebirthLifecycleError> {
    let value = path.to_str().ok_or(RebirthLifecycleError::Durability)?;
    if value.contains('\0') {
        return Err(RebirthLifecycleError::Durability);
    }
    Ok(format!("'{}'", value.replace('\'', "''")))
}

fn write_control_file(path: &Path, bytes: &[u8]) -> Result<(), RebirthLifecycleError> {
    let parent = path.parent().ok_or(RebirthLifecycleError::Durability)?;
    let sequence = NEXT_CONTROL_WRITE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".rebirth-control-{}-{sequence}.tmp",
        std::process::id()
    ));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|_| RebirthLifecycleError::Durability)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| RebirthLifecycleError::Durability)?;
    drop(file);
    fs::rename(&temporary, path).map_err(|_| RebirthLifecycleError::Durability)?;
    Ok(())
}

fn owner_cbor(generation_id: &str, store_uuid: [u8; 16]) -> Vec<u8> {
    let mut bytes = vec![0xa2];
    cbor_text(&mut bytes, "generation_id");
    cbor_text(&mut bytes, generation_id);
    cbor_text(&mut bytes, "store_uuid");
    bytes.push(0x50);
    bytes.extend_from_slice(&store_uuid);
    bytes
}

fn cbor_text(bytes: &mut Vec<u8>, value: &str) {
    let length = value.len();
    if length <= 23 {
        bytes.push(0x60 + length as u8);
    } else {
        bytes.push(0x78);
        bytes.push(length as u8);
    }
    bytes.extend_from_slice(value.as_bytes());
}

fn bootstrap_store_uuid(root: &Path, incarnation_id: &Digest) -> [u8; 16] {
    let root_bytes = root.to_string_lossy();
    let digest = wire::domain_hash(
        b"astr-embodiment/vault-store-uuid-v1",
        &[root_bytes.as_bytes(), incarnation_id],
    );
    let mut uuid = [0; 16];
    uuid.copy_from_slice(&digest[..16]);
    if uuid.iter().all(|byte| *byte == 0) {
        uuid[0] = 1;
    }
    uuid
}

fn rebirth_binding_digest(request: &RebirthPrepareRequestV1) -> Digest {
    rebirth_binding_digest_parts(
        request.scope_token,
        request.expected_incarnation_id,
        request.expected_revision,
        request.action,
    )
}

fn rebirth_binding_digest_parts(
    scope_token: Digest,
    expected_incarnation_id: Digest,
    expected_revision: u64,
    action: RebirthActionV1,
) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/rebirth-binding-v1",
        &[
            &scope_token,
            &expected_incarnation_id,
            &expected_revision.to_le_bytes(),
            &action.tag(),
        ],
    )
}

fn legacy_generation_id(incarnation_id: &Digest) -> String {
    format!("legacy-{}", short_digest(incarnation_id))
}

fn short_digest(digest: &Digest) -> String {
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn digest_hex(digest: &Digest) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn validate_generation_id(value: &str) -> Result<(), RebirthLifecycleError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
    Ok(())
}

fn digest_from_blob(value: Vec<u8>) -> Result<Digest, RebirthLifecycleError> {
    value
        .try_into()
        .map_err(|_| RebirthLifecycleError::ChildInvalid)
}

fn revision_to_sql(value: u64) -> Result<i64, RebirthLifecycleError> {
    i64::try_from(value).map_err(|_| RebirthLifecycleError::Durability)
}

fn revision_from_sql(value: i64) -> Result<u64, RebirthLifecycleError> {
    u64::try_from(value).map_err(|_| RebirthLifecycleError::ChildInvalid)
}

fn sqlite_conversion_error(_: RebirthLifecycleError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Blob,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid rebirth durable bytes",
        )),
    )
}
