// SPDX-License-Identifier: MIT

use super::super::*;
use sts2_harness::context_control::StoreMode;

impl Owner {
    pub(super) fn control_current(
        &self,
        actor: &AuthContext,
        binding: &ContextOwnerBinding,
        command: &sts2_harness::management::ContextControlCommand,
    ) -> Result<sts2_harness::management::ContextControlReceipt, ManagementError> {
        if !actor.can("workflow:control") || !actor.can_run(&binding.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "context_owner_control_forbidden",
                "actor cannot control this context authority",
            ));
        }
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
        ContextOwnerEffectiveLimitsView::compose(&catalog, binding)?;
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&binding.workflow_run_id).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_association_unavailable",
                "current authority is unavailable",
            )
        })?;
        let request = entry.binding_request.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_association_unavailable",
                "current workflow invocation has not bound this context owner",
            )
        })?;
        let current_binding = self.binding_for_request(request, entry, &catalog)?;
        if entry.actor != actor.subject
            || entry.runtime_lease_id.is_empty()
            || entry.runtime_lease_epoch == 0
            || &current_binding != binding
        {
            return Err(ManagementError::conflict(
                "context_owner_control_stale",
                "context control binding is stale",
            ));
        }
        self.validate_control_limits_in_catalog(&catalog, &entry.admitted_control_limits)?;
        let previous_authority = entry.authority.clone();
        entry.authority = entry
            .authority
            .clone()
            .with_max_control_events(entry.admitted_control_limits.max_control_events)
            .map_err(|code| {
                let reason = if code == "context_control_events_exhausted" {
                    "context_control_events_exhausted"
                } else {
                    "context_control_event_limit_invalid"
                };
                ManagementError::conflict(
                    reason,
                    "current context control authority exceeds the admitted run limit",
                )
            })?;
        let outcome = match command {
            sts2_harness::management::ContextControlCommand::Pause {
                idempotency_key,
                expected_control_version,
            } => entry
                .authority
                .request_pause(idempotency_key, *expected_control_version),
            sts2_harness::management::ContextControlCommand::Commit {
                idempotency_key,
                expected_control_version,
                expected_revision_id,
                expected_boundary,
                preview_manifest_digest,
                approved_manifest_digest,
            } => entry.authority.commit(
                idempotency_key,
                *expected_control_version,
                expected_revision_id,
                expected_boundary,
                preview_manifest_digest,
                approved_manifest_digest,
            ),
            sts2_harness::management::ContextControlCommand::Resume {
                idempotency_key,
                expected_control_version,
                expected_boundary,
            } => entry.authority.resume(
                idempotency_key,
                *expected_control_version,
                expected_boundary,
            ),
        };
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                entry.authority = previous_authority;
                let code = if error == "context_control_events_exhausted" {
                    "context_control_events_exhausted"
                } else {
                    "context_owner_control_refused"
                };
                return Err(ManagementError::conflict(code, error));
            }
        };
        let state = entry.authority.state();
        let kind = match command {
            sts2_harness::management::ContextControlCommand::Pause { .. } => {
                sts2_harness::management::ContextControlCommandKind::Pause
            }
            sts2_harness::management::ContextControlCommand::Commit { .. } => {
                sts2_harness::management::ContextControlCommandKind::Commit
            }
            sts2_harness::management::ContextControlCommand::Resume { .. } => {
                sts2_harness::management::ContextControlCommandKind::Resume
            }
        };
        let (revision_id, preview_manifest_digest, approved_manifest_digest) = match command {
            sts2_harness::management::ContextControlCommand::Commit {
                preview_manifest_digest,
                approved_manifest_digest,
                ..
            } => (
                Some(state.active_revision_id.clone()),
                Some(preview_manifest_digest.clone()),
                Some(approved_manifest_digest.clone()),
            ),
            sts2_harness::management::ContextControlCommand::Pause { .. }
            | sts2_harness::management::ContextControlCommand::Resume { .. } => (None, None, None),
        };
        let receipt = sts2_harness::management::ContextControlReceipt {
            schema_version: sts2_harness::management::CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.into(),
            owner_id: self.configuration.owner_id.clone(),
            invocation_id: binding.invocation_id.clone(),
            binding_id: binding.binding_id.clone(),
            binding_digest: binding.binding_digest.clone(),
            command: kind,
            command_id: outcome.command_id,
            idempotency_key: outcome.idempotency_key,
            effect: outcome.effect,
            control_version: outcome.control_version,
            plan_epoch: outcome.plan_epoch,
            controller_epoch: state.boundary.controller_epoch,
            gate_epoch: state.boundary.gate_epoch,
            boundary: state.boundary.clone(),
            revision_id,
            preview_manifest_digest,
            approved_manifest_digest,
        };
        receipt.validate_for(binding, command)?;
        let durable = sts2_harness::context_control::DurableContextOwnerControlReceipt {
            owner_id: self.configuration.owner_id.clone(),
            actor_subject: actor.subject.clone(),
            binding: binding.clone(),
            command: command.clone(),
            receipt: receipt.clone(),
        };
        if let Err(error) = entry.store.persist_with_owner_control_receipt(
            &entry.authority,
            StoreMode::Enabled,
            &durable,
        ) {
            entry.authority = previous_authority;
            return Err(ManagementError::unavailable(
                "context_owner_persist",
                error.to_string(),
            ));
        }
        Ok(receipt)
    }
}
