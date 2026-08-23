use ae_contracts::Digest;
use ae_store::{locate_vault, VaultLocateError, VaultMode};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn fixture_root(name: &str) -> PathBuf {
    let number = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "ae-store-continuity-vault-{name}-{}-{number}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn owner_cbor(generation_id: &str, store_uuid: [u8; 16]) -> Vec<u8> {
    assert!(generation_id.len() <= 23);
    let mut value = vec![0xa2, 0x6d];
    value.extend_from_slice(b"generation_id");
    value.push(0x60 + generation_id.len() as u8);
    value.extend_from_slice(generation_id.as_bytes());
    value.extend_from_slice(&[0x6a]);
    value.extend_from_slice(b"store_uuid");
    value.extend_from_slice(&[0x50]);
    value.extend_from_slice(&store_uuid);
    value
}

fn write_owner(root: &Path, generation_id: &str, store_uuid: [u8; 16]) {
    fs::write(
        root.join("owner.cbor"),
        owner_cbor(generation_id, store_uuid),
    )
    .unwrap();
}

fn write_current(root: &Path, generation_id: &str, incarnation_id: Digest, revision: u64) {
    let current = format!(
        "generation_id={generation_id}\nincarnation_id={}\nrevision={revision}\nmode=ready\n",
        hex(&incarnation_id)
    );
    fs::write(root.join("current"), current).unwrap();
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[test]
fn missing_vault_with_locator_history_is_recovery_required_not_unborn() {
    let root = fixture_root("missing-current");
    write_owner(&root, "generation-alpha", [3; 16]);
    fs::create_dir_all(root.join("history")).unwrap();

    let result = locate_vault(&root).unwrap();

    assert_eq!(result.mode, VaultMode::RecoveryRequired);
    assert!(!result.genesis_authorized);
    assert_eq!(result.generation_id, "generation-alpha");
    assert_eq!(result.store_uuid, [3; 16]);
}

#[test]
fn compatible_current_is_ready_and_preserves_identity() {
    let root = fixture_root("ready");
    let incarnation_id = [7; 32];
    write_owner(&root, "generation-alpha", [9; 16]);
    write_current(&root, "generation-alpha", incarnation_id, 42);

    let result = locate_vault(&root).unwrap();

    assert_eq!(result.mode, VaultMode::Ready);
    assert_eq!(result.incarnation_id, incarnation_id);
    assert_eq!(result.revision, 42);
    assert!(!result.genesis_authorized);
    assert_eq!(result.root, fs::canonicalize(root).unwrap());
}

#[test]
fn plugin_package_path_is_rejected() {
    let package_root = fixture_root("plugin-package");
    fs::write(
        package_root.join("plugin.toml"),
        "[plugin]\nname = 'fixture'\n",
    )
    .unwrap();
    let vault_root = package_root.join("continuity-vault");
    fs::create_dir_all(&vault_root).unwrap();

    let error = locate_vault(&vault_root).unwrap_err();

    assert_eq!(error, VaultLocateError::PluginPackagePath);
}

#[test]
fn corrupt_owner_is_not_normalized_to_unborn() {
    let root = fixture_root("corrupt-owner");
    fs::write(root.join("owner.cbor"), [0xff, 0x00, 0x01]).unwrap();

    let error = locate_vault(&root).unwrap_err();

    assert!(matches!(error, VaultLocateError::InvalidOwner(_)));
}
