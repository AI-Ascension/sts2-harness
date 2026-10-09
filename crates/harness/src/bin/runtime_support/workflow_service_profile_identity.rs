// SPDX-License-Identifier: MIT

use super::runtime_v3_admission::RuntimeV3Admission;
use super::{RuntimeConfig, RuntimeV3Settings, VerifiedProfileIdentity, unsupported};
use sts2_harness::provider_session::NativeCapabilities;
use sts2_harness::{EXO_SOURCE_REVISION, ExoIdentity};

pub(super) fn verified_identity(
    capabilities: &NativeCapabilities,
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<VerifiedProfileIdentity, super::ManagementError> {
    if settings.lookup_agent.is_some() {
        return Err(unsupported(
            "the selected lookup route is not the live decision profile",
        ));
    }
    match &settings.admission {
        RuntimeV3Admission::Ordinary(
            sts2_harness::exo_admission::ExoRuntimeAdmission::Enveloped(plan),
        ) => {
            plan.validate()
                .map_err(|_| unsupported("the inspected Exo deployment did not pass admission"))?;
            verify_identity_binding(
                capabilities,
                plan.trusted_identity(),
                plan.inspected_identity(),
                &settings.exo.revision,
                &config.instance_id,
            )
        }
        RuntimeV3Admission::Synthetic(plan) => {
            let inspected = plan
                .inspected_identity()
                .map_err(|_| unsupported("the synthetic Exo plan is unavailable"))?;
            let expected = plan
                .capabilities()
                .map_err(|_| unsupported("the synthetic Exo profile is unavailable"))?;
            verify_profile_identity_binding(
                capabilities,
                &expected,
                plan.trusted_identity(),
                &inspected,
                &settings.exo.revision,
                &config.instance_id,
            )
        }
        RuntimeV3Admission::Ordinary(sts2_harness::exo_admission::ExoRuntimeAdmission::Legacy) => {
            Err(unsupported(
                "raw-wire Exo admission has no inspected profile identity",
            ))
        }
    }
}

fn verify_identity_binding(
    capabilities: &NativeCapabilities,
    trusted: &ExoIdentity,
    inspected: &ExoIdentity,
    settings_revision: &str,
    instance_id: &str,
) -> Result<VerifiedProfileIdentity, super::ManagementError> {
    let expected = NativeCapabilities::reviewed_exo_one_shot(inspected)
        .map_err(|_| unsupported("the inspected Exo identity has no ordinary profile"))?;
    verify_profile_identity_binding(
        capabilities,
        &expected,
        trusted,
        inspected,
        settings_revision,
        instance_id,
    )
}

fn verify_profile_identity_binding(
    capabilities: &NativeCapabilities,
    expected: &NativeCapabilities,
    trusted: &ExoIdentity,
    inspected: &ExoIdentity,
    settings_revision: &str,
    instance_id: &str,
) -> Result<VerifiedProfileIdentity, super::ManagementError> {
    if !trusted.is_complete()
        || !inspected.is_complete()
        || trusted != inspected
        || inspected.source_revision != EXO_SOURCE_REVISION
        || settings_revision != inspected.source_revision.as_str()
        || inspected.native_instance_id.as_deref() != Some(instance_id)
    {
        return Err(unsupported(
            "the trusted and independently inspected Exo profile identities differ",
        ));
    }

    if capabilities != expected {
        return Err(unsupported(
            "the provider capability descriptor does not match the inspected Exo identity",
        ));
    }

    let requested_model = inspected
        .model_binding
        .as_deref()
        .ok_or_else(|| unsupported("the inspected model identity is unavailable"))?;
    let prompt_revision = inspected
        .prompt_digest
        .as_deref()
        .ok_or_else(|| unsupported("the inspected prompt identity is unavailable"))?;
    let inspected_config_digest = inspected
        .config_digest
        .as_deref()
        .ok_or_else(|| unsupported("the inspected settings identity is unavailable"))?;
    Ok(VerifiedProfileIdentity {
        requested_model: requested_model.to_owned(),
        prompt_revision: prompt_revision.to_owned(),
        inspected_config_digest: inspected_config_digest.to_owned(),
    })
}

#[cfg(test)]
#[path = "workflow_service_profile_identity_tests.rs"]
mod tests;
