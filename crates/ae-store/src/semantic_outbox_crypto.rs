//! Installation-scoped cryptographic authority for the semantic outbox.
//!
//! This is deliberately a narrow owner: it creates exactly one installation
//! key, keeps it native-only, and only seals or opens the semantic-outbox
//! envelope.  It is not a generic key service and it has no rotate/reset API.

use aes_gcm::aead::{AeadInPlace, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce, Tag};
use sha2::{Digest as Sha2Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;
use thiserror::Error;
use zeroize::{Zeroize, Zeroizing};

const AUTHORITY_DIRECTORY: &str = ".native-authority";
const AUTHORITY_RECORD: &str = "semantic-outbox-key.v1";
const RECORD_MAGIC: &[u8; 8] = b"AE-SOK1\0";
const RECORD_SCHEMA: u16 = 1;
const ENVELOPE_MAGIC: &[u8; 8] = b"AE-SOB1\0";
const ENVELOPE_SCHEMA: u16 = 1;
const PROTECTION_WINDOWS_DPAPI: u8 = 1;
#[cfg(target_os = "linux")]
const PROTECTION_LINUX_FILE: u8 = 2;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const CHECKSUM_LEN: usize = 32;
const MAX_RECORD_PAYLOAD_LEN: usize = 1 << 20;
const OUTBOX_AAD_DOMAIN: &[u8] = b"AE-SEMANTIC-OUTBOX-V1\0";

/// Maximum caller-supplied AAD accepted by the narrow async-crypto ABI.
pub const SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1: usize = 4_096;
/// Maximum plaintext accepted by the narrow async-crypto ABI.
pub const SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1: usize = 262_144;
/// Fixed bytes carried by a v1 envelope besides ciphertext.
pub const SEMANTIC_OUTBOX_ENVELOPE_OVERHEAD_BYTES_V1: usize =
    ENVELOPE_MAGIC.len() + 2 + 4 + NONCE_LEN + 4 + TAG_LEN;
/// Maximum binary envelope accepted by the narrow async-crypto ABI.
pub const SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1: usize =
    SEMANTIC_OUTBOX_ENVELOPE_OVERHEAD_BYTES_V1 + SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1;

/// The one and only supported installation-key version.
pub const SEMANTIC_OUTBOX_KEY_VERSION_V1: u32 = 1;

/// Closed availability state returned by the runtime/PyO3 adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticOutboxCryptoStatusValueV1 {
    Ready,
    Unavailable,
    KeyVersionUnsupported,
}

/// Content-free authority status.  The key version is deliberately fixed and
/// never includes a path, key reference, protection blob, or algorithm detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SemanticOutboxCryptoStatusV1 {
    pub status: SemanticOutboxCryptoStatusValueV1,
    pub key_version: u32,
}

/// The three externally meaningful failure classes.  The native handle never
/// serializes an underlying filesystem, DPAPI, or AEAD error.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum SemanticOutboxCryptoError {
    #[error("async key unavailable")]
    Unavailable,
    #[error("async payload authentication failed")]
    PayloadAuthFailed,
    #[error("async key version unsupported")]
    KeyVersionUnsupported,
}

/// Opaque native-only installation-key handle.  Its fields are private so no
/// Rust caller outside this module, and no PyO3 caller, can obtain key bytes.
pub struct SemanticOutboxKeyAuthorityV1 {
    key: Zeroizing<[u8; 32]>,
}

impl SemanticOutboxKeyAuthorityV1 {
    /// Open the single authority below a runtime data directory, creating it
    /// only when the entire authority directory is absent.  A pre-existing
    /// malformed/missing record is never repaired or regenerated.
    pub fn open(storage_parent: &Path) -> Result<Self, SemanticOutboxCryptoError> {
        let key = platform::open_installation_key(storage_parent)?;
        Ok(Self { key })
    }

    pub const fn ready_status_v1(&self) -> SemanticOutboxCryptoStatusV1 {
        SemanticOutboxCryptoStatusV1 {
            status: SemanticOutboxCryptoStatusValueV1::Ready,
            key_version: SEMANTIC_OUTBOX_KEY_VERSION_V1,
        }
    }

    /// Seal one opaque payload under an AAD that is fully caller-owned but
    /// bound to the fixed protocol/version/key-version prefix.
    pub fn seal_v1(
        &self,
        key_version: u32,
        caller_aad: &[u8],
        plaintext: &[u8],
    ) -> Result<Vec<u8>, SemanticOutboxCryptoError> {
        if key_version != SEMANTIC_OUTBOX_KEY_VERSION_V1 {
            return Err(SemanticOutboxCryptoError::KeyVersionUnsupported);
        }
        if caller_aad.len() > SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1
            || plaintext.len() > SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1
        {
            return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
        }
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let aad = authenticated_data(key_version, caller_aad)?;
        let cipher = Aes256Gcm::new_from_slice(&self.key[..])
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let mut ciphertext = plaintext.to_vec();
        let tag = cipher
            .encrypt_in_place_detached(Nonce::from_slice(&nonce), &aad, &mut ciphertext)
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let envelope = encode_envelope(key_version, &nonce, &ciphertext, &tag)?;
        ciphertext.zeroize();
        if envelope.len() > SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1 {
            return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
        }
        Ok(envelope)
    }

    /// Authenticate and open one exact binary envelope.  Parsing, AAD, nonce,
    /// ciphertext and tag failures all collapse to the same fail-closed result.
    pub fn open_v1(
        &self,
        requested_key_version: u32,
        caller_aad: &[u8],
        envelope: &[u8],
    ) -> Result<Vec<u8>, SemanticOutboxCryptoError> {
        if requested_key_version != SEMANTIC_OUTBOX_KEY_VERSION_V1 {
            return Err(SemanticOutboxCryptoError::KeyVersionUnsupported);
        }
        if caller_aad.len() > SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1
            || envelope.len() > SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1
        {
            return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
        }
        let parsed = parse_envelope(envelope)?;
        if parsed.key_version != requested_key_version {
            return Err(SemanticOutboxCryptoError::KeyVersionUnsupported);
        }
        let aad = authenticated_data(parsed.key_version, caller_aad)?;
        let cipher = Aes256Gcm::new_from_slice(&self.key[..])
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let mut plaintext = parsed.ciphertext;
        cipher
            .decrypt_in_place_detached(
                Nonce::from_slice(&parsed.nonce),
                &aad,
                &mut plaintext,
                Tag::from_slice(&parsed.tag),
            )
            .map_err(|_| {
                plaintext.zeroize();
                SemanticOutboxCryptoError::PayloadAuthFailed
            })?;
        Ok(plaintext)
    }
}

struct KeyRecordV1 {
    installation_id: [u8; 16],
    protection_kind: u8,
    payload: Zeroizing<Vec<u8>>,
}

struct EnvelopeV1 {
    key_version: u32,
    nonce: [u8; NONCE_LEN],
    ciphertext: Vec<u8>,
    tag: [u8; TAG_LEN],
}

fn authenticated_data(
    key_version: u32,
    caller_aad: &[u8],
) -> Result<Vec<u8>, SemanticOutboxCryptoError> {
    if caller_aad.len() > SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1 {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let aad_len = u32::try_from(caller_aad.len())
        .map_err(|_| SemanticOutboxCryptoError::PayloadAuthFailed)?;
    let mut aad = Vec::with_capacity(OUTBOX_AAD_DOMAIN.len() + 2 + 4 + 4 + caller_aad.len());
    aad.extend_from_slice(OUTBOX_AAD_DOMAIN);
    aad.extend_from_slice(&ENVELOPE_SCHEMA.to_le_bytes());
    aad.extend_from_slice(&key_version.to_le_bytes());
    aad.extend_from_slice(&aad_len.to_le_bytes());
    aad.extend_from_slice(caller_aad);
    Ok(aad)
}

fn encode_envelope(
    key_version: u32,
    nonce: &[u8; NONCE_LEN],
    ciphertext: &[u8],
    tag: &Tag,
) -> Result<Vec<u8>, SemanticOutboxCryptoError> {
    if ciphertext.len() > SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1 {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let ciphertext_len = u32::try_from(ciphertext.len())
        .map_err(|_| SemanticOutboxCryptoError::PayloadAuthFailed)?;
    let mut body = Vec::with_capacity(
        ENVELOPE_MAGIC.len() + 2 + 4 + NONCE_LEN + 4 + ciphertext.len() + TAG_LEN,
    );
    body.extend_from_slice(ENVELOPE_MAGIC);
    body.extend_from_slice(&ENVELOPE_SCHEMA.to_le_bytes());
    body.extend_from_slice(&key_version.to_le_bytes());
    body.extend_from_slice(nonce);
    body.extend_from_slice(&ciphertext_len.to_le_bytes());
    body.extend_from_slice(ciphertext);
    body.extend_from_slice(tag.as_slice());
    Ok(body)
}

fn parse_envelope(bytes: &[u8]) -> Result<EnvelopeV1, SemanticOutboxCryptoError> {
    const FIXED_PREFIX: usize = 8 + 2 + 4 + NONCE_LEN + 4;
    if bytes.len() < FIXED_PREFIX + TAG_LEN
        || bytes.len() > SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1
        || &bytes[..8] != ENVELOPE_MAGIC
    {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let schema = read_u16(bytes, 8)?;
    if schema != ENVELOPE_SCHEMA {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let key_version = read_u32(bytes, 10)?;
    if key_version != SEMANTIC_OUTBOX_KEY_VERSION_V1 {
        return Err(SemanticOutboxCryptoError::KeyVersionUnsupported);
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[14..14 + NONCE_LEN]);
    let ciphertext_len = usize::try_from(read_u32(bytes, 14 + NONCE_LEN)?)
        .map_err(|_| SemanticOutboxCryptoError::PayloadAuthFailed)?;
    if ciphertext_len > SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1 {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let ciphertext_start = FIXED_PREFIX;
    let ciphertext_end = ciphertext_start
        .checked_add(ciphertext_len)
        .ok_or(SemanticOutboxCryptoError::PayloadAuthFailed)?;
    let tag_end = ciphertext_end
        .checked_add(TAG_LEN)
        .ok_or(SemanticOutboxCryptoError::PayloadAuthFailed)?;
    if tag_end != bytes.len() {
        return Err(SemanticOutboxCryptoError::PayloadAuthFailed);
    }
    let mut tag = [0u8; TAG_LEN];
    tag.copy_from_slice(&bytes[ciphertext_end..tag_end]);
    Ok(EnvelopeV1 {
        key_version,
        nonce,
        ciphertext: bytes[ciphertext_start..ciphertext_end].to_vec(),
        tag,
    })
}

fn encode_record(
    installation_id: &[u8; 16],
    protection_kind: u8,
    payload: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SemanticOutboxCryptoError> {
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    let mut record = Vec::with_capacity(8 + 2 + 4 + 16 + 1 + 4 + payload.len() + CHECKSUM_LEN);
    record.extend_from_slice(RECORD_MAGIC);
    record.extend_from_slice(&RECORD_SCHEMA.to_le_bytes());
    record.extend_from_slice(&SEMANTIC_OUTBOX_KEY_VERSION_V1.to_le_bytes());
    record.extend_from_slice(installation_id);
    record.push(protection_kind);
    record.extend_from_slice(&payload_len.to_le_bytes());
    record.extend_from_slice(payload);
    let checksum = Sha256::digest(&record);
    record.extend_from_slice(&checksum);
    Ok(Zeroizing::new(record))
}

fn parse_record(bytes: &[u8]) -> Result<KeyRecordV1, SemanticOutboxCryptoError> {
    const FIXED_PREFIX: usize = 8 + 2 + 4 + 16 + 1 + 4;
    if bytes.len() < FIXED_PREFIX + CHECKSUM_LEN || &bytes[..8] != RECORD_MAGIC {
        return Err(SemanticOutboxCryptoError::Unavailable);
    }
    let checksum_at = bytes.len() - CHECKSUM_LEN;
    if Sha256::digest(&bytes[..checksum_at]).as_slice() != &bytes[checksum_at..] {
        return Err(SemanticOutboxCryptoError::Unavailable);
    }
    let schema = read_u16(bytes, 8)?;
    if schema != RECORD_SCHEMA {
        return Err(SemanticOutboxCryptoError::KeyVersionUnsupported);
    }
    let key_version = read_u32(bytes, 10)?;
    if key_version != SEMANTIC_OUTBOX_KEY_VERSION_V1 {
        return Err(SemanticOutboxCryptoError::KeyVersionUnsupported);
    }
    let mut installation_id = [0u8; 16];
    installation_id.copy_from_slice(&bytes[14..30]);
    let protection_kind = bytes[30];
    let payload_len = usize::try_from(read_u32(bytes, 31)?)
        .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    if payload_len > MAX_RECORD_PAYLOAD_LEN {
        return Err(SemanticOutboxCryptoError::Unavailable);
    }
    let payload_start = FIXED_PREFIX;
    let payload_end = payload_start
        .checked_add(payload_len)
        .ok_or(SemanticOutboxCryptoError::Unavailable)?;
    if payload_end
        .checked_add(CHECKSUM_LEN)
        .filter(|end| *end == bytes.len())
        .is_none()
    {
        return Err(SemanticOutboxCryptoError::Unavailable);
    }
    Ok(KeyRecordV1 {
        installation_id,
        protection_kind,
        payload: Zeroizing::new(bytes[payload_start..payload_end].to_vec()),
    })
}

fn read_u16(bytes: &[u8], start: usize) -> Result<u16, SemanticOutboxCryptoError> {
    let slice = bytes
        .get(start..start + 2)
        .ok_or(SemanticOutboxCryptoError::Unavailable)?;
    Ok(u16::from_le_bytes([slice[0], slice[1]]))
}

fn read_u32(bytes: &[u8], start: usize) -> Result<u32, SemanticOutboxCryptoError> {
    let slice = bytes
        .get(start..start + 4)
        .ok_or(SemanticOutboxCryptoError::Unavailable)?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn random_suffix() -> Result<String, SemanticOutboxCryptoError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    }
    Ok(output)
}

/// Consume a record file only after the platform opener has rejected reparse
/// points and validated the security properties of this exact handle.  The
/// metadata length also bounds the read, so a concurrently growing file cannot
/// turn an authority open into an unbounded allocation.
fn read_record_handle(file: File) -> Result<KeyRecordV1, SemanticOutboxCryptoError> {
    let metadata = file
        .metadata()
        .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    if !metadata.is_file() || metadata.len() > u64::try_from(MAX_RECORD_PAYLOAD_LEN + 128).unwrap()
    {
        return Err(SemanticOutboxCryptoError::Unavailable);
    }
    let expected_len =
        usize::try_from(metadata.len()).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    let mut bytes = Vec::with_capacity(expected_len);
    file.take(metadata.len().saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
    if bytes.len() != expected_len {
        bytes.zeroize();
        return Err(SemanticOutboxCryptoError::Unavailable);
    }
    let parsed = parse_record(&bytes);
    bytes.zeroize();
    parsed
}

fn write_record_handle(
    mut file: File,
    record: &mut Zeroizing<Vec<u8>>,
) -> Result<(), SemanticOutboxCryptoError> {
    let result = file
        .write_all(record.as_slice())
        .and_then(|()| file.sync_all())
        .map_err(|_| SemanticOutboxCryptoError::Unavailable);
    record.zeroize();
    result
}

#[cfg(target_os = "linux")]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::fs::OpenOptions;
    use std::io::ErrorKind;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::linux::fs::MetadataExt;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    pub(super) fn open_installation_key(
        storage_parent: &Path,
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let authority = storage_parent.join(AUTHORITY_DIRECTORY);
        match fs::symlink_metadata(&authority) {
            Ok(_) => read_existing(&authority),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                create_or_join(storage_parent, &authority)
            }
            Err(_) => Err(SemanticOutboxCryptoError::Unavailable),
        }
    }

    fn create_or_join(
        storage_parent: &Path,
        authority: &Path,
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        for _ in 0..16 {
            let temporary =
                storage_parent.join(format!(".native-authority.tmp-{}", random_suffix()?));
            if !create_secure_directory(&temporary)? {
                continue;
            }
            let result = create_fresh_authority(&temporary)
                .and_then(|()| no_replace_rename(&temporary, authority))
                .and_then(|renamed| {
                    if renamed {
                        sync_directory(storage_parent)?;
                    }
                    read_existing(authority)
                });
            return result;
        }
        Err(SemanticOutboxCryptoError::Unavailable)
    }

    fn create_fresh_authority(authority: &Path) -> Result<(), SemanticOutboxCryptoError> {
        let directory = open_secure_directory(authority)?;
        let mut installation_id = Zeroizing::new([0u8; 16]);
        let mut key = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut installation_id[..])
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        getrandom::fill(&mut key[..]).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let mut record = encode_record(&installation_id, PROTECTION_LINUX_FILE, &key[..])?;
        let record_file = create_secure_record(&directory)?;
        write_record_handle(record_file, &mut record)?;
        let verified = read_existing(authority)?;
        drop(verified);
        directory
            .sync_all()
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        Ok(())
    }

    fn read_existing(authority: &Path) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let directory = open_secure_directory(authority)?;
        let record = read_record_handle(open_secure_record(&directory)?)?;
        if record.protection_kind != PROTECTION_LINUX_FILE || record.payload.len() != 32 {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let mut key = Zeroizing::new([0u8; 32]);
        key.copy_from_slice(&record.payload[..]);
        Ok(key)
    }

    fn create_secure_directory(path: &Path) -> Result<bool, SemanticOutboxCryptoError> {
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        match builder.create(path) {
            Ok(()) => {
                drop(open_secure_directory(path)?);
                Ok(true)
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(false),
            Err(_) => Err(SemanticOutboxCryptoError::Unavailable),
        }
    }

    fn open_secure_directory(path: &Path) -> Result<File, SemanticOutboxCryptoError> {
        let mut options = OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let directory = options
            .open(path)
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        validate_directory_handle(&directory)?;
        Ok(directory)
    }

    fn validate_directory_handle(directory: &File) -> Result<(), SemanticOutboxCryptoError> {
        let metadata = directory
            .metadata()
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        if !metadata.is_dir()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o700
        {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        Ok(())
    }

    fn open_secure_record(directory: &File) -> Result<File, SemanticOutboxCryptoError> {
        let record = open_record_at(directory, libc::O_RDONLY, 0)?;
        validate_record_handle(&record)?;
        Ok(record)
    }

    fn create_secure_record(directory: &File) -> Result<File, SemanticOutboxCryptoError> {
        let record = open_record_at(
            directory,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
            0o600,
        )?;
        validate_record_handle(&record)?;
        Ok(record)
    }

    fn open_record_at(
        directory: &File,
        flags: libc::c_int,
        mode: libc::mode_t,
    ) -> Result<File, SemanticOutboxCryptoError> {
        let name =
            CString::new(AUTHORITY_RECORD).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
                mode,
            )
        };
        if fd < 0 {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        Ok(unsafe { File::from_raw_fd(fd) })
    }

    fn validate_record_handle(record: &File) -> Result<(), SemanticOutboxCryptoError> {
        let metadata = record
            .metadata()
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        if !metadata.file_type().is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o777 != 0o600
        {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        Ok(())
    }

    fn no_replace_rename(from: &Path, to: &Path) -> Result<bool, SemanticOutboxCryptoError> {
        let from = CString::new(from.as_os_str().as_bytes())
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let to = CString::new(to.as_os_str().as_bytes())
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EEXIST) {
            return Ok(false);
        }
        Err(SemanticOutboxCryptoError::Unavailable)
    }

    fn sync_directory(path: &Path) -> Result<(), SemanticOutboxCryptoError> {
        File::open(path)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::io::ErrorKind;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, GENERIC_ALL, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        GetExplicitEntriesFromAclW, GetSecurityInfo, SetEntriesInAclW, EXPLICIT_ACCESS_W,
        GRANT_ACCESS, SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_IS_WELL_KNOWN_GROUP,
        TRUSTEE_W,
    };
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use windows_sys::Win32::Security::{
        CopySid, CreateWellKnownSid, EqualSid, GetLengthSid, GetSecurityDescriptorControl,
        GetTokenInformation, InitializeSecurityDescriptor, SetSecurityDescriptorControl,
        SetSecurityDescriptorDacl, SetSecurityDescriptorOwner, TokenUser, WinLocalSystemSid, ACL,
        DACL_SECURITY_INFORMATION, NO_INHERITANCE, OWNER_SECURITY_INFORMATION, PSID,
        SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SE_DACL_PROTECTED, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateDirectoryW, CreateFileW, GetFileInformationByHandle, GetFinalPathNameByHandleW,
        MoveFileExW, BY_HANDLE_FILE_INFORMATION, CREATE_NEW, FILE_ALL_ACCESS,
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_REPARSE_POINT,
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_GENERIC_WRITE, FILE_SHARE_NONE, FILE_SHARE_READ, MOVEFILE_WRITE_THROUGH,
        OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const DPAPI_ENTROPY_PREFIX: &[u8] = b"AE-SOK-DPAPI-V1\0";

    pub(super) fn open_installation_key(
        storage_parent: &Path,
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let authority = storage_parent.join(AUTHORITY_DIRECTORY);
        match fs::symlink_metadata(&authority) {
            Ok(_) => read_existing(&authority),
            Err(error) if error.kind() == ErrorKind::NotFound => {
                create_or_join(storage_parent, &authority)
            }
            Err(_) => Err(SemanticOutboxCryptoError::Unavailable),
        }
    }

    fn create_or_join(
        storage_parent: &Path,
        authority: &Path,
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let sids = SidSet::current()?;
        for _ in 0..16 {
            let temporary =
                storage_parent.join(format!(".native-authority.tmp-{}", random_suffix()?));
            if !create_secure_directory(&temporary, &sids)? {
                continue;
            }
            let result = create_fresh_authority(&temporary, &sids)
                .and_then(|()| move_no_replace(&temporary, authority))
                .and_then(|_| read_existing_after_competing_create(authority));
            return result;
        }
        Err(SemanticOutboxCryptoError::Unavailable)
    }

    fn read_existing_after_competing_create(
        authority: &Path,
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let mut last_error = SemanticOutboxCryptoError::Unavailable;
        for _ in 0..16 {
            match read_existing(authority) {
                Ok(key) => return Ok(key),
                Err(error) => last_error = error,
            }
            std::thread::yield_now();
        }
        Err(last_error)
    }

    fn create_fresh_authority(
        authority: &Path,
        sids: &SidSet,
    ) -> Result<(), SemanticOutboxCryptoError> {
        let directory = open_secure_directory(authority, sids)?;
        let mut installation_id = Zeroizing::new([0u8; 16]);
        let mut key = Zeroizing::new([0u8; 32]);
        getrandom::fill(&mut installation_id[..])
            .map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        getrandom::fill(&mut key[..]).map_err(|_| SemanticOutboxCryptoError::Unavailable)?;
        let protected = dpapi_protect(&key, &installation_id)?;
        let mut record = encode_record(&installation_id, PROTECTION_WINDOWS_DPAPI, &protected)?;
        let record_file = create_secure_file(authority, &directory, sids)?;
        write_record_handle(record_file, &mut record)?;
        drop(directory);
        let verified = read_existing(authority)?;
        drop(verified);
        Ok(())
    }

    fn read_existing(authority: &Path) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let sids = SidSet::current()?;
        let directory = open_secure_directory(authority, &sids)?;
        let record = read_record_handle(open_secure_record(authority, &directory, &sids)?)?;
        if record.protection_kind != PROTECTION_WINDOWS_DPAPI {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        dpapi_unprotect(&record.payload, &record.installation_id)
    }

    fn wide(path: &Path) -> Result<Vec<u16>, SemanticOutboxCryptoError> {
        let mut text: Vec<u16> = path.as_os_str().encode_wide().collect();
        if text.iter().any(|code| *code == 0) {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        text.push(0);
        Ok(text)
    }

    fn create_secure_directory(
        path: &Path,
        sids: &SidSet,
    ) -> Result<bool, SemanticOutboxCryptoError> {
        let wide_path = wide(path)?;
        let mut security = SecurityDescriptor::new(sids)?;
        let created = unsafe { CreateDirectoryW(wide_path.as_ptr(), &security.attributes()) };
        if created != 0 {
            drop(open_secure_directory(path, sids)?);
            return Ok(true);
        }
        if std::io::Error::last_os_error().kind() == ErrorKind::AlreadyExists {
            return Ok(false);
        }
        Err(SemanticOutboxCryptoError::Unavailable)
    }

    fn open_secure_directory(
        path: &Path,
        sids: &SidSet,
    ) -> Result<File, SemanticOutboxCryptoError> {
        let wide_path = wide(path)?;
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ,
                null_mut(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let directory = unsafe { File::from_raw_handle(handle) };
        validate_directory_handle(raw_handle(&directory), sids)?;
        Ok(directory)
    }

    fn create_secure_file(
        authority: &Path,
        directory: &File,
        sids: &SidSet,
    ) -> Result<File, SemanticOutboxCryptoError> {
        let path = authority.join(AUTHORITY_RECORD);
        let wide_path = wide(&path)?;
        let mut security = SecurityDescriptor::new(sids)?;
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                FILE_GENERIC_WRITE,
                FILE_SHARE_NONE,
                &security.attributes(),
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle != INVALID_HANDLE_VALUE {
            let record = unsafe { File::from_raw_handle(handle) };
            validate_record_handle(raw_handle(&record), sids)?;
            ensure_record_belongs_to_directory(directory, &record)?;
            return Ok(record);
        }
        Err(SemanticOutboxCryptoError::Unavailable)
    }

    fn open_secure_record(
        authority: &Path,
        directory: &File,
        sids: &SidSet,
    ) -> Result<File, SemanticOutboxCryptoError> {
        let path = authority.join(AUTHORITY_RECORD);
        let wide_path = wide(&path)?;
        let handle = unsafe {
            CreateFileW(
                wide_path.as_ptr(),
                FILE_GENERIC_READ,
                FILE_SHARE_READ,
                null_mut(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let record = unsafe { File::from_raw_handle(handle) };
        validate_record_handle(raw_handle(&record), sids)?;
        ensure_record_belongs_to_directory(directory, &record)?;
        Ok(record)
    }

    fn move_no_replace(from: &Path, to: &Path) -> Result<bool, SemanticOutboxCryptoError> {
        let from = wide(from)?;
        let to = wide(to)?;
        let moved = unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if moved != 0 {
            return Ok(true);
        }
        if std::io::Error::last_os_error().kind() == ErrorKind::AlreadyExists {
            return Ok(false);
        }
        Err(SemanticOutboxCryptoError::Unavailable)
    }

    fn raw_handle(file: &File) -> HANDLE {
        file.as_raw_handle()
    }

    fn validate_directory_handle(
        handle: HANDLE,
        sids: &SidSet,
    ) -> Result<(), SemanticOutboxCryptoError> {
        validate_handle_attributes(handle, true)?;
        validate_acl_handle(handle, sids)
    }

    fn validate_record_handle(
        handle: HANDLE,
        sids: &SidSet,
    ) -> Result<(), SemanticOutboxCryptoError> {
        validate_handle_attributes(handle, false)?;
        validate_acl_handle(handle, sids)
    }

    fn validate_handle_attributes(
        handle: HANDLE,
        expect_directory: bool,
    ) -> Result<(), SemanticOutboxCryptoError> {
        let mut information: BY_HANDLE_FILE_INFORMATION = unsafe { zeroed() };
        if unsafe { GetFileInformationByHandle(handle, &mut information) } == 0
            || information.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || (information.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0) != expect_directory
        {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        Ok(())
    }

    fn ensure_record_belongs_to_directory(
        directory: &File,
        record: &File,
    ) -> Result<(), SemanticOutboxCryptoError> {
        let directory_path = final_path(raw_handle(directory))?;
        let record_path = final_path(raw_handle(record))?;
        let record_name: Vec<u16> = AUTHORITY_RECORD.encode_utf16().collect();
        let separator = u16::from(b'\\');
        if record_path.len() != directory_path.len() + 1 + record_name.len()
            || !record_path.starts_with(&directory_path)
            || record_path[directory_path.len()] != separator
            || record_path[directory_path.len() + 1..] != record_name
        {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        Ok(())
    }

    fn final_path(handle: HANDLE) -> Result<Vec<u16>, SemanticOutboxCryptoError> {
        let needed = unsafe { GetFinalPathNameByHandleW(handle, null_mut(), 0, 0) };
        if needed == 0 {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let capacity = usize::try_from(needed)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(SemanticOutboxCryptoError::Unavailable)?;
        let mut path = vec![0u16; capacity];
        let written = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                path.as_mut_ptr(),
                u32::try_from(path.len()).map_err(|_| SemanticOutboxCryptoError::Unavailable)?,
                0,
            )
        };
        if written == 0 || usize::try_from(written).unwrap_or(path.len()) >= path.len() {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        path.truncate(
            usize::try_from(written).map_err(|_| SemanticOutboxCryptoError::Unavailable)?,
        );
        Ok(path)
    }

    fn validate_acl_handle(handle: HANDLE, sids: &SidSet) -> Result<(), SemanticOutboxCryptoError> {
        let mut owner: PSID = null_mut();
        let mut dacl: *mut ACL = null_mut();
        let mut descriptor = null_mut();
        let result = unsafe {
            GetSecurityInfo(
                handle,
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        };
        if result != 0 || descriptor.is_null() || dacl.is_null() || owner.is_null() {
            if !descriptor.is_null() {
                unsafe {
                    LocalFree(descriptor.cast());
                }
            }
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let result = (|| {
            let mut control = 0u16;
            let mut revision = 0u32;
            if unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) } == 0
                || control & SE_DACL_PROTECTED == 0
                || unsafe { EqualSid(owner, sids.user_sid()) } == 0
            {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let mut count = 0u32;
            let mut entries: *mut EXPLICIT_ACCESS_W = null_mut();
            let explicit = unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) };
            if explicit != 0 || entries.is_null() || count != 2 {
                if !entries.is_null() {
                    unsafe {
                        LocalFree(entries.cast());
                    }
                }
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let entries_result = unsafe {
                let entries = std::slice::from_raw_parts(
                    entries,
                    usize::try_from(count).map_err(|_| SemanticOutboxCryptoError::Unavailable)?,
                );
                let mut saw_user = false;
                let mut saw_system = false;
                for entry in entries {
                    if entry.grfAccessMode != GRANT_ACCESS
                        || !matches!(entry.grfAccessPermissions, GENERIC_ALL | FILE_ALL_ACCESS)
                        || entry.grfInheritance != NO_INHERITANCE
                        || entry.Trustee.TrusteeForm != TRUSTEE_IS_SID
                    {
                        return Err(SemanticOutboxCryptoError::Unavailable);
                    }
                    let sid = entry.Trustee.ptstrName.cast();
                    if EqualSid(sid, sids.user_sid()) != 0 {
                        if saw_user {
                            return Err(SemanticOutboxCryptoError::Unavailable);
                        }
                        saw_user = true;
                    } else if EqualSid(sid, sids.system_sid()) != 0 {
                        if saw_system {
                            return Err(SemanticOutboxCryptoError::Unavailable);
                        }
                        saw_system = true;
                    } else {
                        return Err(SemanticOutboxCryptoError::Unavailable);
                    }
                }
                if saw_user && saw_system {
                    Ok(())
                } else {
                    Err(SemanticOutboxCryptoError::Unavailable)
                }
            };
            unsafe {
                LocalFree(entries.cast());
            }
            entries_result
        })();
        unsafe {
            LocalFree(descriptor.cast());
        }
        result
    }

    fn dpapi_protect(
        key: &[u8; 32],
        installation_id: &[u8; 16],
    ) -> Result<Zeroizing<Vec<u8>>, SemanticOutboxCryptoError> {
        let mut entropy = dpapi_entropy(installation_id);
        let input = blob(key)?;
        let entropy_blob = blob(&entropy)?;
        let mut output: CRYPT_INTEGER_BLOB = unsafe { zeroed() };
        let protected = unsafe {
            CryptProtectData(
                &input,
                null(),
                &entropy_blob,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        entropy.zeroize();
        if protected == 0 || output.pbData.is_null() {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let bytes = unsafe {
            let output_bytes = std::slice::from_raw_parts(
                output.pbData,
                usize::try_from(output.cbData).unwrap_or(0),
            );
            Zeroizing::new(output_bytes.to_vec())
        };
        unsafe {
            LocalFree(output.pbData.cast());
        }
        Ok(bytes)
    }

    fn dpapi_unprotect(
        payload: &[u8],
        installation_id: &[u8; 16],
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        let mut entropy = dpapi_entropy(installation_id);
        let input = blob(payload)?;
        let entropy_blob = blob(&entropy)?;
        let mut output: CRYPT_INTEGER_BLOB = unsafe { zeroed() };
        let unprotected = unsafe {
            CryptUnprotectData(
                &input,
                null_mut(),
                &entropy_blob,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        entropy.zeroize();
        if unprotected == 0 || output.pbData.is_null() || output.cbData != 32 {
            if !output.pbData.is_null() {
                unsafe {
                    LocalFree(output.pbData.cast());
                }
            }
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let mut key = Zeroizing::new([0u8; 32]);
        unsafe {
            key.copy_from_slice(std::slice::from_raw_parts(output.pbData, 32));
            LocalFree(output.pbData.cast());
        }
        Ok(key)
    }

    fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, SemanticOutboxCryptoError> {
        Ok(CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(bytes.len())
                .map_err(|_| SemanticOutboxCryptoError::Unavailable)?,
            pbData: bytes.as_ptr().cast_mut(),
        })
    }

    fn dpapi_entropy(installation_id: &[u8; 16]) -> Zeroizing<Vec<u8>> {
        let mut entropy = Vec::with_capacity(DPAPI_ENTROPY_PREFIX.len() + installation_id.len());
        entropy.extend_from_slice(DPAPI_ENTROPY_PREFIX);
        entropy.extend_from_slice(installation_id);
        Zeroizing::new(entropy)
    }

    struct SidSet {
        user: Vec<u8>,
        system: Vec<u8>,
    }

    impl SidSet {
        fn current() -> Result<Self, SemanticOutboxCryptoError> {
            let mut token: HANDLE = null_mut();
            if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let token = Handle(token);
            let mut needed = 0u32;
            unsafe {
                GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut needed);
            }
            if needed == 0 {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let mut token_user = vec![
                0u8;
                usize::try_from(needed)
                    .map_err(|_| SemanticOutboxCryptoError::Unavailable)?
            ];
            if unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    token_user.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                )
            } == 0
            {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let user_sid = token_user_sid_from_buffer(&token_user)?;
            let user_len = unsafe { GetLengthSid(user_sid) };
            if user_len == 0 {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let mut user = vec![
                0u8;
                usize::try_from(user_len)
                    .map_err(|_| SemanticOutboxCryptoError::Unavailable)?
            ];
            if unsafe { CopySid(user_len, user.as_mut_ptr().cast(), user_sid) } == 0 {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let mut system_len = 0u32;
            unsafe {
                CreateWellKnownSid(WinLocalSystemSid, null_mut(), null_mut(), &mut system_len);
            }
            if system_len == 0 {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let mut system = vec![
                0u8;
                usize::try_from(system_len)
                    .map_err(|_| SemanticOutboxCryptoError::Unavailable)?
            ];
            if unsafe {
                CreateWellKnownSid(
                    WinLocalSystemSid,
                    null_mut(),
                    system.as_mut_ptr().cast(),
                    &mut system_len,
                )
            } == 0
            {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            Ok(Self { user, system })
        }

        fn user_sid(&self) -> PSID {
            self.user.as_ptr().cast_mut().cast()
        }

        fn system_sid(&self) -> PSID {
            self.system.as_ptr().cast_mut().cast()
        }
    }

    fn token_user_sid_from_buffer(
        token_user_buffer: &[u8],
    ) -> Result<PSID, SemanticOutboxCryptoError> {
        if token_user_buffer.len() < size_of::<TOKEN_USER>() {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        let token_user =
            unsafe { std::ptr::read_unaligned(token_user_buffer.as_ptr().cast::<TOKEN_USER>()) };
        if token_user.User.Sid.is_null() {
            return Err(SemanticOutboxCryptoError::Unavailable);
        }
        Ok(token_user.User.Sid)
    }

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
                unsafe {
                    CloseHandle(self.0);
                }
            }
        }
    }

    struct SecurityDescriptor {
        descriptor: SECURITY_DESCRIPTOR,
        dacl: *mut ACL,
    }

    impl SecurityDescriptor {
        fn new(sids: &SidSet) -> Result<Self, SemanticOutboxCryptoError> {
            let mut entries = [
                explicit_access(sids.user_sid(), TRUSTEE_IS_USER),
                explicit_access(sids.system_sid(), TRUSTEE_IS_WELL_KNOWN_GROUP),
            ];
            let mut dacl: *mut ACL = null_mut();
            if unsafe { SetEntriesInAclW(2, entries.as_mut_ptr(), null(), &mut dacl) } != 0
                || dacl.is_null()
            {
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            let mut descriptor: SECURITY_DESCRIPTOR = unsafe { zeroed() };
            let descriptor_ptr = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
            let initialized = unsafe { InitializeSecurityDescriptor(descriptor_ptr, 1) };
            let attached = unsafe { SetSecurityDescriptorDacl(descriptor_ptr, 1, dacl, 0) };
            let owned = unsafe { SetSecurityDescriptorOwner(descriptor_ptr, sids.user_sid(), 0) };
            let protected = unsafe {
                SetSecurityDescriptorControl(descriptor_ptr, SE_DACL_PROTECTED, SE_DACL_PROTECTED)
            };
            if initialized == 0 || attached == 0 || owned == 0 || protected == 0 {
                unsafe {
                    LocalFree(dacl.cast());
                }
                return Err(SemanticOutboxCryptoError::Unavailable);
            }
            Ok(Self { descriptor, dacl })
        }

        fn attributes(&mut self) -> SECURITY_ATTRIBUTES {
            SECURITY_ATTRIBUTES {
                nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap(),
                lpSecurityDescriptor: (&mut self.descriptor as *mut SECURITY_DESCRIPTOR).cast(),
                bInheritHandle: 0,
            }
        }
    }

    impl Drop for SecurityDescriptor {
        fn drop(&mut self) {
            if !self.dacl.is_null() {
                unsafe {
                    LocalFree(self.dacl.cast());
                }
            }
        }
    }

    fn explicit_access(sid: PSID, trustee_type: i32) -> EXPLICIT_ACCESS_W {
        EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: NO_INHERITANCE,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: trustee_type,
                ptstrName: sid.cast(),
            },
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::mem::{align_of, size_of, zeroed};

        #[test]
        fn token_user_buffer_reader_accepts_unaligned_os_storage() {
            let mut sid_byte = 0u8;
            let mut expected: TOKEN_USER = unsafe { zeroed() };
            expected.User.Sid = (&mut sid_byte as *mut u8).cast();

            let alignment = align_of::<TOKEN_USER>();
            assert!(alignment > 1);
            let mut storage = vec![0u8; size_of::<TOKEN_USER>() + alignment];
            let offset = (0..alignment)
                .find(|offset| (storage.as_ptr() as usize + offset) % alignment != 0)
                .unwrap();
            let bytes = &mut storage[offset..offset + size_of::<TOKEN_USER>()];
            assert_ne!((bytes.as_ptr() as usize) % alignment, 0);
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (&expected as *const TOKEN_USER).cast::<u8>(),
                    bytes.as_mut_ptr(),
                    size_of::<TOKEN_USER>(),
                );
            }

            assert_eq!(
                token_user_sid_from_buffer(bytes).unwrap(),
                expected.User.Sid
            );
        }

        #[test]
        fn token_user_buffer_reader_rejects_truncation() {
            assert!(matches!(
                token_user_sid_from_buffer(&[0u8; 1]),
                Err(SemanticOutboxCryptoError::Unavailable)
            ));
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use super::*;

    pub(super) fn open_installation_key(
        _storage_parent: &Path,
    ) -> Result<Zeroizing<[u8; 32]>, SemanticOutboxCryptoError> {
        Err(SemanticOutboxCryptoError::Unavailable)
    }
}
