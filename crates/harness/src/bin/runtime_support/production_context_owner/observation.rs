// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::context_control::{ContextBoundary, StoreMode};
use sts2_harness::sha256_hex;

impl LiveContextObservationPort for Owner {
    fn record_observation(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        digest: &str,
        binding: &RuntimeAuthorityBinding,
        observation: &EpisodeObservation,
        control_limits: &ContextOwnerControlLimits,
    ) -> Result<(), ManagementError> {
        self.validate_control_limits(actor, control_limits)?;
        let run_id = run_id(request, digest)?;
        if binding.instance_id != request.instance_id {
            return Err(ManagementError::conflict(
                "context_owner_instance",
                "runtime authority binding does not match the requested instance",
            ));
        }
        if binding.run_id != run_id || binding.lease_id.is_empty() || binding.lease_epoch == 0 {
            return Err(ManagementError::conflict(
                "context_owner_runtime_scope",
                "runtime authority is not bound to the admitted workflow run",
            ));
        }
        let fair_play = observation.fair_play().as_value();
        let boundary = ContextBoundary {
            run_id: run_id.clone(),
            episode_id: binding.episode_id.clone(),
            agent_id: binding.agent_id.clone(),
            state_id: observation.state_id().into(),
            generation: observation.generation(),
            observation_sha256: sha256_hex(serde_json::to_vec(fair_play).map_err(|error| {
                ManagementError::invalid("context_observation_encode", error.to_string())
            })?),
            catalog_sha256: sha256_hex(b"catalog-unavailable"),
            adapter_revision: binding.adapter_revision.clone(),
            model_revision: binding.model_revision.clone(),
            configuration_sha256: binding.configuration_digest.clone(),
            output_schema_sha256: binding.output_schema_digest.clone(),
            controller_epoch: 1,
            gate_epoch: 1,
            control_version: 1,
        };
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        if let Some(entry) = current.get_mut(&run_id) {
            if entry.actor != actor.subject
                || entry.runtime_lease_id != binding.lease_id
                || entry.runtime_lease_epoch != binding.lease_epoch
            {
                return Err(ManagementError::forbidden(
                    "context_owner_runtime_scope",
                    "actor or runtime lease cannot replace this context authority",
                ));
            }
            if entry.admitted_control_limits != *control_limits {
                return Err(ManagementError::conflict(
                    "context_control_limits_changed",
                    "live observation cannot replace the admitted control limits",
                ));
            }
            let retains_catalog = entry.catalog_generation == Some(observation.generation())
                && entry.authority.state().boundary.state_id == observation.state_id();
            entry
                .authority
                .record_observation_boundary(boundary)
                .map_err(|e| ManagementError::conflict("context_owner_observation_stale", e))?;
            entry
                .store
                .persist(&entry.authority, StoreMode::Enabled)
                .map_err(|e| {
                    ManagementError::unavailable("context_owner_persist", e.to_string())
                })?;
            entry.catalog_generation = retains_catalog.then_some(observation.generation());
            return Ok(());
        }
        let mut store = ContextControlStore::open(
            scoped_store_path(&self.configuration.store_path, &run_id),
            self.key,
            &run_id,
        )
        .map_err(|e| ManagementError::unavailable("context_owner_store", e.to_string()))?;
        let mut authority = match store.load() {
            Ok(authority) => {
                if authority.state().boundary.episode_id != binding.episode_id
                    || authority.state().boundary.agent_id != binding.agent_id
                    || authority.state().boundary.configuration_sha256
                        != binding.configuration_digest
                    || authority.state().boundary.output_schema_sha256
                        != binding.output_schema_digest
                {
                    return Err(ManagementError::conflict(
                        "context_owner_recovered_scope",
                        "recovered context authority belongs to a different runtime scope",
                    ));
                }
                authority
                    .with_max_control_events(control_limits.max_control_events)
                    .map_err(control_limit_error)?
            }
            Err(sts2_harness::context_control::DurableControlStoreError::Missing) => {
                let authority = ControlAuthority::new(boundary.clone(), "context.revision.1")
                    .with_max_control_events(control_limits.max_control_events)
                    .map_err(control_limit_error)?;
                store.persist(&authority, StoreMode::Enabled).map_err(|e| {
                    ManagementError::unavailable("context_owner_persist", e.to_string())
                })?;
                authority
            }
            Err(error) => {
                return Err(ManagementError::unavailable(
                    "context_owner_recover",
                    error.to_string(),
                ));
            }
        };
        authority
            .record_observation_boundary(boundary)
            .map_err(|error| ManagementError::conflict("context_owner_observation_stale", error))?;
        store
            .persist(&authority, StoreMode::Enabled)
            .map_err(|error| {
                ManagementError::unavailable("context_owner_persist", error.to_string())
            })?;
        current.insert(
            run_id,
            Current {
                authority,
                store,
                actor: actor.subject.clone(),
                binding_request: None,
                catalog_generation: None,
                runtime_lease_id: binding.lease_id.clone(),
                runtime_lease_epoch: binding.lease_epoch,
                admitted_control_limits: control_limits.clone(),
            },
        );
        Ok(())
    }

    fn record_legal_actions(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        digest: &str,
        binding: &RuntimeAuthorityBinding,
        actions: &EpisodeLegalActionSet,
    ) -> Result<(), ManagementError> {
        let run_id = run_id(request, digest)?;
        if binding.run_id != run_id || binding.lease_id.is_empty() || binding.lease_epoch == 0 {
            return Err(ManagementError::conflict(
                "context_owner_runtime_scope",
                "runtime authority is not bound to the admitted workflow run",
            ));
        }
        let mut current = self.current.lock().map_err(|_| {
            ManagementError::unavailable("context_owner_lock", "context owner is unavailable")
        })?;
        let entry = current.get_mut(&run_id).ok_or_else(|| {
            ManagementError::unavailable(
                "context_owner_observation_missing",
                "current runtime observation is unavailable",
            )
        })?;
        if entry.actor != actor.subject
            || entry.runtime_lease_id != binding.lease_id
            || entry.runtime_lease_epoch != binding.lease_epoch
        {
            return Err(ManagementError::conflict(
                "context_owner_catalog_scope",
                "actor or runtime authority cannot update this legal-action catalog",
            ));
        }
        actions
            .assert_matches(
                &entry.authority.state().boundary.state_id,
                entry.authority.state().boundary.generation,
            )
            .map_err(|_| {
                ManagementError::conflict(
                    "context_owner_catalog_stale",
                    "legal-action catalog is stale for the current MCP observation",
                )
            })?;
        let mut boundary = entry.authority.state().boundary.clone();
        boundary.catalog_sha256 = legal_catalog_digest(actions)?;
        entry
            .authority
            .record_observation_boundary(boundary)
            .map_err(|error| ManagementError::conflict("context_owner_observation_stale", error))?;
        entry
            .store
            .persist(&entry.authority, StoreMode::Enabled)
            .map_err(|error| {
                ManagementError::unavailable("context_owner_persist", error.to_string())
            })?;
        entry.catalog_generation = Some(actions.generation());
        Ok(())
    }
    fn invalidate(&self, actor: &AuthContext, request: &RunRequest, digest: &str) {
        let Ok(run_id) = run_id(request, digest) else {
            return;
        };
        if let Ok(mut current) = self.current.lock() {
            current.retain(|_, entry| {
                entry.actor != actor.subject || entry.authority.state().boundary.run_id != run_id
            });
        }
    }
}

fn control_limit_error(error: String) -> ManagementError {
    let code = if error == "context_control_events_exhausted" {
        "context_control_events_exhausted"
    } else {
        "context_control_event_limit_invalid"
    };
    ManagementError::conflict(
        code,
        "durable context authority exceeds the admitted control-event limit",
    )
}

#[cfg(test)]
#[path = "observation_tests.rs"]
mod tests;
