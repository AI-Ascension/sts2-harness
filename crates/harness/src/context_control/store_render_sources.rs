// SPDX-License-Identifier: MIT

//! Encrypted immutable context-source snapshots and their committed activation.

use super::store::{ContextControlStore, decrypt_with_key};
use super::store_schema::digest;
use super::store_types::{
    DurableActiveContextSource, DurableContextSourceSnapshot, DurableControlStoreError,
    MAX_CONTEXT_SOURCE_BYTES,
};
use rusqlite::{OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};

const MAX_SOURCES_PER_RUN: i64 = 16;

impl ContextControlStore {
    /// Stores owner-published bytes once. Repeating the same source identity and bytes is
    /// idempotent; the same identity with different bytes is rejected.
    pub fn publish_context_source(
        &mut self,
        source: &DurableContextSourceSnapshot,
    ) -> Result<(), DurableControlStoreError> {
        validate_source_identity(&source.source_id, source.version, &source.digest)?;
        let plaintext =
            serde_json::to_vec(&source.document).map_err(|_| DurableControlStoreError::Encode)?;
        if plaintext.len() > MAX_CONTEXT_SOURCE_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        if sha256_hex(&plaintext) != source.digest {
            return Err(DurableControlStoreError::SourceConflict);
        }
        let aad = source_aad(&self.run_id, &source.source_id, source.version);
        let envelope = self.encrypt_with_aad(&plaintext, &aad)?;
        let envelope_digest = digest(&envelope);
        self.claim_owner()?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Self::verify_owner(&transaction, &self.run_id, &self.owner_token)?;
        let existing = transaction
            .query_row(
                "SELECT source_digest, envelope, envelope_digest
                 FROM context_control_context_sources
                 WHERE run_id = ?1 AND source_id = ?2 AND version = ?3",
                params![self.run_id, source.source_id, source.version as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if let Some((stored_digest, stored_envelope, stored_envelope_digest)) = existing {
            if stored_digest != source.digest
                || digest(&stored_envelope) != stored_envelope_digest
                || decrypt_with_key(&self.key, &stored_envelope, &aad)? != plaintext
            {
                return Err(DurableControlStoreError::SourceConflict);
            }
            transaction
                .commit()
                .map_err(|_| DurableControlStoreError::Sqlite)?;
            return Ok(());
        }
        let count = transaction
            .query_row(
                "SELECT COUNT(*) FROM context_control_context_sources WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if count >= MAX_SOURCES_PER_RUN {
            return Err(DurableControlStoreError::TooLarge);
        }
        transaction
            .execute(
                "INSERT INTO context_control_context_sources
                    (run_id, source_id, version, source_digest, envelope, envelope_digest)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    self.run_id,
                    source.source_id,
                    source.version as i64,
                    source.digest,
                    envelope,
                    envelope_digest
                ],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)
    }

    pub fn load_context_source(
        &self,
        source_id: &str,
        version: u64,
        expected_digest: &str,
    ) -> Result<Option<DurableContextSourceSnapshot>, DurableControlStoreError> {
        validate_source_identity(source_id, version, expected_digest)?;
        self.verify_connection_owner()?;
        let record = self
            .connection
            .query_row(
                "SELECT source_digest, envelope, envelope_digest
                 FROM context_control_context_sources
                 WHERE run_id = ?1 AND source_id = ?2 AND version = ?3",
                params![self.run_id, source_id, version as i64],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let Some((source_digest, envelope, envelope_digest)) = record else {
            return Ok(None);
        };
        if source_digest != expected_digest || digest(&envelope) != envelope_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        let plaintext =
            self.decrypt_with_aad(&envelope, &source_aad(&self.run_id, source_id, version))?;
        if plaintext.len() > MAX_CONTEXT_SOURCE_BYTES || sha256_hex(&plaintext) != source_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        let document =
            serde_json::from_slice(&plaintext).map_err(|_| DurableControlStoreError::Decode)?;
        Ok(Some(DurableContextSourceSnapshot {
            source_id: source_id.to_owned(),
            version,
            digest: source_digest,
            document,
        }))
    }

    /// Returns the committed source only when it is attached to the authority's current revision.
    pub fn active_context_source(
        &self,
        active_revision_id: &str,
    ) -> Result<
        Option<(DurableActiveContextSource, DurableContextSourceSnapshot)>,
        DurableControlStoreError,
    > {
        self.verify_connection_owner()?;
        let active = self
            .connection
            .query_row(
                "SELECT source_id, version, source_digest, active_revision_id
                 FROM context_control_active_context_source WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| {
                    Ok(DurableActiveContextSource {
                        source_id: row.get(0)?,
                        version: row.get::<_, i64>(1)? as u64,
                        digest: row.get(2)?,
                        active_revision_id: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let Some(active) = active else {
            return Ok(None);
        };
        if active.active_revision_id != active_revision_id {
            return Err(DurableControlStoreError::Corrupt);
        }
        let source = self
            .load_context_source(&active.source_id, active.version, &active.digest)?
            .ok_or(DurableControlStoreError::Corrupt)?;
        Ok(Some((active, source)))
    }
}

/// Computes the canonical digest used by an owner source allowlist.
pub fn context_source_digest(
    document: &super::types::ContextSourceDocument,
) -> Result<String, DurableControlStoreError> {
    let bytes = serde_json::to_vec(document).map_err(|_| DurableControlStoreError::Encode)?;
    if bytes.len() > MAX_CONTEXT_SOURCE_BYTES {
        return Err(DurableControlStoreError::TooLarge);
    }
    Ok(sha256_hex(&bytes))
}

pub(super) fn persist_active_source(
    transaction: &Transaction<'_>,
    run_id: &str,
    source: &DurableActiveContextSource,
) -> Result<(), DurableControlStoreError> {
    validate_source_identity(&source.source_id, source.version, &source.digest)?;
    if source.active_revision_id.is_empty() {
        return Err(DurableControlStoreError::InvalidSourceId);
    }
    let stored_digest = transaction
        .query_row(
            "SELECT source_digest FROM context_control_context_sources
             WHERE run_id = ?1 AND source_id = ?2 AND version = ?3",
            params![run_id, source.source_id, source.version as i64],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|_| DurableControlStoreError::Sqlite)?
        .ok_or(DurableControlStoreError::Missing)?;
    if stored_digest != source.digest {
        return Err(DurableControlStoreError::SourceConflict);
    }
    transaction
        .execute(
            "INSERT INTO context_control_active_context_source
                (run_id, source_id, version, source_digest, active_revision_id)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(run_id) DO UPDATE SET
                source_id = excluded.source_id,
                version = excluded.version,
                source_digest = excluded.source_digest,
                active_revision_id = excluded.active_revision_id",
            params![
                run_id,
                source.source_id,
                source.version as i64,
                source.digest,
                source.active_revision_id
            ],
        )
        .map_err(|_| DurableControlStoreError::Sqlite)?;
    Ok(())
}

fn validate_source_identity(
    source_id: &str,
    version: u64,
    digest: &str,
) -> Result<(), DurableControlStoreError> {
    if !valid_source_id(source_id)
        || version == 0
        || digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(DurableControlStoreError::InvalidSourceId);
    }
    Ok(())
}

fn valid_source_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
}

fn source_aad(run_id: &str, source_id: &str, version: u64) -> Vec<u8> {
    format!("ascension.context-control.source.v1\0{run_id}\0{source_id}\0{version}").into_bytes()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
