use sha2::{Digest, Sha256};
use std::{env, fs, path::PathBuf, process::Command};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn constant(text: &str, name: &str) -> String {
    text.split(&format!("const {name}: &str = \"")).nth(1).unwrap()
        .split('"').next().unwrap().to_owned()
}
fn git(root: &std::path::Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(root).output().expect("git");
    assert!(output.status.success(), "git identity unavailable");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}
fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    for key in ["AE_SOURCE_SHA", "AE_BUILD_MANIFEST_OUT"] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    // Git state must be rechecked even when Cargo would otherwise reuse a build.
    println!("cargo:rerun-if-changed={}", root.join(".git").display());
    for path in ["crates/ae-contracts/src/core_surface.rs", "crates/ae-store/src/core_boundary_v9.rs", "astr_embodiment/assets/tzdb/manifest.json", "astr_embodiment/assets/tzdb/tzdb-2026c.bin"] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let source = env::var("AE_SOURCE_SHA").ok();
    if let Some(sha) = &source {
        assert!(sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "AE_SOURCE_SHA must be lowercase 40-hex");
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), *sha, "source SHA mismatch");
        assert!(git(&root, &["status", "--porcelain", "--untracked-files=all"]).is_empty(), "release source must be clean");
    } else {
        assert_ne!(env::var("PROFILE").as_deref(), Ok("release"), "release requires AE_SOURCE_SHA");
    }
    let public = ae_contracts::core_public_method_manifest_v1_bytes();
    let retired = ae_contracts::retired_surface_manifest_v1_bytes();
    assert_eq!(hash(&public), ae_contracts::CORE_PUBLIC_METHOD_MANIFEST_V1_SHA256);
    assert_eq!(hash(&retired), ae_contracts::RETIRED_SURFACE_MANIFEST_V1_SHA256);
    let schema = fs::read_to_string(root.join("crates/ae-store/src/core_boundary_v9.rs")).unwrap().replace("\r\n", "\n");
    let schema_source = schema.split("const SCHEMA_SOURCE: &str = r#\"").nth(1).unwrap().split("\"#;").next().unwrap();
    assert_eq!(hash(schema_source.as_bytes()), constant(&schema, "SOURCE_SHA256"));
    let mut sql = String::new();
    for line in schema_source.lines() {
        if line.starts_with("@deny|") {
            let parts: Vec<_> = line.split('|').collect();
            assert_eq!(parts.len(), 3);
            for (operation, suffix) in [("INSERT", "i"), ("UPDATE", "u"), ("DELETE", "d")] {
                sql.push_str(&format!("CREATE TRIGGER core_boundary_retired_{}_{}_{}_v1\nBEFORE {} ON \"{}\"\nBEGIN SELECT RAISE(ABORT,'CORE_BOUNDARY_LEGACY_WRITE_DENIED'); END;\n", parts[1], parts[2], suffix, operation, parts[2]));
            }
        } else { sql.push_str(line); sql.push('\n'); }
    }
    let schema_hash = hash(sql.as_bytes());
    assert_eq!(schema_hash, constant(&schema, "SQL_SHA256"));
    let tz: serde_json::Value = serde_json::from_slice(&fs::read(root.join("astr_embodiment/assets/tzdb/manifest.json")).unwrap()).unwrap();
    let tz_hash = hash(&fs::read(root.join("astr_embodiment/assets/tzdb/tzdb-2026c.bin")).unwrap());
    assert_eq!(tz["content_sha256"], tz_hash);
    assert_eq!(tz_hash, ae_contracts::EMBODIMENT_TZDB_SHA256);
    assert_eq!(tz["tzdb_release"], ae_contracts::EMBODIMENT_TZDB_RELEASE);
    let mut api = Vec::new();
    api.extend((public.len() as u64).to_le_bytes()); api.extend(&public);
    api.extend(ae_contracts::hex::decode32(&hash(&public)).unwrap());
    api.extend((retired.len() as u64).to_le_bytes()); api.extend(&retired);
    api.extend(ae_contracts::hex::decode32(&hash(&retired)).unwrap());
    api.extend(ae_contracts::hex::decode32(&schema_hash).unwrap());
    api.extend(ae_contracts::hex::decode32(&tz_hash).unwrap());
    let identity = serde_json::json!({
        "contract_version": 1, "version": env::var("CARGO_PKG_VERSION").unwrap(),
        "source_sha": source, "release_verified": false,
        "core_public_method_manifest_sha256": hash(&public),
        "retired_surface_manifest_sha256": hash(&retired),
        "core_api_digest": ae_contracts::hex::encode32(&ae_contracts::wire::domain_hash(b"ae.core-public-api.v1", &[&api])),
        "autonomy_schema_v9_sql_sha256": schema_hash,
        "tzdb_release": tz["tzdb_release"], "tzdb_content_sha256": tz_hash,
        "methods": ae_contracts::CORE_PUBLIC_METHOD_MANIFEST_V1.iter().map(|(name, _)| *name).collect::<Vec<_>>()
    });
    let bytes = serde_json::to_vec(&identity).unwrap();
    fs::write(PathBuf::from(env::var("OUT_DIR").unwrap()).join("build_identity.json"), &bytes).unwrap();
    if let Ok(path) = env::var("AE_BUILD_MANIFEST_OUT") {
        assert!(source.is_some(), "manifest export requires release source identity");
        fs::write(path, bytes).unwrap();
    }
}
