// SPDX-License-Identifier: MIT

pub(super) fn authenticate_v1_before_migration(
    connection: &Connection,
    key: &[u8; 32],
    run_id: &str,
) -> Result<(), DurableControlStoreError> {
    let has_metadata = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'context_control_meta'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .is_some();
    if !has_metadata {
        return Ok(());
    }
    let marker = connection
        .query_row(
            "SELECT value FROM context_control_meta WHERE key = 'schema'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let version = connection
        .query_row(
            "SELECT value FROM context_control_meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .and_then(|value| value.parse::<i64>().ok());
    if marker.as_deref() != Some(LEGACY_STORE_SCHEMA) || version != Some(1) {
        return Ok(());
    }

    let journal = if table_exists(connection, "context_control_journal")? {
        connection
            .query_row(
                "SELECT envelope, envelope_digest FROM context_control_journal WHERE run_id = ?1",
                [run_id],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
    } else {
        None
    };
    let Some((envelope, stored_digest)) = journal else {
        if database_has_run_data(connection)? {
            return Err(DurableControlStoreError::MigrationRequired);
        }
        return Ok(());
    };
    validate_and_decrypt(
        key,
        &envelope,
        &stored_digest,
        MAX_JOURNAL_BYTES,
        super::store_types::AAD,
    )?;

    authenticate_source_rows(connection, key, run_id)?;
    authenticate_receipt_rows(connection, key, run_id)?;
    authenticate_lifetime_row(connection, key, run_id)?;

    // A v1 marker cannot own v2 draft bytes. If a non-atomic historical attempt left this table
    // with data, fail closed rather than adopting bytes whose transaction/version is unknown.
    if table_exists(connection, "context_control_owner_state")?
        && connection
            .query_row(
                "SELECT COUNT(*) FROM context_control_owner_state",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?
            != 0
    {
        return Err(DurableControlStoreError::Incompatible);
    }
    Ok(())
}

fn database_has_run_data(connection: &Connection) -> Result<bool, DurableControlStoreError> {
    const TABLES: [&str; 11] = [
        "context_control_journal",
        "context_control_owners",
        "context_control_outbox",
        "context_control_phase1_snapshots",
        "context_control_owner_receipts",
        "context_control_context_sources",
        "context_control_active_context_source",
        "context_control_lifetime",
        "context_control_owner_state",
        "context_control_owner_publications",
        "context_control_active_publication_links",
    ];
    for table in TABLES {
        if !table_exists(connection, table)? {
            continue;
        }
        let query = format!("SELECT EXISTS(SELECT 1 FROM {table} LIMIT 1)");
        let has_rows = connection
            .query_row(&query, [], |row| row.get::<_, bool>(0))
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if has_rows {
            return Ok(true);
        }
    }
    Ok(false)
}

fn authenticate_source_rows(
    connection: &Connection,
    key: &[u8; 32],
    run_id: &str,
) -> Result<(), DurableControlStoreError> {
    if !table_exists(connection, "context_control_context_sources")? {
        return Ok(());
    }
    let mut statement = connection
        .prepare(
            "SELECT source_id, version, source_digest, envelope, envelope_digest
             FROM context_control_context_sources WHERE run_id = ?1 LIMIT 17",
        )
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let mut rows = statement
        .query([run_id])
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let mut count = 0_usize;
    while let Some(row) = rows.next().map_err(|_| DurableControlStoreError::Sqlite)? {
        count += 1;
        if count > 16 {
            return Err(DurableControlStoreError::TooLarge);
        }
        let source_id = row
            .get::<_, String>(0)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let version = row
            .get::<_, i64>(1)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let source_digest = row
            .get::<_, String>(2)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let envelope = row
            .get::<_, Vec<u8>>(3)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let stored_digest = row
            .get::<_, String>(4)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let version = u64::try_from(version).map_err(|_| DurableControlStoreError::Corrupt)?;
        let plaintext = validate_and_decrypt(
            key,
            &envelope,
            &stored_digest,
            MAX_CONTEXT_SOURCE_BYTES,
            &source_aad(run_id, &source_id, version),
        )?;
        if digest(&plaintext) != source_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
    }
    Ok(())
}

fn authenticate_receipt_rows(
    connection: &Connection,
    key: &[u8; 32],
    run_id: &str,
) -> Result<(), DurableControlStoreError> {
    if !table_exists(connection, "context_control_owner_receipts")? {
        return Ok(());
    }
    // Every production control receipt follows one bounded authority transition. This is the
    // exact source-derived ceiling, while the per-record byte ceiling comes from the existing
    // receipt writer. Rows are decrypted one at a time, so the maximum scan is finite without
    // collecting legacy history in memory.
    const MAX_RECEIPTS: usize = MAX_CONTROL_EVENTS as usize;
    const MAX_RECEIPT_BYTES_TOTAL: usize = MAX_V1_RECEIPT_SCAN_BYTES;
    let mut statement = connection
        .prepare(
            "SELECT owner_id, command_digest, envelope, envelope_digest
             FROM context_control_owner_receipts WHERE run_id = ?1 LIMIT 4097",
        )
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let mut rows = statement
        .query([run_id])
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    let mut count = 0_usize;
    let mut total_bytes = 0_usize;
    while let Some(row) = rows.next().map_err(|_| DurableControlStoreError::Sqlite)? {
        count += 1;
        if count > MAX_RECEIPTS {
            return Err(DurableControlStoreError::TooLarge);
        }
        let owner_id = row
            .get::<_, String>(0)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let command_digest = row
            .get::<_, String>(1)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let envelope = row
            .get::<_, Vec<u8>>(2)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let stored_digest = row
            .get::<_, String>(3)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        total_bytes = total_bytes
            .checked_add(envelope.len())
            .ok_or(DurableControlStoreError::TooLarge)?;
        if total_bytes > MAX_RECEIPT_BYTES_TOTAL {
            return Err(DurableControlStoreError::TooLarge);
        }
        validate_and_decrypt(
            key,
            &envelope,
            &stored_digest,
            MAX_OWNER_RECEIPT_BYTES,
            &owner_receipt_aad(run_id, &owner_id, &command_digest)?,
        )?;
    }
    Ok(())
}

fn authenticate_lifetime_row(
    connection: &Connection,
    key: &[u8; 32],
    run_id: &str,
) -> Result<(), DurableControlStoreError> {
    if !table_exists(connection, "context_control_lifetime")? {
        return Ok(());
    }
    if let Some((envelope, stored_digest)) = connection
        .query_row(
            "SELECT envelope, envelope_digest FROM context_control_lifetime WHERE run_id = ?1",
            [run_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
    {
        validate_and_decrypt(
            key,
            &envelope,
            &stored_digest,
            MAX_LIFETIME_STATE_BYTES,
            LIFETIME_AAD,
        )?;
    }
    Ok(())
}

fn validate_and_decrypt(
    key: &[u8; 32],
    envelope: &[u8],
    expected_digest: &str,
    max_plaintext_bytes: usize,
    aad: &[u8],
) -> Result<Vec<u8>, DurableControlStoreError> {
    if envelope.len() > max_plaintext_bytes.saturating_add(64)
        || digest(envelope) != expected_digest
    {
        return Err(DurableControlStoreError::Corrupt);
    }
    let plaintext = decrypt_with_key(key, envelope, aad)?;
    if plaintext.len() > max_plaintext_bytes {
        return Err(DurableControlStoreError::TooLarge);
    }
    Ok(plaintext)
}

fn table_exists(connection: &Connection, name: &str) -> Result<bool, DurableControlStoreError> {
    connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|_| DurableControlStoreError::Sqlite)
}
