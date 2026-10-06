// SPDX-License-Identifier: MIT

pub(super) fn insert_outbox(
    transaction: &rusqlite::Transaction<'_>,
    run_id: &str,
    events: &[ControlEvent],
) -> Result<(), DurableControlStoreError> {
    if events.len() > MAX_EVENTS {
        return Err(DurableControlStoreError::TooLarge);
    }
    for event in events {
        let bytes = serde_json::to_vec(event).map_err(|_| DurableControlStoreError::Encode)?;
        if bytes.len() > MAX_EVENT_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        transaction
            .execute(
                "INSERT OR IGNORE INTO context_control_outbox
                    (run_id, sequence, event, event_digest, published)
                 VALUES (?1, ?2, ?3, ?4, 0)",
                params![run_id, event.sequence as i64, bytes, digest(&bytes)],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
    }
    Ok(())
}

pub(super) fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

pub(super) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS context_control_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS context_control_journal (
    run_id TEXT PRIMARY KEY NOT NULL,
    envelope BLOB NOT NULL,
    envelope_digest TEXT NOT NULL,
    management_active INTEGER NOT NULL CHECK (management_active IN (0, 1)),
    active_revision_id TEXT NOT NULL,
    pause_latched INTEGER NOT NULL CHECK (pause_latched IN (0, 1)),
    controller_epoch INTEGER NOT NULL,
    plan_epoch INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS context_control_owners (
    run_id TEXT PRIMARY KEY NOT NULL,
    owner_token TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS context_control_outbox (
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL,
    event BLOB NOT NULL,
    event_digest TEXT NOT NULL,
    published INTEGER NOT NULL CHECK (published IN (0, 1)),
    PRIMARY KEY (run_id, sequence)
);
CREATE TABLE IF NOT EXISTS context_control_phase1_snapshots (
    run_id TEXT NOT NULL,
    snapshot_id TEXT NOT NULL,
    snapshot BLOB NOT NULL,
    digest TEXT NOT NULL,
    PRIMARY KEY (run_id, snapshot_id)
);
CREATE TABLE IF NOT EXISTS context_control_owner_receipts (
    run_id TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    command_digest TEXT NOT NULL,
    idempotency_digest TEXT NOT NULL,
    envelope BLOB NOT NULL,
    envelope_digest TEXT NOT NULL,
    PRIMARY KEY (run_id, owner_id, command_digest),
    UNIQUE (run_id, owner_id, idempotency_digest)
);
CREATE TABLE IF NOT EXISTS context_control_context_sources (
    run_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    source_digest TEXT NOT NULL,
    envelope BLOB NOT NULL,
    envelope_digest TEXT NOT NULL,
    PRIMARY KEY (run_id, source_id, version)
);
CREATE TABLE IF NOT EXISTS context_control_active_context_source (
    run_id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    source_digest TEXT NOT NULL,
    active_revision_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS context_control_lifetime (
    run_id TEXT PRIMARY KEY NOT NULL,
    envelope BLOB NOT NULL,
    envelope_digest TEXT NOT NULL,
    scope_count INTEGER NOT NULL,
    manifest_count INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS context_control_owner_state (
    run_id TEXT PRIMARY KEY NOT NULL,
    owner_id TEXT NOT NULL,
    record_version INTEGER NOT NULL CHECK (record_version > 0),
    envelope BLOB NOT NULL,
    envelope_digest TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS context_control_owner_publications (
    run_id TEXT NOT NULL,
    source_id TEXT NOT NULL,
    source_version INTEGER NOT NULL CHECK (source_version > 0),
    owner_index_digest TEXT NOT NULL,
    actor_index_digest TEXT NOT NULL,
    request_index_digest TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    expected_owner_state_version INTEGER NOT NULL CHECK (expected_owner_state_version > 0),
    resulting_owner_state_version INTEGER NOT NULL CHECK (resulting_owner_state_version > 0),
    published_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > published_at),
    receipt_envelope BLOB NOT NULL,
    receipt_envelope_digest TEXT NOT NULL,
    PRIMARY KEY (run_id, source_id, source_version),
    UNIQUE (run_id, owner_index_digest, actor_index_digest, request_index_digest)
);
CREATE INDEX IF NOT EXISTS context_control_owner_publications_actor
    ON context_control_owner_publications(run_id, owner_index_digest, actor_index_digest);
CREATE TABLE IF NOT EXISTS context_control_active_publication_links (
    run_id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL,
    source_version INTEGER NOT NULL CHECK (source_version > 0),
    source_digest TEXT NOT NULL,
    owner_index_digest TEXT NOT NULL,
    actor_index_digest TEXT NOT NULL,
    request_index_digest TEXT NOT NULL,
    request_digest TEXT NOT NULL,
    publication_receipt_envelope_digest TEXT NOT NULL,
    control_receipt_envelope_digest TEXT NOT NULL,
    active_revision_id TEXT NOT NULL,
    activated_at INTEGER NOT NULL,
    envelope BLOB NOT NULL,
    envelope_digest TEXT NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::{
        MAX_CONTROL_EVENTS, MAX_OWNER_RECEIPT_BYTES, MAX_RECEIPT_ENVELOPE_OVERHEAD,
        MAX_V1_RECEIPT_SCAN_BYTES,
    };

    #[test]
    fn migration_receipt_scan_bound_covers_maximum_legacy_writer_history() {
        assert_eq!(MAX_RECEIPT_ENVELOPE_OVERHEAD, 40);
        assert_eq!(
            MAX_V1_RECEIPT_SCAN_BYTES,
            MAX_CONTROL_EVENTS as usize * (MAX_OWNER_RECEIPT_BYTES + 40)
        );
    }
}
