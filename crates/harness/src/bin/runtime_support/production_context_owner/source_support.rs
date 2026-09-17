// SPDX-License-Identifier: MIT

use super::*;

impl Owner {
    pub(super) fn validate_snapshot_owner(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<String, ManagementError> {
        if !actor.can_run(&snapshot.workflow_run_id) {
            return Err(ManagementError::forbidden(
                "context_owner_run_scope",
                "actor cannot access this workflow run",
            ));
        }
        let expected_run = snapshot.workflow_run_id.as_str();
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(expected_run).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_source_status_unavailable",
                "fresh runtime observation is unavailable",
            )
        })?;
        self.validate_source_entry(actor, snapshot, entry)?;
        Ok(expected_run.to_owned())
    }

    pub(super) fn validate_source_entry(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
        entry: &Current,
    ) -> Result<(), ManagementError> {
        if entry.actor != actor.subject
            || entry.authority.state().boundary.run_id != snapshot.workflow_run_id
            || entry.definition_digest != snapshot.definition_digest
            || snapshot
                .admission
                .as_ref()
                .is_some_and(|admission| admission.target.instance_id != entry.runtime_instance_id)
        {
            return Err(ManagementError::conflict(
                "context_owner_source_scope",
                "context source owner does not match the admitted workflow run and target",
            ));
        }
        Ok(())
    }

    pub(super) fn advertised_source(
        &self,
        source_id: &str,
    ) -> Result<ContextBindingSource, ManagementError> {
        let mut matches = self
            .configuration
            .sources
            .iter()
            .filter(|source| source.source_id == source_id);
        let source = matches.next().cloned().ok_or_else(|| {
            ManagementError::capability(
                "context_source_not_advertised",
                "source identity is not advertised by this owner",
            )
        })?;
        if matches.next().is_some() {
            return Err(ManagementError::conflict(
                "context_source_ambiguous",
                "owner advertises multiple versions of the source identity",
            ));
        }
        Ok(source)
    }
}
