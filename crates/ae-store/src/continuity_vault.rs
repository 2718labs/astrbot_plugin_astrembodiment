use ae_contracts::Digest;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

const OWNER_METADATA_MAX_BYTES: u64 = 4096;
const CURRENT_METADATA_MAX_BYTES: u64 = 4096;

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

    Ok(VaultLocation {
        root,
        generation_id: owner.generation_id,
        store_uuid: owner.store_uuid,
        mode: current.mode,
        incarnation_id: current.incarnation_id,
        revision: current.revision,
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
