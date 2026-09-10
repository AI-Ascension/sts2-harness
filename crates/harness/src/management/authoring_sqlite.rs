// SPDX-License-Identifier: MIT

use rusqlite::{OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use super::super::contract_authoring::{StudioDefinitionRecord, StudioDraftRecord};
use super::super::store::{SqliteWorkflowStore, StoreError};
use super::sqlite_support::{
    decode_definition_row, decode_json, encode_json, insert_sqlite_draft, read_sqlite_definition,
    read_sqlite_draft, sqlite_error, to_i64,
};
use super::validation::{
    conflict_draft, definition_digest, draft_record, mutation_digest, validate_digest_text,
    validate_draft, validate_id, validate_payload,
};
use super::{AuthoringStore, PublishResult};

impl AuthoringStore for SqliteWorkflowStore {
    fn list_definitions(&self) -> Result<Vec<StudioDefinitionRecord>, StoreError> {
        let connection = self.connection.lock().map_err(|_| {
            StoreError::new(
                "authoring_store_poisoned",
                "authoring database lock is poisoned",
            )
        })?;
        let mut statement = connection
            .prepare(
                "SELECT schema_version, definition_id, title, description, source, version, definition_digest, definition, published_revision
                 FROM studio_publications ORDER BY definition_id, published_revision",
            )
            .map_err(sqlite_error)?;
        let rows = statement
            .query_map([], decode_definition_row)
            .map_err(sqlite_error)?;
        rows.map(|row| row.map_err(sqlite_error)).collect()
    }

    fn get_draft(&self, draft_id: &str) -> Result<Option<StudioDraftRecord>, StoreError> {
        validate_id("draft_id", draft_id)?;
        let connection = self.connection.lock().map_err(|_| {
            StoreError::new(
                "authoring_store_poisoned",
                "authoring database lock is poisoned",
            )
        })?;
        read_sqlite_draft(&connection, draft_id)
    }

    fn create_draft(
        &self,
        draft: StudioDraftRecord,
        mutation_id: &str,
    ) -> Result<StudioDraftRecord, StoreError> {
        validate_draft(&draft)?;
        validate_id("client_mutation_id", mutation_id)?;
        let digest = mutation_digest(&draft.document, &draft.layout)?;
        let mut connection = self.connection.lock().map_err(|_| {
            StoreError::new(
                "authoring_store_poisoned",
                "authoring database lock is poisoned",
            )
        })?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        if let Some(existing) = read_sqlite_draft(&tx, &draft.draft_id)? {
            let previous: Option<String> = tx
                .query_row(
                    "SELECT mutation_digest FROM studio_draft_mutations WHERE draft_id = ?1 AND mutation_id = ?2",
                    params![draft.draft_id, mutation_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            if previous.as_deref() == Some(digest.as_str()) {
                tx.commit().map_err(sqlite_error)?;
                return Ok(existing);
            }
            tx.commit().map_err(sqlite_error)?;
            return Ok(conflict_draft(existing));
        }
        insert_sqlite_draft(&tx, &draft)?;
        tx.execute(
            "INSERT INTO studio_draft_mutations (draft_id, mutation_id, mutation_digest, revision, response)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                draft.draft_id,
                mutation_id,
                digest,
                to_i64(draft.revision)?,
                encode_json(&draft)?
            ],
        )
        .map_err(sqlite_error)?;
        tx.commit().map_err(sqlite_error)?;
        Ok(draft)
    }

    fn save_draft(
        &self,
        draft_id: &str,
        expected_revision: u64,
        expected_etag: &str,
        mutation_id: &str,
        document: Value,
        layout: Value,
    ) -> Result<StudioDraftRecord, StoreError> {
        validate_id("draft_id", draft_id)?;
        validate_id("client_mutation_id", mutation_id)?;
        validate_payload(&document, &layout)?;
        let digest = mutation_digest(&document, &layout)?;
        let mut connection = self.connection.lock().map_err(|_| {
            StoreError::new(
                "authoring_store_poisoned",
                "authoring database lock is poisoned",
            )
        })?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let current = read_sqlite_draft(&tx, draft_id)?
            .ok_or_else(|| StoreError::new("draft_not_found", "Studio draft was not found"))?;
        let previous: Option<(String, Vec<u8>)> = tx
            .query_row(
                "SELECT mutation_digest, response FROM studio_draft_mutations WHERE draft_id = ?1 AND mutation_id = ?2",
                params![draft_id, mutation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(sqlite_error)?;
        if let Some((previous_digest, response)) = previous {
            if previous_digest == digest {
                let record = decode_json::<StudioDraftRecord>(&response)?;
                tx.commit().map_err(sqlite_error)?;
                return Ok(record);
            }
            tx.commit().map_err(sqlite_error)?;
            return Ok(conflict_draft(current));
        }
        if current.revision != expected_revision || current.etag != expected_etag {
            tx.commit().map_err(sqlite_error)?;
            return Ok(conflict_draft(current));
        }
        let revision = current.revision.checked_add(1).ok_or_else(|| {
            StoreError::new("draft_revision_overflow", "draft revision overflowed")
        })?;
        let next = draft_record(
            &current.draft_id,
            &current.definition_id,
            revision,
            document,
            layout,
        )?;
        tx.execute(
            "UPDATE studio_drafts SET revision = ?2, etag = ?3, document = ?4, layout = ?5, updated_at = ?6 WHERE draft_id = ?1",
            params![
                next.draft_id,
                to_i64(next.revision)?,
                next.etag,
                encode_json(&next.document)?,
                encode_json(&next.layout)?,
                next.updated_at
            ],
        )
        .map_err(sqlite_error)?;
        tx.execute(
            "INSERT INTO studio_draft_mutations (draft_id, mutation_id, mutation_digest, revision, response)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                next.draft_id,
                mutation_id,
                digest,
                to_i64(next.revision)?,
                encode_json(&next)?
            ],
        )
        .map_err(sqlite_error)?;
        tx.commit().map_err(sqlite_error)?;
        Ok(next)
    }

    fn publish_draft(
        &self,
        draft_id: &str,
        expected_revision: u64,
        expected_etag: &str,
        expected_definition_digest: &str,
    ) -> Result<PublishResult, StoreError> {
        validate_id("draft_id", draft_id)?;
        validate_digest_text(expected_definition_digest)?;
        let mut connection = self.connection.lock().map_err(|_| {
            StoreError::new(
                "authoring_store_poisoned",
                "authoring database lock is poisoned",
            )
        })?;
        let tx = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sqlite_error)?;
        let current = read_sqlite_draft(&tx, draft_id)?
            .ok_or_else(|| StoreError::new("draft_not_found", "Studio draft was not found"))?;
        if current.revision != expected_revision || current.etag != expected_etag {
            tx.commit().map_err(sqlite_error)?;
            return Ok(PublishResult::Conflict(conflict_draft(current)));
        }
        let digest = definition_digest(&current.document)?;
        if digest != expected_definition_digest {
            tx.commit().map_err(sqlite_error)?;
            return Ok(PublishResult::Conflict(conflict_draft(current)));
        }
        if let Some(existing) = read_sqlite_definition(&tx, &digest)? {
            tx.commit().map_err(sqlite_error)?;
            return Ok(PublishResult::AlreadyPublished(existing));
        }
        let definition = super::validation::published_definition(&current, &digest)?;
        tx.execute(
            "INSERT INTO studio_publications (schema_version, definition_id, title, description, source, version, definition_digest, definition, published_revision)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                definition.schema_version,
                definition.id,
                definition.title,
                definition.description,
                definition.source,
                definition.version,
                definition.definition_digest,
                encode_json(&definition.definition)?,
                to_i64(definition.published_revision)?
            ],
        )
        .map_err(sqlite_error)?;
        tx.commit().map_err(sqlite_error)?;
        Ok(PublishResult::Published(definition))
    }
}
