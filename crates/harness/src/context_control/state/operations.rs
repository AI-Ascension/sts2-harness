// SPDX-License-Identifier: MIT

//! Operation admission and settlement for the control authority.
//!
//! An operation is the unit a plan epoch is spent against. It is admitted under the current plan
//! epoch, settled when the runtime reports it finished, and retained as *unresolved* when the
//! runtime reports an outcome this build cannot interpret. Retaining rather than dropping is what
//! keeps a later decision from silently inheriting an operation whose real effect is unknown.

use super::super::state::{ControlAuthority, GateStatus};
use super::digest::validate_operation_id;

impl ControlAuthority {
    pub fn admit_operation(&mut self, operation_id: &str, plan_epoch: u64) -> Result<(), String> {
        validate_operation_id(operation_id)?;
        self.admit_plan(plan_epoch)?;
        if self
            .state
            .unresolved_operations
            .iter()
            .any(|existing| existing == operation_id)
        {
            return Err("operation_in_flight".to_owned());
        }
        self.reserve_events(1)?;
        self.state
            .unresolved_operations
            .push(operation_id.to_owned());
        self.event("operation.admitted", None, Some("awaiting_settlement"))?;
        Ok(())
    }

    /// Settle one admitted operation and, when pause is latched, publish the safe boundary only
    /// after every unresolved operation has been reconciled.
    pub fn settle_operation(&mut self, operation_id: &str) -> Result<(), String> {
        validate_operation_id(operation_id)?;
        let Some(index) = self
            .state
            .unresolved_operations
            .iter()
            .position(|existing| existing == operation_id)
        else {
            return Err("unknown_operation".to_owned());
        };
        // The safe-boundary event is published only when this settlement empties the ledger, so the
        // reservation must account for both transitions before anything is mutated.
        let events_needed = if self.state.pause_latched
            && self.state.unresolved_operations.len() == 1
            && self.state.status == GateStatus::PauseRequested
        {
            2
        } else {
            1
        };
        self.reserve_events(events_needed)?;
        self.state.unresolved_operations.remove(index);
        self.event("operation.settled", None, None)?;
        if self.state.pause_latched
            && self.state.unresolved_operations.is_empty()
            && self.state.status == GateStatus::PauseRequested
        {
            self.state.status = GateStatus::PausedReady;
            self.event("pause.ready", None, Some("all_operations_settled"))?;
        }
        Ok(())
    }

    /// Retain an unknown operation as an unresolved ledger entry. Recovery must reconcile the
    /// original identity before readiness can be reported or resume can be accepted.
    pub fn retain_unknown_operation(&mut self, operation_id: &str) -> Result<(), String> {
        validate_operation_id(operation_id)?;
        if !self
            .state
            .unresolved_operations
            .iter()
            .any(|existing| existing == operation_id)
        {
            return Err("unknown_operation".to_owned());
        }
        self.reserve_events(1)?;
        self.event("operation.unknown", None, Some("reconciliation_required"))?;
        Ok(())
    }

    /// Accept a provider completion only for the current plan epoch. A late completion from an
    /// older plan is discarded before it can acquire authority or mutate the ledger.
    pub fn complete_provider_attempt(
        &mut self,
        attempt_id: &str,
        plan_epoch: u64,
    ) -> Result<(), String> {
        validate_operation_id(attempt_id)?;
        if plan_epoch != self.state.plan_epoch {
            return Err("obsolete_plan".to_owned());
        }
        self.settle_operation(attempt_id)
    }
}
