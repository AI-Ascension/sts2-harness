// SPDX-License-Identifier: MIT

//! Synthetic context-owner double for the control-command HTTP suite. It
//! applies pause, commit and resume to its own current state and records every
//! receipt it issued; no provider or game is launched.

use std::sync::Mutex;

use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, ContextBindingCatalog,
    ContextBindingRequest, ContextControlCommand, ContextControlCommandKind, ContextControlReceipt,
    ContextControlReceiptRecovery, ContextOwnerBinding, ContextOwnerPort, ManagementError,
    RunSnapshot,
};

#[allow(dead_code)]
#[path = "context_owner_effective_limits.rs"]
mod fixture;

use fixture::{Scenario, binding_for, catalog_for};
pub use fixture::{actor, boundary, error_code, run_id, service};

/// Owner double that applies control commands to its own current state. It
/// deliberately performs no fence or boundary check of its own: the harness
/// route is the unit under test and must refuse a stale command before this
/// owner ever sees it.
#[derive(Default)]
pub struct ControlOwner {
    state: Mutex<OwnerState>,
}

#[derive(Default)]
struct OwnerState {
    boundary: Option<ContextBoundary>,
    plan_epoch: Option<u64>,
    approved_revision_id: Option<String>,
    issued: Vec<(
        ContextControlCommand,
        ContextOwnerBinding,
        ContextControlReceipt,
    )>,
    effects: usize,
}

impl ControlOwner {
    pub fn effects(&self) -> usize {
        self.state.lock().expect("owner state").effects
    }
}

fn idempotency_key(command: &ContextControlCommand) -> String {
    match command {
        ContextControlCommand::Pause {
            idempotency_key, ..
        }
        | ContextControlCommand::Commit {
            idempotency_key, ..
        }
        | ContextControlCommand::Resume {
            idempotency_key, ..
        } => idempotency_key.clone(),
    }
}

impl ContextOwnerPort for ControlOwner {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        Ok(catalog_for(Scenario::Matching))
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        _request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "test_bind_unused",
            "bind is not used by the control path",
        ))
    }

    fn association(
        &self,
        _actor: &AuthContext,
        snapshot: &RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let published = catalog_for(Scenario::Matching).descriptors[0].clone();
        let mut binding = binding_for(Scenario::Matching, &snapshot.workflow_run_id, &published);
        // A real owner binds the admitted run; recovery checks this digest.
        binding.definition_digest = snapshot.definition_digest.clone();
        let state = self.state.lock().expect("owner state");
        if let Some(boundary) = &state.boundary {
            binding.boundary = boundary.clone();
        }
        if let Some(plan_epoch) = state.plan_epoch {
            binding.plan_epoch = plan_epoch;
        }
        if let Some(revision) = &state.approved_revision_id {
            binding.approved_revision_id = revision.clone();
        }
        Ok(binding)
    }

    fn control(
        &self,
        _actor: &AuthContext,
        binding: &ContextOwnerBinding,
        command: &ContextControlCommand,
    ) -> Result<ContextControlReceipt, ManagementError> {
        let mut state = self.state.lock().expect("owner state");
        let key = idempotency_key(command);
        if state
            .issued
            .iter()
            .any(|(issued, _, _)| idempotency_key(issued) == key && issued != command)
        {
            return Err(ManagementError::conflict(
                "context_owner_control_refused",
                "idempotency_conflict",
            ));
        }
        let (kind, effect, advances_gate) = match command {
            ContextControlCommand::Pause { .. } => {
                (ContextControlCommandKind::Pause, "pause_requested", true)
            }
            ContextControlCommand::Commit { .. } => (
                ContextControlCommandKind::Commit,
                "revision_committed",
                false,
            ),
            ContextControlCommand::Resume { .. } => {
                (ContextControlCommandKind::Resume, "resume_accepted", true)
            }
        };
        let mut boundary = binding.boundary.clone();
        boundary.control_version += 1;
        if advances_gate {
            boundary.gate_epoch += 1;
        }
        let mut plan_epoch = binding.plan_epoch;
        let commit_identity = match command {
            ContextControlCommand::Commit {
                preview_manifest_digest,
                approved_manifest_digest,
                ..
            } => {
                plan_epoch += 1;
                Some((
                    format!("test.revision.{plan_epoch}"),
                    preview_manifest_digest.clone(),
                    approved_manifest_digest.clone(),
                ))
            }
            _ => None,
        };
        state.effects += 1;
        let receipt = ContextControlReceipt {
            schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
            owner_id: binding.owner_id.clone(),
            invocation_id: binding.invocation_id.clone(),
            binding_id: binding.binding_id.clone(),
            binding_digest: binding.binding_digest.clone(),
            command: kind,
            command_id: format!("test.command.{}", state.effects),
            idempotency_key: key,
            effect: effect.to_owned(),
            control_version: boundary.control_version,
            plan_epoch,
            controller_epoch: boundary.controller_epoch,
            gate_epoch: boundary.gate_epoch,
            boundary: boundary.clone(),
            revision_id: commit_identity
                .as_ref()
                .map(|(revision, _, _)| revision.clone()),
            preview_manifest_digest: commit_identity
                .as_ref()
                .map(|(_, preview, _)| preview.clone()),
            approved_manifest_digest: commit_identity
                .as_ref()
                .map(|(_, _, approved)| approved.clone()),
        };
        state.boundary = Some(boundary);
        state.plan_epoch = Some(plan_epoch);
        if let Some((revision, _, _)) = commit_identity {
            state.approved_revision_id = Some(revision);
        }
        state
            .issued
            .push((command.clone(), binding.clone(), receipt.clone()));
        Ok(receipt)
    }

    fn recover_control_receipt(
        &self,
        _actor: &AuthContext,
        _snapshot: &RunSnapshot,
        command: &ContextControlCommand,
    ) -> Result<Option<ContextControlReceiptRecovery>, ManagementError> {
        let state = self.state.lock().expect("owner state");
        Ok(state
            .issued
            .iter()
            .find(|(issued, _, _)| issued == command)
            .map(|(_, binding, receipt)| ContextControlReceiptRecovery {
                binding: binding.clone(),
                receipt: receipt.clone(),
            }))
    }

    fn is_available(&self) -> bool {
        true
    }
}
