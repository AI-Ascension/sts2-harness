// SPDX-License-Identifier: MIT

use serde_json::Value;

use super::super::{
    ExecutionMode, ManagementClient, RunTargetConfiguration, TARGET_ADMISSION_SCHEMA_VERSION,
    TargetAdmissionBinding, TargetAdmissionRequest, TargetCatalogResponse, TargetPreflightResponse,
    decode_strict, digest_value, validate_identifier,
};
use super::support::{CliFailure, response_failure};

pub(super) fn preflight_for_run(
    client: &ManagementClient,
    definition: &Value,
    instance_id: &str,
    profile: &str,
    request_id: &str,
) -> Result<TargetAdmissionBinding, CliFailure> {
    validate_identifier("instance_id", instance_id).map_err(CliFailure::local)?;
    validate_identifier("profile", profile).map_err(CliFailure::local)?;
    let target = target_configuration(client, definition, instance_id, profile)?;
    let workflow_definition_digest = digest_value(definition).map_err(CliFailure::local)?;
    let body = serde_json::to_vec(&TargetAdmissionRequest {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: request_id.to_owned(),
        workflow_definition_digest: workflow_definition_digest.clone(),
        target,
    })
    .map_err(CliFailure::local)?;
    let response = client
        .request_json("POST", "/v1/workflow-targets/preflight", Some(&body))
        .map_err(CliFailure::local)?;
    if response.status / 100 != 2 {
        return Err(response_failure(response));
    }
    let response: TargetPreflightResponse =
        decode_strict(&response.body).map_err(CliFailure::local)?;
    response.validate().map_err(CliFailure::local)?;
    let admission = response.admission;
    if admission.request_id != request_id
        || admission.workflow_definition_digest != workflow_definition_digest
        || admission.target.instance_id != instance_id
        || admission.target.execution_profile != profile
    {
        return Err(CliFailure::invalid(
            "target preflight returned a binding for a different request",
        ));
    }
    Ok(admission)
}

fn target_configuration(
    client: &ManagementClient,
    definition: &Value,
    instance_id: &str,
    profile: &str,
) -> Result<RunTargetConfiguration, CliFailure> {
    let response = client
        .request_json("GET", "/v1/workflow-targets", None)
        .map_err(CliFailure::local)?;
    if response.status / 100 != 2 {
        return Err(response_failure(response));
    }
    let catalog: TargetCatalogResponse =
        decode_strict(&response.body).map_err(CliFailure::local)?;
    catalog.validate().map_err(CliFailure::local)?;
    let descriptor = catalog
        .targets
        .iter()
        .find(|target| target.instance_id == instance_id)
        .ok_or_else(|| CliFailure::invalid("requested target is not available"))?;
    let workflow_revision = definition_identifier(definition, "version")?;
    let game_profile = definition_identifier(definition, "game_profile")?;
    let execution_mode = if is_live_profile(profile) {
        ExecutionMode::Live
    } else {
        ExecutionMode::Synthetic
    };
    Ok(RunTargetConfiguration {
        instance_id: instance_id.to_owned(),
        execution_profile: profile.to_owned(),
        execution_mode,
        workflow_revision,
        compatibility_revision: descriptor.compatibility_revision.clone(),
        capability_revision: descriptor.capability_revision.clone(),
        game_profile,
        save_profile: None,
        inference_profile: None,
        context_capability: None,
        provider_capability: None,
    })
}

fn definition_identifier(definition: &Value, field: &str) -> Result<String, CliFailure> {
    let value = definition
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| CliFailure::invalid(format!("workflow definition {field} is required")))?;
    validate_identifier(field, value).map_err(CliFailure::local)?;
    Ok(value.to_owned())
}

pub(super) fn is_live_profile(profile: &str) -> bool {
    profile == "live" || profile.starts_with("live.")
}
