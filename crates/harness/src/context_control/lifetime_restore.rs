// SPDX-License-Identifier: MIT

//! Durable reload of an admitted lifetime window (issue #111).
//!
//! Restoring is the one operation that writes *past* history into the ledger, so it is kept apart
//! from the consumption path and re-proves every fact it loads. Each manifest must still bind its
//! own canonical bytes, must belong to the scope that filed it, and must sit inside that scope's
//! declared window. A persisted image therefore cannot widen a window, invent an invocation, or
//! overwrite a settlement with one that was never committed.

use super::lifetime_error::ContextLifetimeError;
use super::lifetime_ledger::ContextLifetimeLedger;
use super::lifetime_manifest::{DispatchSettlement, LifetimeManifest};

impl ContextLifetimeLedger {
    /// Settles a held invocation. `Released` gives the slot back; `Dispatched` and `Held` do not.
    ///
    /// Settlement is a **one-way** transition: an invocation may only move out of `Held`, and only
    /// once. Re-settling to the settlement it already carries is accepted as an idempotent retry;
    /// changing it is refused. Without this guard a `Dispatched` invocation could later be released
    /// (handing back a slot whose dispatch did happen), and a `Released` one could be marked
    /// dispatched, which would leave the live count disagreeing with the reloaded count.
    pub fn reconcile(
        &mut self,
        invocation_id: &str,
        settlement: DispatchSettlement,
    ) -> Result<LifetimeManifest, ContextLifetimeError> {
        let manifest = self
            .manifests
            .iter_mut()
            .find(|manifest| manifest.invocation_id == invocation_id)
            .ok_or_else(|| ContextLifetimeError::NotHeld {
                invocation_id: invocation_id.to_owned(),
            })?;
        if manifest.settlement != DispatchSettlement::Held {
            return if manifest.settlement == settlement {
                Ok(manifest.clone())
            } else {
                Err(ContextLifetimeError::AlreadySettled {
                    invocation_id: invocation_id.to_owned(),
                })
            };
        }
        if settlement == DispatchSettlement::Held {
            // Settling back to `Held` is a no-op rather than a transition.
            return Ok(manifest.clone());
        }
        manifest.settlement = settlement;
        let scope_id = manifest.scope_id.clone();
        let settled = manifest.clone();
        if let Some(state) = self.scopes.get_mut(&scope_id) {
            state.held.remove(invocation_id);
            if settlement == DispatchSettlement::Released {
                state.released.insert(invocation_id.to_owned());
            }
        }
        Ok(settled)
    }

    /// Restores already-admitted manifests into a freshly issued ledger.
    ///
    /// Admission order is taken from the image, because that is the order the manifests were
    /// committed in; `ordinal` is re-checked rather than trusted, so a reordered or renumbered image
    /// is refused instead of silently becoming the new window.
    pub(crate) fn restore_manifests(
        &mut self,
        manifests: &[LifetimeManifest],
    ) -> Result<(), ContextLifetimeError> {
        for manifest in manifests {
            manifest.verify()?;
            let state = self.state(&manifest.scope_id)?;
            // `ordinal` is a monotonic admission index, not a slot index: a release frees capacity
            // without rewinding the sequence, so a refilled slot legitimately carries an ordinal
            // above the declared bound. It is therefore checked against the scope's own restored
            // sequence rather than against `capacity`.
            let expected_ordinal = u32::try_from(state.admitted.len())
                .unwrap_or(u32::MAX)
                .saturating_add(1);
            if !state.scope.owner.covers_owner(&manifest.owner)
                || manifest.ordinal != expected_ordinal
                || state.manifest_index(&manifest.invocation_id).is_some()
            {
                return Err(ContextLifetimeError::InvalidInput);
            }
            // Provenance is re-proved, not trusted: the record must still name the scope revision it
            // is filed under, and must agree with that scope about the window it claims to belong to.
            if manifest.scope_digest != state.digest
                || manifest.applicability != state.scope.applicability
                || manifest.items != state.scope.items
            {
                return Err(ContextLifetimeError::InvalidInput);
            }
            let scope_id = manifest.scope_id.clone();
            let invocation_id = manifest.invocation_id.clone();
            let settlement = manifest.settlement;
            // Keep the restored budget identical to the live one, so a reloaded ledger refuses the
            // same over-budget growth the live ledger would have refused.
            if self.charge(manifest.bytes.len()) {
                return Err(ContextLifetimeError::InvalidInput);
            }
            self.manifests.push(manifest.clone());
            if let Some(state) = self.scopes.get_mut(&scope_id) {
                state.admitted.push(invocation_id.clone());
                match settlement {
                    DispatchSettlement::Held => {
                        state.held.insert(invocation_id);
                    }
                    DispatchSettlement::Released => {
                        state.released.insert(invocation_id);
                    }
                    DispatchSettlement::Dispatched => {}
                }
            }
        }
        Ok(())
    }
}
