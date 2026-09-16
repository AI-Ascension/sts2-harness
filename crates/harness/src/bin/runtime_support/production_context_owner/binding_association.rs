// SPDX-License-Identifier: MIT

use super::super::*;

impl Owner {
    pub(super) fn current_association(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let catalog = self.catalog(actor)?;
        catalog.validate()?;
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&snapshot.workflow_run_id).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_association_unavailable",
                "fresh runtime observation and legal-action catalog are unavailable",
            )
        })?;
        if entry.actor != actor.subject {
            return Err(ManagementError::forbidden(
                "context_owner_actor",
                "actor cannot read this context authority",
            ));
        }
        if entry.catalog_generation != Some(entry.authority.state().boundary.generation)
            || entry.runtime_lease_id.is_empty()
            || entry.runtime_lease_epoch == 0
        {
            return Err(ManagementError::unavailable(
                "context_owner_association_unavailable",
                "fresh runtime observation and legal-action catalog are unavailable",
            ));
        }
        self.validate_control_limits_in_catalog(&catalog, &entry.admitted_control_limits)?;
        let request = entry.binding_request.as_ref().ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_association_unavailable",
                "current workflow invocation has not bound this context owner",
            )
        })?;
        if request.workflow_run_id != snapshot.workflow_run_id
            || request.definition_digest != snapshot.definition_digest
        {
            return Err(ManagementError::conflict(
                "context_owner_association_scope",
                "current context binding does not match the admitted workflow run",
            ));
        }
        let binding = self.binding_for_request(request, entry, &catalog)?;
        binding.validate(Some(snapshot))?;
        Ok(binding)
    }

    pub(super) fn recover_historical_receipt(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        command: &sts2_harness::management::ContextControlCommand,
    ) -> Result<Option<sts2_harness::management::ContextControlReceiptRecovery>, ManagementError>
    {
        if !actor.can("workflow:read") || !actor.can_run(&snapshot.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "context_owner_receipt_forbidden",
                "actor cannot read context receipts for this workflow run",
            ));
        }
        let store = ContextControlStore::open(
            scoped_store_path(&self.configuration.store_path, &snapshot.workflow_run_id),
            self.key,
            &snapshot.workflow_run_id,
        )
        .map_err(|error| {
            ManagementError::unavailable("context_owner_receipt_store", error.to_string())
        })?;
        let record = store
            .lookup_owner_control_receipt(&self.configuration.owner_id, &actor.subject, command)
            .map_err(|error| {
                ManagementError::unavailable("context_owner_receipt_store", error.to_string())
            })?;
        let Some(record) = record else {
            return Ok(None);
        };
        if record.binding.workflow_run_id != snapshot.workflow_run_id
            || record.binding.definition_digest != snapshot.definition_digest
            || record.binding.owner_id != self.configuration.owner_id
        {
            return Err(ManagementError::conflict(
                "context_control_receipt_scope",
                "stored receipt evidence does not match the admitted workflow run",
            ));
        }
        Ok(Some(
            sts2_harness::management::ContextControlReceiptRecovery {
                binding: record.binding,
                receipt: record.receipt,
            },
        ))
    }
}
