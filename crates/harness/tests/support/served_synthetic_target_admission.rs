// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::management::{
    ManagementClient, RunRequest, RunSubmissionResponse, TARGET_ADMISSION_SCHEMA_VERSION,
    TargetAdmissionRequest, TargetDescriptor, TargetPreflightResponse, live_run_id,
};

use super::fixture::Result;
use super::http_contract::{REQUEST_ID, decode, post, refuse, request_bytes, require};

pub(crate) fn admit_and_submit(
    client: &ManagementClient,
    descriptor: &TargetDescriptor,
    definition: &Value,
    definition_digest: &str,
    seed_run: &mut RunRequest,
    model_request_count: impl Fn() -> usize,
    scenario_class: &mut &'static str,
) -> Result<RunSubmissionResponse> {
    *scenario_class = "target_construction";
    let expected_run_id = live_run_id(seed_run, definition_digest)?;
    let target = sts2_harness::management::RunTargetConfiguration {
        instance_id: descriptor.instance_id.clone(),
        execution_profile: descriptor
            .execution_profiles
            .iter()
            .find(|profile| profile.as_str() == "live.workflow.v1")
            .cloned()
            .ok_or("live profile missing")?,
        execution_mode: descriptor.execution_mode.clone(),
        workflow_revision: definition["version"]
            .as_str()
            .ok_or("workflow version missing")?
            .to_owned(),
        compatibility_revision: descriptor.compatibility_revision.clone(),
        capability_revision: descriptor.capability_revision.clone(),
        game_profile: descriptor
            .game_profiles
            .first()
            .cloned()
            .ok_or("game profile missing")?,
        save_profile: descriptor.save_profiles.first().cloned(),
        inference_profile: descriptor.inference_profiles.first().cloned(),
        context_capability: descriptor
            .capabilities
            .iter()
            .find(|value| value.starts_with("workflow.context."))
            .cloned(),
        provider_capability: descriptor
            .capabilities
            .iter()
            .find(|value| value.starts_with("workflow.provider."))
            .cloned(),
    };
    let preflight = TargetAdmissionRequest {
        schema_version: TARGET_ADMISSION_SCHEMA_VERSION.to_owned(),
        request_id: REQUEST_ID.to_owned(),
        workflow_definition_digest: definition_digest.to_owned(),
        target,
    };

    *scenario_class = "stale_compatibility_refusal";
    let mut stale = preflight.clone();
    stale.target.compatibility_revision = "stale.compatibility.v1".to_owned();
    refuse(client.request_json(
        "POST",
        "/v1/workflow-targets/preflight",
        Some(&request_bytes(&stale)?),
    )?)?;

    *scenario_class = "malformed_digest_refusal";
    let mut invalid_digest = preflight.clone();
    invalid_digest.workflow_definition_digest = "not-a-digest".to_owned();
    refuse(client.request_json(
        "POST",
        "/v1/workflow-targets/preflight",
        Some(&request_bytes(&invalid_digest)?),
    )?)?;
    require(
        model_request_count() == 0,
        "stale target preflight reached the local model",
    )?;

    *scenario_class = "positive_preflight";
    let admitted: TargetPreflightResponse =
        post(client, "/v1/workflow-targets/preflight", &preflight)?;
    *scenario_class = "returned_binding";
    admitted.validate()?;
    require(
        admitted.admission.target == preflight.target
            && admitted.admission.workflow_definition_digest == definition_digest,
        "preflight did not preserve the exact target and definition binding",
    )?;
    seed_run.admission = Some(admitted.admission);

    *scenario_class = "stale_run_digest_refusal";
    let mut stale_run = seed_run.clone();
    if let Some(binding) = stale_run.admission.as_mut() {
        binding.workflow_definition_digest = "0".repeat(64);
    }
    refuse(client.request_json(
        "POST",
        "/v1/workflow-runs",
        Some(&request_bytes(&stale_run)?),
    )?)?;
    require(
        model_request_count() == 0,
        "stale run admission reached the local model",
    )?;

    *scenario_class = "positive_run_submission_encoding";
    let submission_body = request_bytes(&*seed_run)?;
    *scenario_class = "positive_run_submission_transport";
    let response = match client.request_json("POST", "/v1/workflow-runs", Some(&submission_body)) {
        Ok(response) => response,
        Err(error) => {
            *scenario_class = match error.code.as_str() {
                "deadline_exceeded" => "positive_run_transport_deadline",
                "incomplete_response" => "positive_run_transport_incomplete",
                "invalid_response" => "positive_run_transport_invalid",
                "io_error" => "positive_run_transport_io",
                _ => "positive_run_transport_other",
            };
            return Err(error.into());
        }
    };
    *scenario_class = match response.status {
        200 => "positive_run_submission_reply_200",
        400 => "positive_run_submission_reply_400",
        403 => "positive_run_submission_reply_403",
        409 => "positive_run_submission_reply_409",
        500 => "positive_run_submission_reply_500",
        502 => "positive_run_submission_reply_502",
        _ => "positive_run_submission_reply_other_status",
    };
    let submitted: RunSubmissionResponse = decode(response)?;
    *scenario_class = "positive_run_identity";
    require(
        submitted.workflow_run_id == expected_run_id,
        "run submission changed the production live run identity",
    )?;
    Ok(submitted)
}
