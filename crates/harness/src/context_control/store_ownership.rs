// SPDX-License-Identifier: MIT

//! Single-writer ownership and fencing for the durable context-control store.

use super::store::ContextControlStore;
use super::store_schema::digest;
use super::store_types::{DurableControlStoreError, MAX_JOURNAL_BYTES};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

impl ContextControlStore {
    /// Claims the single-writer fence for this store handle. Existing journal bytes are
    /// authenticated before the claim so a wrong key cannot evict a live owner. A handle may
    /// claim once; after another handle takes the fence it remains fenced for its lifetime.
    pub(super) fn claim_owner(&self) -> Result<(), DurableControlStoreError> {
        self.authenticate_existing_journal()?;
        let transaction =
            rusqlite::Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
                .map_err(|_| DurableControlStoreError::Sqlite)?;
        let current = transaction
            .query_row(
                "SELECT owner_token FROM context_control_owners WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if self.owner_claimed.get() {
            if current.as_deref() != Some(self.owner_token.as_str()) {
                return Err(DurableControlStoreError::Fenced);
            }
        } else {
            transaction
                .execute(
                    "INSERT INTO context_control_owners(run_id, owner_token)
                     VALUES (?1, ?2)
                     ON CONFLICT(run_id) DO UPDATE SET owner_token = excluded.owner_token",
                    params![self.run_id, self.owner_token],
                )
                .map_err(|_| DurableControlStoreError::Sqlite)?;
        }
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        self.owner_claimed.set(true);
        Ok(())
    }

    fn authenticate_existing_journal(&self) -> Result<(), DurableControlStoreError> {
        let Some((envelope, envelope_digest)) = self
            .connection
            .query_row(
                "SELECT envelope, envelope_digest FROM context_control_journal
                 WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
        else {
            return Ok(());
        };
        if envelope.len() > MAX_JOURNAL_BYTES || digest(&envelope) != envelope_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        self.decrypt(&envelope).map(|_| ())
    }

    pub(super) fn verify_connection_owner(&self) -> Result<(), DurableControlStoreError> {
        if !self.owner_claimed.get() {
            return Ok(());
        }
        let current = self
            .connection
            .query_row(
                "SELECT owner_token FROM context_control_owners WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if current.as_deref() == Some(self.owner_token.as_str()) {
            Ok(())
        } else {
            Err(DurableControlStoreError::Fenced)
        }
    }

    pub(super) fn verify_owner(
        transaction: &rusqlite::Transaction<'_>,
        run_id: &str,
        owner_token: &str,
    ) -> Result<(), DurableControlStoreError> {
        let current = transaction
            .query_row(
                "SELECT owner_token FROM context_control_owners WHERE run_id = ?1",
                [run_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if current.as_deref() == Some(owner_token) {
            Ok(())
        } else {
            Err(DurableControlStoreError::Fenced)
        }
    }
}
