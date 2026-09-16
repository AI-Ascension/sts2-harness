// SPDX-License-Identifier: MIT

//! Target discovery and the last admission fence before an execution port is
//! allowed to launch work.

use super::super::contract::ExecutionMode;
use super::support::authorize;
use super::*;

#[path = "service_target_validation.rs"]
mod target_validation;

use target_validation::validate_target_selection;

impl ManagementService {
    /// Returns the target catalog already scoped by the authoritative target
    /// owner. The management service validates only its bounded, redacted
    /// shape; it never discovers instances or credentials itself.
    pub fn target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        authorize(actor, "workflow:read", None)?;
        self.load_target_catalog(actor)
    }

    /// Performs a read-only admission preflight against the current,
    /// actor-scoped catalog. No execution, provider, game, or lease effect is
    /// reachable from this method.
    pub fn preflight_target(
        &self,
        actor: &AuthContext,
        request: TargetAdmissionRequest,
    ) -> Result<TargetPreflightResponse, ManagementError> {
        authorize(actor, "workflow:control", None)?;
        request.validate().map_err(ManagementError::from)?;
        let catalog = self.load_target_catalog(actor)?;
        let descriptor = catalog
            .targets
            .iter()
            .find(|target| target.instance_id == request.target.instance_id)
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "target_unavailable",
                    "requested target is not available to this actor",
                )
            })?;
        validate_target_selection(descriptor, &request.target)?;
        let binding = TargetAdmissionBinding {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            request_id: request.request_id,
            workflow_definition_digest: request.workflow_definition_digest,
            target: request.target,
            descriptor_digest: descriptor.digest().map_err(ManagementError::from)?,
            catalog_revision: catalog.catalog_revision,
        };
        binding.validate().map_err(ManagementError::from)?;
        Ok(TargetPreflightResponse {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            admission: binding,
        })
    }

    /// Rechecks the actor-scoped authority after definition admission and
    /// immediately before the execution port is called. A synthetic request
    /// without a binding remains compatible with the fixture lane; any live
    /// request must carry and revalidate a server-issued binding.
    pub(super) fn revalidate_target_admission(
        &self,
        actor: &AuthContext,
        request: &RunRequest,
        definition_digest: &str,
    ) -> Result<Option<TargetAdmissionBinding>, ManagementError> {
        let live = is_live_profile(&request.profile);
        let Some(binding) = request.admission.as_ref() else {
            if live {
                return Err(ManagementError::conflict(
                    "target_admission_required",
                    "live workflow submission requires an exact target admission binding",
                ));
            }
            return Ok(None);
        };
        validate_run_target_admission(request)?;
        if binding.workflow_definition_digest != definition_digest {
            return Err(ManagementError::conflict(
                "target_admission_digest_mismatch",
                "target admission is bound to a different workflow definition",
            ));
        }

        let catalog = self.load_target_catalog(actor)?;
        let descriptor = catalog
            .targets
            .iter()
            .find(|target| target.instance_id == binding.target.instance_id)
            .ok_or_else(|| {
                ManagementError::unavailable(
                    "target_unavailable",
                    "admitted target is no longer visible to this actor",
                )
            })?;
        validate_target_selection(descriptor, &binding.target)?;
        let descriptor_digest = descriptor.digest().map_err(ManagementError::from)?;
        if catalog.catalog_revision != binding.catalog_revision {
            return Err(ManagementError::conflict(
                "target_catalog_stale",
                "target catalog changed after preflight",
            ));
        }
        if descriptor_digest != binding.descriptor_digest {
            return Err(ManagementError::conflict(
                "target_descriptor_stale",
                "target descriptor changed after preflight",
            ));
        }
        Ok(Some(TargetAdmissionBinding {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            request_id: binding.request_id.clone(),
            workflow_definition_digest: definition_digest.to_owned(),
            target: binding.target.clone(),
            descriptor_digest,
            catalog_revision: catalog.catalog_revision,
        }))
    }

    fn load_target_catalog(
        &self,
        actor: &AuthContext,
    ) -> Result<TargetCatalogResponse, ManagementError> {
        let catalog = self.capabilities.target_catalog(actor)?;
        catalog.validate().map_err(ManagementError::from)?;
        Ok(catalog)
    }
}

pub(super) fn validate_run_target_admission(request: &RunRequest) -> Result<(), ManagementError> {
    let live = is_live_profile(&request.profile);
    let Some(admission) = request.admission.as_ref() else {
        if live {
            return Err(ManagementError::conflict(
                "target_admission_required",
                "live workflow submission requires an exact target admission binding",
            ));
        }
        return Ok(());
    };
    admission.validate().map_err(ManagementError::from)?;
    if admission.request_id != request.request_id {
        return Err(ManagementError::conflict(
            "target_request_mismatch",
            "target admission request identity does not match the run request",
        ));
    }
    if admission.target.instance_id != request.instance_id {
        return Err(ManagementError::conflict(
            "target_instance_mismatch",
            "target admission instance does not match the run request",
        ));
    }
    if admission.target.execution_profile != request.profile {
        return Err(ManagementError::conflict(
            "target_profile_mismatch",
            "target admission execution profile does not match the run request",
        ));
    }
    let admission_is_live = matches!(admission.target.execution_mode, ExecutionMode::Live);
    if admission_is_live != live {
        return Err(ManagementError::conflict(
            "target_mode_mismatch",
            "target admission execution mode does not match the run profile",
        ));
    }
    if let Some(definition) = request.definition.as_ref() {
        let parsed = super::super::workflow_ports::parse_definition(definition)?;
        if admission.target.workflow_revision != parsed.version.as_str() {
            return Err(ManagementError::conflict(
                "target_admission_stale",
                "target admission workflow revision is stale",
            ));
        }
        if admission.target.game_profile != parsed.game_profile.as_str() {
            return Err(ManagementError::conflict(
                "target_game_profile_mismatch",
                "target admission game profile does not match the workflow",
            ));
        }
    }
    Ok(())
}

pub(super) fn bind_snapshot_admission(
    mut snapshot: RunSnapshot,
    binding: Option<&TargetAdmissionBinding>,
) -> Result<RunSnapshot, ManagementError> {
    match (snapshot.admission.as_ref(), binding) {
        (Some(existing), Some(expected)) if existing != expected => {
            return Err(ManagementError::conflict(
                "port_admission_mismatch",
                "execution port returned a different target admission",
            ));
        }
        (Some(_), None) => {
            return Err(ManagementError::conflict(
                "port_admission_unexpected",
                "execution port returned an unrequested target admission",
            ));
        }
        _ => {}
    }
    // The execution port declares the mode it actually served. A synthetic
    // adapter that claims live execution (or the reverse) is refused here,
    // before the snapshot is persisted or returned to a consumer.
    let expected_mode = binding.map_or(ExecutionMode::Synthetic, |binding| {
        binding.target.execution_mode.clone()
    });
    if let Some(reported) = snapshot.execution_mode.as_ref()
        && reported != &expected_mode
    {
        return Err(ManagementError::conflict(
            "port_execution_mode_mismatch",
            "execution port reported a different execution mode than the admitted target",
        ));
    }
    snapshot.admission = binding.cloned();
    Ok(snapshot)
}

pub(super) fn is_live_profile(profile: &str) -> bool {
    profile == "live" || profile.starts_with("live.")
}

#[cfg(test)]
mod execution_mode_binding_tests {
    #![allow(clippy::expect_used)]

    use super::*;
    use crate::management::contract::{
        Budget, CleanupState, Cursor, GameOutcome, RUN_SCHEMA_VERSION, RunTargetConfiguration,
        TARGET_ADMISSION_SCHEMA_VERSION, WorkflowRunStatus,
    };

    fn snapshot(mode: Option<ExecutionMode>) -> RunSnapshot {
        RunSnapshot {
            schema_version: RUN_SCHEMA_VERSION.to_owned(),
            workflow_run_id: "run-1".to_owned(),
            definition_digest: "a".repeat(64),
            run_revision: 1,
            status: WorkflowRunStatus::Running,
            game_outcome: GameOutcome::NotTerminal,
            cursor: Cursor {
                graph_id: "graph".to_owned(),
                node_id: "node".to_owned(),
                node_execution_id: "node-exec".to_owned(),
            },
            pending_operation: None,
            budget: Budget::default(),
            cleanup: CleanupState::NotStarted,
            admission: None,
            execution_mode: mode,
        }
    }

    fn live_binding() -> TargetAdmissionBinding {
        TargetAdmissionBinding {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
            request_id: "request-1".to_owned(),
            workflow_definition_digest: "a".repeat(64),
            target: RunTargetConfiguration {
                instance_id: "instance-1".to_owned(),
                execution_profile: "live.workflow.v1".to_owned(),
                execution_mode: ExecutionMode::Live,
                workflow_revision: "0.1.0".to_owned(),
                compatibility_revision: "live.compatibility.v1".to_owned(),
                capability_revision: "live.capabilities.v1".to_owned(),
                game_profile: "sts2-live-v1".to_owned(),
                save_profile: None,
                inference_profile: None,
                context_capability: None,
                provider_capability: None,
            },
            descriptor_digest: "b".repeat(64),
            catalog_revision: "live.catalog.v1".to_owned(),
        }
    }

    #[test]
    fn a_synthetic_report_cannot_satisfy_a_live_admission() {
        let error = bind_snapshot_admission(
            snapshot(Some(ExecutionMode::Synthetic)),
            Some(&live_binding()),
        )
        .expect_err("a synthetic port report must not satisfy a live admission");
        assert_eq!(error.code, "port_execution_mode_mismatch");
    }

    #[test]
    fn live_execution_is_not_reported_without_a_live_admission() {
        let error = bind_snapshot_admission(snapshot(Some(ExecutionMode::Live)), None)
            .expect_err("live mode must not be reported without a live admission");
        assert_eq!(error.code, "port_execution_mode_mismatch");
    }

    #[test]
    fn a_matching_report_preserves_the_mode_and_binds_the_admission() {
        let binding = live_binding();
        let bound = bind_snapshot_admission(snapshot(Some(ExecutionMode::Live)), Some(&binding))
            .expect("a matching report binds");
        assert_eq!(bound.execution_mode, Some(ExecutionMode::Live));
        assert_eq!(bound.admission, Some(binding));
    }
}
