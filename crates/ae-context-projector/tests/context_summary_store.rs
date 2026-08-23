use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use ae_context_projector::{
    ContextSummaryStore, ContextSummaryV1, DeliveryOutcome, ReceiptCommitStatus, ReceiptEnvelopeV1,
    ReceiptValidationError, StoreError, ValidatedCommittedReceiptV1,
};

static NEXT_DB: AtomicU64 = AtomicU64::new(1);

fn db_path(label: &str) -> PathBuf {
    let root = std::env::var_os("CODEX_TASK_TEMP")
        .map(PathBuf::from)
        .expect("CODEX_TASK_TEMP must be provided by the task harness");
    let sequence = NEXT_DB.fetch_add(1, Ordering::Relaxed);
    root.join(format!("{label}-{}-{sequence}.sqlite", std::process::id()))
}

fn receipt(relation: u8, event: u8, value: i64) -> ValidatedCommittedReceiptV1 {
    ValidatedCommittedReceiptV1::try_from_envelope(envelope(relation, event, value))
        .expect("fixture is a valid committed receipt")
}

fn envelope(relation: u8, event: u8, value: i64) -> ReceiptEnvelopeV1 {
    ReceiptEnvelopeV1 {
        commit_status: ReceiptCommitStatus::Committed,
        event_id: [event; 16],
        relation_token: [relation; 16],
        source_continuum_revision: u64::from(event),
        dimensions_fxp6: [value; 15],
        unresolved_boundary: event.is_multiple_of(2),
        unresolved_repair: event.is_multiple_of(3),
        repetition_increment: 1,
        delivery_outcome: if event.is_multiple_of(2) {
            DeliveryOutcome::Delivered
        } else {
            DeliveryOutcome::Pending
        },
    }
}

#[test]
fn non_committed_or_forged_envelopes_are_rejected_before_sqlite_mutation() {
    let path = db_path("receipt-validation");
    let store = ContextSummaryStore::open(&path).expect("open store");

    let mut non_committed = envelope(1, 1, 1_000_000);
    non_committed.commit_status = ReceiptCommitStatus::Pending;
    assert_eq!(
        ValidatedCommittedReceiptV1::try_from_envelope(non_committed),
        Err(ReceiptValidationError::NotCommitted)
    );

    let mut zero_event = envelope(1, 1, 1_000_000);
    zero_event.event_id = [0; 16];
    assert_eq!(
        ValidatedCommittedReceiptV1::try_from_envelope(zero_event),
        Err(ReceiptValidationError::InvalidEventId)
    );

    let mut zero_relation = envelope(1, 1, 1_000_000);
    zero_relation.relation_token = [0; 16];
    assert_eq!(
        ValidatedCommittedReceiptV1::try_from_envelope(zero_relation),
        Err(ReceiptValidationError::InvalidRelationToken)
    );

    let mut zero_revision = envelope(1, 1, 1_000_000);
    zero_revision.source_continuum_revision = 0;
    assert_eq!(
        ValidatedCommittedReceiptV1::try_from_envelope(zero_revision),
        Err(ReceiptValidationError::InvalidSourceRevision)
    );

    let mut out_of_range = envelope(1, 1, 1_000_000);
    out_of_range.dimensions_fxp6 = [ValidatedCommittedReceiptV1::MAX_DIMENSION_FXP6 + 1; 15];
    assert_eq!(
        ValidatedCommittedReceiptV1::try_from_envelope(out_of_range),
        Err(ReceiptValidationError::DimensionOutOfRange)
    );

    let inspect = rusqlite::Connection::open(&path).expect("inspect rejected store");
    let rows: i64 = inspect
        .query_row(
            "SELECT (SELECT COUNT(*) FROM relation_summaries) + (SELECT COUNT(*) FROM relation_turns)",
            [],
            |row| row.get(0),
        )
        .expect("count rows");
    assert_eq!(rows, 0);
    drop(store);
}

#[test]
fn summary_schema_is_fixed_canonical_and_has_no_text_payload() {
    let path = db_path("schema");
    let mut store = ContextSummaryStore::open(&path).expect("open store");
    let summary = store
        .apply_committed_receipt(&receipt(1, 1, 200_000))
        .expect("write committed receipt");

    let canonical = summary.canonical_bytes();
    assert!(
        canonical.len() <= 4096,
        "canonical summary must remain bounded"
    );
    assert_eq!(canonical.len(), ContextSummaryV1::CANONICAL_BYTES_LEN);
    assert_eq!(summary.summary_revision, ContextSummaryV1::SCHEMA_VERSION);
    assert_eq!(summary.dimensions_ema_fxp6, [200_000; 15]);
    assert_eq!(
        summary.summary_digest,
        ContextSummaryV1::digest_of(&canonical)
    );
    assert!(canonical.windows(b"pending".len()).all(|w| w != b"pending"));
    assert!(canonical
        .windows(b"delivered".len())
        .all(|w| w != b"delivered"));
}

#[test]
fn committed_receipt_replay_is_idempotent() {
    let path = db_path("replay");
    let mut store = ContextSummaryStore::open(&path).expect("open store");
    let committed = receipt(2, 7, 400_000);

    let first = store
        .apply_committed_receipt(&committed)
        .expect("first receipt");
    let replay = store
        .apply_committed_receipt(&committed)
        .expect("replayed receipt");

    assert_eq!(replay, first);
    assert_eq!(replay.repetition_count, 1);
}

#[test]
fn same_event_id_with_different_canonical_receipt_is_rejected_without_mutation() {
    let path = db_path("event-identity-conflict");
    let mut store = ContextSummaryStore::open(&path).expect("open store");
    let accepted = store
        .apply_committed_receipt(&receipt(10, 1, 100_000))
        .expect("accept original event");

    assert!(matches!(
        store.apply_committed_receipt(&receipt(10, 1, 200_000)),
        Err(StoreError::EventIdentityConflict)
    ));
    assert_eq!(store.turn_count_for_relation([10; 16]).unwrap(), 1);
    assert_eq!(
        store.summary_for_relation([10; 16]).unwrap(),
        Some(accepted)
    );
}

#[test]
fn source_revision_must_strictly_increase_after_reopen_without_mutation() {
    let path = db_path("stale-source-revision");
    {
        let mut store = ContextSummaryStore::open(&path).expect("open initial store");
        store
            .apply_committed_receipt(&receipt(11, 2, 100_000))
            .expect("accept source revision two");
    }

    let mut stale_envelope = envelope(11, 3, 200_000);
    stale_envelope.source_continuum_revision = 2;
    let stale = ValidatedCommittedReceiptV1::try_from_envelope(stale_envelope)
        .expect("stale source revision is syntactically valid");
    let mut reopened = ContextSummaryStore::open(&path).expect("reopen store");
    assert!(matches!(
        reopened.apply_committed_receipt(&stale),
        Err(StoreError::StaleSourceRevision)
    ));
    assert_eq!(reopened.turn_count_for_relation([11; 16]).unwrap(), 1);
}

#[test]
fn fxp6_dimensions_are_bounded_to_the_closed_unit_interval() {
    for value in [0, ValidatedCommittedReceiptV1::MAX_DIMENSION_FXP6] {
        assert!(ValidatedCommittedReceiptV1::try_from_envelope(envelope(12, 1, value)).is_ok());
    }
    for value in [-1, ValidatedCommittedReceiptV1::MAX_DIMENSION_FXP6 + 1] {
        assert_eq!(
            ValidatedCommittedReceiptV1::try_from_envelope(envelope(12, 1, value)),
            Err(ReceiptValidationError::DimensionOutOfRange)
        );
    }
}

#[test]
fn summary_revision_increments_per_relation_and_survives_reopen() {
    let path = db_path("summary-revision");
    let first = {
        let mut store = ContextSummaryStore::open(&path).expect("open store");
        store
            .apply_committed_receipt(&receipt(8, 1, 1_000_000))
            .expect("first committed receipt")
    };
    assert_eq!(first.summary_revision, 1);

    let second = {
        let mut store = ContextSummaryStore::open(&path).expect("reopen for second receipt");
        let second = store
            .apply_committed_receipt(&receipt(8, 2, 200_000))
            .expect("second committed receipt");
        assert_eq!(second.summary_revision, 2);
        let replay = store
            .apply_committed_receipt(&receipt(8, 2, 200_000))
            .expect("replayed second receipt");
        assert_eq!(replay.summary_revision, 2);
        second
    };

    let mut reopened = ContextSummaryStore::open(&path).expect("reopen for third receipt");
    let third = reopened
        .apply_committed_receipt(&receipt(8, 3, 300_000))
        .expect("third committed receipt");
    assert_eq!(third.summary_revision, 3);
    assert_ne!(third.summary_digest, second.summary_digest);
}

#[test]
fn summary_revision_overflow_rejects_before_partial_write() {
    let path = db_path("summary-overflow");
    let mut store = ContextSummaryStore::open(&path).expect("open store");
    let mut maximum = store
        .apply_committed_receipt(&receipt(9, 1, 1_000_000))
        .expect("seed committed receipt");
    maximum.summary_revision = u32::MAX;
    maximum.summary_digest = ContextSummaryV1::digest_of(&maximum.canonical_bytes());

    let inspect = rusqlite::Connection::open(&path).expect("inspect seeded store");
    inspect
        .execute(
            "UPDATE relation_summaries SET summary_revision = ?1, summary_digest = ?2",
            rusqlite::params![i64::from(u32::MAX), &maximum.summary_digest[..]],
        )
        .expect("force revision boundary");
    let before_turns: i64 = inspect
        .query_row("SELECT COUNT(*) FROM relation_turns", [], |row| row.get(0))
        .expect("count turns before overflow");

    assert!(matches!(
        store.apply_committed_receipt(&receipt(9, 2, 200_000)),
        Err(StoreError::SummaryRevisionOverflow)
    ));
    let after_turns: i64 = inspect
        .query_row("SELECT COUNT(*) FROM relation_turns", [], |row| row.get(0))
        .expect("count turns after overflow");
    assert_eq!(after_turns, before_turns);
}

#[test]
fn relation_scopes_are_isolated() {
    let path = db_path("isolation");
    let mut store = ContextSummaryStore::open(&path).expect("open store");

    let left = store
        .apply_committed_receipt(&receipt(3, 1, 1_000_000))
        .expect("left relation");
    let right = store
        .apply_committed_receipt(&receipt(4, 1, 900_000))
        .expect("right relation");

    assert_ne!(left.summary_digest, right.summary_digest);
    assert_eq!(store.summary_for_relation([3; 16]).unwrap(), Some(left));
    assert_eq!(store.summary_for_relation([4; 16]).unwrap(), Some(right));
}

#[test]
fn reopen_preserves_summary_digest() {
    let path = db_path("reopen");
    let expected = {
        let mut store = ContextSummaryStore::open(&path).expect("open first store");
        store
            .apply_committed_receipt(&receipt(5, 1, 300_000))
            .expect("write receipt")
    };

    let reopened = ContextSummaryStore::open(&path).expect("reopen store");
    let actual = reopened
        .summary_for_relation([5; 16])
        .expect("read summary")
        .expect("summary remains present");

    assert_eq!(actual.summary_digest, expected.summary_digest);
    assert_eq!(actual, expected);
}

#[test]
fn per_relation_window_is_32_turns() {
    let path = db_path("window");
    let mut store = ContextSummaryStore::open(&path).expect("open store");

    for event in 1..=33 {
        store
            .apply_committed_receipt(&receipt(6, event, i64::from(event) * 10_000))
            .expect("write bounded turn");
    }

    let summary = store
        .summary_for_relation([6; 16])
        .expect("read bounded summary")
        .expect("summary exists");
    assert_eq!(summary.repetition_count, 32);
    assert_eq!(store.turn_count_for_relation([6; 16]).unwrap(), 32);
}

#[test]
fn relation_cap_is_eight_and_eviction_is_stable() {
    let first_path = db_path("cap-one");
    let second_path = db_path("cap-two");

    let first_membership = populate_and_read_membership(&first_path);
    let second_membership = populate_and_read_membership(&second_path);

    assert_eq!(first_membership, second_membership);
    assert_eq!(first_membership.len(), 8);
}

fn populate_and_read_membership(path: &PathBuf) -> Vec<u8> {
    let mut store = ContextSummaryStore::open(path).expect("open capped store");
    for relation in 1..=9 {
        store
            .apply_committed_receipt(&receipt(relation, 1, i64::from(relation)))
            .expect("write relation");
    }
    assert_eq!(store.active_relation_count().unwrap(), 8);

    (1..=9)
        .filter(|relation| {
            store
                .summary_for_relation([*relation; 16])
                .expect("read relation")
                .is_some()
        })
        .collect()
}

#[test]
fn public_receipt_and_summary_are_aggregate_only() {
    let path = db_path("privacy");
    let mut store = ContextSummaryStore::open(&path).expect("open store");
    let summary = store
        .apply_committed_receipt(&receipt(7, 1, 1_000_000))
        .expect("write aggregate receipt");

    let payload = summary.canonical_bytes();
    assert!(payload.len() <= ContextSummaryV1::CANONICAL_BYTES_LEN);

    let inspect = rusqlite::Connection::open(&path).expect("inspect persisted store");
    let schema: String = inspect
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'relation_summaries'",
            [],
            |row| row.get(0),
        )
        .expect("read schema");
    assert!(!schema.contains("hmac_key"));
    assert!(!schema.contains("relation_scope"));
    let relation_hmac: Vec<u8> = inspect
        .query_row("SELECT relation_hmac FROM relation_summaries", [], |row| {
            row.get(0)
        })
        .expect("read stored relation digest");
    assert_ne!(relation_hmac, vec![7; 16]);
}
