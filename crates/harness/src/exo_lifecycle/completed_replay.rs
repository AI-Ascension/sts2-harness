// SPDX-License-Identifier: MIT

use super::{InvocationManifest, LifecycleError, LifecycleOwner, LifecyclePhase};

impl LifecycleOwner {
    /// Finds an exact completed request before the runtime tries to admit another one-shot native
    /// operation. `stored` still revalidates the broker and current authority before returning it.
    pub(super) fn completed_manifest_for_request(
        &mut self,
        request: &InvocationManifest,
        input: &[u8],
    ) -> Result<Option<InvocationManifest>, LifecycleError> {
        self.check()?;
        super::validation::input(request, input)?;
        let Some(entry) = self
            .snapshot
            .entries
            .iter()
            .find(|entry| entry.manifest.execution_id == request.execution_id)
        else {
            return Ok(None);
        };
        if !same_request(&entry.manifest, request) {
            return Err(LifecycleError::Stale);
        }
        match entry.phase {
            LifecyclePhase::Completed => Ok(Some(entry.manifest.clone())),
            LifecyclePhase::Unknown | LifecyclePhase::Sent => Err(LifecycleError::Unknown),
            _ => Err(LifecycleError::Held),
        }
    }
}

fn same_request(completed: &InvocationManifest, request: &InvocationManifest) -> bool {
    completed.scope == request.scope
        && completed.execution_id == request.execution_id
        && completed.episode_attempt_id == request.episode_attempt_id
        && completed.trajectory_id == request.trajectory_id
        && completed.request_id == request.request_id
        && completed.host_turn_id == request.host_turn_id
        && completed.input_digest == request.input_digest
        && completed.input_length == request.input_length
        && completed.config_digest == request.config_digest
        && completed.package_digest == request.package_digest
        && completed.profile_digest == request.profile_digest
        && completed.model_revision == request.model_revision
        && completed.reserved_units == request.reserved_units
        && completed.authority.lease_id == request.authority.lease_id
        && completed.authority.lease_epoch == request.authority.lease_epoch
        && completed.authority.state_id == request.authority.state_id
        && completed.authority.generation == request.authority.generation
        && completed.authority.catalog_digest == request.authority.catalog_digest
}
