// SPDX-License-Identifier: MIT

//! The single explicit write path for a held prepared dispatch.
//!
//! `resume` revalidates a held approval against the caller's current fences and hands the approved
//! material to a recording write port exactly once.  A recorded receipt is authoritative, so a
//! duplicate resume, a restart or an indeterminate write can never dispatch the same approval
//! twice.

use super::dispatch_controller::PreparedDispatchController;
use super::dispatch_error::DispatchError;
use super::dispatch_fences::DispatchFences;
use super::dispatch_lifecycle::{DispatchOutcome, DispatchReceipt, DispatchState};
use super::dispatch_port::{ApprovedDispatchMaterial, PreparedDispatchPort};
use super::valid_identity;

impl PreparedDispatchController {
    /// Revalidates a held dispatch and hands the approved material to `port` exactly once.
    ///
    /// # Errors
    ///
    /// Returns a binding, drift, staleness, cancellation or not-held refusal.  When the port
    /// reports an indeterminate outcome the receipt is still recorded, so a later resume returns
    /// the retained `unknown` receipt instead of writing again.
    pub fn resume(
        &mut self,
        dispatch_id: &str,
        current: &DispatchFences,
        port: &mut dyn PreparedDispatchPort,
    ) -> Result<DispatchReceipt, DispatchError> {
        if !valid_identity(dispatch_id) {
            return Err(DispatchError::InvalidBinding);
        }
        if let Some(receipt) = self.ledger.receipt(dispatch_id) {
            return Ok(receipt.clone());
        }
        if self.ledger.is_cancelled(dispatch_id) {
            return Err(DispatchError::Cancelled);
        }
        current.validate()?;
        let stop_epoch = self.stop_epoch;
        let fenced = {
            let dispatch = self
                .dispatches
                .get(dispatch_id)
                .ok_or(DispatchError::UnknownDispatch)?;
            if dispatch.approved_stop_epoch() != stop_epoch {
                Some(DispatchError::Stale)
            } else {
                match dispatch.state {
                    DispatchState::CommittedHeld => {
                        dispatch.fences.drift(current).map(DispatchError::Drift)
                    }
                    DispatchState::Stale => return Err(DispatchError::Stale),
                    DispatchState::Cancelled => return Err(DispatchError::Cancelled),
                    DispatchState::Drafted | DispatchState::Dispatched => {
                        return Err(DispatchError::NotHeld);
                    }
                }
            }
        };
        if let Some(error) = fenced {
            if let Some(dispatch) = self.dispatches.get_mut(dispatch_id) {
                dispatch.state = DispatchState::Stale;
            }
            return Err(error);
        }
        let write = {
            let dispatch = self
                .dispatches
                .get(dispatch_id)
                .ok_or(DispatchError::UnknownDispatch)?;
            port.write_prepared(ApprovedDispatchMaterial {
                dispatch_id,
                execution_id: &dispatch.input.execution_id,
                attempt_id: dispatch.input.attempt_id.as_deref(),
                adapter_id: &dispatch.input.adapter_id,
                boundary: dispatch.input.boundary,
                approved_material_sha256: &dispatch.input.approved_material_sha256,
                manifest_sha256: &dispatch.input.manifest_sha256,
                components: dispatch.input.components(),
            })
        };
        let (outcome, written_bytes, failure) = match write {
            Ok(bytes) => (DispatchOutcome::WriteCompleted, bytes, None),
            Err(error) => (DispatchOutcome::Unknown, 0, Some(error)),
        };
        let dispatch = self
            .dispatches
            .get(dispatch_id)
            .ok_or(DispatchError::UnknownDispatch)?;
        let receipt = DispatchReceipt {
            dispatch_id: dispatch_id.to_owned(),
            execution_id: dispatch.input.execution_id.clone(),
            attempt_id: dispatch.input.attempt_id.clone(),
            adapter_id: dispatch.input.adapter_id.clone(),
            boundary: dispatch.input.boundary,
            approved_material_sha256: dispatch.input.approved_material_sha256.clone(),
            manifest_sha256: dispatch.input.manifest_sha256.clone(),
            outcome,
            written_bytes,
            boundary_writes: 1,
            gameplay_effects: u32::from(matches!(outcome, DispatchOutcome::WriteCompleted)),
        };
        self.boundary_writes = self.boundary_writes.saturating_add(1);
        if matches!(outcome, DispatchOutcome::WriteCompleted) {
            self.gameplay_effects = self.gameplay_effects.saturating_add(1);
        }
        self.ledger.record_receipt(receipt.clone());
        if let Some(dispatch) = self.dispatches.get_mut(dispatch_id) {
            dispatch.state = DispatchState::Dispatched;
        }
        match failure {
            Some(error) => Err(error),
            None => Ok(receipt),
        }
    }
}
