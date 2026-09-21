CREATE TABLE IF NOT EXISTS legacy_semantic_formula_upgrades (
    scope_digest BLOB NOT NULL CHECK (length(scope_digest) = 32),
    from_formula_digest BLOB NOT NULL CHECK (length(from_formula_digest) = 32),
    to_formula_digest BLOB NOT NULL CHECK (length(to_formula_digest) = 32),
    base_revision INTEGER NOT NULL,
    next_revision INTEGER NOT NULL,
    event_digest BLOB NOT NULL CHECK (length(event_digest) = 32),
    receipt_digest BLOB NOT NULL CHECK (length(receipt_digest) = 32),
    source_state_digest BLOB NOT NULL CHECK (length(source_state_digest) = 32),
    target_state_before BLOB NOT NULL CHECK (length(target_state_before) = 32),
    source_graph_digest BLOB NOT NULL CHECK (length(source_graph_digest) = 32),
    prior_chain_digest BLOB NOT NULL CHECK (length(prior_chain_digest) = 32),
    migration_id BLOB NOT NULL CHECK (length(migration_id) = 32),
    upgrade_bytes BLOB NOT NULL,
    PRIMARY KEY (scope_digest, from_formula_digest, to_formula_digest),
    UNIQUE (scope_digest, next_revision),
    UNIQUE (migration_id)
);
