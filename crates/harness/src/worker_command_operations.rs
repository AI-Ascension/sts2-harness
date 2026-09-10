// SPDX-License-Identifier: MIT

//! Store-backed command mappings kept separate from binding and preparation policy.

use crate::execution::{
    ExecutionStore, ExecutionStoreError, StoredWorkerHandoff, WorkerCompletionStatus,
    WorkerControlMode, WorkerControlRequest, WorkerLookup,
};

use super::super::{
    AcknowledgmentStatus, DispatchReply, LookupReply, ProbeReply, WorkerCommand, WorkerReply,
    WorkerRequest,
};
use super::WorkerCommandAdmission;
use super::support::{
    AuthenticatedWorkerRequest, WorkerCommandError, WorkerCommandResult, number, text, tuple,
};

impl WorkerCommandAdmission {
    pub(super) fn dispatch_reply(
        &self,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        if authenticated.request.command() == WorkerCommand::Dispatch {
            Err(WorkerCommandError::ReservationMismatch)
        } else {
            Err(WorkerCommandError::InvalidRequest)
        }
    }

    pub(super) fn duplicate_dispatch_reply(
        &self,
        handoff: StoredWorkerHandoff,
    ) -> Result<DispatchReply, WorkerCommandError> {
        let Some(terminal) = handoff.terminal else {
            return Ok(DispatchReply::Busy);
        };
        let record = terminal.terminal_record()?;
        Ok(match terminal.status {
            WorkerCompletionStatus::Completed => DispatchReply::AlreadyCompleted(record),
            WorkerCompletionStatus::Failed => DispatchReply::Terminal(record),
        })
    }

    pub(super) fn probe(
        &self,
        store: &ExecutionStore,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        let control = store.worker_control()?;
        let ready = control.is_some_and(|current| {
            current.deployment_id == self.config.deployment_id
                && current.worker_owner_id == self.config.worker_owner_id
                && current.worker_profile_digest == self.config.worker_profile_digest
                && current.worker_boot_id == self.config.worker_boot_id
                && current.watchdog_boot_id.as_deref() == Some(&self.config.watchdog_boot_id)
                && current.mode == WorkerControlMode::Running
                && current.mode_sequence > 0
                && current.authenticated
                && current.admitting
        });
        Ok(WorkerCommandResult::reply(WorkerReply::Probe(ProbeReply {
            deployment_id: self.config.deployment_id.clone(),
            worker_owner_id: self.config.worker_owner_id.clone(),
            worker_profile_digest: self.config.worker_profile_digest.clone(),
            release_digest: self.config.release_digest.clone(),
            config_digest: self.config.config_digest.clone(),
            ready,
        })))
    }

    pub(super) fn lookup(
        &self,
        store: &mut ExecutionStore,
        request: &WorkerRequest,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        let tuple = tuple(request)?;
        let reply = match store.lookup_worker_handoff(&tuple)? {
            WorkerLookup::Unknown { .. } => LookupReply::Unknown,
            WorkerLookup::Known(handoff) => match handoff.terminal {
                Some(terminal) => LookupReply::Terminal(terminal.terminal_record()?),
                None => match handoff.state {
                    crate::execution::WorkerHandoffState::Admitted
                    | crate::execution::WorkerHandoffState::Running => LookupReply::Running,
                    crate::execution::WorkerHandoffState::Unknown => LookupReply::Unknown,
                    crate::execution::WorkerHandoffState::Terminal
                    | crate::execution::WorkerHandoffState::Acknowledged => {
                        return Err(WorkerCommandError::Store);
                    }
                },
            },
        };
        Ok(WorkerCommandResult::reply(WorkerReply::Lookup(reply)))
    }

    pub(super) fn acknowledge(
        &self,
        store: &mut ExecutionStore,
        request: &WorkerRequest,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        let tuple = tuple(request)?;
        let Some(existing) = store.worker_handoff(&tuple.handoff_id)? else {
            return Ok(WorkerCommandResult::reply(WorkerReply::Acknowledge(
                AcknowledgmentStatus::Rejected,
            )));
        };
        if existing.tuple != tuple {
            return Ok(WorkerCommandResult::reply(WorkerReply::Acknowledge(
                AcknowledgmentStatus::Conflict,
            )));
        }
        let was_acknowledged = existing.acknowledged;
        let terminal_digest = text(request.fields(), "terminal_digest")?;
        let reply = match store.acknowledge_worker_handoff(&tuple, terminal_digest) {
            Ok(_) if was_acknowledged => AcknowledgmentStatus::AlreadyAcknowledged,
            Ok(_) => AcknowledgmentStatus::Acknowledged,
            Err(ExecutionStoreError::Conflict) => AcknowledgmentStatus::Conflict,
            Err(ExecutionStoreError::Missing) => AcknowledgmentStatus::Rejected,
            Err(_) => return Err(WorkerCommandError::Store),
        };
        Ok(WorkerCommandResult::reply(WorkerReply::Acknowledge(reply)))
    }

    pub(super) fn set_control(
        &self,
        store: &mut ExecutionStore,
        authenticated: &AuthenticatedWorkerRequest,
    ) -> Result<WorkerCommandResult, WorkerCommandError> {
        let fields = authenticated.request.fields();
        let mode = match text(fields, "mode")? {
            "running" => WorkerControlMode::Running,
            "paused" => WorkerControlMode::Paused,
            "draining" => WorkerControlMode::Draining,
            "stopped" => WorkerControlMode::Stopped,
            _ => return Err(WorkerCommandError::InvalidRequest),
        };
        let control = WorkerControlRequest::new(
            text(fields, "deployment_id")?,
            text(fields, "worker_owner_id")?,
            text(fields, "worker_profile_digest")?,
            text(fields, "watchdog_boot_id")?,
            text(fields, "worker_boot_id")?,
            mode,
            number(fields, "mode_sequence")?,
        )
        .map_err(|_| WorkerCommandError::InvalidRequest)?;
        let accepted = match store.set_worker_control_mode(&control, &authenticated.owner_proof) {
            Ok(_) => true,
            Err(ExecutionStoreError::Conflict | ExecutionStoreError::Busy) => false,
            Err(_) => return Err(WorkerCommandError::Store),
        };
        Ok(WorkerCommandResult::reply(WorkerReply::Control {
            accepted,
        }))
    }
}
