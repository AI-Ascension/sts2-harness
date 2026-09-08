// SPDX-License-Identifier: MIT

pub(super) const MIGRATION_1: &str = r#"
CREATE TABLE IF NOT EXISTS store_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY NOT NULL,
    seed TEXT NOT NULL,
    build_digest TEXT NOT NULL,
    state_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    provider_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS episodes (
    episode_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    current_attempt_id TEXT NOT NULL,
    current_trajectory_id TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS attempts (
    attempt_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    trajectory_id TEXT NOT NULL,
    parent_attempt_id TEXT,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    seed TEXT NOT NULL,
    build_digest TEXT NOT NULL,
    state_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    provider_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY(parent_attempt_id) REFERENCES attempts(attempt_id)
);
CREATE TABLE IF NOT EXISTS trajectories (
    trajectory_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL REFERENCES runs(run_id),
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    provenance_ref TEXT,
    provenance_digest TEXT,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS checkpoints (
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    sequence INTEGER NOT NULL,
    state_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    seed TEXT NOT NULL,
    build_digest TEXT NOT NULL,
    state_digest TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    provider_digest TEXT NOT NULL,
    observation BLOB NOT NULL,
    legal_actions_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY(episode_id, attempt_id, sequence)
);
CREATE TABLE IF NOT EXISTS operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    state_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    action_id TEXT NOT NULL,
    payload_digest TEXT NOT NULL,
    input_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    result_ref TEXT,
    result_digest TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS operations_episode_state
    ON operations(episode_id, state);
CREATE TABLE IF NOT EXISTS decisions (
    execution_id TEXT PRIMARY KEY NOT NULL,
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    input_fingerprint TEXT NOT NULL,
    model_revision TEXT NOT NULL,
    config_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    result_ref TEXT,
    result_digest TEXT,
    provider_reservation_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS provider_reservations (
    reservation_id TEXT PRIMARY KEY NOT NULL,
    execution_id TEXT NOT NULL UNIQUE REFERENCES decisions(execution_id),
    run_id TEXT NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    provider_execution_id TEXT NOT NULL UNIQUE,
    reserved_units INTEGER NOT NULL,
    actual_units INTEGER,
    state TEXT NOT NULL,
    failure_class TEXT,
    result_ref TEXT,
    result_digest TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS completions (
    episode_id TEXT PRIMARY KEY NOT NULL REFERENCES episodes(episode_id),
    run_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL REFERENCES attempts(attempt_id),
    trajectory_id TEXT NOT NULL,
    status TEXT NOT NULL,
    terminal_ref TEXT NOT NULL,
    checkpoint_sequence INTEGER NOT NULL,
    result_digest TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS jobs (
    job_id TEXT PRIMARY KEY NOT NULL,
    episode_id TEXT NOT NULL REFERENCES episodes(episode_id),
    payload_digest TEXT NOT NULL,
    state TEXT NOT NULL,
    claim_token TEXT,
    worker_id TEXT,
    result_ref TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS execution_events (
    event_id INTEGER PRIMARY KEY AUTOINCREMENT,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    state TEXT NOT NULL,
    detail_digest TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS execution_events_entity
    ON execution_events(entity_kind, entity_id, event_id);
"#;
