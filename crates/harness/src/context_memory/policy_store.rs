// SPDX-License-Identifier: MIT

use super::{records::PolicyJournal, store_schema::*, types::*};
use crate::context_memory::MemoryScope;
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rusqlite::{Connection, TransactionBehavior};
use std::path::{Path, PathBuf};
use zeroize::Zeroize;

#[cfg(test)]
#[path = "policy_store_quota.rs"]
mod quota;

#[cfg(test)]
#[path = "policy_store_write_tests.rs"]
mod write_tests;

/// Construction-time retention choice; not a request field or a grant to collect private data.
pub enum PolicyStoreConsent {
    SyntheticOnly,
    /// Caller must already hold the independently accepted private-retention policy.
    ApprovedPrivate {
        policy_ref: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyStoreFailpoint {
    BeforeCommit,
    AfterCommit,
}

pub(super) struct PolicyStore {
    connection: Connection,
    path: PathBuf,
    key: [u8; 32],
    scope: MemoryScope,
    epoch: u64,
    authority_lease_active: bool,
    pub failpoint: Option<PolicyStoreFailpoint>,
}

impl Drop for PolicyStore {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

impl PolicyStore {
    pub fn open(
        path: &Path,
        key: [u8; 32],
        scope: MemoryScope,
        consent: PolicyStoreConsent,
    ) -> Result<Self, PolicyOwnerError> {
        if key.iter().all(|byte| *byte == 0) {
            return Err(PolicyOwnerError::PermissionDenied);
        }
        if let PolicyStoreConsent::ApprovedPrivate { policy_ref } = consent
            && !valid_id(&policy_ref)
        {
            return Err(PolicyOwnerError::PermissionDenied);
        }
        check_file(path)?;
        let connection = Connection::open(path).map_err(|_| PolicyOwnerError::Unavailable)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(1))
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        check_pages(&connection)?;
        let existing = has_schema(&connection)?;
        if existing {
            check_schema(&connection)?;
        }
        connection
            .execute_batch(
                "PRAGMA page_size=4096; PRAGMA max_page_count=8192;
             PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA temp_store=MEMORY;",
            )
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let mut store = Self {
            connection,
            path: path.to_owned(),
            key,
            scope,
            epoch: 0,
            authority_lease_active: false,
            failpoint: None,
        };
        store.claim(existing)?;
        Ok(store)
    }

    fn claim(&mut self, existing: bool) -> Result<(), PolicyOwnerError> {
        // Authenticate before taking ownership. Re-read under IMMEDIATE so an intervening
        // committed old-owner write cannot be overwritten with an earlier authenticated copy.
        if existing {
            self.load_unfenced()?;
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        let mut journal = if existing {
            check_schema(&transaction)?;
            let (epoch, envelope) = read_envelope(&transaction)?;
            let journal = decrypt(&self.key, &self.scope, &envelope)?;
            if journal.store_epoch != epoch {
                return Err(PolicyOwnerError::Corrupt);
            }
            journal
        } else {
            if has_schema(&transaction)? {
                return Err(PolicyOwnerError::StoreIncompatible);
            }
            transaction
                .execute_batch(SCHEMA)
                .map_err(|_| PolicyOwnerError::Unavailable)?;
            PolicyJournal::empty(self.scope.clone())
        };
        if existing {
            journal.store_epoch = journal
                .store_epoch
                .checked_add(1)
                .filter(|value| *value <= 9_007_199_254_740_991)
                .ok_or(PolicyOwnerError::Capacity)?;
        }
        let envelope = encrypt(&self.key, &self.scope, &journal)?;
        write_envelope(&transaction, journal.store_epoch, &envelope)?;
        check_schema(&transaction)?;
        transaction
            .commit()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        self.epoch = journal.store_epoch;
        Ok(())
    }

    fn load_unfenced(&self) -> Result<PolicyJournal, PolicyOwnerError> {
        check_file(&self.path)?;
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        check_pages(&transaction)?;
        check_schema(&transaction)?;
        let (epoch, envelope) = read_envelope(&transaction)?;
        let journal = decrypt(&self.key, &self.scope, &envelope)?;
        if epoch != journal.store_epoch {
            return Err(PolicyOwnerError::Corrupt);
        }
        transaction
            .commit()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        Ok(journal)
    }

    pub fn load(&self) -> Result<PolicyJournal, PolicyOwnerError> {
        let journal = self.load_unfenced()?;
        if journal.store_epoch != self.epoch {
            return Err(PolicyOwnerError::OwnerFenced);
        }
        Ok(journal)
    }

    pub fn change<T>(
        &mut self,
        change: impl FnOnce(&mut PolicyJournal) -> Result<T, PolicyOwnerError>,
        before_commit: impl FnOnce() -> Result<(), PolicyOwnerError>,
    ) -> Result<T, PolicyOwnerError> {
        check_file(&self.path)?;
        check_pages(&self.connection)?;
        check_schema(&self.connection)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        check_schema(&transaction)?;
        let (epoch, envelope) = read_envelope(&transaction)?;
        if epoch != self.epoch {
            return Err(PolicyOwnerError::OwnerFenced);
        }
        let mut journal = decrypt(&self.key, &self.scope, &envelope)?;
        if journal.store_epoch != epoch {
            return Err(PolicyOwnerError::Corrupt);
        }
        let result = change(&mut journal)?;
        let envelope = encrypt(&self.key, &self.scope, &journal)?;
        write_envelope(&transaction, self.epoch, &envelope)?;
        check_schema(&transaction)?;
        let failpoint = self.failpoint.take();
        if failpoint == Some(PolicyStoreFailpoint::BeforeCommit) {
            return Err(PolicyOwnerError::PersistenceFailure);
        }
        // Last check before linearization: clock advances independently of the authority mutex.
        before_commit()?;
        transaction
            .commit()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        if failpoint == Some(PolicyStoreFailpoint::AfterCommit) {
            return Err(PolicyOwnerError::LostReply);
        }
        Ok(result)
    }

    pub fn read_lease<T>(
        &mut self,
        action: impl FnOnce(&PolicyJournal) -> Result<T, PolicyOwnerError>,
    ) -> Result<T, PolicyOwnerError> {
        check_file(&self.path)?;
        check_pages(&self.connection)?;
        check_schema(&self.connection)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        check_schema(&transaction)?;
        let (epoch, envelope) = read_envelope(&transaction)?;
        if epoch != self.epoch {
            return Err(PolicyOwnerError::OwnerFenced);
        }
        let journal = decrypt(&self.key, &self.scope, &envelope)?;
        if journal.store_epoch != epoch {
            return Err(PolicyOwnerError::Corrupt);
        }
        let result = action(&journal)?;
        transaction
            .commit()
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        Ok(result)
    }

    /// Opens an immediate SQLite transaction and leaves it active while a
    /// lookup-authority guard holds this store mutex. The caller must end the
    /// lease before releasing that mutex.
    pub fn begin_authority_lease(&mut self) -> Result<PolicyJournal, PolicyOwnerError> {
        if self.authority_lease_active {
            return Err(PolicyOwnerError::Unavailable);
        }
        check_file(&self.path)?;
        check_pages(&self.connection)?;
        check_schema(&self.connection)?;
        self.connection
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|_| PolicyOwnerError::Unavailable)?;
        self.authority_lease_active = true;
        let result = (|| {
            check_schema(&self.connection)?;
            let (epoch, envelope) = read_envelope(&self.connection)?;
            if epoch != self.epoch {
                return Err(PolicyOwnerError::OwnerFenced);
            }
            let journal = decrypt(&self.key, &self.scope, &envelope)?;
            if journal.store_epoch != epoch {
                return Err(PolicyOwnerError::Corrupt);
            }
            Ok(journal)
        })();
        if result.is_err() {
            self.end_authority_lease();
        }
        result
    }

    pub fn end_authority_lease(&mut self) {
        if self.authority_lease_active {
            let _ = self.connection.execute_batch("ROLLBACK");
            self.authority_lease_active = false;
        }
    }
}

fn aad(scope: &MemoryScope) -> Result<Vec<u8>, PolicyOwnerError> {
    serde_json::to_vec(&(
        "ascension.context-memory.policy-store.v1",
        scope,
        "journal",
        1_u64,
    ))
    .map_err(|_| PolicyOwnerError::Corrupt)
}

fn encrypt(
    key: &[u8; 32],
    scope: &MemoryScope,
    journal: &PolicyJournal,
) -> Result<Vec<u8>, PolicyOwnerError> {
    journal.validate()?;
    let mut plaintext = encode_bounded(journal)?;
    let mut nonce = [0_u8; 24];
    getrandom::fill(&mut nonce).map_err(|_| PolicyOwnerError::Unavailable)?;
    let result = XChaCha20Poly1305::new(key.into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: &aad(scope)?,
            },
        )
        .map_err(|_| PolicyOwnerError::Unavailable);
    plaintext.zeroize();
    let ciphertext = result?;
    let mut envelope = nonce.to_vec();
    envelope.extend_from_slice(&ciphertext);
    Ok(envelope)
}

fn decrypt(
    key: &[u8; 32],
    scope: &MemoryScope,
    envelope: &[u8],
) -> Result<PolicyJournal, PolicyOwnerError> {
    if envelope.len() < 40 || envelope.len() > MAX_POLICY_JOURNAL_BYTES + 40 {
        return Err(PolicyOwnerError::Capacity);
    }
    let mut plaintext = XChaCha20Poly1305::new(key.into())
        .decrypt(
            XNonce::from_slice(&envelope[..24]),
            Payload {
                msg: &envelope[24..],
                aad: &aad(scope)?,
            },
        )
        .map_err(|_| PolicyOwnerError::Corrupt)?;
    let result =
        serde_json::from_slice::<PolicyJournal>(&plaintext).map_err(|_| PolicyOwnerError::Corrupt);
    plaintext.zeroize();
    let journal = result?;
    journal.validate()?;
    if journal.scope != *scope {
        return Err(PolicyOwnerError::ScopeMismatch);
    }
    Ok(journal)
}
