// SPDX-License-Identifier: MIT

//! Opt-in durable storage for the harness-owned context-control authority.
//!
//! The console remains replaceable: the authority journal is encrypted before it enters SQLite,
//! and one transaction updates the journal envelope and its outbox rows. Phase 1 snapshots are
//! copied into a separate additive table and are never rewritten. This module is intentionally
//! a fixture-facing seam; callers still need an approved private key and scoped authorization.

use super::state::ControlAuthority;
use super::store_schema::{digest, ensure_schema, insert_outbox, now_seconds};
use super::store_types::{
    AAD, DurableControlStoreError, DurableStoreFailpoint, MAX_JOURNAL_BYTES, StoreMode,
};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct ContextControlStore {
    pub(super) path: PathBuf,
    pub(super) key: [u8; 32],
    pub(super) run_id: String,
    pub(super) connection: Connection,
    pub(super) failpoint: Option<DurableStoreFailpoint>,
}

impl ContextControlStore {
    pub fn create(
        path: impl AsRef<Path>,
        key: [u8; 32],
        run_id: impl Into<String>,
        authority: &ControlAuthority,
        mode: StoreMode,
    ) -> Result<Self, DurableControlStoreError> {
        let run_id = run_id.into();
        let mut store = Self::open_connection(path.as_ref(), key, run_id)?;
        store.persist(authority, mode)?;
        Ok(store)
    }

    pub fn open(
        path: impl AsRef<Path>,
        key: [u8; 32],
        run_id: impl Into<String>,
    ) -> Result<Self, DurableControlStoreError> {
        Self::open_connection(path.as_ref(), key, run_id.into())
    }

    pub fn set_failpoint(&mut self, failpoint: Option<DurableStoreFailpoint>) {
        self.failpoint = failpoint;
    }

    pub fn persist(
        &mut self,
        authority: &ControlAuthority,
        mode: StoreMode,
    ) -> Result<(), DurableControlStoreError> {
        let journal = authority
            .export_journal()
            .map_err(|_| DurableControlStoreError::Encode)?;
        if journal.len() > MAX_JOURNAL_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let encrypted = self.encrypt(&journal)?;
        let journal_digest = digest(&encrypted);
        if self.failpoint == Some(DurableStoreFailpoint::BeforeJournalWrite) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        let state = authority.state();
        if state.boundary.run_id != self.run_id {
            return Err(DurableControlStoreError::ScopeMismatch);
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        transaction
            .execute(
                "INSERT INTO context_control_journal
                    (run_id, envelope, envelope_digest, management_active, active_revision_id,
                     pause_latched, controller_epoch, plan_epoch, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(run_id) DO UPDATE SET
                    envelope = excluded.envelope,
                    envelope_digest = excluded.envelope_digest,
                    management_active = excluded.management_active,
                    active_revision_id = excluded.active_revision_id,
                    pause_latched = excluded.pause_latched,
                    controller_epoch = excluded.controller_epoch,
                    plan_epoch = excluded.plan_epoch,
                    updated_at = excluded.updated_at",
                params![
                    self.run_id,
                    encrypted,
                    journal_digest,
                    mode.as_i64(),
                    state.active_revision_id,
                    i64::from(state.pause_latched),
                    state.boundary.controller_epoch as i64,
                    state.plan_epoch as i64,
                    now_seconds(),
                ],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        insert_outbox(&transaction, &self.run_id, authority.events())?;
        if self.failpoint == Some(DurableStoreFailpoint::BeforeCommit) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)
    }

    pub fn load(&self) -> Result<ControlAuthority, DurableControlStoreError> {
        let (envelope, envelope_digest, active_revision, paused, controller_epoch, plan_epoch) =
            self.connection
                .query_row(
                    "SELECT envelope, envelope_digest, active_revision_id, pause_latched,
                            controller_epoch, plan_epoch
                     FROM context_control_journal WHERE run_id = ?1",
                    [self.run_id.as_str()],
                    |row| {
                        Ok((
                            row.get::<_, Vec<u8>>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    },
                )
                .optional()
                .map_err(|_| DurableControlStoreError::Sqlite)?
                .ok_or(DurableControlStoreError::Missing)?;
        if envelope.len() > MAX_JOURNAL_BYTES || digest(&envelope) != envelope_digest {
            return Err(DurableControlStoreError::Corrupt);
        }
        let journal = self.decrypt(&envelope)?;
        let authority =
            ControlAuthority::recover(&journal).map_err(|_| DurableControlStoreError::Decode)?;
        let state = authority.state();
        if state.boundary.run_id != self.run_id
            || state.active_revision_id != active_revision
            || i64::from(state.pause_latched) != paused
            || state.boundary.controller_epoch as i64 != controller_epoch.saturating_add(1)
            || state.plan_epoch as i64 != plan_epoch
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        Ok(authority)
    }

    pub fn mode(&self) -> Result<StoreMode, DurableControlStoreError> {
        let value = self
            .connection
            .query_row(
                "SELECT management_active FROM context_control_journal WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?
            .ok_or(DurableControlStoreError::Missing)?;
        StoreMode::from_i64(value)
    }

    fn open_connection(
        path: &Path,
        key: [u8; 32],
        run_id: String,
    ) -> Result<Self, DurableControlStoreError> {
        if path.as_os_str().is_empty() || run_id.is_empty() {
            return Err(DurableControlStoreError::InvalidPath);
        }
        if key.iter().all(|byte| *byte == 0) {
            return Err(DurableControlStoreError::InvalidKey);
        }
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
            && !parent.exists()
        {
            return Err(DurableControlStoreError::ParentMissing);
        }
        let connection = Connection::open(path).map_err(|_| DurableControlStoreError::Sqlite)?;
        connection
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 PRAGMA synchronous = FULL;
                 PRAGMA foreign_keys = ON;",
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        ensure_schema(&connection)?;
        Ok(Self {
            path: path.to_owned(),
            key,
            run_id,
            connection,
            failpoint: None,
        })
    }

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, DurableControlStoreError> {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut nonce = [0_u8; 24];
        nonce[..16].copy_from_slice(first.as_bytes());
        nonce[16..].copy_from_slice(&second.as_bytes()[..8]);
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&self.key));
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| DurableControlStoreError::AuthenticationFailed)?;
        let mut envelope = Vec::with_capacity(nonce.len() + ciphertext.len());
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }

    fn decrypt(&self, envelope: &[u8]) -> Result<Vec<u8>, DurableControlStoreError> {
        if envelope.len() < 24 + 16 {
            return Err(DurableControlStoreError::Corrupt);
        }
        let (nonce, ciphertext) = envelope.split_at(24);
        let cipher = XChaCha20Poly1305::new(Key::from_slice(&self.key));
        let plaintext = cipher
            .decrypt(
                XNonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: AAD,
                },
            )
            .map_err(|_| DurableControlStoreError::AuthenticationFailed)?;
        if plaintext.len() > MAX_JOURNAL_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        Ok(plaintext)
    }
}
