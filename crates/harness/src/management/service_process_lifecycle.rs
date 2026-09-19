// SPDX-License-Identifier: MIT

//! Authenticated lifecycle command application for the served-live composition.
//!
//! Ordering is enforced here, not left to the caller:
//!
//! 1. the command envelope and its exact instance are validated;
//! 2. the run is resolved and its scope authorized;
//! 3. durable intent is recorded **before** any gateway call;
//! 4. the gateway is asked exactly once;
//! 5. the answer is validated against the submitted operation and instance;
//! 6. the settled outcome is recorded, and the run event is appended.
//!
//! A transport failure between steps 4 and 5 is *not* an error to the caller:
//! the intent is durable and the operation is retained as `Unknown`, so the
//! next reconciliation reads the gateway's own record by identity instead of
//! guessing. That is the difference between a lost response and a lost effect.

use std::sync::{Arc, Mutex};

use super::support::authorize;
use super::*;
use crate::management::lifecycle::{
    LifecycleCommand, LifecycleCommandResponse, LifecycleState, LifecycleTarget,
    PROCESS_LIFECYCLE_STATUS_SCHEMA_VERSION, ProcessLifecycleCapability, ProcessLifecyclePort,
    validate_lifecycle_command,
};
use crate::management::lifecycle_intent::LifecycleIntentStore;

#[path = "service_process_lifecycle_outcome.rs"]
mod outcome;

use outcome::lock_error;

impl ManagementService {
    /// Attaches the gateway-owned lifecycle surface and its durable intent store.
    ///
    /// Both are required together: a port without durable intent could accept an
    /// operation it cannot reconcile after a restart, so the composition refuses
    /// that pairing rather than defaulting one of them.
    pub fn with_process_lifecycle(
        mut self,
        port: Arc<dyn ProcessLifecyclePort>,
        intents: Arc<Mutex<LifecycleIntentStore>>,
    ) -> Self {
        self.process_lifecycle = port;
        self.lifecycle_intents = Some(intents);
        self
    }

    /// The attached lifecycle surface.
    #[must_use]
    pub fn process_lifecycle_port(&self) -> &dyn ProcessLifecyclePort {
        self.process_lifecycle.as_ref()
    }

    /// Reports the gateway's advertised lifecycle capability for one run.
    pub fn lifecycle_capability(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<ProcessLifecycleCapability, ManagementError> {
        let (_snapshot, target) = self.lifecycle_scope(actor, run_id)?;
        let capability = self.process_lifecycle.capability(actor, &target)?;
        capability.validate(&target.instance_id)?;
        Ok(capability)
    }

    /// Applies one lifecycle command, persisting intent before any effect.
    pub fn lifecycle_command(
        &self,
        actor: &AuthContext,
        command: LifecycleCommand,
    ) -> Result<LifecycleCommandResponse, ManagementError> {
        if command.schema_version
            != crate::management::lifecycle::PROCESS_LIFECYCLE_COMMAND_SCHEMA_VERSION
        {
            return Err(ManagementError::invalid(
                "lifecycle_command_schema",
                "lifecycle command schema version is unsupported",
            ));
        }
        let (snapshot, target) = self.lifecycle_scope(actor, &command.run_id)?;
        validate_lifecycle_command(&command, &target.instance_id)?;
        if command.actor_scope != actor.subject {
            return Err(ManagementError::forbidden(
                "lifecycle_actor_scope_mismatch",
                "lifecycle command actor scope does not match the authenticated actor",
            ));
        }
        if command.expected_revision != snapshot.run_revision {
            return Err(ManagementError::conflict(
                "stale_revision",
                "lifecycle command expected revision is not current",
            ));
        }
        if let Some(existing) = self.replayed_lifecycle_command(&command)? {
            return Ok(existing);
        }
        let intents = self.lifecycle_intents.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "process_lifecycle_owner_unavailable",
                "no durable lifecycle intent owner is attached to this composition",
            )
        })?;
        let intent = {
            let mut intents = intents.lock().map_err(lock_error)?;
            intents.record_intent(
                &command.command_id,
                &command.run_id,
                &target.instance_id,
                command.operation_id,
                command.authority_epoch,
                &command.action,
            )?
        };
        if intent.classification.is_some() {
            // A duplicate submission of an already-settled operation replays the
            // retained outcome and never re-issues the effect.
            return self.replay_settled_lifecycle(&command, &intent);
        }
        if self.stop_precedes_start(&target.instance_id, &command, intent.sequence)? {
            return Err(ManagementError::conflict(
                "process_lifecycle_stop_dominates",
                "a settled stop dominates this later lifecycle start",
            ));
        }
        let action_kind = command.action.kind();
        let outcome = self
            .process_lifecycle
            .submit(actor, &target, &command, &command.action);
        self.settle_lifecycle(
            &command.command_id,
            &command.run_id,
            command.operation_id,
            action_kind,
            &target,
            outcome,
        )
    }

    /// Reconciles one retained operation by its identity.
    pub fn reconcile_lifecycle_operation(
        &self,
        actor: &AuthContext,
        run_id: &str,
        operation_id: u64,
    ) -> Result<LifecycleCommandResponse, ManagementError> {
        let (_snapshot, target) = self.lifecycle_scope(actor, run_id)?;
        let intents = self.lifecycle_intents.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "process_lifecycle_owner_unavailable",
                "no durable lifecycle intent owner is attached to this composition",
            )
        })?;
        let intent = {
            let intents = intents.lock().map_err(lock_error)?;
            intents.intent(operation_id).cloned().ok_or_else(|| {
                ManagementError::invalid(
                    "process_lifecycle_operation_not_found",
                    "no durable lifecycle intent is retained for this operation",
                )
            })?
        };
        let outcome = self.process_lifecycle.lookup(actor, &target, operation_id);
        self.settle_lifecycle(
            &intent.command_id,
            run_id,
            intent.operation_id,
            &intent.action_kind,
            &target,
            outcome,
        )
    }

    /// Returns the durable intents that still need reconciliation.
    pub fn unresolved_lifecycle_operations(
        &self,
    ) -> Result<Vec<crate::management::lifecycle_intent::LifecycleIntent>, ManagementError> {
        let intents = self.lifecycle_intents.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "process_lifecycle_owner_unavailable",
                "no durable lifecycle intent owner is attached to this composition",
            )
        })?;
        let intents = intents.lock().map_err(lock_error)?;
        Ok(intents.unresolved())
    }

    /// Resolves the authorized run and the exact instance its admission names.
    ///
    /// The instance comes from the run's admitted target binding. A run without
    /// one is refused, so a lifecycle command can never fall back to a
    /// configured or default instance.
    fn lifecycle_scope(
        &self,
        actor: &AuthContext,
        run_id: &str,
    ) -> Result<(RunSnapshot, LifecycleTarget), ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:control", Some(run_id))?;
        let snapshot = self.store.get_run(run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let admission = snapshot.admission.as_ref().ok_or_else(|| {
            ManagementError::conflict(
                "lifecycle_target_not_admitted",
                "lifecycle commands require a run with an admitted target binding",
            )
        })?;
        let target = LifecycleTarget::new(admission.target.instance_id.clone())?;
        Ok((snapshot, target))
    }

    fn stop_precedes_start(
        &self,
        instance_id: &str,
        command: &LifecycleCommand,
        sequence: u64,
    ) -> Result<bool, ManagementError> {
        if !command.action.is_effect_bearing() {
            return Ok(false);
        }
        let intents = self.lifecycle_intents.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "process_lifecycle_owner_unavailable",
                "no durable lifecycle intent owner is attached to this composition",
            )
        })?;
        let intents = intents.lock().map_err(lock_error)?;
        Ok(intents.stop_dominates(instance_id, sequence))
    }

    /// Finds an already-applied lifecycle command by its command identity.
    fn replayed_lifecycle_command(
        &self,
        command: &LifecycleCommand,
    ) -> Result<Option<LifecycleCommandResponse>, ManagementError> {
        let Some(intents) = self.lifecycle_intents.as_ref() else {
            return Ok(None);
        };
        let intents = intents.lock().map_err(lock_error)?;
        let retained = intents
            .intents()
            .values()
            .find(|intent| intent.command_id == command.command_id)
            .cloned();
        drop(intents);
        match retained {
            Some(intent) if intent.classification.is_some() => {
                self.replay_settled_lifecycle(command, &intent).map(Some)
            }
            Some(_) => Ok(None),
            None => Ok(None),
        }
    }

    fn replay_settled_lifecycle(
        &self,
        command: &LifecycleCommand,
        intent: &crate::management::lifecycle_intent::LifecycleIntent,
    ) -> Result<LifecycleCommandResponse, ManagementError> {
        let snapshot = self.store.get_run(&command.run_id)?.ok_or_else(|| {
            ManagementError::invalid("run_not_found", "workflow run was not found")
        })?;
        let classification = intent.classification.ok_or_else(|| {
            ManagementError::unresolved(
                "process_lifecycle_unresolved",
                "lifecycle operation has no settled outcome to replay",
            )
        })?;
        Ok(LifecycleCommandResponse {
            schema_version: PROCESS_LIFECYCLE_STATUS_SCHEMA_VERSION.to_owned(),
            command_id: intent.command_id.clone(),
            workflow_run_id: command.run_id.clone(),
            operation_id: intent.operation_id,
            instance_id: intent.instance_id.clone(),
            run_revision: snapshot.run_revision,
            classification,
            operation_state: intent
                .operation_state
                .unwrap_or(crate::management::lifecycle::LifecycleOperationState::Unknown),
            state: intent.state.unwrap_or(LifecycleState::Unknown),
            authority_epoch: intent.authority_epoch,
            reason_code: format!("lifecycle_{}", classification.as_str()),
            gameplay_ready: false,
            failure: intent.failure.clone(),
        })
    }
}
