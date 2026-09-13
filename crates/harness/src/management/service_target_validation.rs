// SPDX-License-Identifier: MIT

use super::super::*;

pub(super) fn validate_target_selection(
    descriptor: &TargetDescriptor,
    selection: &RunTargetConfiguration,
) -> Result<(), ManagementError> {
    if descriptor.instance_id != selection.instance_id {
        return Err(ManagementError::conflict(
            "target_instance_mismatch",
            "target admission instance does not match the catalog descriptor",
        ));
    }
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
    if descriptor.execution_mode != selection.execution_mode {
        return Err(ManagementError::conflict(
            "target_mode_mismatch",
            "target admission execution mode does not match the target",
        ));
    }
    if !descriptor
        .execution_profiles
        .iter()
        .any(|profile| profile == &selection.execution_profile)
    {
        return Err(ManagementError::capability(
            "target_profile_unavailable",
            "requested execution profile is not available on the target",
        ));
    }
    if descriptor.compatibility_revision != selection.compatibility_revision {
        return Err(ManagementError::conflict(
            "target_compatibility_stale",
            "target compatibility revision is stale",
        ));
    }
    if descriptor.capability_revision != selection.capability_revision {
        return Err(ManagementError::conflict(
            "target_capability_stale",
            "target capability revision is stale",
        ));
    }
    validate_target_profile(
        "game",
        &selection.game_profile,
        &descriptor.game_profiles,
        "target_game_profile_unavailable",
    )?;
    validate_optional_target_profile(
        "save",
        selection.save_profile.as_deref(),
        &descriptor.save_profiles,
        "target_save_profile_unavailable",
    )?;
    validate_optional_target_profile(
        "inference",
        selection.inference_profile.as_deref(),
        &descriptor.inference_profiles,
        "target_inference_profile_unavailable",
    )?;
    validate_optional_capability(
        "context",
        selection.context_capability.as_deref(),
        &descriptor.capabilities,
        "target_context_capability_unavailable",
    )?;
    validate_optional_capability(
        "provider",
        selection.provider_capability.as_deref(),
        &descriptor.capabilities,
        "target_provider_capability_unavailable",
    )?;
    Ok(())
}

fn validate_target_profile(
    namespace: &str,
    requested: &str,
    supported: &[String],
    code: &str,
) -> Result<(), ManagementError> {
    if supported.iter().any(|value| value == requested) {
        return Ok(());
    }
    Err(ManagementError::capability(
        code,
        format!("requested {namespace} profile is not available on the target"),
    ))
}

fn validate_optional_target_profile(
    namespace: &str,
    requested: Option<&str>,
    supported: &[String],
    code: &str,
) -> Result<(), ManagementError> {
    if let Some(requested) = requested {
        validate_target_profile(namespace, requested, supported, code)?;
    }
    Ok(())
}

fn validate_optional_capability(
    namespace: &str,
    requested: Option<&str>,
    supported: &[String],
    code: &str,
) -> Result<(), ManagementError> {
    if let Some(requested) = requested
        && !supported.iter().any(|value| value == requested)
    {
        return Err(ManagementError::capability(
            code,
            format!("requested {namespace} capability is not available on the target"),
        ));
    }
    Ok(())
}
