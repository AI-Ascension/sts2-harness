// SPDX-License-Identifier: MIT

//! Durable lifetime consumption at dispatch admission (issue #111).
//!
//! ## Consumption happens at exactly one place
//!
//! A scope's applicability is consumed by [`ContextLifetimeLedger::admit`] and nowhere else.
//! [`ContextLifetimeLedger::preview`] is a pure read: a preview, a browser reload, a receipt lookup
//! and a transport retry all leave the counters untouched. A retry carries the *same*
//! [`LogicalInvocationIdentity::invocation_id`] with a different `attempt`, and admitting it again
//! returns the manifest that already exists rather than consuming a second slot — so a retry can
//! neither extend nor resurrect applicability.
//!
//! ## The two sides of a crash are not the same
//!
//! [`LifetimeFailpoint::BeforeDurableAdmission`] leaves the ledger exactly as it was: the count is
//! preserved because nothing was consumed. [`LifetimeFailpoint::AfterDurableAdmission`] stops the
//! caller *after* the admission became durable, so the slot is consumed and the invocation stays
//! **held** as a possible dispatch until [`ContextLifetimeLedger::reconcile`] settles it. Holding
//! is the only safe default: a dispatch that may have happened must not be silently given back.
//!
//! ## History is never rewritten
//!
//! Manifests are append-only. Reconciliation updates a settlement field, and expiry never deletes or
//! rewrites a record, so an earlier effective-context decision stays inspectable under retention
//! policy instead of being relabelled as erased.

use std::collections::{BTreeMap, BTreeSet};

use super::lifetime_error::ContextLifetimeError;
use super::lifetime_manifest::{
    DispatchSettlement, LifetimeApproval, LifetimeManifest, MAX_LIFETIME_MANIFEST_BYTES,
};
use super::lifetime_scope::{
    CONTEXT_LIFETIME_SCHEMA, ContextLifetimeScope, LIFETIME_BODY_CHARGE_FACTOR,
    LIFETIME_RECORD_OVERHEAD, LogicalInvocationIdentity, MAX_LIFETIME_MANIFESTS,
    MAX_LIFETIME_SCOPES, MAX_LIFETIME_STATE_BYTES,
};
use super::lifetime_state::{LifetimePreview, ScopeState};

/// A deterministic stop injected around durable admission.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifetimeFailpoint {
    /// Stop before the admission is written; nothing is consumed.
    BeforeDurableAdmission,
    /// Stop after the admission is durable; the slot stays consumed and held.
    AfterDurableAdmission,
}

/// The owner-scoped ledger that consumes lifetime applicability at durable admission.
#[derive(Default)]
pub struct ContextLifetimeLedger {
    pub(super) scopes: BTreeMap<String, ScopeState>,
    pub(super) manifests: Vec<LifetimeManifest>,
    /// Bytes already charged against the run's persisted-image budget.
    pub(super) charged: usize,
}

impl ContextLifetimeLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an owner-issued scope. A reused id must describe the same scope exactly.
    pub fn issue(&mut self, scope: ContextLifetimeScope) -> Result<String, ContextLifetimeError> {
        scope.validate()?;
        let digest = scope.digest()?;
        if let Some(existing) = self.scopes.get(&scope.scope_id) {
            if existing.digest != digest {
                return Err(ContextLifetimeError::RepeatedScope {
                    scope_id: scope.scope_id,
                });
            }
            return Ok(digest);
        }
        if self.scopes.len() >= MAX_LIFETIME_SCOPES {
            return Err(ContextLifetimeError::InvalidInput);
        }
        // Charge the scope before recording it, so a state reachable here is always persistable.
        if self.charge(scope.body_len()?) {
            return Err(ContextLifetimeError::InvalidInput);
        }
        self.scopes.insert(
            scope.scope_id.clone(),
            ScopeState {
                scope,
                digest: digest.clone(),
                admitted: Vec::new(),
                held: BTreeSet::new(),
                released: BTreeSet::new(),
            },
        );
        Ok(digest)
    }

    /// Reads remaining applicability. This never consumes, extends, or resurrects anything.
    pub fn preview(
        &self,
        scope_id: &str,
        now: u64,
    ) -> Result<LifetimePreview, ContextLifetimeError> {
        let state = self.state(scope_id)?;
        Ok(state.preview(now))
    }

    /// Consumes one slot at durable dispatch admission and returns the manifest.
    ///
    /// A repeat of an already admitted logical invocation returns that invocation's manifest
    /// unchanged: a retry cannot double-consume, extend, or resurrect applicability.
    pub fn admit(
        &mut self,
        scope_id: &str,
        identity: &LogicalInvocationIdentity,
        now: u64,
        failpoint: Option<LifetimeFailpoint>,
    ) -> Result<LifetimeManifest, ContextLifetimeError> {
        if !identity.valid() {
            return Err(ContextLifetimeError::InvalidInput);
        }
        let (scope_digest, ordinal) = {
            let state = self.state(scope_id)?;
            Self::check_admissible(state, identity, now)?;
            if state.manifest_index(&identity.invocation_id).is_some() {
                return self.existing_manifest(scope_id, &identity.invocation_id);
            }
            (
                state.digest.clone(),
                u32::try_from(state.admitted.len()).unwrap_or(u32::MAX) + 1,
            )
        };
        if failpoint == Some(LifetimeFailpoint::BeforeDurableAdmission) {
            return Err(ContextLifetimeError::InterruptedBeforeAdmission);
        }
        let manifest = self.commit_admission(scope_id, identity, now, ordinal, &scope_digest)?;
        if failpoint == Some(LifetimeFailpoint::AfterDurableAdmission) {
            return Err(ContextLifetimeError::InterruptedAfterAdmission);
        }
        Ok(manifest)
    }

    /// Every manifest ever admitted, in admission order. Never rewritten by expiry.
    #[must_use]
    pub fn manifests(&self) -> &[LifetimeManifest] {
        &self.manifests
    }

    /// Charges a canonical record body against the run's persisted-image byte budget.
    ///
    /// Returns `true` when the record would not fit, which the caller turns into a refusal. This is
    /// what keeps "any reachable state is persistable" true: without it a window could grow until
    /// `persist_lifetime` rejected it permanently.
    pub(super) fn charge(&mut self, body_len: usize) -> bool {
        let cost = body_len
            .saturating_mul(LIFETIME_BODY_CHARGE_FACTOR)
            .saturating_add(LIFETIME_RECORD_OVERHEAD);
        if self.charged.saturating_add(cost) > MAX_LIFETIME_STATE_BYTES {
            return true;
        }
        self.charged = self.charged.saturating_add(cost);
        false
    }

    /// Every issued scope, ordered by scope id, exactly as it was issued.
    #[must_use]
    pub fn issued_scopes(&self) -> Vec<ContextLifetimeScope> {
        self.scopes
            .values()
            .map(|state| state.scope.clone())
            .collect()
    }

    /// The manifests admitted under one scope, in admission order.
    #[must_use]
    pub fn manifests_for(&self, scope_id: &str) -> Vec<LifetimeManifest> {
        self.manifests
            .iter()
            .filter(|manifest| manifest.scope_id == scope_id)
            .cloned()
            .collect()
    }

    /// Mints an approval over the scope's current manifests at the scope's ceiling.
    pub fn approve(&self, scope_id: &str) -> Result<LifetimeApproval, ContextLifetimeError> {
        let state = self.state(scope_id)?;
        let manifests = self.manifests_for(scope_id);
        Ok(LifetimeApproval::mint(
            &state.scope.scope_id,
            &state.digest,
            state.scope.ceiling,
            &manifests,
        ))
    }

    pub(super) fn state(&self, scope_id: &str) -> Result<&ScopeState, ContextLifetimeError> {
        self.scopes
            .get(scope_id)
            .ok_or_else(|| ContextLifetimeError::UnknownScope {
                scope_id: scope_id.to_owned(),
            })
    }

    /// Owner, ceiling and capacity checks shared by preview-time and admission-time callers.
    fn check_admissible(
        state: &ScopeState,
        identity: &LogicalInvocationIdentity,
        now: u64,
    ) -> Result<(), ContextLifetimeError> {
        if !state.scope.owner.covers_owner(&identity.owner) {
            return Err(ContextLifetimeError::SiblingScopeRefused {
                scope_id: state.scope.scope_id.clone(),
            });
        }
        if now >= state.scope.ceiling {
            return Err(ContextLifetimeError::Expired {
                scope_id: state.scope.scope_id.clone(),
            });
        }
        if state.manifest_index(&identity.invocation_id).is_some() {
            return Ok(());
        }
        if state.consumed() >= state.scope.applicability.capacity() {
            return Err(ContextLifetimeError::Exhausted {
                scope_id: state.scope.scope_id.clone(),
            });
        }
        Ok(())
    }

    fn existing_manifest(
        &self,
        scope_id: &str,
        invocation_id: &str,
    ) -> Result<LifetimeManifest, ContextLifetimeError> {
        self.manifests
            .iter()
            .find(|manifest| {
                manifest.scope_id == scope_id && manifest.invocation_id == invocation_id
            })
            .cloned()
            .ok_or_else(|| ContextLifetimeError::UnknownScope {
                scope_id: scope_id.to_owned(),
            })
    }

    fn commit_admission(
        &mut self,
        scope_id: &str,
        identity: &LogicalInvocationIdentity,
        now: u64,
        ordinal: u32,
        scope_digest: &str,
    ) -> Result<LifetimeManifest, ContextLifetimeError> {
        if self.manifests.len() >= MAX_LIFETIME_MANIFESTS {
            return Err(ContextLifetimeError::InvalidInput);
        }
        let (items, applicability, manifest_id) = {
            let state = self.state(scope_id)?;
            (
                state.scope.items.clone(),
                state.scope.applicability,
                format!("{}#{}", state.scope.scope_id, ordinal),
            )
        };
        // Record the capacity that genuinely remains after this admission. Deriving it from the
        // admission index would overstate exhaustion once a release has freed a slot, because the
        // index keeps advancing while the window does not.
        let remaining_after = {
            let state = self.state(scope_id)?;
            let capacity = state.scope.applicability.capacity();
            let consumed = state.consumed().saturating_add(1);
            capacity.saturating_sub(consumed)
        };
        let manifest = LifetimeManifest {
            schema: CONTEXT_LIFETIME_SCHEMA.to_owned(),
            manifest_id,
            scope_id: scope_id.to_owned(),
            scope_digest: scope_digest.to_owned(),
            invocation_id: identity.invocation_id.clone(),
            attempt: identity.attempt,
            owner: identity.owner.clone(),
            applicability,
            ordinal,
            admitted_at: now,
            remaining_after,
            items,
            settlement: DispatchSettlement::Held,
            bytes: Vec::new(),
            manifest_digest: String::new(),
        }
        .mint()?;
        if manifest.bytes.len() > MAX_LIFETIME_MANIFEST_BYTES {
            return Err(ContextLifetimeError::InvalidInput);
        }
        // Charge only after the record is known to be well-formed, so a refused admission leaves the
        // budget and the ledger untouched.
        if self.charge(manifest.bytes.len()) {
            return Err(ContextLifetimeError::InvalidInput);
        }
        if let Some(state) = self.scopes.get_mut(scope_id) {
            state.admitted.push(identity.invocation_id.clone());
            state.held.insert(identity.invocation_id.clone());
        }
        self.manifests.push(manifest.clone());
        Ok(manifest)
    }
}
