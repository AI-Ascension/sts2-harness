// SPDX-License-Identifier: MIT

use super::types::*;
use rusqlite::Connection;
use serde::Serialize;
use std::io::Write;
use std::path::Path;

pub(super) const SCHEMA: &str = "
CREATE TABLE policy_store_meta(version INTEGER NOT NULL);
INSERT INTO policy_store_meta VALUES(1);
CREATE TABLE policy_journal(
 id INTEGER PRIMARY KEY CHECK(id=1), epoch INTEGER NOT NULL,
 envelope BLOB NOT NULL CHECK(length(envelope)<=16777256));
";

pub(super) fn check_file(path: &Path) -> Result<(), PolicyOwnerError> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_POLICY_DATABASE_BYTES => {
            Err(PolicyOwnerError::Capacity)
        }
        Ok(metadata) if !metadata.is_file() => Err(PolicyOwnerError::StoreIncompatible),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(PolicyOwnerError::Unavailable),
    }
}

pub(super) fn check_pages(connection: &Connection) -> Result<(), PolicyOwnerError> {
    let page_size: u64 = connection
        .query_row("PRAGMA page_size", [], |row| unsigned(row, 0))
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    let count: u64 = connection
        .query_row("PRAGMA page_count", [], |row| unsigned(row, 0))
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    if page_size != 4096
        || count > 8192
        || page_size
            .checked_mul(count)
            .is_none_or(|bytes| bytes > MAX_POLICY_DATABASE_BYTES)
    {
        return Err(PolicyOwnerError::Capacity);
    }
    Ok(())
}

pub(super) fn has_schema(connection: &Connection) -> Result<bool, PolicyOwnerError> {
    let count: u64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'",
            [],
            |row| unsigned(row, 0),
        )
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    if count == 0 {
        return Ok(false);
    }
    let recognized: u64 = connection
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table'
         AND name IN ('policy_store_meta', 'policy_journal')",
            [],
            |row| unsigned(row, 0),
        )
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    if count != 2 || recognized != 2 {
        return Err(PolicyOwnerError::StoreIncompatible);
    }
    Ok(true)
}

pub(super) fn check_schema(connection: &Connection) -> Result<(), PolicyOwnerError> {
    if !has_schema(connection)? {
        return Err(PolicyOwnerError::StoreIncompatible);
    }
    let count: u64 = connection
        .query_row("SELECT count(*) FROM policy_store_meta", [], |row| {
            unsigned(row, 0)
        })
        .map_err(|_| PolicyOwnerError::StoreIncompatible)?;
    if count != 1 {
        return Err(PolicyOwnerError::StoreIncompatible);
    }
    let version: u64 = connection
        .query_row("SELECT version FROM policy_store_meta", [], |row| {
            unsigned(row, 0)
        })
        .map_err(|_| PolicyOwnerError::StoreIncompatible)?;
    if version != 1 {
        return Err(PolicyOwnerError::StoreIncompatible);
    }
    Ok(())
}

pub(super) fn read_envelope(connection: &Connection) -> Result<(u64, Vec<u8>), PolicyOwnerError> {
    let count: u64 = connection
        .query_row("SELECT count(*) FROM policy_journal", [], |row| {
            unsigned(row, 0)
        })
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    if count != 1 {
        return Err(PolicyOwnerError::Corrupt);
    }
    // Ask SQLite for length before retrieving or allocating the untrusted BLOB.
    let (id, length): (u64, u64) = connection
        .query_row(
            "SELECT id, length(envelope) FROM policy_journal",
            [],
            |row| Ok((unsigned(row, 0)?, unsigned(row, 1)?)),
        )
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    if id != 1 || !(40..=MAX_POLICY_JOURNAL_BYTES as u64 + 40).contains(&length) {
        return Err(PolicyOwnerError::Capacity);
    }
    connection
        .query_row(
            "SELECT epoch, envelope FROM policy_journal
             WHERE id=1 AND typeof(envelope)='blob' AND length(envelope) BETWEEN 40 AND ?1",
            [MAX_POLICY_JOURNAL_BYTES as i64 + 40],
            |row| Ok((unsigned(row, 0)?, row.get(1)?)),
        )
        .map_err(|_| PolicyOwnerError::Corrupt)
}

fn unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

pub(super) fn encode_bounded(value: &impl Serialize) -> Result<Vec<u8>, PolicyOwnerError> {
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > MAX_POLICY_JOURNAL_BYTES {
                return Err(std::io::Error::other("policy journal capacity"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Bounded(Vec::new());
    serde_json::to_writer(&mut writer, value).map_err(|_| PolicyOwnerError::Capacity)?;
    Ok(writer.0)
}
