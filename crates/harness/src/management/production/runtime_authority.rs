// SPDX-License-Identifier: MIT

//! Admission check for the runtime authority one served run is opened under.

use super::*;

pub(super) fn validate_runtime_authority_binding(
    request: &RunRequest,
    definition_digest: &str,
    binding: &RuntimeAuthorityBinding,
) -> Result<(), ManagementError> {
    let workflow_run_id = super::super::execution_records::live_run_id(request, definition_digest)?;
    if binding.instance_id != request.instance_id
        || binding.run_id != workflow_run_id
        || binding.session_id.is_empty()
        || binding.lease_id.is_empty()
        || binding.lease_epoch == 0
        || binding.episode_id.is_empty()
        || binding.trajectory_id.is_empty()
        || binding.trace_id.is_empty()
        || binding.artifact_id.is_empty()
        || binding.agent_id.is_empty()
        || binding.adapter_revision.is_empty()
        || binding.model_revision.is_empty()
        || binding.configuration_digest.len() != 64
        || binding.output_schema_digest.len() != 64
    {
        return Err(ManagementError::conflict(
            "runtime_authority_scope_mismatch",
            "runtime authority is not bound to the admitted workflow run",
        ));
    }
    Ok(())
}
