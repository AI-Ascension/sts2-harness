// SPDX-License-Identifier: MIT

//! Immutable per-invocation manifests and the approvals that fence them (issue #111).
//!
//! A manifest is the record of one durable admission. It is append-only: expiry, release and
//! reconciliation never rewrite it, so a later effective-context decision can always be audited
//! against the bytes that were actually admitted.

use serde::{Deserialize, Serialize};

use super::lifetime_error::ContextLifetimeError;
use super::lifetime_scope::{InvocationOwnerScope, LifetimeApplicability};

/// Upper bound on one canonical lifetime manifest.
pub const MAX_LIFETIME_MANIFEST_BYTES: usize = 32 * 1024;

/// How one admitted logical invocation ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchSettlement {
    /// Admitted, and later reconciled as actually dispatched.
    Dispatched,
    /// Admitted, and later reconciled as never dispatched; its slot is released.
    Released,
    /// Admitted and still held because the outcome is unknown.
    Held,
}

/// The durable record of one logical invocation's admission against a scope.
///
/// `ordinal` is the 1-based position of this invocation in its scope's **admission sequence**, so a
/// consumer can prove *why* the invocation was inside the window rather than trusting a counter.
/// The sequence is monotonic and is not rewound by a release, so an invocation that refills a freed
/// slot carries an ordinal above the declared bound while still being inside the window.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifetimeManifest {
    pub schema: String,
    pub manifest_id: String,
    pub scope_id: String,
    pub scope_digest: String,
    pub invocation_id: String,
    pub attempt: u32,
    pub owner: InvocationOwnerScope,
    pub applicability: LifetimeApplicability,
    pub ordinal: u32,
    pub admitted_at: u64,
    /// Remaining distinct invocations after this admission, held admissions included.
    pub remaining_after: u32,
    /// The item ids this admission made applicable, ordered.
    pub items: Vec<String>,
    pub settlement: DispatchSettlement,
    /// Canonical bytes of this record as admitted.
    pub bytes: Vec<u8>,
    /// Digest of `bytes`.
    pub manifest_digest: String,
}

impl LifetimeManifest {
    /// Re-derives the manifest digest from the immutable admission fields.
    ///
    /// `settlement` is deliberately excluded: it is the one field reconciliation is allowed to
    /// update, and folding it into the digest would make every settlement invalidate the record it
    /// just settled.
    pub fn recompute_digest(&self) -> Result<String, ContextLifetimeError> {
        let mut copy = self.clone();
        copy.bytes = Vec::new();
        copy.manifest_digest = String::new();
        copy.settlement = DispatchSettlement::Held;
        let bytes = serde_json::to_vec(&copy).map_err(|_| ContextLifetimeError::Encode)?;
        Ok(crate::sha256_hex(bytes))
    }

    /// Canonically encodes this record with `bytes` and `manifest_digest` cleared.
    fn encode_body(&self) -> Result<Vec<u8>, ContextLifetimeError> {
        let mut copy = self.clone();
        copy.bytes = Vec::new();
        copy.manifest_digest = String::new();
        copy.settlement = DispatchSettlement::Held;
        serde_json::to_vec(&copy).map_err(|_| ContextLifetimeError::Encode)
    }

    /// Mints a manifest whose digest is bound to its own canonical body.
    pub(crate) fn mint(mut self) -> Result<Self, ContextLifetimeError> {
        let body = self.encode_body()?;
        self.manifest_digest = crate::sha256_hex(&body);
        self.bytes = body;
        if self.bytes.len() > MAX_LIFETIME_MANIFEST_BYTES {
            return Err(ContextLifetimeError::InvalidInput);
        }
        Ok(self)
    }

    /// Confirms this record still describes exactly the bytes it carries.
    ///
    /// This is an **integrity** check against corruption, not an authentication boundary: every
    /// input is public, so a caller that deliberately rewrites a field and re-derives the digest can
    /// always produce a self-consistent record. What it does catch is an honest field set paired
    /// with `bytes` that no longer describe it, because the digest is re-derived from the fields
    /// *and* recomputed over the carried bytes, so the two must agree.
    ///
    /// Authenticity comes from the surrounding envelope, not from this method: a persisted manifest
    /// is stored inside the control store's AEAD envelope under the store key, and a reload
    /// cross-checks the record against its scope's own digest (see `restore_manifests`).
    ///
    /// Because the digest covers the admission rather than the settlement, a reconciled record
    /// still verifies. Settlement is therefore *not* protected by these bytes, so `reconcile` is the
    /// only supported way to change it and enforces a one-way transition; a caller that mutates
    /// `settlement` directly holds an unauthenticated record and must not be trusted to have minted
    /// capacity.
    pub fn verify(&self) -> Result<(), ContextLifetimeError> {
        if self.bytes.is_empty()
            || self.recompute_digest()? != self.manifest_digest
            || crate::sha256_hex(&self.bytes) != self.manifest_digest
        {
            return Err(ContextLifetimeError::InvalidInput);
        }
        Ok(())
    }
}

/// An approval minted against one exact scope revision, bound to whole admitted invocations.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LifetimeApproval {
    pub scope_id: String,
    pub scope_digest: String,
    /// The manifest digests this approval covers, in admission order.
    pub manifest_digests: Vec<String>,
    /// The wall-clock ceiling the approval was minted under.
    pub ceiling: u64,
}

impl LifetimeApproval {
    /// Mints an approval bound to a scope and the manifests admitted under it.
    #[must_use]
    pub fn mint(
        scope_id: &str,
        scope_digest: &str,
        ceiling: u64,
        manifests: &[LifetimeManifest],
    ) -> Self {
        Self {
            scope_id: scope_id.to_owned(),
            scope_digest: scope_digest.to_owned(),
            manifest_digests: manifests
                .iter()
                .map(|manifest| manifest.manifest_digest.clone())
                .collect(),
            ceiling,
        }
    }

    /// Confirms this approval still describes the scope and the given manifests at `now`.
    ///
    /// Expiry is evaluated here rather than trusted from mint time, so a preview obtained before the
    /// ceiling cannot be replayed after it.
    pub fn verify(
        &self,
        scope_id: &str,
        scope_digest: &str,
        now: u64,
        manifests: &[LifetimeManifest],
    ) -> Result<(), ContextLifetimeError> {
        if self.scope_id != scope_id || self.scope_digest != scope_digest {
            return Err(ContextLifetimeError::InvalidInput);
        }
        if now >= self.ceiling {
            return Err(ContextLifetimeError::Expired {
                scope_id: scope_id.to_owned(),
            });
        }
        let current: Vec<&str> = manifests
            .iter()
            .map(|manifest| manifest.manifest_digest.as_str())
            .collect();
        if current.len() != self.manifest_digests.len()
            || !current
                .iter()
                .zip(&self.manifest_digests)
                .all(|(left, right)| left == right)
        {
            return Err(ContextLifetimeError::InvalidInput);
        }
        Ok(())
    }
}
