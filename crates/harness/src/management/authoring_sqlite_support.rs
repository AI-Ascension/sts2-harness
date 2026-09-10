// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, params};

use super::super::contract_authoring::{StudioDefinitionRecord, StudioDraftRecord};
use super::super::store::StoreError;

pub(super) fn sqlite_error(error: rusqlite::Error) -> StoreError {
    StoreError::new("authoring_sqlite", error.to_string())
}

pub(super) fn to_i64(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| {
        StoreError::new(
            "authoring_integer",
            "authoring integer exceeds SQLite range",
        )
    })
}

pub(super) fn encode_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, StoreError> {
    serde_json::to_vec(value)
        .map_err(|error| StoreError::new("authoring_encode", error.to_string()))
}

pub(super) fn decode_json<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, StoreError> {
    serde_json::from_slice(bytes)
        .map_err(|error| StoreError::new("authoring_decode", error.to_string()))
}

pub(super) fn read_sqlite_draft(
    connection: &rusqlite::Connection,
    draft_id: &str,
) -> Result<Option<StudioDraftRecord>, StoreError> {
    connection
        .query_row(
            "SELECT schema_version, draft_id, definition_id, revision, etag, document, layout, updated_at
             FROM studio_drafts WHERE draft_id = ?1",
            params![draft_id],
            |row| {
                let document: Vec<u8> = row.get(5)?;
                let layout: Vec<u8> = row.get(6)?;
                Ok(StudioDraftRecord {
                    schema_version: row.get(0)?,
                    draft_id: row.get(1)?,
                    definition_id: row.get(2)?,
                    revision: from_i64(row.get(3)?)?,
                    etag: row.get(4)?,
                    document: decode_json(&document).map_err(to_sql_error)?,
                    layout: decode_json(&layout).map_err(to_sql_error)?,
                    updated_at: row.get(7)?,
                    conflict: None,
                })
            },
        )
        .optional()
        .map_err(sqlite_error)
}

pub(super) fn insert_sqlite_draft(
    connection: &rusqlite::Connection,
    draft: &StudioDraftRecord,
) -> Result<(), StoreError> {
    connection
        .execute(
            "INSERT INTO studio_drafts (schema_version, draft_id, definition_id, revision, etag, document, layout, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                draft.schema_version,
                draft.draft_id,
                draft.definition_id,
                to_i64(draft.revision)?,
                draft.etag,
                encode_json(&draft.document)?,
                encode_json(&draft.layout)?,
                draft.updated_at
            ],
        )
        .map_err(sqlite_error)?;
    Ok(())
}

pub(super) fn decode_definition_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<StudioDefinitionRecord> {
    let definition: Vec<u8> = row.get(7)?;
    Ok(StudioDefinitionRecord {
        schema_version: row.get(0)?,
        id: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        source: row.get(4)?,
        version: row.get(5)?,
        definition_digest: row.get(6)?,
        definition: decode_json(&definition).map_err(to_sql_error)?,
        published_revision: from_i64(row.get(8)?)?,
    })
}

pub(super) fn read_sqlite_definition(
    connection: &rusqlite::Connection,
    digest: &str,
) -> Result<Option<StudioDefinitionRecord>, StoreError> {
    connection
        .query_row(
            "SELECT schema_version, definition_id, title, description, source, version, definition_digest, definition, published_revision
             FROM studio_publications WHERE definition_digest = ?1",
            params![digest],
            decode_definition_row,
        )
        .optional()
        .map_err(sqlite_error)
}

fn from_i64(value: i64) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|_| rusqlite::Error::InvalidQuery)
}

fn to_sql_error(error: StoreError) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(error))
}
