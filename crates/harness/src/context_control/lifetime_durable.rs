// SPDX-License-Identifier: MIT

//! Durable persistence of lifetime consumption counters and identities (issue #111).
//!
//! Item 3 of the issue requires the counters and identities to be **persisted**, not merely held in
//! memory. This module binds [`ContextLifetimeLedger`] to the existing [`ContextControlStore`]:
//! scopes, admitted logical invocations and their settlements are written inside one SQLite
//! transaction, so a process that restarts mid-window reloads the same counters and cannot hand a
//! consumed slot back.
//!
//! ## Why the ledger is stored as one envelope
//!
//! The store's journal is already encrypted, digest-fenced and single-writer. Reusing that shape
//! keeps the lifetime state under the same key, the same AAD and the same ownership fence as the
//! rest of the control authority, rather than introducing a second, weaker trust boundary.
//!
//! ## What is *not* persisted
//!
//! Nothing here re-derives applicability from wall-clock time. The ceiling is stored as data and is
//! re-evaluated by the ledger on load, so a restart cannot resurrect an expired window and cannot
//! expire a window that was still open when the process stopped.

use serde::{Deserialize, Serialize};

use super::lifetime_error::ContextLifetimeError;
use super::lifetime_ledger::ContextLifetimeLedger;
use super::lifetime_scope::{
    MAX_LIFETIME_MANIFESTS, MAX_LIFETIME_SCOPES, MAX_LIFETIME_STATE_BYTES,
};
use super::store::ContextControlStore;
use super::store_schema::digest;
use super::store_types::DurableControlStoreError;

/// AAD binding the lifetime envelope to its purpose inside the control store.
pub(super) const LIFETIME_AAD: &[u8] = b"ascension.context-control.lifetime.v1\0";

/// Slack allowed on the *encrypted* envelope over the plaintext bound.
///
/// The write path bounds the plaintext body; the read path sees the envelope, which additionally
/// carries the 24-byte nonce and 16-byte tag. Bounding both by the same number would let a
/// near-limit state persist and then fail to load.
const LIFETIME_ENVELOPE_SLACK: usize = 64;

/// The durable image of one ledger.
///
/// This is a faithful, versioned copy of the ledger's own state: the scopes exactly as issued and
/// the manifests exactly as admitted. It is deliberately not a recomputed summary, so a reload can
/// prove the counters it restores are the counters that were committed.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableLifetimeState {
    pub schema: String,
    pub run_id: String,
    pub scope_count: u32,
    pub manifest_count: u32,
    /// Every scope exactly as issued, ordered by scope id.
    pub scopes: Vec<super::lifetime_scope::ContextLifetimeScope>,
    /// Every manifest exactly as admitted, in admission order.
    pub manifests: Vec<super::lifetime_manifest::LifetimeManifest>,
}

/// The schema identifier of the durable lifetime image.
pub const DURABLE_LIFETIME_SCHEMA: &str = "ascension.context-control.lifetime-state.v1";

impl DurableLifetimeState {
    /// Builds the durable image of a ledger for one run.
    #[must_use]
    pub fn capture(run_id: &str, ledger: &ContextLifetimeLedger) -> Self {
        let scopes = ledger.issued_scopes();
        Self {
            schema: DURABLE_LIFETIME_SCHEMA.to_owned(),
            run_id: run_id.to_owned(),
            scope_count: u32::try_from(scopes.len()).unwrap_or(u32::MAX),
            manifest_count: u32::try_from(ledger.manifests().len()).unwrap_or(u32::MAX),
            scopes,
            manifests: ledger.manifests().to_vec(),
        }
    }

    /// Confirms the image is internally consistent before it is written or trusted.
    pub fn validate(&self) -> Result<(), ContextLifetimeError> {
        if self.schema != DURABLE_LIFETIME_SCHEMA || self.run_id.is_empty() {
            return Err(ContextLifetimeError::InvalidInput);
        }
        if self.scope_count as usize != self.scopes.len()
            || self.manifest_count as usize != self.manifests.len()
            || self.scopes.len() > MAX_LIFETIME_SCOPES
            || self.manifests.len() > MAX_LIFETIME_MANIFESTS
        {
            return Err(ContextLifetimeError::InvalidInput);
        }
        for scope in &self.scopes {
            scope.validate()?;
            if scope.owner.run_id != self.run_id {
                return Err(ContextLifetimeError::InvalidOwner);
            }
        }
        // Every manifest must still name the scope revision it is filed under, so an image cannot
        // rewrite provenance by pairing an honest scope with a manifest that claims another one.
        for manifest in &self.manifests {
            manifest.verify()?;
            if manifest.owner.run_id != self.run_id {
                return Err(ContextLifetimeError::InvalidOwner);
            }
            let digest = self
                .scopes
                .iter()
                .find(|scope| scope.scope_id == manifest.scope_id)
                .ok_or(ContextLifetimeError::UnknownScope {
                    scope_id: manifest.scope_id.clone(),
                })?
                .digest()?;
            if digest != manifest.scope_digest {
                return Err(ContextLifetimeError::InvalidInput);
            }
        }
        Ok(())
    }

    /// Rebuilds a ledger from this image.
    ///
    /// Every scope is re-issued through the ledger's own validation and every manifest is re-verified
    /// against its carried bytes, so a reloaded window is exactly the window that was committed. The
    /// counters are rebuilt from the manifests rather than trusted from `scope_count` /
    /// `manifest_count`, so a corrupted summary cannot silently shrink a consumed window.
    pub fn restore(&self) -> Result<ContextLifetimeLedger, ContextLifetimeError> {
        self.validate()?;
        let mut ledger = ContextLifetimeLedger::new();
        for scope in &self.scopes {
            ledger.issue(scope.clone())?;
        }
        ledger.restore_manifests(&self.manifests)?;
        Ok(ledger)
    }
}

impl ContextControlStore {
    /// Persists the lifetime counters and identities for this run.
    ///
    /// The envelope is encrypted and written in its own transaction, so a failpoint either leaves
    /// the previously committed counters intact or commits the new ones — never a half-updated
    /// window.
    pub fn persist_lifetime(
        &mut self,
        ledger: &ContextLifetimeLedger,
    ) -> Result<(), DurableControlStoreError> {
        let state = DurableLifetimeState::capture(&self.run_id, ledger);
        state
            .validate()
            .map_err(|_| DurableControlStoreError::Encode)?;
        let body = serde_json::to_vec(&state).map_err(|_| DurableControlStoreError::Encode)?;
        if body.len() > MAX_LIFETIME_STATE_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let envelope = self
            .encrypt_with_aad(&body, LIFETIME_AAD)
            .map_err(|_| DurableControlStoreError::AuthenticationFailed)?;
        let envelope_digest = digest(&envelope);
        if self.failpoint == Some(super::store_types::DurableStoreFailpoint::BeforeJournalWrite) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        self.claim_owner()?;
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Self::verify_owner(&transaction, &self.run_id, &self.owner_token)?;
        transaction
            .execute(
                "INSERT INTO context_control_lifetime
                    (run_id, envelope, envelope_digest, scope_count, manifest_count, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(run_id) DO UPDATE SET
                    envelope = excluded.envelope,
                    envelope_digest = excluded.envelope_digest,
                    scope_count = excluded.scope_count,
                    manifest_count = excluded.manifest_count,
                    updated_at = excluded.updated_at",
                rusqlite::params![
                    self.run_id.as_str(),
                    envelope,
                    envelope_digest,
                    i64::from(state.scope_count),
                    i64::from(state.manifest_count),
                    super::store_schema::now_seconds(),
                ],
            )
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        if self.failpoint == Some(super::store_types::DurableStoreFailpoint::BeforeCommit) {
            self.failpoint = None;
            return Err(DurableControlStoreError::Failpoint);
        }
        transaction
            .commit()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        Ok(())
    }

    /// Reloads the persisted lifetime counters and identities for this run.
    ///
    /// Returns `Ok(None)` when this run has never persisted lifetime state, so a caller can
    /// distinguish "no window yet" from "an empty window".
    pub fn load_lifetime(&self) -> Result<Option<DurableLifetimeState>, DurableControlStoreError> {
        use rusqlite::OptionalExtension;
        self.verify_connection_owner()?;
        let row = self
            .connection
            .query_row(
                "SELECT envelope, envelope_digest, scope_count, manifest_count
                 FROM context_control_lifetime WHERE run_id = ?1",
                [self.run_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|_| DurableControlStoreError::Sqlite)?;
        let Some((envelope, envelope_digest, scope_count, manifest_count)) = row else {
            return Ok(None);
        };
        if envelope.len() > MAX_LIFETIME_STATE_BYTES + LIFETIME_ENVELOPE_SLACK
            || digest(&envelope) != envelope_digest
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        let body = self
            .decrypt_with_aad(&envelope, LIFETIME_AAD)
            .map_err(|_| DurableControlStoreError::AuthenticationFailed)?;
        if body.len() > MAX_LIFETIME_STATE_BYTES {
            return Err(DurableControlStoreError::TooLarge);
        }
        let state: DurableLifetimeState =
            serde_json::from_slice(&body).map_err(|_| DurableControlStoreError::Decode)?;
        state
            .validate()
            .map_err(|_| DurableControlStoreError::Decode)?;
        // The indexed counts are a second, independent statement of the same fact; disagreement
        // means the row was tampered with rather than merely stale.
        if i64::from(state.scope_count) != scope_count
            || i64::from(state.manifest_count) != manifest_count
            || state.run_id != self.run_id
        {
            return Err(DurableControlStoreError::Corrupt);
        }
        Ok(Some(state))
    }
}
