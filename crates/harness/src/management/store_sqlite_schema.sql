PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS management_submissions (
    request_id TEXT PRIMARY KEY NOT NULL,
    request_digest TEXT NOT NULL,
    workflow_run_id TEXT NOT NULL UNIQUE
);

CREATE TABLE IF NOT EXISTS management_runs (
    workflow_run_id TEXT PRIMARY KEY NOT NULL,
    request_id TEXT NOT NULL UNIQUE,
    request_digest TEXT NOT NULL,
    snapshot BLOB NOT NULL,
    oldest_sequence INTEGER
);

CREATE TABLE IF NOT EXISTS management_events (
    workflow_run_id TEXT NOT NULL REFERENCES management_runs(workflow_run_id),
    sequence INTEGER NOT NULL,
    event BLOB NOT NULL,
    PRIMARY KEY (workflow_run_id, sequence)
);

CREATE TABLE IF NOT EXISTS management_commands (
    workflow_run_id TEXT NOT NULL REFERENCES management_runs(workflow_run_id),
    command_id TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    application_in_flight INTEGER NOT NULL CHECK (application_in_flight IN (0, 1)),
    response BLOB,
    PRIMARY KEY (workflow_run_id, command_id)
);

CREATE TABLE IF NOT EXISTS management_runtime (
    workflow_run_id TEXT PRIMARY KEY NOT NULL,
    definition_digest TEXT NOT NULL,
    definition BLOB NOT NULL,
    runtime_snapshot BLOB NOT NULL,
    cancelled INTEGER NOT NULL CHECK (cancelled IN (0, 1))
);

CREATE INDEX IF NOT EXISTS management_events_run_sequence
    ON management_events(workflow_run_id, sequence);
