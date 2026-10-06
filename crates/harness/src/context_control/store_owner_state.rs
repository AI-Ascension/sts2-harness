// SPDX-License-Identifier: MIT

//! Encrypted, compare-and-swap storage for additive context-owner drafts.

use super::super::store_schema::{digest, now_seconds};
use super::super::store_types::{
    DurableControlStoreError, DurableOwnerContextState, MAX_OWNER_CONTEXT_STATE_BYTES,
};
use super::ContextControlStore;
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl ContextControlStore {
    /// Reads the encrypted owner-local draft state without requiring an active
    /// authority fence. The production owner still authenticates the actor and
    /// current run before projecting any data; this read-only path also permits
    /// exact same-actor receipt recovery after a binding epoch becomes inactive.
    pub fn load_owner_context_state(
        &self,
        owner_id: &str,
    ) -> Result<Option<DurableOwnerContextState>, DurableControlStoreError> {
        validate_owner_id(owner_id)?;
        let row = self
            .connection
            .query_row(
                "SELECT owner_id, record_version, envelope, envelope_digest
                 FROM context_control_owner_state WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let Some((stored_owner, record_version, envelope, envelope_digest)) = row else {
            return Ok(None);
        };
        if stored_owner != owner_id {
            return Err(DurableControlStoreError::ScopeMismatch);
        }
        if record_version <= 0
            || envelope.len() > MAX_OWNER_CONTEXT_STATE_BYTES + 40
            || digest(&envelope) != envelope_digest
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        let aad = owner_state_aad(&self.run_id, owner_id)?;
        let bytes = self.decrypt_with_aad(&envelope, &aad)?;
        if bytes.is_empty() || bytes.len() > MAX_OWNER_CONTEXT_STATE_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        Ok(Some(DurableOwnerContextState {
            record_version: u64::try_from(record_version)
                .map_err(|_| DurableControlStoreError::Corrupt)?,
            bytes,
        }))
    }

    /// Atomically replaces one encrypted owner state at its expected database
    /// version. Version zero means no record exists. Draft changes and their
    /// terminal mutation receipt must be encoded in the same bounded payload.
    pub fn compare_exchange_owner_context_state(
        &mut self,
        owner_id: &str,
        expected_version: u64,
        bytes: &[u8],
    ) -> Result<u64, DurableControlStoreError> {
        validate_owner_id(owner_id)?;
        if bytes.is_empty() || bytes.len() > MAX_OWNER_CONTEXT_STATE_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let next_version = expected_version
            .checked_add(1)
            .ok_or(DurableControlStoreError::TooLarge)?;
        let next_version_i64 =
            i64::try_from(next_version).map_err(|_| DurableControlStoreError::TooLarge)?;
        let aad = owner_state_aad(&self.run_id, owner_id)?;
        let encrypted = self.encrypt_with_aad(bytes, &aad)?;
        let envelope_digest = digest(&encrypted);
        self.claim_owner()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Self::verify_owner(&transaction, &self.run_id, &self.owner_token)?;
        let current = transaction
            .query_row(
                "SELECT owner_id, record_version FROM context_control_owner_state WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        match current {
            None if expected_version == 0 => {
                transaction
                    .execute(
                        "INSERT INTO context_control_owner_state
                            (run_id, owner_id, record_version, envelope, envelope_digest, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            self.run_id,
                            owner_id,
                            next_version_i64,
                            encrypted,
                            envelope_digest,
                            now_seconds(),
                        ],
                    )
                    .map_err(|_| DurableControlStoreError::Sqlite)?;
            }
            Some((stored_owner, stored_version))
                if stored_owner == owner_id
                    && u64::try_from(stored_version).ok() == Some(expected_version) =>
            {
                let changed = transaction
                    .execute(
                        "UPDATE context_control_owner_state
                         SET record_version = ?1, envelope = ?2, envelope_digest = ?3, updated_at = ?4
                         WHERE run_id = ?5 AND owner_id = ?6 AND record_version = ?7",
                        params![
                            next_version_i64,
                            encrypted,
                            envelope_digest,
                            now_seconds(),
                            self.run_id,
                            owner_id,
                            stored_version,
                        ],
                    )
                    .map_err(|_| DurableControlStoreError::Sqlite)?;
                if changed != 1 {
                    return Err(DurableControlStoreError::OwnerContextConflict);
                }
            }
            Some((stored_owner, _)) if stored_owner != owner_id => {
                return Err(DurableControlStoreError::ScopeMismatch);
            }
            _ => return Err(DurableControlStoreError::OwnerContextConflict),
        }
        if self.failpoint == Some(super::super::store_types::DurableStoreFailpoint::BeforeCommit) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Ok(next_version)
    }
}

fn validate_owner_id(owner_id: &str) -> Result<(), DurableControlStoreError> {
    if owner_id.is_empty()
        || owner_id.len() > 128
        || !owner_id.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
        })
    {
        return Err(DurableControlStoreError::ScopeMismatch);
    }
    Ok(())
}

fn owner_state_aad(run_id: &str, owner_id: &str) -> Result<Vec<u8>, DurableControlStoreError> {
    let mut aad = b"ascension.context-control.owner-state.v1\0".to_vec();
    for component in [run_id.as_bytes(), owner_id.as_bytes()] {
        let length =
            u64::try_from(component.len()).map_err(|_| DurableControlStoreError::TooLarge)?;
        aad.extend_from_slice(&length.to_be_bytes());
        aad.extend_from_slice(component);
    }
    Ok(aad)
}
