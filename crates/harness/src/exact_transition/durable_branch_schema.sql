PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS branch_schema_meta (
    schema_version TEXT PRIMARY KEY NOT NULL,
    migration_revision INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS branch_experiments (
    experiment_id TEXT PRIMARY KEY NOT NULL,
    root_branch_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS branch_occurrences (
    experiment_id TEXT NOT NULL REFERENCES branch_experiments(experiment_id),
    occurrence_id TEXT NOT NULL,
    parent_occurrence_id TEXT,
    state_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (experiment_id, occurrence_id),
    FOREIGN KEY (experiment_id, parent_occurrence_id)
        REFERENCES branch_occurrences(experiment_id, occurrence_id)
);

CREATE TABLE IF NOT EXISTS durable_branches (
    experiment_id TEXT NOT NULL REFERENCES branch_experiments(experiment_id),
    root_branch_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    parent_branch_id TEXT,
    fork_occurrence_id TEXT NOT NULL,
    strategy TEXT NOT NULL,
    source_handle TEXT,
    trajectory_prefix TEXT,
    effective_seed TEXT,
    setup_digest TEXT,
    boundary TEXT NOT NULL,
    assurance TEXT NOT NULL,
    run_id TEXT NOT NULL,
    episode_id TEXT,
    trajectory_id TEXT,
    context_id TEXT,
    policy_revision TEXT NOT NULL,
    config_revision TEXT NOT NULL,
    name TEXT NOT NULL,
    notes TEXT,
    status TEXT NOT NULL,
    metadata_revision INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (experiment_id, branch_id),
    UNIQUE (experiment_id, run_id),
    FOREIGN KEY (experiment_id, parent_branch_id)
        REFERENCES durable_branches(experiment_id, branch_id),
    FOREIGN KEY (experiment_id, fork_occurrence_id)
        REFERENCES branch_occurrences(experiment_id, occurrence_id)
);

CREATE TABLE IF NOT EXISTS branch_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    operation_kind TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    experiment_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    FOREIGN KEY (experiment_id, branch_id)
        REFERENCES durable_branches(experiment_id, branch_id)
);

CREATE TABLE IF NOT EXISTS branch_continuation_claims (
    experiment_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    operation_id TEXT NOT NULL UNIQUE REFERENCES branch_operations(operation_id),
    claim_state TEXT NOT NULL CHECK (
        claim_state IN (
            'prepared', 'owner_snapshotted', 'claimed', 'unknown', 'boundary_verified', 'resuming'
        )
    ),
    owner_json TEXT,
    owner_digest TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (experiment_id, branch_id),
    FOREIGN KEY (experiment_id, branch_id)
        REFERENCES durable_branches(experiment_id, branch_id),
    CHECK (
        (owner_json IS NULL AND owner_digest IS NULL)
        OR (owner_json IS NOT NULL AND owner_digest IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS branch_prune_plans (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES branch_operations(operation_id),
    experiment_id TEXT NOT NULL,
    branch_ids TEXT NOT NULL,
    retained_artifacts TEXT NOT NULL,
    collectable_artifacts TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS branch_artifacts (
    experiment_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    role TEXT NOT NULL,
    tombstoned INTEGER NOT NULL CHECK (tombstoned IN (0, 1)),
    operation_id TEXT NOT NULL REFERENCES branch_operations(operation_id),
    PRIMARY KEY (experiment_id, branch_id, artifact_id, role),
    FOREIGN KEY (experiment_id, branch_id)
        REFERENCES durable_branches(experiment_id, branch_id)
);

CREATE TABLE IF NOT EXISTS branch_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    experiment_id TEXT NOT NULL REFERENCES branch_experiments(experiment_id),
    branch_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL,
    metadata_revision INTEGER NOT NULL,
    occurred_at INTEGER NOT NULL,
    FOREIGN KEY (experiment_id, branch_id)
        REFERENCES durable_branches(experiment_id, branch_id)
);

CREATE TABLE IF NOT EXISTS branch_tombstones (
    experiment_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    pruned_at INTEGER NOT NULL,
    PRIMARY KEY (experiment_id, branch_id),
    FOREIGN KEY (experiment_id, branch_id)
        REFERENCES durable_branches(experiment_id, branch_id)
);

CREATE TABLE IF NOT EXISTS branch_tombstone_artifacts (
    experiment_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    artifact_id TEXT NOT NULL,
    role TEXT NOT NULL,
    collectable INTEGER NOT NULL CHECK (collectable IN (0, 1)),
    PRIMARY KEY (experiment_id, branch_id, artifact_id, role),
    FOREIGN KEY (experiment_id, branch_id)
        REFERENCES branch_tombstones(experiment_id, branch_id)
);

CREATE INDEX IF NOT EXISTS durable_branches_experiment_order
    ON durable_branches(experiment_id, branch_id);

CREATE INDEX IF NOT EXISTS branch_events_experiment_cursor
    ON branch_events(experiment_id, event_id);
