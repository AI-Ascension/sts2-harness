// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::ActiveContextSource;
use sts2_harness::management::{
    CONTEXT_OWNER_SOURCE_STATUS_SCHEMA_VERSION, ContextOwnerSourceStatus,
};

impl Owner {
    pub(super) fn source_status_current(
        &self,
        actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerSourceStatus, ManagementError> {
        let run = self.validate_snapshot_owner(actor, snapshot)?;
        let current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get(&run).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_source_status_unavailable",
                "fresh runtime observation and legal-action catalog are unavailable",
            )
        })?;
        self.validate_source_entry(actor, snapshot, entry)?;
        if entry.catalog_generation != Some(entry.authority.state().boundary.generation) {
            return Err(ManagementError::unavailable(
                "context_owner_source_status_unavailable",
                "fresh runtime legal-action catalog is unavailable",
            ));
        }
        let state = entry.authority.state();
        let active_source = entry
            .store
            .active_context_source(&state.active_revision_id)
            .map_err(|error| {
                ManagementError::unavailable("context_owner_source_status", error.to_string())
            })?
            .map(|(source, _)| ActiveContextSource {
                source_id: source.source_id,
                version: source.version,
                digest: source.digest,
                active_revision_id: source.active_revision_id,
            });
        Ok(ContextOwnerSourceStatus {
            schema_version: CONTEXT_OWNER_SOURCE_STATUS_SCHEMA_VERSION.to_owned(),
            owner_id: self.configuration.owner_id.clone(),
            owner_version: self.configuration.owner_version.clone(),
            workflow_run_id: run,
            definition_digest: snapshot.definition_digest.clone(),
            instance_id: entry.runtime_instance_id.clone(),
            boundary: state.boundary.clone(),
            active_revision_id: state.active_revision_id.clone(),
            active_source,
        })
    }
}
