use ae_store::{
    SemanticOutboxCryptoError, SemanticOutboxKeyAuthorityV1, SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1,
    SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1, SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1,
};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};

fn task_temp_dir(label: &str) -> PathBuf {
    let root = std::env::var_os("CODEX_TASK_TEMP")
        .map(PathBuf::from)
        .expect("CODEX_TASK_TEMP must be set for semantic outbox crypto tests");
    let path = root.join(format!(
        "semantic-outbox-crypto-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn concurrent_create_converges_and_reopen_authenticates_old_envelope() {
    let parent = task_temp_dir("concurrent");
    let barrier = Arc::new(Barrier::new(2));
    let mut joins = Vec::new();
    for _ in 0..2 {
        let parent = parent.clone();
        let barrier = Arc::clone(&barrier);
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            SemanticOutboxKeyAuthorityV1::open(&parent)
        }));
    }

    let first = joins.remove(0).join().unwrap().unwrap();
    let second = joins.remove(0).join().unwrap().unwrap();
    let envelope = first.seal_v1(1, b"job-aad", b"old plaintext").unwrap();
    assert_eq!(
        second.open_v1(1, b"job-aad", &envelope).unwrap(),
        b"old plaintext"
    );

    drop(first);
    drop(second);
    let reopened = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    assert_eq!(
        reopened.open_v1(1, b"job-aad", &envelope).unwrap(),
        b"old plaintext"
    );
}

#[test]
fn malformed_authority_never_rebuilds_and_every_envelope_mutation_fails_closed() {
    let parent = task_temp_dir("fail-closed");
    let authority = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    let envelope = authority.seal_v1(1, b"bound-aad", b"payload").unwrap();

    assert!(matches!(
        authority.open_v1(1, b"other-aad", &envelope),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));
    let mut ciphertext_tampered = envelope.clone();
    ciphertext_tampered[30] ^= 1;
    assert!(matches!(
        authority.open_v1(1, b"bound-aad", &ciphertext_tampered),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));
    let mut tampered = envelope.clone();
    let tag_tail = tampered.len() - 1;
    tampered[tag_tail] ^= 1;
    assert!(matches!(
        authority.open_v1(1, b"bound-aad", &tampered),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));
    assert!(matches!(
        authority.open_v1(2, b"bound-aad", &envelope),
        Err(SemanticOutboxCryptoError::KeyVersionUnsupported)
    ));
    let mut unsupported_envelope_key_version = envelope.clone();
    unsupported_envelope_key_version[10] ^= 1;
    assert!(matches!(
        authority.open_v1(1, b"bound-aad", &unsupported_envelope_key_version),
        Err(SemanticOutboxCryptoError::KeyVersionUnsupported)
    ));
    let mut unsupported_schema = envelope.clone();
    unsupported_schema[8] ^= 1;
    assert!(matches!(
        authority.open_v1(1, b"bound-aad", &unsupported_schema),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));

    let record = parent
        .join(".native-authority")
        .join("semantic-outbox-key.v1");
    drop(authority);
    let mut corrupted = std::fs::read(&record).unwrap();
    let checksum_tail = corrupted.len() - 1;
    corrupted[checksum_tail] ^= 1;
    std::fs::write(&record, &corrupted).unwrap();
    assert!(matches!(
        SemanticOutboxKeyAuthorityV1::open(&parent),
        Err(SemanticOutboxCryptoError::Unavailable)
    ));
    assert_eq!(std::fs::read(&record).unwrap(), corrupted);
    std::fs::remove_file(&record).unwrap();
    assert!(matches!(
        SemanticOutboxKeyAuthorityV1::open(&parent),
        Err(SemanticOutboxCryptoError::Unavailable)
    ));
    assert!(!record.exists(), "a missing record must not be recreated");
}

#[test]
fn public_store_crypto_size_limits_fail_closed() {
    let parent = task_temp_dir("size-limits");
    let authority = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    let oversized_aad = vec![0u8; SEMANTIC_OUTBOX_MAX_AAD_BYTES_V1 + 1];
    assert!(matches!(
        authority.seal_v1(1, &oversized_aad, b"payload"),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));

    let oversized_plaintext = vec![0u8; SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1 + 1];
    assert!(matches!(
        authority.seal_v1(1, b"aad", &oversized_plaintext),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));

    let oversized_envelope = vec![0u8; SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1 + 1];
    assert!(matches!(
        authority.open_v1(1, b"aad", &oversized_envelope),
        Err(SemanticOutboxCryptoError::PayloadAuthFailed)
    ));

    let maximum_plaintext = vec![0x5Au8; SEMANTIC_OUTBOX_MAX_PLAINTEXT_BYTES_V1];
    let envelope = authority
        .seal_v1(1, b"aad", &maximum_plaintext)
        .expect("the documented maximum must remain accepted");
    assert!(envelope.len() <= SEMANTIC_OUTBOX_MAX_ENVELOPE_BYTES_V1);
    assert_eq!(
        authority.open_v1(1, b"aad", &envelope).unwrap(),
        maximum_plaintext
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_authority_permissions_are_owner_only() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    let parent = task_temp_dir("linux-mode");
    let authority = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    let directory = parent.join(".native-authority");
    let record = directory.join("semantic-outbox-key.v1");
    let directory_metadata = std::fs::metadata(&directory).unwrap();
    let record_metadata = std::fs::metadata(&record).unwrap();
    assert_eq!(directory_metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(record_metadata.uid(), unsafe { libc::geteuid() });
    assert_eq!(directory_metadata.mode() & 0o777, 0o700);
    assert_eq!(record_metadata.mode() & 0o777, 0o600);

    drop(authority);
    let mut overly_broad = record_metadata.permissions();
    overly_broad.set_mode(0o644);
    std::fs::set_permissions(&record, overly_broad).unwrap();
    assert!(matches!(
        SemanticOutboxKeyAuthorityV1::open(&parent),
        Err(SemanticOutboxCryptoError::Unavailable)
    ));

    let mut owner_only = std::fs::metadata(&record).unwrap().permissions();
    owner_only.set_mode(0o600);
    std::fs::set_permissions(&record, owner_only).unwrap();
    let target = parent.join("record-target");
    std::fs::rename(&record, &target).unwrap();
    std::os::unix::fs::symlink(&target, &record).unwrap();
    assert!(matches!(
        SemanticOutboxKeyAuthorityV1::open(&parent),
        Err(SemanticOutboxCryptoError::Unavailable)
    ));
}

#[cfg(windows)]
#[test]
fn windows_authority_record_is_dpapi_backed_non_reparse_and_acl_locked() {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, GENERIC_ALL, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        GetExplicitEntriesFromAclW, GetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
        SE_FILE_OBJECT, TRUSTEE_IS_SID,
    };
    use windows_sys::Win32::Security::{
        CreateWellKnownSid, EqualSid, GetLengthSid, GetSecurityDescriptorControl,
        GetTokenInformation, TokenUser, WinLocalSystemSid, ACL, DACL_SECURITY_INFORMATION,
        NO_INHERITANCE, OWNER_SECURITY_INFORMATION, PSID, SE_DACL_PROTECTED, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, FILE_ALL_ACCESS, FILE_ATTRIBUTE_REPARSE_POINT, INVALID_FILE_ATTRIBUTES,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn current_user_sid() -> Vec<u8> {
        unsafe {
            let mut token: HANDLE = null_mut();
            assert_ne!(
                OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token),
                0
            );
            let mut needed = 0u32;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
            assert_ne!(needed, 0);
            let mut token_user = vec![0u8; usize::try_from(needed).unwrap()];
            assert_ne!(
                GetTokenInformation(
                    token,
                    TokenUser,
                    token_user.as_mut_ptr().cast(),
                    needed,
                    &mut needed,
                ),
                0
            );
            assert!(token_user.len() >= std::mem::size_of::<TOKEN_USER>());
            let sid = std::ptr::read_unaligned(token_user.as_ptr().cast::<TOKEN_USER>())
                .User
                .Sid;
            let length = GetLengthSid(sid);
            assert_ne!(length, 0);
            let bytes =
                std::slice::from_raw_parts(sid.cast::<u8>(), usize::try_from(length).unwrap())
                    .to_vec();
            CloseHandle(token);
            bytes
        }
    }

    fn system_sid() -> Vec<u8> {
        unsafe {
            let mut needed = 0u32;
            CreateWellKnownSid(WinLocalSystemSid, null_mut(), null_mut(), &mut needed);
            assert_ne!(needed, 0);
            let mut sid = vec![0u8; usize::try_from(needed).unwrap()];
            assert_ne!(
                CreateWellKnownSid(
                    WinLocalSystemSid,
                    null_mut(),
                    sid.as_mut_ptr().cast(),
                    &mut needed,
                ),
                0
            );
            sid
        }
    }

    let parent = task_temp_dir("windows-acl");
    let authority = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    let envelope = authority.seal_v1(1, b"windows-aad", b"payload").unwrap();
    drop(authority);
    let reopened = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    assert_eq!(
        reopened.open_v1(1, b"windows-aad", &envelope).unwrap(),
        b"payload"
    );

    let record = parent
        .join(".native-authority")
        .join("semantic-outbox-key.v1");
    let record_bytes = std::fs::read(&record).unwrap();
    assert_eq!(
        record_bytes[30], 1,
        "record must use the DPAPI protection tag"
    );
    let mut wide: Vec<u16> = record.as_os_str().encode_wide().collect();
    wide.push(0);
    let attributes = unsafe { GetFileAttributesW(wide.as_ptr()) };
    assert_ne!(attributes, INVALID_FILE_ATTRIBUTES);
    assert_eq!(attributes & FILE_ATTRIBUTE_REPARSE_POINT, 0);

    let user_sid = current_user_sid();
    let system_sid = system_sid();
    let mut owner: PSID = null_mut();
    let mut dacl: *mut ACL = null_mut();
    let mut descriptor = null_mut();
    assert_eq!(
        unsafe {
            GetNamedSecurityInfoW(
                wide.as_ptr(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                null_mut(),
                &mut dacl,
                null_mut(),
                &mut descriptor,
            )
        },
        0
    );
    assert!(!descriptor.is_null());
    assert!(!owner.is_null());
    assert!(!dacl.is_null());
    assert_ne!(
        unsafe { EqualSid(owner, user_sid.as_ptr().cast_mut().cast()) },
        0
    );

    let mut control = 0u16;
    let mut revision = 0u32;
    assert_ne!(
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) },
        0
    );
    assert_ne!(control & SE_DACL_PROTECTED, 0);

    let mut count = 0u32;
    let mut entries: *mut EXPLICIT_ACCESS_W = null_mut();
    assert_eq!(
        unsafe { GetExplicitEntriesFromAclW(dacl, &mut count, &mut entries) },
        0
    );
    assert_eq!(count, 2);
    assert!(!entries.is_null());
    let entry_result = (|| {
        let entries =
            unsafe { std::slice::from_raw_parts(entries, usize::try_from(count).unwrap()) };
        let mut saw_user = false;
        let mut saw_system = false;
        for entry in entries {
            assert_eq!(entry.grfAccessMode, GRANT_ACCESS);
            assert!(matches!(
                entry.grfAccessPermissions,
                GENERIC_ALL | FILE_ALL_ACCESS
            ));
            assert_eq!(entry.grfInheritance, NO_INHERITANCE);
            assert_eq!(entry.Trustee.TrusteeForm, TRUSTEE_IS_SID);
            let sid = entry.Trustee.ptstrName.cast();
            if unsafe { EqualSid(sid, user_sid.as_ptr().cast_mut().cast()) } != 0 {
                assert!(!saw_user);
                saw_user = true;
            } else if unsafe { EqualSid(sid, system_sid.as_ptr().cast_mut().cast()) } != 0 {
                assert!(!saw_system);
                saw_system = true;
            } else {
                panic!("authority DACL contained an unexpected SID");
            }
        }
        assert!(saw_user && saw_system);
    })();
    unsafe {
        LocalFree(entries.cast());
        LocalFree(descriptor.cast());
    }
    entry_result
}

#[cfg(windows)]
#[test]
fn windows_reparse_record_is_rejected_when_symlink_creation_is_permitted() {
    let parent = task_temp_dir("windows-reparse");
    let authority = SemanticOutboxKeyAuthorityV1::open(&parent).unwrap();
    let record = parent
        .join(".native-authority")
        .join("semantic-outbox-key.v1");
    drop(authority);
    let target = parent.join("record-target");
    std::fs::rename(&record, &target).unwrap();
    match std::os::windows::fs::symlink_file(&target, &record) {
        Ok(()) => {}
        Err(error) if error.raw_os_error() == Some(1314) => return,
        Err(error) => panic!("failed to construct test reparse point: {error}"),
    }
    assert!(matches!(
        SemanticOutboxKeyAuthorityV1::open(&parent),
        Err(SemanticOutboxCryptoError::Unavailable)
    ));
}
