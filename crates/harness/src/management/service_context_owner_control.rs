// SPDX-License-Identifier: MIT

//! Owner control-command submission and receipt recovery over the management
//! surface.
//!
//! The submission path forwards a caller-supplied `ContextControlCommand` to
//! the authoritative context owner for the run's **current** binding and
//! returns the owner's v2 receipt. The harness never mints control authority
//! here: it proves the command names the owner's current pre-command fence and
//! boundary before the owner sees it, and proves the returned receipt describes
//! exactly that command afterwards. The recovery path returns receipts the
//! owner already recorded and never re-issues, re-applies or infers an effect.

use super::super::super::context_owner::{
    ContextControlCommand, ContextControlReceipt, ContextOwnerBinding,
};
use super::super::support::authorize;
use super::{AuthContext, ManagementError, ManagementService, RunSnapshot, validate_identifier};

impl ManagementService {
    /// Submits one owner control command (pause, commit or resume) for the
    /// run's current binding and returns the owner-issued receipt.
    ///
    /// Every check runs before any owner effect:
    /// 1. scoped `workflow:control` for the run;
    /// 2. a command whose exact identity (idempotency key and payload) already
    ///    has an owner-recorded receipt returns that receipt and is not
    ///    re-submitted, so a retried request cannot apply a second effect;
    /// 3. the command's pre-command fence (`expected_control_version`, the
    ///    `expected_boundary` of a commit or resume, and a commit's
    ///    `expected_revision_id`) must be the owner's current binding; a stale
    ///    fence is refused with a typed conflict and the owner is never called.
    ///
    /// The owner's receipt is validated against the binding and command before
    /// it is returned, so an owner answering for a different transition is
    /// refused rather than projected.
    pub fn submit_context_control_command(
        &self,
        actor: &AuthContext,
        run_id: &str,
        command: &ContextControlCommand,
    ) -> Result<ContextControlReceipt, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:control", Some(run_id))?;
        let (_, binding) = self.current_context_composition(actor, run_id)?;
        if binding.continuity.receipt_recovery {
            let snapshot = self.admitted_run(run_id)?;
            if let Some(recorded) =
                self.recorded_context_control_receipt(actor, &snapshot, command)?
            {
                return Ok(recorded);
            }
        }
        ensure_current_fence(&binding, command)?;
        let receipt = self.context_owner.control(actor, &binding, command)?;
        receipt.validate_for(&binding, command)?;
        Ok(receipt)
    }

    /// Recovers the receipt the authoritative owner already issued for `command`,
    /// for a caller whose reply was lost or ambiguous.
    ///
    /// This path never re-issues, re-applies or infers an effect. It requires
    /// current scoped `workflow:read` for the run and exact owner-issued
    /// historical evidence for the supplied command. Receipt recovery does not
    /// assert that the old binding is a current runtime association. A recovered
    /// receipt must satisfy exact owner/invocation/binding/command identity
    /// before it is returned, so a receipt for one command cannot be replayed as
    /// another.
    pub fn recover_context_control_receipt(
        &self,
        actor: &AuthContext,
        run_id: &str,
        command: &ContextControlCommand,
    ) -> Result<ContextControlReceipt, ManagementError> {
        validate_identifier("run_id", run_id)?;
        authorize(actor, "workflow:read", Some(run_id))?;
        let snapshot = self.admitted_run(run_id)?;
        self.recorded_context_control_receipt(actor, &snapshot, command)?
            .ok_or_else(|| {
                ManagementError::invalid(
                    "context_control_receipt_not_recorded",
                    "the context owner has no recorded receipt for the supplied command",
                )
            })
    }

    /// The owner's recorded receipt for exactly `command`, or `None` when the
    /// owner has no such record. Shared by explicit recovery and by submission,
    /// so both answer a retried command with the same original evidence.
    fn recorded_context_control_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &RunSnapshot,
        command: &ContextControlCommand,
    ) -> Result<Option<ContextControlReceipt>, ManagementError> {
        let Some(recovery) = self
            .context_owner
            .recover_control_receipt(actor, snapshot, command)?
        else {
            return Ok(None);
        };
        let binding = recovery.binding;
        binding.validate(None)?;
        if binding.workflow_run_id != snapshot.workflow_run_id
            || binding.definition_digest != snapshot.definition_digest
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_scope",
                "historical receipt evidence is not attached to the admitted workflow run",
            ));
        }
        if !binding.continuity.receipt_recovery {
            return Err(ManagementError::unavailable(
                "context_control_receipt_recovery_unsupported",
                "the historical binding does not advertise receipt recovery",
            ));
        }
        recovery.receipt.validate_for(&binding, command)?;
        Ok(Some(recovery.receipt))
    }

    fn admitted_run(&self, run_id: &str) -> Result<RunSnapshot, ManagementError> {
        self.store
            .get_run(run_id)?
            .ok_or_else(|| ManagementError::invalid("run_not_found", "workflow run was not found"))
    }
}

/// Refuses a command whose pre-command fence is not the owner's current
/// binding, before the owner is asked to apply anything.
///
/// Each mismatch keeps its own code so a caller can tell a stale control
/// version, a stale boundary and a stale revision fence apart. No receipt is
/// issued for any of them.
fn ensure_current_fence(
    binding: &ContextOwnerBinding,
    command: &ContextControlCommand,
) -> Result<(), ManagementError> {
    let (expected_control_version, expected_boundary, expected_revision_id) = match command {
        ContextControlCommand::Pause {
            expected_control_version,
            ..
        } => (*expected_control_version, None, None),
        ContextControlCommand::Commit {
            expected_control_version,
            expected_revision_id,
            expected_boundary,
            ..
        } => (
            *expected_control_version,
            Some(expected_boundary),
            Some(expected_revision_id),
        ),
        ContextControlCommand::Resume {
            expected_control_version,
            expected_boundary,
            ..
        } => (*expected_control_version, Some(expected_boundary), None),
    };
    if expected_control_version != binding.boundary.control_version {
        return Err(ManagementError::conflict(
            "context_control_fence_stale",
            "command control version is not the owner's current pre-command fence",
        ));
    }
    if expected_boundary.is_some_and(|boundary| boundary != &binding.boundary) {
        return Err(ManagementError::conflict(
            "context_control_boundary_stale",
            "command boundary is not the owner's current authoritative boundary",
        ));
    }
    if expected_revision_id.is_some_and(|revision| revision != &binding.approved_revision_id) {
        return Err(ManagementError::conflict(
            "context_control_revision_stale",
            "command revision fence is not the owner's current approved revision",
        ));
    }
    Ok(())
}
