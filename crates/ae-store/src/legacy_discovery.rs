use ae_contracts::{hex, Digest, Id128};
use rusqlite::{Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};

const CANDIDATE_FENCES_FILE: &str = "candidate-fences.v1";
const CANDIDATE_DATABASE_FILE: &str = "continuity.sqlite3";
const CANDIDATE_FENCES_MAX_BYTES: u64 = 4096;
const INSTALLATION_RECORD_MAX_BYTES: u64 = 65536;
const LEGACY_DISCOVERY_APPLICATION_ID_V1: i64 = 0x4145_4332;

/// The immutable identity and compatibility fences a legacy continuity store
/// must satisfy before a later migration stage can consider it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CandidateFences {
    pub store_uuid: Id128,
    pub scope_digest: Digest,
    pub incarnation_id: Digest,
    pub seed_code_digest: Digest,
    pub formula_digest: Digest,
    pub graph_digest: Digest,
    pub revision: u64,
}

/// An accepted, read-only view of a structurally sound legacy candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyCandidate {
    pub root: PathBuf,
    pub fences: CandidateFences,
    content_identity: CandidateContentIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct CandidateContentIdentity {
    fences: CandidateFences,
    journal_event_digest: Digest,
    journal_chain_digest: Digest,
    snapshot_digest: Digest,
    replay_digest: Digest,
}

/// The only sources from which legacy candidate roots may be supplied.
///
/// All paths are consumed directly. Discovery never enumerates a directory,
/// recursively scans a drive, uses modification time, or merges candidates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoverySources {
    pub explicit_configuration: Vec<PathBuf>,
    pub host_persistent_root: Option<PathBuf>,
    pub historical_allowlist: Vec<PathBuf>,
    pub installation_records: Vec<PathBuf>,
}

/// Closed discovery outcome. A caller must handle ambiguity and rejection
/// explicitly; neither state selects a candidate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Discovery {
    None,
    Selected(Box<LegacyCandidate>),
    Ambiguous,
    Rejected(DiscoveryRejectCode),
}

/// Stable, machine-readable refusal codes for legacy discovery and fence
/// verification. These routines are read-only and never perform migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscoveryRejectCode {
    WriteRefusedCandidate,
    WriteRefusedIdentity,
    WriteRefusedScope,
    WriteRefusedSeedCode,
    WriteRefusedFormula,
    WriteRefusedGraph,
    WriteRefusedRevision,
}

impl DiscoveryRejectCode {
    pub const fn code(self) -> &'static str {
        match self {
            Self::WriteRefusedCandidate => "WRITE_REFUSED_CANDIDATE",
            Self::WriteRefusedIdentity => "WRITE_REFUSED_IDENTITY",
            Self::WriteRefusedScope => "WRITE_REFUSED_SCOPE",
            Self::WriteRefusedSeedCode => "WRITE_REFUSED_SEED_CODE",
            Self::WriteRefusedFormula => "WRITE_REFUSED_FORMULA",
            Self::WriteRefusedGraph => "WRITE_REFUSED_GRAPH",
            Self::WriteRefusedRevision => "WRITE_REFUSED_REVISION",
        }
    }
}

/// Validate one candidate root without modifying any candidate file or SQLite
/// database. Structural validation covers the fence file, SQLite integrity,
/// schema, journal, snapshot, graph and replay records.
pub fn validate_legacy_candidate(
    root: impl AsRef<Path>,
) -> Result<LegacyCandidate, DiscoveryRejectCode> {
    let root = canonical_candidate_root(root.as_ref())?;
    let fences = read_candidate_fences(&root.join(CANDIDATE_FENCES_FILE))?;
    let content_identity =
        validate_candidate_database(&root.join(CANDIDATE_DATABASE_FILE), &fences)?;

    Ok(LegacyCandidate {
        root,
        fences,
        content_identity,
    })
}

/// Discover structurally valid candidates from the fixed, caller-provided
/// sources only. Duplicate byte-equivalent identities collapse to one result;
/// distinct candidates fail closed as ambiguous.
pub fn discover_legacy(sources: &DiscoverySources) -> Discovery {
    let roots = match authorized_candidate_roots(sources) {
        Ok(roots) => roots,
        Err(code) => return Discovery::Rejected(code),
    };
    let mut candidates = Vec::new();
    for root in roots {
        let candidate = match validate_legacy_candidate(&root) {
            Ok(candidate) => candidate,
            Err(code) => return Discovery::Rejected(code),
        };
        if !candidates.iter().any(|existing: &LegacyCandidate| {
            existing.content_identity == candidate.content_identity
        }) {
            candidates.push(candidate);
        }
    }
    candidates.sort_by(|left, right| left.root.cmp(&right.root));

    match candidates.len() {
        0 => Discovery::None,
        1 => Discovery::Selected(Box::new(candidates.remove(0))),
        _ => Discovery::Ambiguous,
    }
}

/// Re-open a selected candidate in read-only mode and enforce every immutable
/// fence. A mismatch is always a stable refusal, never an implicit selection
/// or an automatic migration.
pub fn verify_candidate(
    candidate: &LegacyCandidate,
    expected: &CandidateFences,
) -> Result<LegacyCandidate, DiscoveryRejectCode> {
    let current = validate_legacy_candidate(&candidate.root)?;
    if current.content_identity != candidate.content_identity {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    if current.fences.store_uuid != expected.store_uuid
        || current.fences.incarnation_id != expected.incarnation_id
    {
        return Err(DiscoveryRejectCode::WriteRefusedIdentity);
    }
    if current.fences.scope_digest != expected.scope_digest {
        return Err(DiscoveryRejectCode::WriteRefusedScope);
    }
    if current.fences.seed_code_digest != expected.seed_code_digest {
        return Err(DiscoveryRejectCode::WriteRefusedSeedCode);
    }
    if current.fences.formula_digest != expected.formula_digest {
        return Err(DiscoveryRejectCode::WriteRefusedFormula);
    }
    if current.fences.graph_digest != expected.graph_digest {
        return Err(DiscoveryRejectCode::WriteRefusedGraph);
    }
    if current.fences.revision != expected.revision {
        return Err(DiscoveryRejectCode::WriteRefusedRevision);
    }
    Ok(current)
}

fn authorized_candidate_roots(
    sources: &DiscoverySources,
) -> Result<Vec<PathBuf>, DiscoveryRejectCode> {
    let mut roots = Vec::new();
    roots.extend(sources.explicit_configuration.iter().cloned());
    if let Some(root) = &sources.host_persistent_root {
        roots.push(root.clone());
    }
    roots.extend(sources.historical_allowlist.iter().cloned());
    for record in &sources.installation_records {
        roots.extend(read_installation_record(record)?);
    }
    Ok(roots)
}

fn read_installation_record(record: &Path) -> Result<Vec<PathBuf>, DiscoveryRejectCode> {
    let bytes = read_bounded(record, INSTALLATION_RECORD_MAX_BYTES)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let mut roots = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            continue;
        }
        let path = line
            .strip_prefix("legacy_candidate=")
            .filter(|path| !path.is_empty())
            .ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?;
        let root = PathBuf::from(path);
        if !root.is_absolute() {
            return Err(DiscoveryRejectCode::WriteRefusedCandidate);
        }
        roots.push(root);
    }
    Ok(roots)
}

fn canonical_candidate_root(root: &Path) -> Result<PathBuf, DiscoveryRejectCode> {
    let root = fs::canonicalize(root).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if !root.is_dir() {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    Ok(root)
}

fn read_candidate_fences(path: &Path) -> Result<CandidateFences, DiscoveryRejectCode> {
    let bytes = read_bounded(path, CANDIDATE_FENCES_MAX_BYTES)?;
    let text =
        std::str::from_utf8(&bytes).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let mut version = None;
    let mut store_uuid = None;
    let mut scope_digest = None;
    let mut incarnation_id = None;
    let mut seed_code_digest = None;
    let mut formula_digest = None;
    let mut graph_digest = None;
    let mut revision = None;

    for line in text.lines() {
        let (key, value) = line
            .split_once('=')
            .ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?;
        match key {
            "version" if version.is_none() && value == "1" => version = Some(()),
            "store_uuid" if store_uuid.is_none() => {
                store_uuid = Some(
                    hex::decode16(value).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            "scope_digest" if scope_digest.is_none() => {
                scope_digest = Some(
                    hex::decode32(value).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            "incarnation_id" if incarnation_id.is_none() => {
                incarnation_id = Some(
                    hex::decode32(value).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            "seed_code_digest" if seed_code_digest.is_none() => {
                seed_code_digest = Some(
                    hex::decode32(value).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            "formula_digest" if formula_digest.is_none() => {
                formula_digest = Some(
                    hex::decode32(value).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            "graph_digest" if graph_digest.is_none() => {
                graph_digest = Some(
                    hex::decode32(value).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            "revision" if revision.is_none() => {
                revision = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
                );
            }
            _ => return Err(DiscoveryRejectCode::WriteRefusedCandidate),
        }
    }

    let fences = CandidateFences {
        store_uuid: store_uuid.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
        scope_digest: scope_digest.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
        incarnation_id: incarnation_id.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
        seed_code_digest: seed_code_digest.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
        formula_digest: formula_digest.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
        graph_digest: graph_digest.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
        revision: revision.ok_or(DiscoveryRejectCode::WriteRefusedCandidate)?,
    };
    if version.is_none() || fences.store_uuid == [0; 16] || fences.incarnation_id == [0; 32] {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    Ok(fences)
}

fn validate_candidate_database(
    path: &Path,
    fences: &CandidateFences,
) -> Result<CandidateContentIdentity, DiscoveryRejectCode> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if integrity != "ok" {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    let application_id: i64 = connection
        .query_row("PRAGMA application_id", [], |row| row.get(0))
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let user_version: i64 = connection
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let journal_mode: String = connection
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if application_id != LEGACY_DISCOVERY_APPLICATION_ID_V1
        || user_version != 1
        || !matches!(journal_mode.as_str(), "delete" | "wal")
    {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    validate_schema(&connection)?;

    let database_fences = read_database_fences(&connection)?;
    if database_fences != *fences {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    let (journal_event_digest, journal_chain_digest) = validate_journal(&connection, fences)?;
    let snapshot_digest = validate_snapshot(&connection, fences)?;
    validate_graph(&connection, fences)?;
    let replay_digest = validate_replay(&connection, fences)?;

    Ok(CandidateContentIdentity {
        fences: fences.clone(),
        journal_event_digest,
        journal_chain_digest,
        snapshot_digest,
        replay_digest,
    })
}

fn validate_schema(connection: &Connection) -> Result<(), DiscoveryRejectCode> {
    validate_table_columns(
        connection,
        "legacy_identity_v1",
        &[
            "store_uuid",
            "scope_digest",
            "incarnation_id",
            "seed_code_digest",
            "formula_digest",
            "graph_digest",
            "revision",
        ],
    )?;
    validate_table_columns(
        connection,
        "legacy_journal_v1",
        &[
            "revision",
            "scope_digest",
            "base_revision",
            "event_digest",
            "chain_digest",
        ],
    )?;
    validate_table_columns(
        connection,
        "legacy_snapshots_v1",
        &["revision", "scope_digest", "state_digest"],
    )?;
    validate_table_columns(connection, "legacy_graph_v1", &["revision", "graph_digest"])?;
    validate_table_columns(
        connection,
        "legacy_replay_v1",
        &["revision", "replay_digest"],
    )?;
    Ok(())
}

fn validate_table_columns(
    connection: &Connection,
    table: &str,
    expected: &[&str],
) -> Result<(), DiscoveryRejectCode> {
    let query = format!("PRAGMA table_info({table})");
    let mut statement = connection
        .prepare(&query)
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if columns
        .iter()
        .map(String::as_str)
        .ne(expected.iter().copied())
    {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    Ok(())
}

fn read_database_fences(connection: &Connection) -> Result<CandidateFences, DiscoveryRejectCode> {
    require_exactly_one_row(connection, "legacy_identity_v1")?;
    let row = connection
        .query_row(
            "SELECT store_uuid, scope_digest, incarnation_id, seed_code_digest, formula_digest, graph_digest, revision FROM legacy_identity_v1",
            [],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                    row.get::<_, Vec<u8>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            },
        )
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    Ok(CandidateFences {
        store_uuid: id128_from_blob(row.0)?,
        scope_digest: digest_from_blob(row.1)?,
        incarnation_id: digest_from_blob(row.2)?,
        seed_code_digest: digest_from_blob(row.3)?,
        formula_digest: digest_from_blob(row.4)?,
        graph_digest: digest_from_blob(row.5)?,
        revision: u64::try_from(row.6).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?,
    })
}

fn validate_journal(
    connection: &Connection,
    fences: &CandidateFences,
) -> Result<(Digest, Digest), DiscoveryRejectCode> {
    require_exactly_one_row(connection, "legacy_journal_v1")?;
    let row = connection
        .query_row(
            "SELECT revision, scope_digest, base_revision, event_digest, chain_digest FROM legacy_journal_v1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Vec<u8>>(3)?,
                    row.get::<_, Vec<u8>>(4)?,
                ))
            },
        )
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let revision = u64::try_from(row.0).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    let base_revision =
        u64::try_from(row.2).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if revision != fences.revision
        || base_revision >= revision
        || digest_from_blob(row.1)? != fences.scope_digest
    {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    Ok((digest_from_blob(row.3)?, digest_from_blob(row.4)?))
}

fn validate_snapshot(
    connection: &Connection,
    fences: &CandidateFences,
) -> Result<Digest, DiscoveryRejectCode> {
    require_exactly_one_row(connection, "legacy_snapshots_v1")?;
    let row = connection
        .query_row(
            "SELECT revision, scope_digest, state_digest FROM legacy_snapshots_v1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if u64::try_from(row.0).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?
        != fences.revision
        || digest_from_blob(row.1)? != fences.scope_digest
    {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    digest_from_blob(row.2)
}

fn validate_graph(
    connection: &Connection,
    fences: &CandidateFences,
) -> Result<(), DiscoveryRejectCode> {
    require_exactly_one_row(connection, "legacy_graph_v1")?;
    let row = connection
        .query_row(
            "SELECT revision, graph_digest FROM legacy_graph_v1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if u64::try_from(row.0).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?
        != fences.revision
        || digest_from_blob(row.1)? != fences.graph_digest
    {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    Ok(())
}

fn validate_replay(
    connection: &Connection,
    fences: &CandidateFences,
) -> Result<Digest, DiscoveryRejectCode> {
    require_exactly_one_row(connection, "legacy_replay_v1")?;
    let row = connection
        .query_row(
            "SELECT revision, replay_digest FROM legacy_replay_v1",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if u64::try_from(row.0).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?
        != fences.revision
    {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    digest_from_blob(row.1)
}

fn require_exactly_one_row(
    connection: &Connection,
    table: &str,
) -> Result<(), DiscoveryRejectCode> {
    let query = format!("SELECT COUNT(*) FROM {table}");
    let rows: i64 = connection
        .query_row(&query, [], |row| row.get(0))
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if rows != 1 {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    Ok(())
}

fn id128_from_blob(bytes: Vec<u8>) -> Result<Id128, DiscoveryRejectCode> {
    bytes
        .try_into()
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)
}

fn digest_from_blob(bytes: Vec<u8>) -> Result<Digest, DiscoveryRejectCode> {
    bytes
        .try_into()
        .map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)
}

fn read_bounded(path: &Path, max_bytes: u64) -> Result<Vec<u8>, DiscoveryRejectCode> {
    let metadata = fs::metadata(path).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)?;
    if !metadata.is_file() || metadata.len() > max_bytes {
        return Err(DiscoveryRejectCode::WriteRefusedCandidate);
    }
    fs::read(path).map_err(|_| DiscoveryRejectCode::WriteRefusedCandidate)
}
