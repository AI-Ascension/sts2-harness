// SPDX-License-Identifier: MIT

//! The owner API for holding exact application input.
//!
//! Nothing here calls a provider.  Drafting, previewing, committing and reading metadata leave the
//! boundary-write and gameplay counters at zero, and a commit stays held until an explicit resume
//! (see [`super::dispatch_resume`]) hands the approved bytes to a recording write port exactly once.

use super::dispatch_error::DispatchError;
use super::dispatch_fences::DispatchFences;
use super::dispatch_lifecycle::{
    DispatchLedger, DispatchMetadata, DispatchPreview, DispatchState, PreparedDispatch,
};
use super::dispatch_material::PreparedApplicationInput;
use super::dispatch_support::adapter_support;
use super::valid_identity;
use std::collections::BTreeMap;

/// The owner API for holding and dispatching exact application input.
#[derive(Clone, Debug)]
pub struct PreparedDispatchController {
    pub(super) ledger: DispatchLedger,
    pub(super) dispatches: BTreeMap<String, PreparedDispatch>,
    pub(super) stop_epoch: u64,
    pub(super) boundary_writes: u32,
    pub(super) gameplay_effects: u32,
}

impl PreparedDispatchController {
    /// A fresh controller with an empty ledger.
    pub fn new() -> Self {
        Self {
            ledger: DispatchLedger::new(),
            dispatches: BTreeMap::new(),
            stop_epoch: 0,
            boundary_writes: 0,
            gameplay_effects: 0,
        }
    }

    /// Rebuilds a controller around a retained ledger after a restart.
    ///
    /// Held approvals are not preserved: a restarted caller must re-draft, and the retained
    /// receipt then refuses a second dispatch of the same identity.
    pub fn restart(ledger: DispatchLedger) -> Self {
        Self {
            ledger,
            ..Self::new()
        }
    }

    /// Application-boundary writes this controller handed to a port.
    pub const fn boundary_writes(&self) -> u32 {
        self.boundary_writes
    }

    /// Gameplay effects this controller caused.
    pub const fn gameplay_effects(&self) -> u32 {
        self.gameplay_effects
    }

    /// Current stop epoch.  A resume needs the stop epoch it approved under.
    pub const fn stop_epoch(&self) -> u64 {
        self.stop_epoch
    }

    /// The retained ledger.
    pub const fn ledger(&self) -> &DispatchLedger {
        &self.ledger
    }

    /// One held dispatch, when it is still held.
    pub fn dispatch(&self, dispatch_id: &str) -> Option<&PreparedDispatch> {
        self.dispatches.get(dispatch_id)
    }

    /// Lifecycle state of one held dispatch.
    pub fn state(&self, dispatch_id: &str) -> Option<DispatchState> {
        self.dispatches
            .get(dispatch_id)
            .map(|dispatch| dispatch.state)
    }

    /// Binds a freshly prepared input as a draft.
    ///
    /// Drafting causes no inference, no game effect and no write.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::DuplicateDispatch`] when the identity is already held or already
    /// recorded, and [`DispatchError::Cancelled`] when it was cancelled terminally.
    pub fn draft(
        &mut self,
        dispatch_id: &str,
        input: PreparedApplicationInput,
        fences: DispatchFences,
    ) -> Result<(), DispatchError> {
        if !valid_identity(dispatch_id) {
            return Err(DispatchError::InvalidBinding);
        }
        if self.ledger.receipt(dispatch_id).is_some()
            || self.ledger.is_cancelled(dispatch_id)
            || self.dispatches.contains_key(dispatch_id)
        {
            return Err(if self.ledger.is_cancelled(dispatch_id) {
                DispatchError::Cancelled
            } else {
                DispatchError::DuplicateDispatch
            });
        }
        input.verify()?;
        fences.validate()?;
        if fences.adapter_id != input.adapter_id
            || adapter_support(&input.adapter_id).exact_boundary() != Some(input.boundary)
        {
            return Err(DispatchError::UnsupportedAdapter);
        }
        let approved_stop_epoch = self.stop_epoch;
        self.dispatches.insert(
            dispatch_id.to_owned(),
            PreparedDispatch::new(dispatch_id, input, fences, approved_stop_epoch),
        );
        Ok(())
    }

    /// Bounded, byte-free preview of a held dispatch.  Causes no inference and no write.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::UnknownDispatch`] when no dispatch is held under the identity.
    pub fn preview(&self, dispatch_id: &str) -> Result<DispatchPreview, DispatchError> {
        let dispatch = self
            .dispatches
            .get(dispatch_id)
            .ok_or(DispatchError::UnknownDispatch)?;
        Ok(DispatchPreview {
            dispatch_id: dispatch.dispatch_id.clone(),
            adapter_id: dispatch.input.adapter_id.clone(),
            boundary: dispatch.input.boundary,
            claim: dispatch.input.claim(),
            state: dispatch.state,
            approved_material_sha256: dispatch.input.approved_material_sha256.clone(),
            manifest_sha256: dispatch.input.manifest_sha256.clone(),
            entries: dispatch.input.entries(),
            bytes_exposed: false,
        })
    }

    /// Bounded metadata read of a held dispatch.  Causes no inference and no write.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::UnknownDispatch`] when no dispatch is held under the identity.
    pub fn metadata(&self, dispatch_id: &str) -> Result<DispatchMetadata, DispatchError> {
        let dispatch = self
            .dispatches
            .get(dispatch_id)
            .ok_or(DispatchError::UnknownDispatch)?;
        Ok(DispatchMetadata {
            dispatch_id: dispatch.dispatch_id.clone(),
            execution_id: dispatch.input.execution_id.clone(),
            attempt_id: dispatch.input.attempt_id.clone(),
            adapter_id: dispatch.input.adapter_id.clone(),
            boundary: dispatch.input.boundary,
            claim: dispatch.input.claim(),
            state: dispatch.state,
            component_count: dispatch.input.component_count(),
            approved_bytes: dispatch.input.approved_bytes(),
            approved_material_sha256: dispatch.input.approved_material_sha256.clone(),
            manifest_sha256: dispatch.input.manifest_sha256.clone(),
            receipt: self.ledger.receipt(dispatch_id).cloned(),
        })
    }

    /// Acknowledges a draft and holds it.  Causes no inference and no write.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::Drift`] or [`DispatchError::Stale`] when a bound axis or the stop
    /// epoch already moved, and [`DispatchError::NotHeld`] when the draft is not in `Drafted`.
    pub fn commit(
        &mut self,
        dispatch_id: &str,
        current: &DispatchFences,
    ) -> Result<DispatchState, DispatchError> {
        current.validate()?;
        let stop_epoch = self.stop_epoch;
        let dispatch = self
            .dispatches
            .get_mut(dispatch_id)
            .ok_or(DispatchError::UnknownDispatch)?;
        if dispatch.approved_stop_epoch() != stop_epoch {
            dispatch.state = DispatchState::Stale;
            return Err(DispatchError::Stale);
        }
        match dispatch.state {
            DispatchState::Drafted => {}
            DispatchState::Stale => return Err(DispatchError::Stale),
            DispatchState::Cancelled => return Err(DispatchError::Cancelled),
            DispatchState::CommittedHeld | DispatchState::Dispatched => {
                return Err(DispatchError::NotHeld);
            }
        }
        if let Some(axis) = dispatch.fences.drift(current) {
            dispatch.state = DispatchState::Stale;
            return Err(DispatchError::Drift(axis));
        }
        dispatch.state = DispatchState::CommittedHeld;
        Ok(DispatchState::CommittedHeld)
    }

    /// Stops every pending approval.  A stop dominates a later resume.
    pub fn request_stop(&mut self) -> u64 {
        self.stop_epoch = self.stop_epoch.saturating_add(1);
        for dispatch in self.dispatches.values_mut() {
            if matches!(
                dispatch.state,
                DispatchState::Drafted | DispatchState::CommittedHeld
            ) {
                dispatch.state = DispatchState::Stale;
            }
        }
        self.stop_epoch
    }

    /// Cancels one dispatch terminally.  A cancelled identity can never be dispatched.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::DuplicateDispatch`] when the identity was already written.
    pub fn cancel(&mut self, dispatch_id: &str) -> Result<DispatchState, DispatchError> {
        if !valid_identity(dispatch_id) {
            return Err(DispatchError::InvalidBinding);
        }
        if self.ledger.receipt(dispatch_id).is_some() {
            return Err(DispatchError::DuplicateDispatch);
        }
        self.ledger.mark_cancelled(dispatch_id);
        if let Some(dispatch) = self.dispatches.get_mut(dispatch_id) {
            dispatch.state = DispatchState::Cancelled;
        }
        Ok(DispatchState::Cancelled)
    }
}

impl Default for PreparedDispatchController {
    fn default() -> Self {
        Self::new()
    }
}
