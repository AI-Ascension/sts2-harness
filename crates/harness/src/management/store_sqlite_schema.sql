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

-- Opt-in historical metadata, never a current owner attachment or grant.
CREATE TABLE IF NOT EXISTS management_context_binding_history (
    workflow_run_id TEXT NOT NULL,
    node_execution_id TEXT NOT NULL,
    command_id TEXT NOT NULL,
    record BLOB NOT NULL CHECK(length(record) <= 16384),
    PRIMARY KEY (workflow_run_id, node_execution_id),
    UNIQUE (workflow_run_id, command_id),
    FOREIGN KEY (workflow_run_id, command_id)
        REFERENCES management_commands(workflow_run_id, command_id)
);

CREATE TABLE IF NOT EXISTS management_runtime (
    workflow_run_id TEXT PRIMARY KEY NOT NULL,
    definition_digest TEXT NOT NULL,
    definition BLOB NOT NULL,
    runtime_snapshot BLOB NOT NULL,
    cancelled INTEGER NOT NULL CHECK (cancelled IN (0, 1))
);

-- A restart without this Harness-owned journal must not manufacture a new
-- authority for a previously admitted workflow run.
CREATE TABLE IF NOT EXISTS management_runtime_control (
    workflow_run_id TEXT PRIMARY KEY NOT NULL REFERENCES management_runtime(workflow_run_id),
    journal BLOB NOT NULL
);

CREATE INDEX IF NOT EXISTS management_events_run_sequence
    ON management_events(workflow_run_id, sequence);

CREATE TABLE IF NOT EXISTS studio_drafts (
    schema_version TEXT NOT NULL,
    draft_id TEXT PRIMARY KEY NOT NULL,
    definition_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    etag TEXT NOT NULL,
    document BLOB NOT NULL,
    layout BLOB NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS studio_draft_mutations (
    draft_id TEXT NOT NULL REFERENCES studio_drafts(draft_id),
    mutation_id TEXT NOT NULL,
    mutation_digest TEXT NOT NULL,
    revision INTEGER NOT NULL,
    response BLOB NOT NULL,
    PRIMARY KEY (draft_id, mutation_id)
);

CREATE TABLE IF NOT EXISTS studio_publications (
    schema_version TEXT NOT NULL,
    definition_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    source TEXT NOT NULL,
    version TEXT NOT NULL,
    definition_digest TEXT PRIMARY KEY NOT NULL,
    definition BLOB NOT NULL,
    published_revision INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS management_inference_profile_revisions (
    profile_id TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    version TEXT NOT NULL,
    digest TEXT NOT NULL,
    revision BLOB NOT NULL,
    PRIMARY KEY (profile_id, ordinal),
    UNIQUE (profile_id, digest),
    UNIQUE (profile_id, version)
);

CREATE TABLE IF NOT EXISTS management_inference_profile_heads (
    profile_id TEXT PRIMARY KEY NOT NULL,
    digest TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS management_inference_profile_mutations (
    profile_id TEXT NOT NULL,
    mutation_id TEXT NOT NULL,
    mutation_digest TEXT NOT NULL,
    revision BLOB NOT NULL,
    PRIMARY KEY (profile_id, mutation_id)
);
