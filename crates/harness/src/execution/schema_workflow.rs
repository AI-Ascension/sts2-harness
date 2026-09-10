// SPDX-License-Identifier: MIT

pub(super) const MIGRATION_2: &str = r#"
CREATE TABLE IF NOT EXISTS workflow_definitions (
    workflow_id TEXT PRIMARY KEY NOT NULL,
    definition_digest TEXT NOT NULL UNIQUE,
    definition BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_plans (
    plan_id TEXT PRIMARY KEY NOT NULL,
    workflow_id TEXT NOT NULL REFERENCES workflow_definitions(workflow_id),
    plan_digest TEXT NOT NULL UNIQUE,
    plan BLOB NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    workflow_id TEXT NOT NULL REFERENCES workflow_definitions(workflow_id),
    plan_id TEXT NOT NULL REFERENCES workflow_plans(plan_id),
    episode_id TEXT NOT NULL UNIQUE,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    status TEXT NOT NULL,
    initial_projection BLOB NOT NULL,
    projection BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_events (
    event_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    payload_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(run_id, sequence)
);
CREATE INDEX IF NOT EXISTS workflow_events_run_sequence
    ON workflow_events(run_id, sequence);
CREATE TABLE IF NOT EXISTS workflow_invocations (
    invocation_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES workflow_runs(run_id),
    episode_id TEXT NOT NULL,
    plan_id TEXT NOT NULL REFERENCES workflow_plans(plan_id),
    command_id TEXT NOT NULL,
    game_operation_id TEXT,
    payload_digest TEXT NOT NULL,
    payload BLOB NOT NULL,
    state TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    send_marker INTEGER NOT NULL CHECK (send_marker IN (0, 1)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(run_id, command_id),
    UNIQUE(game_operation_id),
    FOREIGN KEY(episode_id) REFERENCES workflow_runs(episode_id)
);
CREATE INDEX IF NOT EXISTS workflow_invocations_run_state
    ON workflow_invocations(run_id, state);
INSERT OR IGNORE INTO store_metadata(key, value)
    VALUES ('workflow_schema_version', '1');
INSERT OR IGNORE INTO store_metadata(key, value)
    VALUES ('workflow_contract', 'workflow-v1');
"#;
