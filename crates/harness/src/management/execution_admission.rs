// SPDX-License-Identifier: MIT

//! Execution-side admission checks.
//!
//! Submission admission is intentionally checked twice by the live adapter:
//! once before constructing a reservation and again immediately before opening
//! the session.  The second check closes the revocation/drift window between
//! durable reservation and any mutating gateway operation.

use super::super::session::LiveWorkflowSessionFactory;
use crate::management::auth::AuthContext;
use crate::management::contract::{
    ExecutionMode, RunRequest, TargetAdmissionBinding, TargetAvailability, TargetDescriptor,
};
use crate::management::service::ManagementError;

pub(super) fn validate_live_admission(
    request: &RunRequest,
    definition_digest: &str,
    admission: &TargetAdmissionBinding,
) -> Result<(), ManagementError> {
    admission.validate().map_err(ManagementError::from)?;
    if admission.workflow_definition_digest != definition_digest {
        return Err(ManagementError::conflict(
            "target_admission_digest_mismatch",
            "target admission is bound to a different workflow definition",
        ));
    }
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
    if admission.target.execution_profile != request.profile
        || !matches!(admission.target.execution_mode, ExecutionMode::Live)
    {
        return Err(ManagementError::conflict(
            "target_mode_mismatch",
            "target admission execution mode does not match the live run request",
        ));
    }
    let definition = request.definition.as_ref().ok_or_else(|| {
        ManagementError::unavailable(
            "artifact_port_unavailable",
            "live execution requires an admitted workflow definition",
        )
    })?;
    let parsed = super::super::super::workflow_ports::parse_definition(definition)?;
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
    Ok(())
}

pub(super) fn validate_live_catalog(
    factory: &dyn LiveWorkflowSessionFactory,
    actor: &AuthContext,
    binding: &TargetAdmissionBinding,
) -> Result<(), ManagementError> {
    let catalog = factory.target_catalog(actor)?;
    catalog.validate().map_err(ManagementError::from)?;
    if catalog.catalog_revision != binding.catalog_revision {
        return Err(ManagementError::conflict(
            "target_catalog_stale",
            "target catalog changed after preflight",
        ));
    }
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
    validate_descriptor(descriptor, binding)?;
    Ok(())
}

fn validate_descriptor(
    descriptor: &TargetDescriptor,
    binding: &TargetAdmissionBinding,
) -> Result<(), ManagementError> {
    match descriptor.availability {
        TargetAvailability::Available => {}
        TargetAvailability::Unavailable => {
            return Err(ManagementError::unavailable(
                "target_unavailable",
                "requested target is currently unavailable",
            ));
        }
        TargetAvailability::Revoked => {
            return Err(ManagementError::forbidden(
                "target_revoked",
                "requested target admission has been revoked",
            ));
        }
        TargetAvailability::Expired => {
            return Err(ManagementError::conflict(
                "target_expired",
                "requested target admission has expired",
            ));
        }
    }
    if descriptor.execution_mode != ExecutionMode::Live {
        return Err(ManagementError::conflict(
            "target_mode_mismatch",
            "target admission execution mode does not match the live target",
        ));
    }
    if !descriptor
        .execution_profiles
        .iter()
        .any(|profile| profile == &binding.target.execution_profile)
    {
        return Err(ManagementError::capability(
            "target_profile_unavailable",
            "requested execution profile is not available on the target",
        ));
    }
    if !descriptor
        .supported_operations
        .iter()
        .any(|operation| operation == "workflow:live")
    {
        return Err(ManagementError::capability(
            "target_operation_unavailable",
            "target does not support live workflow execution",
        ));
    }
    if descriptor.compatibility_revision != binding.target.compatibility_revision {
        return Err(ManagementError::conflict(
            "target_compatibility_stale",
            "target compatibility revision is stale",
        ));
    }
    if descriptor.capability_revision != binding.target.capability_revision {
        return Err(ManagementError::conflict(
            "target_capability_stale",
            "target capability revision is stale",
        ));
    }
    if !descriptor
        .game_profiles
        .iter()
        .any(|profile| profile == &binding.target.game_profile)
    {
        return Err(ManagementError::capability(
            "target_game_profile_unavailable",
            "requested game profile is not available on the target",
        ));
    }
    if let Some(profile) = binding.target.save_profile.as_deref()
        && !descriptor
            .save_profiles
            .iter()
            .any(|value| value == profile)
    {
        return Err(ManagementError::capability(
            "target_save_profile_unavailable",
            "requested save profile is not available on the target",
        ));
    }
    if let Some(profile) = binding.target.inference_profile.as_deref()
        && !descriptor
            .inference_profiles
            .iter()
            .any(|value| value == profile)
    {
        return Err(ManagementError::capability(
            "target_inference_profile_unavailable",
            "requested inference profile is not available on the target",
        ));
    }
    if let Some(capability) = binding.target.context_capability.as_deref()
        && !descriptor
            .capabilities
            .iter()
            .any(|value| value == capability)
    {
        return Err(ManagementError::capability(
            "target_context_capability_unavailable",
            "requested context capability is not available on the target",
        ));
    }
    if let Some(capability) = binding.target.provider_capability.as_deref()
        && !descriptor
            .capabilities
            .iter()
            .any(|value| value == capability)
    {
        return Err(ManagementError::capability(
            "target_provider_capability_unavailable",
            "requested provider capability is not available on the target",
        ));
    }
    let descriptor_digest = descriptor.digest().map_err(ManagementError::from)?;
    if descriptor_digest != binding.descriptor_digest {
        return Err(ManagementError::conflict(
            "target_descriptor_stale",
            "target descriptor changed after preflight",
        ));
    }
    Ok(())
}
