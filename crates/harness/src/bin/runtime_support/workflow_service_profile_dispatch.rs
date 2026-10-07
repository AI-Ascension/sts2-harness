// SPDX-License-Identifier: MIT

//! Trusted provider configuration and exact profile dispatch for the served Exo factory.

use sts2_harness::ExoConfig;
use sts2_harness::exo_admission::ExoRuntimeAdmission;
use sts2_harness::management::{
    AdmittedInferenceProfileBinding, AdmittedInferenceProfileDispatch,
    INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION, INFERENCE_PROFILE_SCHEMA_VERSION,
    InferenceProfileBindingSet, InferenceProfileBudgets, InferenceProfileCatalog,
    InferenceProfileContinuity, InferenceProfileDescriptor, InferenceProfileGrants,
    InferenceProfileState, LiveProviderSessionAdmission, MAX_INFERENCE_OUTPUT_TOKENS,
    MAX_INFERENCE_PROVIDER_CALLS, ManagementError, RunRequest, RuntimeAuthorityBinding,
};
use sts2_harness::provider_session::NativeCapabilities;

use super::{RuntimeConfig, runtime_v3, runtime_v3_admission, runtime_v3_settings};
use runtime_v3_settings::RuntimeV3Settings;
#[path = "workflow_service_profile_identity.rs"]
mod identity;
#[path = "workflow_service_profile_dispatch_provider.rs"]
mod provider;
pub(super) use provider::Provider;
#[path = "workflow_service_profile_dispatch_source.rs"]
mod source;
use source::ProfiledExoAdmission;

const LIVE_DECISION_PROFILE_ID: &str = "decision.live.v1";
const LIVE_DECISION_PROFILE_VERSION: &str = "1.0.0";
const LIVE_DECISION_ADAPTER: &str = "exo.runtime-v3";
const LIVE_CONTEXT_REF: &str = "context.live.v1";
const UNPINNED_REVISION: &str = "legacy.unpinned";

#[derive(Clone)]
struct VerifiedProfileIdentity {
    requested_model: String,
    prompt_revision: String,
    inspected_config_digest: String,
}

fn verified_identity(
    capabilities: &NativeCapabilities,
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<VerifiedProfileIdentity, ManagementError> {
    identity::verified_identity(capabilities, config, settings)
}

fn descriptor(
    capabilities: &NativeCapabilities,
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<InferenceProfileDescriptor, ManagementError> {
    let verified = verified_identity(capabilities, config, settings).ok();
    descriptor_from_verified_identity(capabilities, &settings.exo, verified)
}

fn descriptor_from_verified_identity(
    capabilities: &NativeCapabilities,
    exo: &ExoConfig,
    verified: Option<VerifiedProfileIdentity>,
) -> Result<InferenceProfileDescriptor, ManagementError> {
    let settings_revision = verified
        .as_ref()
        .map(|identity| provider_settings_revision(&identity.inspected_config_digest, exo))
        .transpose()?;
    let descriptor = InferenceProfileDescriptor {
        schema_version: INFERENCE_PROFILE_SCHEMA_VERSION.to_owned(),
        profile_id: LIVE_DECISION_PROFILE_ID.to_owned(),
        version: LIVE_DECISION_PROFILE_VERSION.to_owned(),
        digest: String::new(),
        adapter: LIVE_DECISION_ADAPTER.to_owned(),
        requested_model: verified
            .as_ref()
            .map(|identity| identity.requested_model.clone())
            .unwrap_or_else(|| capabilities.binding.model_revision.clone()),
        resolved_model: None,
        prompt_revision: verified
            .as_ref()
            .map(|identity| identity.prompt_revision.clone())
            .unwrap_or_else(|| UNPINNED_REVISION.to_owned()),
        settings_revision: settings_revision.unwrap_or_else(|| UNPINNED_REVISION.to_owned()),
        supported_settings: vec![
            "max_provider_calls".to_owned(),
            "max_output_tokens".to_owned(),
        ],
        operations: vec!["decide".to_owned()],
        node_kinds: vec!["decide".to_owned()],
        context_compatibility: vec![LIVE_CONTEXT_REF.to_owned()],
        continuity: InferenceProfileContinuity {
            provider_session_continuity: false,
            survives_controller_restart: false,
        },
        effective_budgets: InferenceProfileBudgets {
            max_input_bytes: u64::try_from(exo.max_request_bytes).map_err(|_| {
                ManagementError::invalid(
                    "inference_profile_budget_invalid",
                    "Exo request bound does not fit the profile budget",
                )
            })?,
            // These are registry ceilings, not measured provider limits.
            max_output_tokens: MAX_INFERENCE_OUTPUT_TOKENS,
            max_provider_calls: MAX_INFERENCE_PROVIDER_CALLS,
        },
        grants: InferenceProfileGrants {
            select: true,
            edit: false,
        },
        state: if verified.is_some() {
            InferenceProfileState::Available
        } else {
            InferenceProfileState::Unsupported
        },
    }
    .seal()?;
    Ok(descriptor)
}

/// Binds the shipped provider settings to the independently inspected deployment identity.
/// The value is a credential-free settings revision, not an operator-declared digest or a run ID.
fn provider_settings_revision(
    inspected_config_digest: &str,
    exo: &ExoConfig,
) -> Result<String, ManagementError> {
    let value = serde_json::json!({
        "schema_version": "ascension.inference-profile-exo-settings/v1",
        "inspected_config_digest": inspected_config_digest,
        "adapter_revision": exo.revision.as_str(),
        "max_request_bytes": exo.max_request_bytes,
        "max_response_bytes": exo.max_response_bytes,
        "timeout_millis": exo.timeout_millis,
        "forward_visible_seed": exo.forward_visible_seed,
        "tool_catalog_sha256": exo.tool_catalog.catalog_digest(),
    });
    serde_json::to_vec(&value)
        .map(sts2_harness::sha256_hex)
        .map_err(|_| unsupported("the inspected provider settings could not be fingerprinted"))
}

pub(super) fn catalog(
    capabilities: &NativeCapabilities,
    config: &RuntimeConfig,
    settings: &RuntimeV3Settings,
) -> Result<InferenceProfileCatalog, ManagementError> {
    let catalog = InferenceProfileCatalog {
        schema_version: INFERENCE_PROFILE_CATALOG_SCHEMA_VERSION.to_owned(),
        owner_id: capabilities.binding.owner.clone(),
        owner_version: capabilities.binding.owner_revision.clone(),
        catalog_digest: String::new(),
        descriptors: vec![descriptor(capabilities, config, settings)?],
    }
    .seal()?;
    catalog.validate()?;
    Ok(catalog)
}

pub(super) fn prepare_provider(
    capabilities: &NativeCapabilities,
    config: &RuntimeConfig,
    settings: RuntimeV3Settings,
    request: &RunRequest,
    definition_digest: &str,
    authority: &RuntimeAuthorityBinding,
    profiles: &AdmittedInferenceProfileDispatch,
) -> Result<Box<dyn LiveProviderSessionAdmission>, ManagementError> {
    if config.instance_id != request.instance_id {
        return Err(ManagementError::conflict(
            "runtime_instance_mismatch",
            "configured runtime differs from target",
        ));
    }
    let run_id = sts2_harness::management::live_run_id(request, definition_digest)?;
    let config_digest = runtime_v3::authority_configuration_digest(config, &settings)
        .map_err(|error| ManagementError::unavailable("runtime_configuration_digest", error))?;
    if authority.instance_id != config.instance_id
        || authority.run_id != run_id
        || authority.adapter_revision != capabilities.binding.adapter_revision
        || authority.model_revision != capabilities.binding.model_revision
        || authority.output_schema_digest != capabilities.native_schema_sha256
        || authority.configuration_digest != config_digest
    {
        return Err(ManagementError::conflict(
            "provider_profile_authority_mismatch",
            "the inspected provider configuration does not match the admitted runtime authority",
        ));
    }
    let current_catalog = catalog(capabilities, config, &settings)?;
    validate_profile_dispatch(
        profiles.binding_set(),
        profiles.bindings(),
        &current_catalog,
    )?;
    let ExoRuntimeAdmission::Enveloped(_) = &settings.admission else {
        return Err(unsupported(
            "the selected Exo route is not envelope-admitted",
        ));
    };
    Ok(Box::new(ProfiledExoAdmission {
        process: settings.process,
        config: settings.exo,
        admission: settings.admission,
        profiles: profiles.bindings().to_vec(),
    }))
}

fn validate_profile_dispatch(
    binding_set: &InferenceProfileBindingSet,
    bindings: &[AdmittedInferenceProfileBinding],
    current_catalog: &InferenceProfileCatalog,
) -> Result<(), ManagementError> {
    let current_descriptor = current_catalog
        .descriptors
        .first()
        .ok_or_else(|| unsupported("the served decision profile is unavailable"))?;
    if current_descriptor.state != InferenceProfileState::Available
        || binding_set.catalog_digest != current_catalog.catalog_digest
        || bindings.is_empty()
        || binding_set.bindings.len() != bindings.len()
    {
        return Err(unsupported(
            "the admitted workflow does not match the current inspected decision profile",
        ));
    }
    for admitted in bindings {
        validate_binding(admitted, current_catalog, current_descriptor)?;
    }
    Ok(())
}

fn validate_binding(
    admitted: &AdmittedInferenceProfileBinding,
    catalog: &InferenceProfileCatalog,
    current: &InferenceProfileDescriptor,
) -> Result<(), ManagementError> {
    let binding = &admitted.binding;
    let resolved = catalog.resolve(&binding.profile_ref, &binding.node_kind)?;
    if binding.node_kind != "decide"
        || binding.profile_id != LIVE_DECISION_PROFILE_ID
        || binding.adapter != LIVE_DECISION_ADAPTER
        || binding.requested_model != current.requested_model
        || binding.resolved_model.is_some()
        || admitted.descriptor != *current
        || resolved != current
    {
        return Err(unsupported(
            "the node binding does not identify the exact served Exo decision profile",
        ));
    }
    Ok(())
}

fn unsupported(message: &str) -> ManagementError {
    ManagementError::capability("provider_profile_dispatch_unsupported", message)
}

#[cfg(test)]
#[path = "workflow_service_profile_dispatch_composition_tests.rs"]
mod composition_tests;
#[cfg(test)]
#[path = "workflow_service_profile_dispatch_tests.rs"]
mod tests;
