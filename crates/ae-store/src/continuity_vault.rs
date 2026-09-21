use crate::continuity_migration::ContinuityAuthority;
use crate::{ClaimOutcome, Store};
use ae_contracts::{hex, wire, Digest};
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, TransactionBehavior};
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
#[cfg(windows)]
use std::iter::once;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use thiserror::Error;
#[cfg(windows)]
use windows_sys::core::PCWSTR;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

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

/// Closed observation values accepted by the seed-configuration lifecycle.
/// These values deliberately preserve the distinction between an explicit
/// empty value, a missing key, and a failed host read; collapsing those cases
/// would make an upgrade/default read look like a destructive user action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedConfigObservationV1 {
    PresentNonempty,
    PresentEmpty,
    Missing,
    ReadFailed,
}

/// The host path that produced a seed configuration observation.  Rust keeps
/// this closed so a new Python caller cannot accidentally acquire destructive
/// authority merely by inventing an origin string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedConfigOriginV1 {
    UserSaveEvent,
    StartupRead,
    PluginWriteback,
    LegacyConfigMigration,
}

/// Coarse result state of the seed configuration reconciler.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedConfigStateV1 {
    Unchanged,
    WriteMirror,
    Deferred,
    RebirthCommitted,
    RebirthReplayed,
}

/// Closed writeback acknowledgement state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedConfigAckStateV1 {
    MirrorActive,
    Replayed,
    Stale,
}

/// The immediate, non-persisted host repair values.  This intentionally has
/// no `Debug` implementation: SeedCode, guard and token must never enter a
/// trace or an error formatter.
#[derive(Clone, PartialEq, Eq)]
pub struct SeedConfigWritebackV1 {
    pub seed_code: String,
    pub mirror_guard: String,
    pub writeback_token: String,
}

/// Raw host observations cross this boundary once.  The lifecycle persists
/// only one-way digests of the guard/token and never derives identity from the
/// presented SeedCode.
#[derive(Clone, PartialEq, Eq)]
pub struct SeedConfigReconcileRequestV1 {
    pub scope_token: Digest,
    pub observation: SeedConfigObservationV1,
    pub origin: SeedConfigOriginV1,
    pub seed_code: Option<String>,
    pub mirror_guard: Option<String>,
    pub previous_observation: Option<SeedConfigObservationV1>,
    pub package_epoch: String,
    pub config_schema_version: u16,
    pub host_config_revision: u64,
}

/// A result is deliberately non-Debug because it may carry immediate raw
/// writeback values.  The `reason` member is a closed, non-secret code.
#[derive(Clone, PartialEq, Eq)]
pub struct SeedConfigReconcileResultV1 {
    pub state: SeedConfigStateV1,
    pub writeback: Option<SeedConfigWritebackV1>,
    pub before_revision: Option<u64>,
    pub after_revision: Option<u64>,
    pub reason: &'static str,
}

/// Raw acknowledgement capability.  A false acknowledgement is never an
/// activation request and is rejected at the native boundary.
#[derive(Clone, PartialEq, Eq)]
pub struct SeedConfigWritebackAckV1 {
    pub scope_token: Digest,
    pub writeback_token: String,
    pub write_succeeded: bool,
    pub host_config_revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeedConfigAckResultV1 {
    pub state: SeedConfigAckStateV1,
}

/// A dedicated seed-clear permit.  It is intentionally unrelated to manual
/// rebirth challenge/nonce/confirmation state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedClearCommitPermitV1 {
    pub scope_token: Digest,
    pub intent_id: Digest,
    pub source_guard_digest: Digest,
    pub parent_generation_id: String,
    pub parent_authority: ContinuityAuthority,
    pub parent_seed_code_digest: Digest,
    pub package_epoch_digest: Digest,
    pub config_schema_version: u16,
    pub host_config_revision: u64,
    pub created_at_ms: u64,
    stage_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeedClearStagedChildV1 {
    pub scope_token: Digest,
    pub intent_id: Digest,
    pub source_guard_digest: Digest,
    pub parent_generation_id: String,
    pub parent_authority: ContinuityAuthority,
    pub parent_seed_code_digest: Digest,
    pub child_generation_id: String,
    pub child_authority: ContinuityAuthority,
    pub child_seed_code_digest: Digest,
}

#[derive(Clone, PartialEq, Eq)]
pub enum SeedConfigPreflightV1 {
    Result(SeedConfigReconcileResultV1),
    Stage(Box<SeedClearCommitPermitV1>),
}

/// Fixed-code failures for the new seed-config ABI.  Do not add dynamic
/// details here: PyO3 must expose only this closed code set.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SeedConfigLifecycleError {
    #[error("SEED_CONFIG_SCHEMA_INVALID")]
    SchemaInvalid,
    #[error("SEED_CONFIG_OBSERVATION_UNCERTAIN")]
    ObservationUncertain,
    #[error("SEED_CONFIG_MIRROR_STALE")]
    MirrorStale,
    #[error("SEED_CLEAR_FENCE_STALE")]
    FenceStale,
    #[error("SEED_CLEAR_IN_FLIGHT")]
    InFlight,
    #[error("SEED_CLEAR_STORAGE_FAILED")]
    StorageFailed,
    #[error("SEED_CLEAR_LOCATOR_INVALID")]
    LocatorInvalid,
    #[error("SEED_CLEAR_UNKNOWN")]
    Unknown,
}

impl SeedConfigLifecycleError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::SchemaInvalid => "SEED_CONFIG_SCHEMA_INVALID",
            Self::ObservationUncertain => "SEED_CONFIG_OBSERVATION_UNCERTAIN",
            Self::MirrorStale => "SEED_CONFIG_MIRROR_STALE",
            Self::FenceStale => "SEED_CLEAR_FENCE_STALE",
            Self::InFlight => "SEED_CLEAR_IN_FLIGHT",
            Self::StorageFailed => "SEED_CLEAR_STORAGE_FAILED",
            Self::LocatorInvalid => "SEED_CLEAR_LOCATOR_INVALID",
            Self::Unknown => "SEED_CLEAR_UNKNOWN",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SeedConfigCurrentV1 {
    current: RebirthCurrentV1,
    scope_token: Digest,
    seed_code_digest: Digest,
}

#[derive(Clone, Debug)]
struct StoredSeedConfigMirrorV1 {
    mirror_id: Digest,
    scope_token: Digest,
    authority_generation_id: String,
    authority_incarnation_id: Digest,
    authority_revision: u64,
    native_seed_digest: Digest,
    package_epoch_digest: Digest,
    config_schema_version: u16,
    guard_digest: Digest,
    host_config_revision: u64,
    status: String,
    writeback_token_digest: Digest,
}

#[derive(Clone, Debug)]
struct StoredSeedClearIntentV1 {
    intent_id: Digest,
    scope_token: Digest,
    source_mirror_id: Digest,
    source_guard_digest: Digest,
    expected_generation_id: String,
    expected_incarnation_id: Digest,
    expected_revision: u64,
    expected_seed_digest: Digest,
    package_epoch_digest: Digest,
    config_schema_version: u16,
    host_config_revision: u64,
    status: String,
    stage_epoch: u64,
    created_at_ms: u64,
}

#[derive(Clone, Debug)]
struct StoredSeedClearConsumptionV1 {
    guard_digest: Digest,
    intent_id: Digest,
    scope_token: Digest,
    child_generation_id: String,
    child_incarnation_id: Digest,
    child_seed_digest: Digest,
    before_revision: u64,
    after_revision: u64,
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
        let stage_epoch = match stored.status.as_str() {
            // A durable staging lease survives process restart.  Its exact
            // persisted epoch is the only permit that may resume it.
            "staging" if stored.stage_epoch != 0 => stored.stage_epoch,
            "staging" => return Err(RebirthLifecycleError::Durability),
            "pending" => {
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
                next_epoch
            }
            _ => return Err(RebirthLifecycleError::Durability),
        };
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
            stage_epoch,
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
                match durably_install_staged_generation(&temporary, directory, &temporary_database)?
                {
                    true => stage_existing_child(
                        &database,
                        &genesis,
                        &permit.parent_authority.incarnation_id,
                    )?,
                    false => stage_existing_child(
                        &database,
                        &genesis,
                        &permit.parent_authority.incarnation_id,
                    )?,
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

    /// Reconcile one closed host configuration observation.  This is the
    /// first Rust-owned authority fence: it re-reads the lifecycle-selected
    /// generation and its authority DB before creating a mirror or seed-clear
    /// intent.  Python supplies no incarnation/revision fence.
    pub fn reconcile_seed_config_preflight_v1(
        &self,
        request: &SeedConfigReconcileRequestV1,
    ) -> Result<SeedConfigPreflightV1, SeedConfigLifecycleError> {
        validate_seed_config_request(request)?;
        let initial_current = self.current_seed_config_authority_v1()?;
        if initial_current.scope_token != request.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let package_epoch_digest = seed_config_package_epoch_digest(&request.package_epoch);
        let current = self.current_seed_config_authority_v1()?;
        if current != initial_current || current.scope_token != request.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let locator_path = self.root.join(LOCATOR_DATABASE);
        let authority_path = self
            .child_authority_database_path(&current.current.generation_id)
            .map_err(seed_error_from_rebirth)?;
        if !locator_path.is_file() || !authority_path.is_file() {
            return Err(SeedConfigLifecycleError::LocatorInvalid);
        }
        let mut connection = self.open_seed_ledger()?;
        attach_seed_config_fence_databases(&connection, &locator_path, &authority_path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        initialize_ledger_schema(&transaction).map_err(seed_error_from_rebirth)?;
        initialize_seed_config_ledger_schema(&transaction)?;
        validate_attached_locator(&transaction).map_err(seed_error_from_rebirth)?;
        let locator_generation: String = transaction
            .query_row(
                "SELECT generation_id FROM rebirth_locator.continuity_generation_locator WHERE slot = ?1",
                params![LOCATOR_SLOT],
                |row| row.get(0),
            )
            .map_err(|_| SeedConfigLifecycleError::LocatorInvalid)?;
        if locator_generation != current.current.generation_id {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let locked_current = attached_seed_config_current(
            &transaction,
            AttachedSeedConfigAuthority::Parent,
            &current.current.generation_id,
        )?;
        if locked_current != current || locked_current.scope_token != request.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        ensure_ledger_current(&transaction, &locked_current.current)
            .map_err(seed_error_from_rebirth)?;
        let current = locked_current;

        let active = load_active_seed_config_mirror(&transaction, request.scope_token)?;
        let current_matching = active.as_ref().is_some_and(|mirror| {
            seed_config_mirror_matches_current(
                mirror,
                &current,
                package_epoch_digest,
                request.config_schema_version,
            )
        });

        // Consumption is checked before the current active mirror.  After an
        // authority-first commit, an old empty config may be retried while the
        // new mirror is still pending host writeback; it must replay rather
        // than mint a second child.
        if request.observation == SeedConfigObservationV1::PresentEmpty {
            if let Some(raw_guard) = request.mirror_guard.as_deref() {
                let guard_digest = seed_config_guard_digest(raw_guard)?;
                if let Some(consumption) = load_seed_clear_consumption(&transaction, guard_digest)?
                {
                    let result = replay_seed_clear_result(
                        &transaction,
                        &current,
                        request,
                        package_epoch_digest,
                        &consumption,
                    )?;
                    return finish_seed_preflight(transaction, result);
                }
            }
        }

        match request.observation {
            SeedConfigObservationV1::PresentNonempty => {
                let native_seed = ae_genesis::format_seed_code(&current.seed_code_digest);
                let seed_matches = request.seed_code.as_deref() == Some(native_seed.as_str());
                let guard_matches = active
                    .as_ref()
                    .zip(request.mirror_guard.as_deref())
                    .map(|(mirror, guard)| seed_config_mirror_guard_matches(mirror, guard))
                    .transpose()?
                    .unwrap_or(false);
                if seed_matches
                    && current_matching
                    && active
                        .as_ref()
                        .is_some_and(|mirror| mirror.status == "ACTIVE")
                    && guard_matches
                {
                    return finish_seed_preflight(
                        transaction,
                        seed_config_result(
                            SeedConfigStateV1::Unchanged,
                            None,
                            None,
                            None,
                            "SEED_CONFIG_NATIVE_MATCH",
                        ),
                    );
                }

                // Raw writeback material is intentionally not recoverable
                // from the ledger.  A pending acknowledgement retry therefore
                // rotates the pending mirror to a new safe repair capability.
                let (_, writeback) = create_seed_config_mirror(
                    &transaction,
                    &current,
                    package_epoch_digest,
                    request.config_schema_version,
                    request.host_config_revision,
                )?;
                finish_seed_preflight(
                    transaction,
                    seed_config_result(
                        SeedConfigStateV1::WriteMirror,
                        Some(writeback),
                        None,
                        None,
                        "SEED_CONFIG_REPAIR_REQUIRED",
                    ),
                )
            }
            SeedConfigObservationV1::PresentEmpty => {
                let Some(mirror) = active else {
                    let (_, writeback) = create_seed_config_mirror(
                        &transaction,
                        &current,
                        package_epoch_digest,
                        request.config_schema_version,
                        request.host_config_revision,
                    )?;
                    return finish_seed_preflight(
                        transaction,
                        seed_config_result(
                            SeedConfigStateV1::WriteMirror,
                            Some(writeback),
                            None,
                            None,
                            "SEED_CONFIG_REPAIR_REQUIRED",
                        ),
                    );
                };
                if !current_matching || mirror.status != "ACTIVE" {
                    let (_, writeback) = create_seed_config_mirror(
                        &transaction,
                        &current,
                        package_epoch_digest,
                        request.config_schema_version,
                        request.host_config_revision,
                    )?;
                    return finish_seed_preflight(
                        transaction,
                        seed_config_result(
                            SeedConfigStateV1::WriteMirror,
                            Some(writeback),
                            None,
                            None,
                            "SEED_CONFIG_REPAIR_REQUIRED",
                        ),
                    );
                }
                let Some(raw_guard) = request.mirror_guard.as_deref() else {
                    return finish_seed_preflight(
                        transaction,
                        seed_config_result(
                            SeedConfigStateV1::Deferred,
                            None,
                            None,
                            None,
                            "SEED_CONFIG_OBSERVATION_DEFERRED",
                        ),
                    );
                };
                if !seed_config_mirror_guard_matches(&mirror, raw_guard)?
                    || !seed_config_empty_authorized(request, &mirror)
                {
                    return finish_seed_preflight(
                        transaction,
                        seed_config_result(
                            SeedConfigStateV1::Deferred,
                            None,
                            None,
                            None,
                            "SEED_CONFIG_OBSERVATION_DEFERRED",
                        ),
                    );
                }
                let guard_digest = seed_config_guard_digest(raw_guard)?;
                let permit = begin_seed_clear_intent(
                    &transaction,
                    &current,
                    &mirror,
                    guard_digest,
                    package_epoch_digest,
                    request.config_schema_version,
                    request.host_config_revision,
                )?;
                transaction
                    .commit()
                    .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
                Ok(SeedConfigPreflightV1::Stage(Box::new(permit)))
            }
            SeedConfigObservationV1::Missing | SeedConfigObservationV1::ReadFailed => {
                finish_seed_preflight(
                    transaction,
                    seed_config_result(
                        SeedConfigStateV1::Deferred,
                        None,
                        None,
                        None,
                        "SEED_CONFIG_OBSERVATION_DEFERRED",
                    ),
                )
            }
        }
    }

    /// Acknowledge only a successful host writeback.  This is the second Rust
    /// authority fence: a stale pending mirror cannot be activated after a
    /// generation switch or authority revision change.
    pub fn ack_seed_config_writeback_v1(
        &self,
        request: &SeedConfigWritebackAckV1,
    ) -> Result<SeedConfigAckResultV1, SeedConfigLifecycleError> {
        if !request.write_succeeded || request.writeback_token.is_empty() {
            return Err(SeedConfigLifecycleError::SchemaInvalid);
        }
        let initial_current = self.current_seed_config_authority_v1()?;
        if initial_current.scope_token != request.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let token_digest = seed_config_writeback_token_digest(&request.writeback_token)?;
        let current = self.current_seed_config_authority_v1()?;
        if current != initial_current || current.scope_token != request.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let locator_path = self.root.join(LOCATOR_DATABASE);
        let authority_path = self
            .child_authority_database_path(&current.current.generation_id)
            .map_err(seed_error_from_rebirth)?;
        if !locator_path.is_file() || !authority_path.is_file() {
            return Err(SeedConfigLifecycleError::LocatorInvalid);
        }
        let mut connection = self.open_seed_ledger()?;
        attach_seed_config_fence_databases(&connection, &locator_path, &authority_path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        initialize_ledger_schema(&transaction).map_err(seed_error_from_rebirth)?;
        initialize_seed_config_ledger_schema(&transaction)?;
        validate_attached_locator(&transaction).map_err(seed_error_from_rebirth)?;
        let locator_generation: String = transaction
            .query_row(
                "SELECT generation_id FROM rebirth_locator.continuity_generation_locator WHERE slot = ?1",
                params![LOCATOR_SLOT],
                |row| row.get(0),
            )
            .map_err(|_| SeedConfigLifecycleError::LocatorInvalid)?;
        if locator_generation != current.current.generation_id {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let locked_current = attached_seed_config_current(
            &transaction,
            AttachedSeedConfigAuthority::Parent,
            &current.current.generation_id,
        )?;
        if locked_current != current || locked_current.scope_token != request.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        ensure_ledger_current(&transaction, &locked_current.current)
            .map_err(seed_error_from_rebirth)?;
        let current = locked_current;
        let mirror = load_seed_config_mirror_by_writeback_token(&transaction, token_digest)?;
        let Some(mirror) = mirror else {
            transaction
                .commit()
                .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
            return Ok(SeedConfigAckResultV1 {
                state: SeedConfigAckStateV1::Stale,
            });
        };
        if mirror.writeback_token_digest != token_digest
            || mirror.scope_token != request.scope_token
            || !seed_config_mirror_matches_current(
                &mirror,
                &current,
                mirror.package_epoch_digest,
                mirror.config_schema_version,
            )
        {
            transaction
                .commit()
                .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
            return Ok(SeedConfigAckResultV1 {
                state: SeedConfigAckStateV1::Stale,
            });
        }
        let state = match mirror.status.as_str() {
            "ACTIVE" => SeedConfigAckStateV1::Replayed,
            "PENDING_WRITEBACK" => {
                if request.host_config_revision != 0
                    && request.host_config_revision < mirror.host_config_revision
                {
                    SeedConfigAckStateV1::Stale
                } else {
                    let updated = transaction
                        .execute(
                            "UPDATE seed_config_mirror_v1
                             SET status = 'ACTIVE', host_config_revision = ?2,
                                 updated_at_ms = ?3
                             WHERE mirror_id = ?1 AND status = 'PENDING_WRITEBACK'",
                            params![
                                mirror.mirror_id.to_vec(),
                                revision_to_sql(request.host_config_revision)
                                    .map_err(seed_error_from_rebirth)?,
                                revision_to_sql(crate::now_ms())
                                    .map_err(seed_error_from_rebirth)?,
                            ],
                        )
                        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
                    if updated == 1 {
                        SeedConfigAckStateV1::MirrorActive
                    } else {
                        SeedConfigAckStateV1::Stale
                    }
                }
            }
            _ => SeedConfigAckStateV1::Stale,
        };
        transaction
            .commit()
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        Ok(SeedConfigAckResultV1 { state })
    }

    /// Seed-clear children reuse the durable Store staging mechanics but have
    /// their own permit/domain and never touch manual rebirth challenge state.
    pub fn stage_seed_clear_child_v1(
        &self,
        permit: &SeedClearCommitPermitV1,
        request: RebirthChildStageRequestV1,
    ) -> Result<SeedClearStagedChildV1, SeedConfigLifecycleError> {
        let expected_nonce = Self::seed_clear_child_genesis_nonce_digest_for_permit(permit);
        let source_scope = wire::persona_scope_digest(
            &request.genesis.source.scope.bot_token,
            &request.genesis.source.scope.persona_token,
            None,
        );
        if request.genesis.nonce_digest != expected_nonce
            || source_scope != permit.scope_token
            || request.genesis.incarnation_id == permit.parent_authority.incarnation_id
            || request.genesis.seed_code_digest == permit.parent_seed_code_digest
            || request.genesis.receipt.incarnation_id != request.genesis.incarnation_id
            || request.genesis.receipt.seed_code_digest != request.genesis.seed_code_digest
        {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let expected_incarnation = request.genesis.incarnation_id;
        let child_seed_code_digest = request.genesis.seed_code_digest;
        let generation_id = Self::seed_clear_child_generation_id_for(&expected_incarnation);
        let database = self
            .seed_clear_child_authority_database_path(&generation_id)
            .map_err(seed_error_from_rebirth)?;
        let directory = database
            .parent()
            .ok_or(SeedConfigLifecycleError::StorageFailed)?;
        let existing = if directory.exists() {
            Some(
                stage_existing_child(
                    &database,
                    &request.genesis,
                    &permit.parent_authority.incarnation_id,
                )
                .map_err(seed_error_from_rebirth)?,
            )
        } else {
            None
        };
        let child_authority = match existing {
            Some(authority) => authority,
            None => {
                let generations = directory
                    .parent()
                    .ok_or(SeedConfigLifecycleError::StorageFailed)?;
                fs::create_dir_all(generations)
                    .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
                let stage_sequence = NEXT_CONTROL_WRITE.fetch_add(1, Ordering::Relaxed);
                let temporary = generations.join(format!(
                    ".seed-clear-stage-{generation_id}-{}-{stage_sequence}",
                    std::process::id()
                ));
                fs::create_dir(&temporary).map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
                let temporary_database = temporary.join(AUTHORITY_DATABASE);
                let mut store = Store::open(&temporary_database)
                    .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
                let mut genesis = request.genesis;
                match store
                    .claim_lease(&genesis.scope_key, Some(genesis.nonce_digest))
                    .map_err(|_| SeedConfigLifecycleError::StorageFailed)?
                {
                    ClaimOutcome::Claimed { lease_epoch, nonce }
                        if nonce == genesis.nonce_digest =>
                    {
                        genesis.lease_epoch = lease_epoch;
                    }
                    ClaimOutcome::InFlight => return Err(SeedConfigLifecycleError::InFlight),
                    ClaimOutcome::Committed => return Err(SeedConfigLifecycleError::FenceStale),
                    ClaimOutcome::Claimed { .. } => {
                        return Err(SeedConfigLifecycleError::FenceStale)
                    }
                }
                store
                    .commit_genesis(&genesis)
                    .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
                store
                    .close()
                    .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
                install_child_lineage(
                    &temporary_database,
                    &genesis.source.scope.bot_token,
                    &genesis.source.scope.persona_token,
                    &genesis.incarnation_id,
                    &permit.parent_authority.incarnation_id,
                )
                .map_err(seed_error_from_rebirth)?;
                match durably_install_staged_generation(&temporary, directory, &temporary_database)
                    .map_err(seed_error_from_rebirth)?
                {
                    true => stage_existing_child(
                        &database,
                        &genesis,
                        &permit.parent_authority.incarnation_id,
                    )
                    .map_err(seed_error_from_rebirth)?,
                    false => stage_existing_child(
                        &database,
                        &genesis,
                        &permit.parent_authority.incarnation_id,
                    )
                    .map_err(seed_error_from_rebirth)?,
                }
            }
        };
        if child_authority.revision != 0 || child_authority.incarnation_id != expected_incarnation {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        Ok(SeedClearStagedChildV1 {
            scope_token: permit.scope_token,
            intent_id: permit.intent_id,
            source_guard_digest: permit.source_guard_digest,
            parent_generation_id: permit.parent_generation_id.clone(),
            parent_authority: permit.parent_authority.clone(),
            parent_seed_code_digest: permit.parent_seed_code_digest,
            child_generation_id: generation_id,
            child_authority,
            child_seed_code_digest,
        })
    }

    /// Second Rust authority fence and atomic locator/consumption commit for
    /// seed-clear.  Manual rebirth tables are neither read nor written here.
    pub fn commit_seed_clear_v1(
        &self,
        permit: &SeedClearCommitPermitV1,
        child: &SeedClearStagedChildV1,
    ) -> Result<SeedConfigReconcileResultV1, SeedConfigLifecycleError> {
        let current = self.current_seed_config_authority_v1()?;
        if current.scope_token != permit.scope_token {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        if current.current.generation_id != permit.parent_generation_id
            || current.current.authority != permit.parent_authority
            || current.seed_code_digest != permit.parent_seed_code_digest
        {
            if self.seed_clear_consumed_by_permit_v1(permit)? {
                // A peer can have committed after this caller staged the same
                // deterministic child.  The normal next reconcile will
                // return the durable REBIRTH_REPLAYED response; this in-flight
                // commit must not report a generic stale fence or attempt a
                // second locator switch.
                return Err(SeedConfigLifecycleError::InFlight);
            }
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        validate_seed_clear_staged_child_shape(permit, child)?;
        let locator_path = self.root.join(LOCATOR_DATABASE);
        let authority_path = self
            .child_authority_database_path(&current.current.generation_id)
            .map_err(seed_error_from_rebirth)?;
        let child_authority_path = self
            .seed_clear_child_authority_database_path(&child.child_generation_id)
            .map_err(seed_error_from_rebirth)?;
        if !locator_path.is_file() || !authority_path.is_file() || !child_authority_path.is_file() {
            return Err(SeedConfigLifecycleError::LocatorInvalid);
        }
        let mut connection = self.open_seed_ledger()?;
        attach_seed_config_commit_fence_databases(
            &connection,
            &locator_path,
            &authority_path,
            &child_authority_path,
        )?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        initialize_ledger_schema(&transaction).map_err(seed_error_from_rebirth)?;
        initialize_seed_config_ledger_schema(&transaction)?;
        validate_attached_locator(&transaction).map_err(seed_error_from_rebirth)?;
        let locator_generation: String = transaction
            .query_row(
                "SELECT generation_id FROM rebirth_locator.continuity_generation_locator WHERE slot = ?1",
                params![LOCATOR_SLOT],
                |row| row.get(0),
            )
            .map_err(|_| SeedConfigLifecycleError::LocatorInvalid)?;
        if locator_generation != permit.parent_generation_id {
            if load_seed_clear_consumption(&transaction, permit.source_guard_digest)?
                .as_ref()
                .is_some_and(|consumption| {
                    seed_clear_consumption_matches_permit(consumption, permit)
                })
            {
                return Err(SeedConfigLifecycleError::InFlight);
            }
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let locked_current = attached_seed_config_current(
            &transaction,
            AttachedSeedConfigAuthority::Parent,
            &current.current.generation_id,
        )?;
        if locked_current != current
            || locked_current.scope_token != permit.scope_token
            || locked_current.current.generation_id != permit.parent_generation_id
            || locked_current.current.authority != permit.parent_authority
            || locked_current.seed_code_digest != permit.parent_seed_code_digest
        {
            if load_seed_clear_consumption(&transaction, permit.source_guard_digest)?
                .as_ref()
                .is_some_and(|consumption| {
                    seed_clear_consumption_matches_permit(consumption, permit)
                })
            {
                return Err(SeedConfigLifecycleError::InFlight);
            }
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        ensure_ledger_current(&transaction, &locked_current.current)
            .map_err(seed_error_from_rebirth)?;
        let current = locked_current;

        if let Some(consumption) =
            load_seed_clear_consumption(&transaction, permit.source_guard_digest)?
        {
            if consumption.guard_digest != permit.source_guard_digest
                || consumption.intent_id != permit.intent_id
            {
                return Err(SeedConfigLifecycleError::StorageFailed);
            }
            let result = replay_seed_clear_result_from_consumption(
                &transaction,
                &current,
                permit.package_epoch_digest,
                permit.config_schema_version,
                &consumption,
            )?;
            transaction
                .commit()
                .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
            return Ok(result);
        }
        let intent = load_seed_clear_intent(&transaction, permit.intent_id)?
            .ok_or(SeedConfigLifecycleError::FenceStale)?;
        if !seed_clear_intent_matches_permit(&intent, permit) {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        if intent.status != "STAGING" || intent.stage_epoch != permit.stage_epoch {
            return Err(SeedConfigLifecycleError::InFlight);
        }
        validate_attached_locator(&transaction).map_err(seed_error_from_rebirth)?;
        let locator_generation: String = transaction
            .query_row(
                "SELECT generation_id FROM rebirth_locator.continuity_generation_locator WHERE slot = ?1",
                params![LOCATOR_SLOT],
                |row| row.get(0),
            )
            .map_err(|_| SeedConfigLifecycleError::LocatorInvalid)?;
        if locator_generation != permit.parent_generation_id {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        validate_attached_seed_clear_child(&transaction, permit, child)?;
        let receipt_id = seed_clear_receipt_id(permit, child);
        let now = crate::now_ms();
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
            .map_err(|_| SeedConfigLifecycleError::LocatorInvalid)?;
        if changed != 1 {
            return Err(SeedConfigLifecycleError::FenceStale);
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
                    revision_to_sql(child.child_authority.revision)
                        .map_err(seed_error_from_rebirth)?,
                    child.child_authority.state_digest.to_vec(),
                    child.child_authority.graph_digest.to_vec(),
                    child.child_authority.history_digest.to_vec(),
                ],
            )
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        let parent_consumed = transaction
            .execute(
                "UPDATE seed_config_mirror_v1 SET status = 'CONSUMED', updated_at_ms = ?2
                 WHERE mirror_id = ?1 AND guard_digest = ?3
                       AND status = 'ACTIVE'",
                params![
                    intent.source_mirror_id.to_vec(),
                    revision_to_sql(now).map_err(seed_error_from_rebirth)?,
                    permit.source_guard_digest.to_vec(),
                ],
            )
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        if parent_consumed != 1 {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        transaction
            .execute(
                "INSERT INTO seed_clear_consumption_v1 (
                     guard_digest, intent_id, receipt_id, scope_token,
                     child_generation_id, child_incarnation_id, child_seed_digest,
                     before_revision, after_revision, committed_at_ms
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
                params![
                    permit.source_guard_digest.to_vec(),
                    permit.intent_id.to_vec(),
                    receipt_id.to_vec(),
                    permit.scope_token.to_vec(),
                    child.child_generation_id,
                    child.child_authority.incarnation_id.to_vec(),
                    child.child_seed_code_digest.to_vec(),
                    revision_to_sql(permit.parent_authority.revision)
                        .map_err(seed_error_from_rebirth)?,
                    revision_to_sql(now).map_err(seed_error_from_rebirth)?,
                ],
            )
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        let intent_updated = transaction
            .execute(
                "UPDATE seed_clear_intent_v1
                 SET status = 'COMMITTED', staged_child_generation_id = ?2,
                     receipt_id = ?3, updated_at_ms = ?4
                 WHERE intent_id = ?1 AND status = 'STAGING' AND stage_epoch = ?5",
                params![
                    permit.intent_id.to_vec(),
                    child.child_generation_id,
                    receipt_id.to_vec(),
                    revision_to_sql(now).map_err(seed_error_from_rebirth)?,
                    revision_to_sql(permit.stage_epoch).map_err(seed_error_from_rebirth)?,
                ],
            )
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        if intent_updated != 1 {
            return Err(SeedConfigLifecycleError::FenceStale);
        }
        let child_current = SeedConfigCurrentV1 {
            current: RebirthCurrentV1 {
                generation_id: child.child_generation_id.clone(),
                authority: child.child_authority.clone(),
            },
            scope_token: permit.scope_token,
            seed_code_digest: child.child_seed_code_digest,
        };
        let (_, writeback) = create_seed_config_mirror(
            &transaction,
            &child_current,
            permit.package_epoch_digest,
            permit.config_schema_version,
            permit.host_config_revision,
        )?;
        transaction
            .commit()
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        Ok(seed_config_result(
            SeedConfigStateV1::RebirthCommitted,
            Some(writeback),
            Some(permit.parent_authority.revision),
            Some(0),
            "SEED_CLEAR_REBIRTH_COMMITTED",
        ))
    }

    /// Deterministic retry nonce for exactly one seed-clear child.  It is a
    /// separate domain from manual rebirth and contains no raw guard/token.
    pub fn seed_clear_child_genesis_nonce_digest_for_permit(
        permit: &SeedClearCommitPermitV1,
    ) -> Digest {
        wire::domain_hash(
            b"astr-embodiment/seed-config-clear-child-genesis-nonce-v1",
            &[&permit.intent_id, &permit.source_guard_digest],
        )
    }

    pub fn seed_clear_child_generation_id_for(incarnation_id: &Digest) -> String {
        format!("seed-clear-{}", short_digest(incarnation_id))
    }

    fn seed_clear_child_authority_database_path(
        &self,
        generation_id: &str,
    ) -> Result<PathBuf, RebirthLifecycleError> {
        self.child_authority_database_path(generation_id)
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
        let (bot, persona, incarnation_id, _) = read_single_binding(&database)?;
        if incarnation_id != current.authority.incarnation_id {
            return Err(RebirthLifecycleError::LocatorInvalid);
        }
        Ok((current, wire::persona_scope_digest(&bot, &persona, None)))
    }

    /// Capture the full native authority fence used by the seed-config
    /// lifecycle.  This is intentionally private to Rust; neither Python nor
    /// the JSON ABI receives incarnation/generation/revision fence material.
    fn current_seed_config_authority_v1(
        &self,
    ) -> Result<SeedConfigCurrentV1, SeedConfigLifecycleError> {
        let (current, scope_token) = self.current_with_scope().map_err(seed_error_from_rebirth)?;
        let database = self
            .child_authority_database_path(&current.generation_id)
            .map_err(seed_error_from_rebirth)?;
        let seed_code_digest = read_seed_code_digest(&database, &current.authority.incarnation_id)
            .map_err(seed_error_from_rebirth)?;
        Ok(SeedConfigCurrentV1 {
            current,
            scope_token,
            seed_code_digest,
        })
    }

    fn open_seed_ledger(&self) -> Result<Connection, SeedConfigLifecycleError> {
        self.open_ledger().map_err(seed_error_from_rebirth)
    }

    /// Resolve a post-stage authority change without trusting a Python-side
    /// observation.  This is intentionally only a durable consumption lookup:
    /// it cannot create a mirror, intent, child, or locator mutation.  A
    /// caller that loses the race receives `SEED_CLEAR_IN_FLIGHT` and a later
    /// reconcile replays the committed writeback from the child authority.
    fn seed_clear_consumed_by_permit_v1(
        &self,
        permit: &SeedClearCommitPermitV1,
    ) -> Result<bool, SeedConfigLifecycleError> {
        let mut connection = self.open_seed_ledger()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        initialize_seed_config_ledger_schema(&transaction)?;
        let consumed = load_seed_clear_consumption(&transaction, permit.source_guard_digest)?
            .as_ref()
            .is_some_and(|consumption| seed_clear_consumption_matches_permit(consumption, permit));
        transaction
            .commit()
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
        Ok(consumed)
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

/// Attach the two durable databases which participate in the seed-config
/// authority fence before `BEGIN IMMEDIATE`. SQLite then acquires the writer
/// reservation for the selected authority database as well as the lifecycle
/// ledger/locator, so a concurrent event cannot advance its revision between
/// the second read and the locator CAS.
fn attach_seed_config_fence_databases(
    connection: &Connection,
    locator_path: &Path,
    authority_path: &Path,
) -> Result<(), SeedConfigLifecycleError> {
    let locator_literal = sqlite_string_literal(locator_path).map_err(seed_error_from_rebirth)?;
    let authority_literal =
        sqlite_string_literal(authority_path).map_err(seed_error_from_rebirth)?;
    connection
        .execute_batch(&format!(
            "ATTACH DATABASE {locator_literal} AS rebirth_locator;\
             PRAGMA rebirth_locator.journal_mode = DELETE;\
             PRAGMA rebirth_locator.synchronous = FULL;\
             ATTACH DATABASE {authority_literal} AS seed_parent;"
        ))
        .map_err(|_| SeedConfigLifecycleError::LocatorInvalid)
}

/// Commit additionally attaches the already-installed child before obtaining
/// the transaction's immediate lock.  The child is re-read through that exact
/// attachment before the locator CAS; no pre-transaction child inspection is
/// treated as commit authority.
fn attach_seed_config_commit_fence_databases(
    connection: &Connection,
    locator_path: &Path,
    parent_authority_path: &Path,
    child_authority_path: &Path,
) -> Result<(), SeedConfigLifecycleError> {
    attach_seed_config_fence_databases(connection, locator_path, parent_authority_path)?;
    let child_literal =
        sqlite_string_literal(child_authority_path).map_err(seed_error_from_rebirth)?;
    connection
        .execute_batch(&format!("ATTACH DATABASE {child_literal} AS seed_child;"))
        .map_err(|_| SeedConfigLifecycleError::FenceStale)
}

#[derive(Clone, Copy)]
enum AttachedSeedConfigAuthority {
    Parent,
    Child,
}

impl AttachedSeedConfigAuthority {
    const fn schema(self) -> &'static str {
        match self {
            Self::Parent => "seed_parent",
            Self::Child => "seed_child",
        }
    }
}

/// Reconstruct the full current authority from the already attached selected
/// generation. This intentionally does not call `locate_vault`: that helper
/// opens a second connection and would not be protected by this transaction's
/// authority lock.
fn attached_seed_config_current(
    transaction: &rusqlite::Transaction<'_>,
    attachment: AttachedSeedConfigAuthority,
    generation_id: &str,
) -> Result<SeedConfigCurrentV1, SeedConfigLifecycleError> {
    let schema = attachment.schema();
    let mut statement = transaction
        .prepare(&format!(
            "SELECT bot_token, persona_token, incarnation_id
             FROM {schema}.active_bindings
             ORDER BY bot_token ASC, persona_token ASC"
        ))
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let mut rows = statement
        .query([])
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let row = rows
        .next()
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?
        .ok_or(SeedConfigLifecycleError::FenceStale)?;
    let bot_token: [u8; 16] = row
        .get::<_, Vec<u8>>(0)
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?
        .try_into()
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let persona_token: [u8; 16] = row
        .get::<_, Vec<u8>>(1)
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?
        .try_into()
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let incarnation_id = digest_from_blob(
        row.get::<_, Vec<u8>>(2)
            .map_err(|_| SeedConfigLifecycleError::FenceStale)?,
    )
    .map_err(seed_error_from_rebirth)?;
    if rows
        .next()
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?
        .is_some()
    {
        return Err(SeedConfigLifecycleError::FenceStale);
    }
    drop(rows);
    drop(statement);

    let scope_token = wire::persona_scope_digest(&bot_token, &persona_token, None);
    let journal_revision: Option<i64> = transaction
        .query_row(
            &format!("SELECT MAX(logical_revision) FROM {schema}.journal WHERE scope_digest = ?1"),
            params![scope_token.to_vec()],
            |row| row.get(0),
        )
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let revision_sql = match journal_revision {
        None => 0,
        Some(value) if value > 0 => value,
        Some(_) => return Err(SeedConfigLifecycleError::FenceStale),
    };
    let revision = revision_from_sql(revision_sql).map_err(seed_error_from_rebirth)?;
    let snapshot_revision: Option<i64> = transaction
        .query_row(
            &format!("SELECT MAX(revision) FROM {schema}.snapshots WHERE scope_digest = ?1"),
            params![scope_token.to_vec()],
            |row| row.get(0),
        )
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    if snapshot_revision != Some(revision_sql) {
        return Err(SeedConfigLifecycleError::FenceStale);
    }
    let (graph_digest, seed_code_digest): (Digest, Digest) = transaction
        .query_row(
            &format!(
                "SELECT graph_digest, seed_code_digest
                 FROM {schema}.incarnations WHERE incarnation_id = ?1"
            ),
            params![incarnation_id.to_vec()],
            |row| {
                Ok((
                    digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error)?,
                    digest_from_blob(row.get(1)?).map_err(sqlite_conversion_error)?,
                ))
            },
        )
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let state_digest = transaction
        .query_row(
            &format!(
                "SELECT state_digest FROM {schema}.snapshots
                 WHERE scope_digest = ?1 AND revision = ?2"
            ),
            params![scope_token.to_vec(), revision_sql],
            |row| digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error),
        )
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    let history_digest = if journal_revision.is_none() {
        wire::domain_hash(
            b"astr-embodiment/continuity-empty-history-v1",
            &[&incarnation_id, &revision.to_le_bytes()],
        )
    } else {
        transaction
            .query_row(
                &format!(
                    "SELECT chain_digest FROM {schema}.journal
                     WHERE scope_digest = ?1 AND logical_revision = ?2"
                ),
                params![scope_token.to_vec(), revision_sql],
                |row| digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error),
            )
            .map_err(|_| SeedConfigLifecycleError::FenceStale)?
    };
    Ok(SeedConfigCurrentV1 {
        current: RebirthCurrentV1 {
            generation_id: generation_id.to_owned(),
            authority: ContinuityAuthority {
                incarnation_id,
                revision,
                state_digest,
                graph_digest,
                history_digest,
            },
        },
        scope_token,
        seed_code_digest,
    })
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

/// Dedicated seed-config lifecycle ledger.  It shares the durable lifecycle
/// SQLite file and atomic locator transaction, but never shares the manual
/// rebirth challenge/confirmation tables or their nonce domain.
fn initialize_seed_config_ledger_schema(
    transaction: &rusqlite::Transaction<'_>,
) -> Result<(), SeedConfigLifecycleError> {
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS seed_config_mirror_v1 (
                 mirror_id BLOB PRIMARY KEY CHECK (length(mirror_id) = 32),
                 scope_token BLOB NOT NULL CHECK (length(scope_token) = 32),
                 authority_generation_id TEXT NOT NULL,
                 authority_incarnation_id BLOB NOT NULL
                     CHECK (length(authority_incarnation_id) = 32),
                 authority_revision INTEGER NOT NULL,
                 native_seed_digest BLOB NOT NULL CHECK (length(native_seed_digest) = 32),
                 package_epoch_digest BLOB NOT NULL
                     CHECK (length(package_epoch_digest) = 32),
                 config_schema_version INTEGER NOT NULL,
                 guard_digest BLOB NOT NULL UNIQUE CHECK (length(guard_digest) = 32),
                 host_config_revision INTEGER NOT NULL,
                 status TEXT NOT NULL CHECK (
                     status IN ('PENDING_WRITEBACK', 'ACTIVE', 'CONSUMED')
                 ),
                 writeback_token_digest BLOB NOT NULL UNIQUE
                     CHECK (length(writeback_token_digest) = 32),
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE UNIQUE INDEX IF NOT EXISTS seed_config_active_scope_v1
                 ON seed_config_mirror_v1 (scope_token)
                 WHERE status IN ('PENDING_WRITEBACK', 'ACTIVE');
             CREATE TABLE IF NOT EXISTS seed_clear_intent_v1 (
                 intent_id BLOB PRIMARY KEY CHECK (length(intent_id) = 32),
                 scope_token BLOB NOT NULL CHECK (length(scope_token) = 32),
                 source_mirror_id BLOB NOT NULL CHECK (length(source_mirror_id) = 32),
                 source_guard_digest BLOB NOT NULL UNIQUE
                     CHECK (length(source_guard_digest) = 32),
                 expected_generation_id TEXT NOT NULL,
                 expected_incarnation_id BLOB NOT NULL
                     CHECK (length(expected_incarnation_id) = 32),
                 expected_revision INTEGER NOT NULL,
                 expected_seed_digest BLOB NOT NULL
                     CHECK (length(expected_seed_digest) = 32),
                 package_epoch_digest BLOB NOT NULL
                     CHECK (length(package_epoch_digest) = 32),
                 config_schema_version INTEGER NOT NULL,
                 host_config_revision INTEGER NOT NULL,
                 status TEXT NOT NULL CHECK (status IN ('STAGING', 'COMMITTED')),
                 stage_epoch INTEGER NOT NULL,
                 staged_child_generation_id TEXT,
                 receipt_id BLOB CHECK (receipt_id IS NULL OR length(receipt_id) = 32),
                 created_at_ms INTEGER NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS seed_clear_consumption_v1 (
                 guard_digest BLOB PRIMARY KEY CHECK (length(guard_digest) = 32),
                 intent_id BLOB NOT NULL UNIQUE CHECK (length(intent_id) = 32),
                 receipt_id BLOB NOT NULL UNIQUE CHECK (length(receipt_id) = 32),
                 scope_token BLOB NOT NULL CHECK (length(scope_token) = 32),
                 child_generation_id TEXT NOT NULL,
                 child_incarnation_id BLOB NOT NULL
                     CHECK (length(child_incarnation_id) = 32),
                 child_seed_digest BLOB NOT NULL CHECK (length(child_seed_digest) = 32),
                 before_revision INTEGER NOT NULL,
                 after_revision INTEGER NOT NULL CHECK (after_revision = 0),
                 committed_at_ms INTEGER NOT NULL
             );",
        )
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)
}

fn seed_error_from_rebirth(error: RebirthLifecycleError) -> SeedConfigLifecycleError {
    match error {
        RebirthLifecycleError::FenceStale => SeedConfigLifecycleError::FenceStale,
        RebirthLifecycleError::InFlight => SeedConfigLifecycleError::InFlight,
        RebirthLifecycleError::LocatorInvalid => SeedConfigLifecycleError::LocatorInvalid,
        RebirthLifecycleError::ChildInvalid => SeedConfigLifecycleError::FenceStale,
        RebirthLifecycleError::BootstrapConflict => SeedConfigLifecycleError::FenceStale,
        RebirthLifecycleError::ConfirmationRequired
        | RebirthLifecycleError::NonceConflict
        | RebirthLifecycleError::Durability
        | RebirthLifecycleError::InjectedFault(_) => SeedConfigLifecycleError::StorageFailed,
    }
}

fn validate_seed_config_request(
    request: &SeedConfigReconcileRequestV1,
) -> Result<(), SeedConfigLifecycleError> {
    if request.config_schema_version != 1
        || request.package_epoch.is_empty()
        || request.package_epoch.len() > 128
        || !request
            .package_epoch
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        || request
            .previous_observation
            .is_some_and(|observation| observation != SeedConfigObservationV1::PresentNonempty)
    {
        return Err(SeedConfigLifecycleError::SchemaInvalid);
    }
    if let Some(raw_guard) = request.mirror_guard.as_deref() {
        let _ = seed_config_guard_digest(raw_guard)?;
    }
    match request.observation {
        SeedConfigObservationV1::PresentNonempty => {
            let Some(seed_code) = request.seed_code.as_deref() else {
                return Err(SeedConfigLifecycleError::SchemaInvalid);
            };
            if seed_code.is_empty() || seed_code.len() > 256 {
                return Err(SeedConfigLifecycleError::SchemaInvalid);
            }
        }
        SeedConfigObservationV1::PresentEmpty
        | SeedConfigObservationV1::Missing
        | SeedConfigObservationV1::ReadFailed => {
            if request.seed_code.is_some() {
                return Err(SeedConfigLifecycleError::SchemaInvalid);
            }
        }
    }
    Ok(())
}

fn finish_seed_preflight(
    transaction: rusqlite::Transaction<'_>,
    result: SeedConfigReconcileResultV1,
) -> Result<SeedConfigPreflightV1, SeedConfigLifecycleError> {
    transaction
        .commit()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    Ok(SeedConfigPreflightV1::Result(result))
}

fn seed_config_result(
    state: SeedConfigStateV1,
    writeback: Option<SeedConfigWritebackV1>,
    before_revision: Option<u64>,
    after_revision: Option<u64>,
    reason: &'static str,
) -> SeedConfigReconcileResultV1 {
    SeedConfigReconcileResultV1 {
        state,
        writeback,
        before_revision,
        after_revision,
        reason,
    }
}

fn seed_config_package_epoch_digest(package_epoch: &str) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/seed-config-package-epoch-v1",
        &[package_epoch.as_bytes()],
    )
}

fn decode_seed_config_capability(value: &str) -> Result<Digest, SeedConfigLifecycleError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(SeedConfigLifecycleError::SchemaInvalid);
    }
    hex::decode32(value).map_err(|_| SeedConfigLifecycleError::SchemaInvalid)
}

fn seed_config_guard_digest(value: &str) -> Result<Digest, SeedConfigLifecycleError> {
    let raw = decode_seed_config_capability(value)?;
    Ok(wire::domain_hash(
        b"astr-embodiment/seed-config-mirror-guard-v1",
        &[&raw],
    ))
}

fn seed_config_writeback_token_digest(value: &str) -> Result<Digest, SeedConfigLifecycleError> {
    let raw = decode_seed_config_capability(value)?;
    Ok(wire::domain_hash(
        b"astr-embodiment/seed-config-writeback-token-v1",
        &[&raw],
    ))
}

/// Obtain independent CSPRNG material. The raw bytes live only in the stack
/// until the immediate writeback is encoded; the ledger receives only a
/// domain-separated digest.
fn fresh_seed_config_material() -> Result<Digest, SeedConfigLifecycleError> {
    let mut material = [0_u8; 32];
    getrandom::fill(&mut material).map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    Ok(material)
}

fn fresh_seed_config_capability() -> Result<String, SeedConfigLifecycleError> {
    Ok(hex::encode32(&fresh_seed_config_material()?))
}

fn seed_config_mirror_matches_current(
    mirror: &StoredSeedConfigMirrorV1,
    current: &SeedConfigCurrentV1,
    package_epoch_digest: Digest,
    config_schema_version: u16,
) -> bool {
    mirror.scope_token == current.scope_token
        && mirror.authority_generation_id == current.current.generation_id
        && mirror.authority_incarnation_id == current.current.authority.incarnation_id
        && mirror.authority_revision == current.current.authority.revision
        && mirror.native_seed_digest == current.seed_code_digest
        && mirror.package_epoch_digest == package_epoch_digest
        && mirror.config_schema_version == config_schema_version
}

fn seed_config_mirror_guard_matches(
    mirror: &StoredSeedConfigMirrorV1,
    raw_guard: &str,
) -> Result<bool, SeedConfigLifecycleError> {
    Ok(seed_config_guard_digest(raw_guard)? == mirror.guard_digest)
}

fn seed_config_empty_authorized(
    request: &SeedConfigReconcileRequestV1,
    mirror: &StoredSeedConfigMirrorV1,
) -> bool {
    match request.origin {
        SeedConfigOriginV1::StartupRead => true,
        SeedConfigOriginV1::UserSaveEvent => {
            request.previous_observation == Some(SeedConfigObservationV1::PresentNonempty)
                && request.host_config_revision != 0
                && request.host_config_revision > mirror.host_config_revision
        }
        SeedConfigOriginV1::PluginWriteback | SeedConfigOriginV1::LegacyConfigMigration => false,
    }
}

type SeedConfigMirrorRow = (
    Vec<u8>,
    Vec<u8>,
    String,
    Vec<u8>,
    i64,
    Vec<u8>,
    Vec<u8>,
    i64,
    Vec<u8>,
    i64,
    String,
    Vec<u8>,
);

fn decode_seed_config_mirror(
    row: SeedConfigMirrorRow,
) -> Result<StoredSeedConfigMirrorV1, SeedConfigLifecycleError> {
    let (
        mirror_id,
        scope_token,
        authority_generation_id,
        authority_incarnation_id,
        authority_revision,
        native_seed_digest,
        package_epoch_digest,
        config_schema_version,
        guard_digest,
        host_config_revision,
        status,
        writeback_token_digest,
    ) = row;
    let config_schema_version = u16::try_from(config_schema_version)
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    Ok(StoredSeedConfigMirrorV1 {
        mirror_id: digest_from_blob(mirror_id).map_err(seed_error_from_rebirth)?,
        scope_token: digest_from_blob(scope_token).map_err(seed_error_from_rebirth)?,
        authority_generation_id,
        authority_incarnation_id: digest_from_blob(authority_incarnation_id)
            .map_err(seed_error_from_rebirth)?,
        authority_revision: revision_from_sql(authority_revision)
            .map_err(seed_error_from_rebirth)?,
        native_seed_digest: digest_from_blob(native_seed_digest)
            .map_err(seed_error_from_rebirth)?,
        package_epoch_digest: digest_from_blob(package_epoch_digest)
            .map_err(seed_error_from_rebirth)?,
        config_schema_version,
        guard_digest: digest_from_blob(guard_digest).map_err(seed_error_from_rebirth)?,
        host_config_revision: revision_from_sql(host_config_revision)
            .map_err(seed_error_from_rebirth)?,
        status,
        writeback_token_digest: digest_from_blob(writeback_token_digest)
            .map_err(seed_error_from_rebirth)?,
    })
}

fn mirror_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SeedConfigMirrorRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
    ))
}

fn load_active_seed_config_mirror(
    connection: &Connection,
    scope_token: Digest,
) -> Result<Option<StoredSeedConfigMirrorV1>, SeedConfigLifecycleError> {
    let mut statement = connection
        .prepare(
            "SELECT mirror_id, scope_token, authority_generation_id,
                    authority_incarnation_id, authority_revision, native_seed_digest,
                    package_epoch_digest, config_schema_version, guard_digest,
                    host_config_revision, status, writeback_token_digest
             FROM seed_config_mirror_v1
             WHERE scope_token = ?1 AND status IN ('PENDING_WRITEBACK', 'ACTIVE')",
        )
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    let mut rows = statement
        .query(params![scope_token.to_vec()])
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    let first = rows
        .next()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?
        .map(mirror_row_from_row)
        .transpose()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    if rows
        .next()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?
        .is_some()
    {
        return Err(SeedConfigLifecycleError::StorageFailed);
    }
    first.map(decode_seed_config_mirror).transpose()
}

fn load_seed_config_mirror_by_writeback_token(
    connection: &Connection,
    token_digest: Digest,
) -> Result<Option<StoredSeedConfigMirrorV1>, SeedConfigLifecycleError> {
    connection
        .query_row(
            "SELECT mirror_id, scope_token, authority_generation_id,
                    authority_incarnation_id, authority_revision, native_seed_digest,
                    package_epoch_digest, config_schema_version, guard_digest,
                    host_config_revision, status, writeback_token_digest
             FROM seed_config_mirror_v1 WHERE writeback_token_digest = ?1",
            params![token_digest.to_vec()],
            mirror_row_from_row,
        )
        .optional()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?
        .map(decode_seed_config_mirror)
        .transpose()
}

fn create_seed_config_mirror(
    transaction: &rusqlite::Transaction<'_>,
    current: &SeedConfigCurrentV1,
    package_epoch_digest: Digest,
    config_schema_version: u16,
    host_config_revision: u64,
) -> Result<(StoredSeedConfigMirrorV1, SeedConfigWritebackV1), SeedConfigLifecycleError> {
    transaction
        .execute(
            "UPDATE seed_config_mirror_v1 SET status = 'CONSUMED', updated_at_ms = ?2
             WHERE scope_token = ?1 AND status IN ('PENDING_WRITEBACK', 'ACTIVE')",
            params![
                current.scope_token.to_vec(),
                revision_to_sql(crate::now_ms()).map_err(seed_error_from_rebirth)?,
            ],
        )
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    let mirror_id = fresh_seed_config_material()?;
    let mirror_guard = fresh_seed_config_capability()?;
    let writeback_token = fresh_seed_config_capability()?;
    let guard_digest = seed_config_guard_digest(&mirror_guard)?;
    let writeback_token_digest = seed_config_writeback_token_digest(&writeback_token)?;
    let now = crate::now_ms();
    transaction
        .execute(
            "INSERT INTO seed_config_mirror_v1 (
                 mirror_id, scope_token, authority_generation_id,
                 authority_incarnation_id, authority_revision, native_seed_digest,
                 package_epoch_digest, config_schema_version, guard_digest,
                 host_config_revision, status, writeback_token_digest,
                 created_at_ms, updated_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                       'PENDING_WRITEBACK', ?11, ?12, ?12)",
            params![
                mirror_id.to_vec(),
                current.scope_token.to_vec(),
                current.current.generation_id,
                current.current.authority.incarnation_id.to_vec(),
                revision_to_sql(current.current.authority.revision)
                    .map_err(seed_error_from_rebirth)?,
                current.seed_code_digest.to_vec(),
                package_epoch_digest.to_vec(),
                i64::from(config_schema_version),
                guard_digest.to_vec(),
                revision_to_sql(host_config_revision).map_err(seed_error_from_rebirth)?,
                writeback_token_digest.to_vec(),
                revision_to_sql(now).map_err(seed_error_from_rebirth)?,
            ],
        )
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
    let mirror = StoredSeedConfigMirrorV1 {
        mirror_id,
        scope_token: current.scope_token,
        authority_generation_id: current.current.generation_id.clone(),
        authority_incarnation_id: current.current.authority.incarnation_id,
        authority_revision: current.current.authority.revision,
        native_seed_digest: current.seed_code_digest,
        package_epoch_digest,
        config_schema_version,
        guard_digest,
        host_config_revision,
        status: "PENDING_WRITEBACK".to_owned(),
        writeback_token_digest,
    };
    Ok((
        mirror,
        SeedConfigWritebackV1 {
            seed_code: ae_genesis::format_seed_code(&current.seed_code_digest),
            mirror_guard,
            writeback_token,
        },
    ))
}

type SeedClearIntentRow = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    String,
    Vec<u8>,
    i64,
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
    String,
    i64,
    i64,
);

fn intent_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SeedClearIntentRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
    ))
}

fn decode_seed_clear_intent(
    row: SeedClearIntentRow,
) -> Result<StoredSeedClearIntentV1, SeedConfigLifecycleError> {
    let (
        intent_id,
        scope_token,
        source_mirror_id,
        source_guard_digest,
        expected_generation_id,
        expected_incarnation_id,
        expected_revision,
        expected_seed_digest,
        package_epoch_digest,
        config_schema_version,
        host_config_revision,
        status,
        stage_epoch,
        created_at_ms,
    ) = row;
    Ok(StoredSeedClearIntentV1 {
        intent_id: digest_from_blob(intent_id).map_err(seed_error_from_rebirth)?,
        scope_token: digest_from_blob(scope_token).map_err(seed_error_from_rebirth)?,
        source_mirror_id: digest_from_blob(source_mirror_id).map_err(seed_error_from_rebirth)?,
        source_guard_digest: digest_from_blob(source_guard_digest)
            .map_err(seed_error_from_rebirth)?,
        expected_generation_id,
        expected_incarnation_id: digest_from_blob(expected_incarnation_id)
            .map_err(seed_error_from_rebirth)?,
        expected_revision: revision_from_sql(expected_revision).map_err(seed_error_from_rebirth)?,
        expected_seed_digest: digest_from_blob(expected_seed_digest)
            .map_err(seed_error_from_rebirth)?,
        package_epoch_digest: digest_from_blob(package_epoch_digest)
            .map_err(seed_error_from_rebirth)?,
        config_schema_version: u16::try_from(config_schema_version)
            .map_err(|_| SeedConfigLifecycleError::StorageFailed)?,
        host_config_revision: revision_from_sql(host_config_revision)
            .map_err(seed_error_from_rebirth)?,
        status,
        stage_epoch: revision_from_sql(stage_epoch).map_err(seed_error_from_rebirth)?,
        created_at_ms: revision_from_sql(created_at_ms).map_err(seed_error_from_rebirth)?,
    })
}

fn load_seed_clear_intent(
    connection: &Connection,
    intent_id: Digest,
) -> Result<Option<StoredSeedClearIntentV1>, SeedConfigLifecycleError> {
    connection
        .query_row(
            "SELECT intent_id, scope_token, source_mirror_id, source_guard_digest,
                    expected_generation_id, expected_incarnation_id, expected_revision,
                    expected_seed_digest, package_epoch_digest, config_schema_version,
                    host_config_revision, status, stage_epoch, created_at_ms
             FROM seed_clear_intent_v1 WHERE intent_id = ?1",
            params![intent_id.to_vec()],
            intent_row_from_row,
        )
        .optional()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?
        .map(decode_seed_clear_intent)
        .transpose()
}

type SeedClearConsumptionRow = (
    Vec<u8>,
    Vec<u8>,
    Vec<u8>,
    String,
    Vec<u8>,
    Vec<u8>,
    i64,
    i64,
);

fn consumption_row_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SeedClearConsumptionRow> {
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
}

fn decode_seed_clear_consumption(
    row: SeedClearConsumptionRow,
) -> Result<StoredSeedClearConsumptionV1, SeedConfigLifecycleError> {
    let (
        guard_digest,
        intent_id,
        scope_token,
        child_generation_id,
        child_incarnation_id,
        child_seed_digest,
        before_revision,
        after_revision,
    ) = row;
    Ok(StoredSeedClearConsumptionV1 {
        guard_digest: digest_from_blob(guard_digest).map_err(seed_error_from_rebirth)?,
        intent_id: digest_from_blob(intent_id).map_err(seed_error_from_rebirth)?,
        scope_token: digest_from_blob(scope_token).map_err(seed_error_from_rebirth)?,
        child_generation_id,
        child_incarnation_id: digest_from_blob(child_incarnation_id)
            .map_err(seed_error_from_rebirth)?,
        child_seed_digest: digest_from_blob(child_seed_digest).map_err(seed_error_from_rebirth)?,
        before_revision: revision_from_sql(before_revision).map_err(seed_error_from_rebirth)?,
        after_revision: revision_from_sql(after_revision).map_err(seed_error_from_rebirth)?,
    })
}

fn load_seed_clear_consumption(
    connection: &Connection,
    guard_digest: Digest,
) -> Result<Option<StoredSeedClearConsumptionV1>, SeedConfigLifecycleError> {
    connection
        .query_row(
            "SELECT guard_digest, intent_id, scope_token, child_generation_id,
                    child_incarnation_id, child_seed_digest, before_revision, after_revision
             FROM seed_clear_consumption_v1 WHERE guard_digest = ?1",
            params![guard_digest.to_vec()],
            consumption_row_from_row,
        )
        .optional()
        .map_err(|_| SeedConfigLifecycleError::StorageFailed)?
        .map(decode_seed_clear_consumption)
        .transpose()
}

fn seed_clear_intent_id(
    current: &SeedConfigCurrentV1,
    guard_digest: Digest,
    package_epoch_digest: Digest,
    config_schema_version: u16,
    host_config_revision: u64,
) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/seed-config-clear-intent-v1",
        &[
            &current.scope_token,
            &guard_digest,
            current.current.generation_id.as_bytes(),
            &current.current.authority.incarnation_id,
            &current.current.authority.revision.to_le_bytes(),
            &current.seed_code_digest,
            &package_epoch_digest,
            &config_schema_version.to_le_bytes(),
            &host_config_revision.to_le_bytes(),
        ],
    )
}

fn begin_seed_clear_intent(
    transaction: &rusqlite::Transaction<'_>,
    current: &SeedConfigCurrentV1,
    mirror: &StoredSeedConfigMirrorV1,
    source_guard_digest: Digest,
    package_epoch_digest: Digest,
    config_schema_version: u16,
    host_config_revision: u64,
) -> Result<SeedClearCommitPermitV1, SeedConfigLifecycleError> {
    if mirror.guard_digest != source_guard_digest
        || !seed_config_mirror_matches_current(
            mirror,
            current,
            package_epoch_digest,
            config_schema_version,
        )
    {
        return Err(SeedConfigLifecycleError::MirrorStale);
    }
    let intent_id = seed_clear_intent_id(
        current,
        source_guard_digest,
        package_epoch_digest,
        config_schema_version,
        host_config_revision,
    );
    let now = crate::now_ms();
    let existing = load_seed_clear_intent(transaction, intent_id)?;
    let intent = match existing {
        Some(intent) => intent,
        None => {
            transaction
                .execute(
                    "INSERT INTO seed_clear_intent_v1 (
                         intent_id, scope_token, source_mirror_id, source_guard_digest,
                         expected_generation_id, expected_incarnation_id, expected_revision,
                         expected_seed_digest, package_epoch_digest, config_schema_version,
                         host_config_revision, status, stage_epoch, staged_child_generation_id,
                         receipt_id, created_at_ms, updated_at_ms
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                               'STAGING', 1, NULL, NULL, ?12, ?12)",
                    params![
                        intent_id.to_vec(),
                        current.scope_token.to_vec(),
                        mirror.mirror_id.to_vec(),
                        source_guard_digest.to_vec(),
                        current.current.generation_id,
                        current.current.authority.incarnation_id.to_vec(),
                        revision_to_sql(current.current.authority.revision)
                            .map_err(seed_error_from_rebirth)?,
                        current.seed_code_digest.to_vec(),
                        package_epoch_digest.to_vec(),
                        i64::from(config_schema_version),
                        revision_to_sql(host_config_revision).map_err(seed_error_from_rebirth)?,
                        revision_to_sql(now).map_err(seed_error_from_rebirth)?,
                    ],
                )
                .map_err(|_| SeedConfigLifecycleError::StorageFailed)?;
            load_seed_clear_intent(transaction, intent_id)?
                .ok_or(SeedConfigLifecycleError::StorageFailed)?
        }
    };
    if intent.status != "STAGING"
        || intent.stage_epoch == 0
        || intent.scope_token != current.scope_token
        || intent.source_mirror_id != mirror.mirror_id
        || intent.source_guard_digest != source_guard_digest
        || intent.expected_generation_id != current.current.generation_id
        || intent.expected_incarnation_id != current.current.authority.incarnation_id
        || intent.expected_revision != current.current.authority.revision
        || intent.expected_seed_digest != current.seed_code_digest
        || intent.package_epoch_digest != package_epoch_digest
        || intent.config_schema_version != config_schema_version
        || intent.host_config_revision != host_config_revision
    {
        return Err(SeedConfigLifecycleError::FenceStale);
    }
    Ok(SeedClearCommitPermitV1 {
        scope_token: current.scope_token,
        intent_id,
        source_guard_digest,
        parent_generation_id: current.current.generation_id.clone(),
        parent_authority: current.current.authority.clone(),
        parent_seed_code_digest: current.seed_code_digest,
        package_epoch_digest,
        config_schema_version,
        host_config_revision,
        created_at_ms: intent.created_at_ms,
        stage_epoch: intent.stage_epoch,
    })
}

fn seed_clear_intent_matches_permit(
    intent: &StoredSeedClearIntentV1,
    permit: &SeedClearCommitPermitV1,
) -> bool {
    intent.intent_id == permit.intent_id
        && intent.scope_token == permit.scope_token
        && intent.source_guard_digest == permit.source_guard_digest
        && intent.expected_generation_id == permit.parent_generation_id
        && intent.expected_incarnation_id == permit.parent_authority.incarnation_id
        && intent.expected_revision == permit.parent_authority.revision
        && intent.expected_seed_digest == permit.parent_seed_code_digest
        && intent.package_epoch_digest == permit.package_epoch_digest
        && intent.config_schema_version == permit.config_schema_version
        && intent.host_config_revision == permit.host_config_revision
}

fn seed_clear_consumption_matches_permit(
    consumption: &StoredSeedClearConsumptionV1,
    permit: &SeedClearCommitPermitV1,
) -> bool {
    consumption.guard_digest == permit.source_guard_digest
        && consumption.intent_id == permit.intent_id
        && consumption.scope_token == permit.scope_token
}

fn replay_seed_clear_result(
    transaction: &rusqlite::Transaction<'_>,
    current: &SeedConfigCurrentV1,
    request: &SeedConfigReconcileRequestV1,
    package_epoch_digest: Digest,
    consumption: &StoredSeedClearConsumptionV1,
) -> Result<SeedConfigReconcileResultV1, SeedConfigLifecycleError> {
    let guard = request
        .mirror_guard
        .as_deref()
        .ok_or(SeedConfigLifecycleError::SchemaInvalid)?;
    if seed_config_guard_digest(guard)? != consumption.guard_digest {
        return Err(SeedConfigLifecycleError::StorageFailed);
    }
    replay_seed_clear_result_from_consumption(
        transaction,
        current,
        package_epoch_digest,
        request.config_schema_version,
        consumption,
    )
}

fn replay_seed_clear_result_from_consumption(
    transaction: &rusqlite::Transaction<'_>,
    current: &SeedConfigCurrentV1,
    package_epoch_digest: Digest,
    config_schema_version: u16,
    consumption: &StoredSeedClearConsumptionV1,
) -> Result<SeedConfigReconcileResultV1, SeedConfigLifecycleError> {
    if consumption.scope_token != current.scope_token
        || consumption.child_generation_id != current.current.generation_id
        || consumption.child_incarnation_id != current.current.authority.incarnation_id
        || consumption.child_seed_digest != current.seed_code_digest
        || consumption.after_revision != 0
    {
        return Ok(seed_config_result(
            SeedConfigStateV1::Deferred,
            None,
            None,
            None,
            "SEED_CONFIG_OBSERVATION_DEFERRED",
        ));
    }
    // The durable ledger intentionally retains only capability digests.
    // Replaying after a crash rotates any old pending/active mirror to a new
    // repair capability instead of regenerating a raw guard or token from
    // persisted material.
    let (_, writeback) = create_seed_config_mirror(
        transaction,
        current,
        package_epoch_digest,
        config_schema_version,
        0,
    )?;
    Ok(seed_config_result(
        SeedConfigStateV1::RebirthReplayed,
        Some(writeback),
        Some(consumption.before_revision),
        Some(consumption.after_revision),
        "SEED_CLEAR_REBIRTH_REPLAYED",
    ))
}

fn seed_clear_receipt_id(
    permit: &SeedClearCommitPermitV1,
    child: &SeedClearStagedChildV1,
) -> Digest {
    wire::domain_hash(
        b"astr-embodiment/seed-config-clear-receipt-v1",
        &[
            &permit.intent_id,
            &permit.source_guard_digest,
            &permit.parent_authority.incarnation_id,
            &child.child_authority.incarnation_id,
            &child.child_seed_code_digest,
            &child.child_authority.state_digest,
            &child.child_authority.graph_digest,
        ],
    )
}

fn validate_seed_clear_staged_child_shape(
    permit: &SeedClearCommitPermitV1,
    child: &SeedClearStagedChildV1,
) -> Result<(), SeedConfigLifecycleError> {
    if child.scope_token != permit.scope_token
        || child.intent_id != permit.intent_id
        || child.source_guard_digest != permit.source_guard_digest
        || child.parent_generation_id != permit.parent_generation_id
        || child.parent_authority != permit.parent_authority
        || child.parent_seed_code_digest != permit.parent_seed_code_digest
        || child.child_authority.revision != 0
        || child.child_authority.incarnation_id == permit.parent_authority.incarnation_id
        || child.child_seed_code_digest == permit.parent_seed_code_digest
        || child.child_generation_id
            != VaultLifecycle::seed_clear_child_generation_id_for(
                &child.child_authority.incarnation_id,
            )
    {
        return Err(SeedConfigLifecycleError::FenceStale);
    }
    Ok(())
}

/// Re-read the complete staged child fence through the child attachment held
/// by the same `BEGIN IMMEDIATE` transaction that will CAS the locator.
fn validate_attached_seed_clear_child(
    transaction: &rusqlite::Transaction<'_>,
    permit: &SeedClearCommitPermitV1,
    child: &SeedClearStagedChildV1,
) -> Result<(), SeedConfigLifecycleError> {
    validate_seed_clear_staged_child_shape(permit, child)?;
    let actual = attached_seed_config_current(
        transaction,
        AttachedSeedConfigAuthority::Child,
        &child.child_generation_id,
    )?;
    if actual.scope_token != permit.scope_token
        || actual.current.authority != child.child_authority
        || actual.seed_code_digest != child.child_seed_code_digest
        || actual.current.authority.revision != 0
        || actual.current.authority.incarnation_id == permit.parent_authority.incarnation_id
    {
        return Err(SeedConfigLifecycleError::FenceStale);
    }
    let stored_parent: Option<Vec<u8>> = transaction
        .query_row(
            "SELECT parent_incarnation_id FROM seed_child.incarnations
             WHERE incarnation_id = ?1",
            params![child.child_authority.incarnation_id.to_vec()],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| SeedConfigLifecycleError::FenceStale)?;
    if stored_parent.as_deref() != Some(permit.parent_authority.incarnation_id.as_slice()) {
        return Err(SeedConfigLifecycleError::FenceStale);
    }
    Ok(())
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

type StoredChallengeRow = (Vec<u8>, Vec<u8>, Vec<u8>, i64, String, String, i64);
type StoredReceiptRow = (Vec<u8>, String, Vec<u8>, Vec<u8>, Vec<u8>, i64, i64, i64);

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
    let row: Option<StoredChallengeRow> = connection
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
    let row: Option<StoredReceiptRow> = connection
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
    let (bot, persona, incarnation_id, _) = read_single_binding(database)?;
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let scope = wire::persona_scope_digest(&bot, &persona, None);
    // `active_bindings.revision` is a binding-row epoch: `Store::commit_genesis`
    // writes it as 1 while the only initial snapshot is revision 0.  The
    // authority fence is instead the continuity lane's journal/snapshot
    // revision, exactly as Store::current_revision reports it.  This avoids
    // fabricating a non-existent revision-1 state during legacy bootstrap.
    let journal_revision: Option<i64> = connection
        .query_row(
            "SELECT MAX(logical_revision) FROM journal WHERE scope_digest = ?1",
            params![scope.to_vec()],
            |row| row.get(0),
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    let revision_sql = match journal_revision {
        None => 0,
        Some(value) if value > 0 => value,
        Some(_) => return Err(RebirthLifecycleError::ChildInvalid),
    };
    let revision = revision_from_sql(revision_sql)?;
    let snapshot_revision: Option<i64> = connection
        .query_row(
            "SELECT MAX(revision) FROM snapshots WHERE scope_digest = ?1",
            params![scope.to_vec()],
            |row| row.get(0),
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    if snapshot_revision != Some(revision_sql) {
        return Err(RebirthLifecycleError::ChildInvalid);
    }
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
    let history_digest = if journal_revision.is_none() {
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

fn read_seed_code_digest(
    database: &Path,
    incarnation_id: &Digest,
) -> Result<Digest, RebirthLifecycleError> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| RebirthLifecycleError::ChildInvalid)?;
    connection
        .query_row(
            "SELECT seed_code_digest FROM incarnations WHERE incarnation_id = ?1",
            params![incarnation_id.to_vec()],
            |row| digest_from_blob(row.get(0)?).map_err(sqlite_conversion_error),
        )
        .map_err(|_| RebirthLifecycleError::ChildInvalid)
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
    {
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
    }
    // `FlushFileBuffers` (which backs `File::sync_all` on Windows) requires a
    // writable handle.  The SQLite validation connection is closed above, so
    // reopen the database read/write solely to make the file-data flush real.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|_| RebirthLifecycleError::Durability)?;
    Ok(())
}

#[cfg(unix)]
fn sync_generation_directory(path: &Path) -> Result<(), RebirthLifecycleError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| RebirthLifecycleError::Durability)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn durable_rename_generation(
    temporary: &Path,
    destination: &Path,
) -> Result<(), RebirthLifecycleError> {
    let temporary: Vec<u16> = temporary.as_os_str().encode_wide().chain(once(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(once(0))
        .collect();
    // MoveFileExW with WRITE_THROUGH is the Windows durable counterpart to
    // the Unix rename-plus-parent-directory-fsync sequence below.
    // SAFETY: both UTF-16 vectors are NUL-terminated and remain alive for the
    // duration of this synchronous Win32 call.
    let moved = unsafe {
        MoveFileExW(
            temporary.as_ptr() as PCWSTR,
            destination.as_ptr() as PCWSTR,
            MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(RebirthLifecycleError::Durability);
    }
    Ok(())
}

#[cfg(not(windows))]
fn durable_rename_generation(
    temporary: &Path,
    destination: &Path,
) -> Result<(), RebirthLifecycleError> {
    fs::rename(temporary, destination).map_err(|_| RebirthLifecycleError::Durability)
}

/// Close the complete staging durability loop before returning a child as a
/// commit candidate: SQLite integrity/file flush, temporary directory flush,
/// atomic rename, and finally flushes of both the generations parent and the
/// installed directory. A race that installed the same deterministic child is
/// reported separately so its existing complete generation can be validated.
fn durably_install_staged_generation(
    temporary: &Path,
    destination: &Path,
    temporary_database: &Path,
) -> Result<bool, RebirthLifecycleError> {
    sync_sqlite_database(temporary_database)?;
    #[cfg(unix)]
    sync_generation_directory(temporary)?;
    match durable_rename_generation(temporary, destination) {
        Ok(()) => {
            #[cfg(unix)]
            {
                let generations = destination
                    .parent()
                    .ok_or(RebirthLifecycleError::Durability)?;
                sync_generation_directory(generations)?;
                sync_generation_directory(destination)?;
            }
            sync_sqlite_database(&destination.join(AUTHORITY_DATABASE))?;
            Ok(true)
        }
        Err(_) if destination.exists() => Ok(false),
        Err(_) => Err(RebirthLifecycleError::Durability),
    }
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
