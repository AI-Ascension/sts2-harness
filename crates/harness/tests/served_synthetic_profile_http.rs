// SPDX-License-Identifier: MIT

#![cfg(unix)]

#[path = "support/exo_test_source.rs"]
mod exo_test_source;
#[path = "support/served_synthetic_profile_http.rs"]
mod fixture;
#[path = "support/served_synthetic_gateway.rs"]
mod gateway;
#[path = "support/served_synthetic_http_contract.rs"]
mod http_contract;
#[path = "support/served_synthetic_http_exchange.rs"]
mod http_exchange;
#[path = "support/served_synthetic_responses.rs"]
mod responses;
#[path = "support/served_synthetic_target_admission.rs"]
mod target_admission;

use std::fs;

use fixture::{
    INSTANCE_ID, OwnedService, PrivateRoot, Result, ServiceBindings, mcp_script as write_mcp,
    policy_configuration, prepare_exo, reject_invalid_config_examples, seed_policy,
    service_command, wait_for_service,
};
use gateway::GatewayPeer;
use http_contract::{
    MANAGEMENT_PATH, REQUEST_ID, cancel_before_decision, decode, get_json,
    invalid_workflow_requests, post, require, served_definition, submit_step,
};
use serde_json::Value;
use sts2_harness::management::{
    CommandOutcome, EventPage, ExecutionMode, MANAGEMENT_SCHEMA_VERSION, ManagementClient,
    RunRequest, RunSubmissionResponse, TargetAvailability, TargetCatalogResponse, ValidateRequest,
    digest_value, live_run_id,
};

#[test]
fn served_synthetic_profile_http_runs_one_synthetic_turn_and_refuses_invalid_admissions()
-> Result<()> {
    require(
        std::env::var_os("STS2_EXO_TEST_NODE").is_some(),
        "root-qualified STS2_EXO_TEST_NODE is required",
    )?;
    let source = exo_test_source::pinned_exo_test_source().map_err(std::io::Error::other)?;
    let node = exo_test_source::pinned_node_test_binary().map_err(std::io::Error::other)?;
    let executor = std::env::var_os("STS2_EXO_TEST_EXECUTOR_BINARY")
        .map(std::path::PathBuf::from)
        .ok_or("root-qualified Harness Exo executor is required")?;
    let keys = fixture::TestKeys::new()?;
    let root = PrivateRoot::new()?;
    let mut responder = responses::Loopback::start()?;
    let exo = prepare_exo(&root, &source, &node, &executor, &responder.endpoint)?;

    reject_invalid_config_examples(&root, &exo)?;

    let definition = served_definition()?;
    let definition_digest = digest_value(&definition)?;
    let mut seed_run = RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: REQUEST_ID.to_owned(),
        definition: Some(definition.clone()),
        artifact_id: None,
        instance_id: INSTANCE_ID.to_owned(),
        profile: "live.workflow.v1".to_owned(),
        admission: None,
    };
    let run_id = live_run_id(&seed_run, &definition_digest)?;
    seed_policy(&root, &run_id, &exo.capabilities, &keys)?;
    let policy = policy_configuration(&root, &run_id, &exo.capabilities)?;
    let (mcp, peer_log) = write_mcp(&root)?;
    let mut gateway = GatewayPeer::bind()?;
    let workflow_address = fixture::unused_loopback_address()?;
    let cleanup_client = ManagementClient::new(workflow_address, "synthetic-test-token")?;
    let mut command = service_command(ServiceBindings {
        root: &root,
        exo: &exo,
        keys: &keys,
        policy: &policy,
        mcp: &mcp,
        gateway: gateway.address,
        workflow: workflow_address,
        run_id: &run_id,
    })?;
    if gateway.start().is_err() {
        let responses_result = responder.finish();
        let model_class = responder.result_class(&responses_result);
        drop(responder);
        let fixture_cleanup = root.cleanup();
        let fixture_class = if fixture_cleanup.is_ok() {
            "removed"
        } else {
            "cleanup_failed"
        };
        return Err(format!(
            "served_synthetic_http_failure; scenario=gateway_start_failed; cancellation=not_needed; gateway=start_failed; process=not_started; private_fixture={fixture_class}; model={model_class}"
        )
        .into());
    }
    let mut service = match OwnedService::spawn(&mut command) {
        Ok(service) => service,
        Err(_) => {
            let responses_result = responder.finish();
            let model_class = responder.result_class(&responses_result);
            let gateway_result = gateway.finish(false);
            drop(responder);
            let fixture_cleanup = root.cleanup();
            let gateway_class = if gateway_result.is_ok() {
                "cancelled_after_service_start_failure"
            } else {
                "cleanup_failed"
            };
            let fixture_class = if fixture_cleanup.is_ok() {
                "removed"
            } else {
                "cleanup_failed"
            };
            return Err(format!(
                "served_synthetic_http_failure; scenario=service_start_failed; cancellation=not_needed; gateway={gateway_class}; process=spawn_failed; private_fixture={fixture_class}; model={model_class}"
            )
            .into());
        }
    };
    let mut safe_cancel = None;

    let mut scenario_class = "service_readiness";
    let result = (|| -> Result<()> {
        let client = wait_for_service(&mut service, workflow_address)?;
        scenario_class = "catalog_and_profile_identity";
        let catalog: TargetCatalogResponse =
            decode(client.request_json("GET", "/v1/workflow-targets", None)?)?;
        catalog.validate()?;
        let descriptor = catalog
            .targets
            .iter()
            .find(|target| {
                target.instance_id == INSTANCE_ID
                    && target
                        .execution_profiles
                        .iter()
                        .any(|p| p.as_str() == "live.workflow.v1")
            })
            .ok_or("served target catalog omitted the selected live workflow descriptor")?;
        require(
            descriptor.availability == TargetAvailability::Available
                && descriptor.execution_mode == ExecutionMode::Live
                && descriptor.game_profiles.first().map(String::as_str) == Some("sts2-live-v1")
                && descriptor.inference_profiles.len() == 1
                && descriptor.inference_profiles.first().map(String::as_str)
                    == Some("exo.runtime-v3"),
            "served target descriptor did not match the live workflow profile",
        )?;
        let caps = get_json(&client, "/v1/capabilities")?;
        require(
            caps["capabilities"].as_object().is_some(),
            "served capabilities response was not an object",
        )?;
        let profiles = get_json(&client, "/v1/inference-profiles")?;
        let selected = profiles["descriptors"]
            .as_array()
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item["profile_id"] == "decision.live.v1")
            })
            .ok_or("served inference catalog omitted decision.live.v1")?;
        require(
            selected["adapter"] == "exo.runtime-v3"
                && selected["state"] == "available"
                && selected["context_compatibility"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|v| v == "context.live.v1")),
            "served inference profile did not bind the synthetic Exo decision route",
        )?;
        require(
            selected["profile_id"] == "decision.live.v1",
            "logical decision catalog identity changed",
        )?;

        let validate = ValidateRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            definition: definition.clone(),
            capabilities: caps["capabilities"].clone(),
        };
        scenario_class = "invalid_workflow_requests";
        invalid_workflow_requests(&client, workflow_address, &validate)?;
        require(
            responder.request_count() == 0,
            "invalid workflow requests reached the local model",
        )?;
        scenario_class = "valid_workflow_http_validation";
        let validated: Value = post(&client, MANAGEMENT_PATH, &validate)?;
        require(
            validated["valid"] == true && validated["definition_digest"] == definition_digest,
            "HTTP workflow validation did not return the typed definition digest",
        )?;

        let submitted = target_admission::admit_and_submit(
            &client,
            descriptor,
            &definition,
            &definition_digest,
            &mut seed_run,
            || responder.request_count(),
            &mut scenario_class,
        )?;
        require(
            submitted.workflow_run_id == run_id,
            "run submission changed the production live run identity",
        )?;
        safe_cancel = Some((run_id.to_owned(), submitted.run_revision));
        scenario_class = "run_execution_and_idempotent_replay";
        let observed = submit_step(&client, &run_id, submitted.run_revision, "observe-step")?;
        require(
            observed.outcome == CommandOutcome::Applied,
            "observation step was not applied",
        )?;
        // Once the provider decision request begins, its outcome may be ambiguous; never cancel-and-replay it.
        safe_cancel = None;
        let decided = submit_step(&client, &run_id, observed.run_revision, "decision-step")?;
        require(
            decided.outcome == CommandOutcome::Applied,
            "synthetic decision step was not applied",
        )?;
        require(
            responder.request_count() == 1,
            "decision step did not make exactly one local Responses request",
        )?;
        let duplicate = submit_step(&client, &run_id, observed.run_revision, "decision-step")?;
        require(
            duplicate.outcome == CommandOutcome::Duplicate && responder.request_count() == 1,
            "duplicate decision command caused a second model effect",
        )?;
        let status = get_json(&client, &format!("/v1/workflow-runs/{run_id}"))?;
        require(
            status["run"]["status"] == "completed"
                && status["run"]["workflow_run_id"] == run_id
                && status["run"]["definition_digest"] == definition_digest
                && responder.request_count() == 1,
            "served synthetic run did not complete with one provider response",
        )?;
        let events: EventPage = decode(client.request_json(
            "GET",
            &format!("/v1/workflow-runs/{run_id}/events?after_sequence=0&limit=64"),
            None,
        )?)?;
        require(
            events.workflow_run_id == run_id
                && !events.events.is_empty()
                && events.events.iter().all(|event| {
                    event.workflow_run_id == run_id && event.definition_digest == definition_digest
                }),
            "served event page did not join to the exact run and definition",
        )?;
        // The closed public RunEvent has node_execution_id, not model_execution_id; keep those namespaces distinct.
        let replay: RunSubmissionResponse = post(&client, "/v1/workflow-runs", &seed_run)?;
        require(
            replay.workflow_run_id == run_id && responder.request_count() == 1,
            "idempotent run replay repeated the provider effect",
        )?;
        let mcp_log = fs::read_to_string(&peer_log)?;
        require(
            mcp_log.contains("tools/call sts2.observe")
                && mcp_log.contains("tools/call sts2.legal_actions")
                && !mcp_log.contains("sts2.dispatch_action"),
            "controlled MCP peer did not observe without dispatching a game action",
        )?;
        require(
            !root.path.join("mcp-action.json").exists(),
            "controlled MCP peer recorded a game action",
        )?;
        Ok(())
    })();
    scenario_class = http_contract::failure_stage(&result, scenario_class);
    let responses_result = responder.finish();
    let response_count = responder.request_count();
    let cancellation_result = match (result.is_err(), safe_cancel.as_ref()) {
        (true, Some((active_run_id, revision))) => Some(cancel_before_decision(
            &cleanup_client,
            active_run_id,
            *revision,
        )),
        _ => None,
    };
    let gateway_result = gateway.finish(result.is_ok());
    let gateway_release_observed = matches!(
        gateway_result.as_ref(),
        Ok(evidence) if evidence.confirms_exact_release()
    );
    // The preceding Gateway finish records the release independently before process settlement.
    let service_result = service.settle();
    let model_class = responder.result_class(&responses_result);
    drop(responder);
    let fixture_cleanup = root.cleanup();
    if result.is_ok()
        && cancellation_result
            .as_ref()
            .is_none_or(|outcome| outcome.is_ok())
        && gateway_release_observed
        && service_result.is_ok()
        && fixture_cleanup.is_ok()
        && responses_result.is_ok()
        && response_count == 1
    {
        return Ok(());
    }
    let scenario = if result.is_ok() {
        "terminal_scenario_complete"
    } else {
        scenario_class
    };
    let cancellation_class = match cancellation_result.as_ref() {
        Some(outcome) if outcome.is_ok() => "confirmed",
        Some(_) => "failed",
        None if result.is_err() && scenario_class == "run_execution_and_idempotent_replay" => {
            "not_attempted_after_ambiguous_decision"
        }
        None => "not_needed",
    };
    let gateway_class = match gateway_result.as_ref() {
        Ok(_) if gateway_release_observed => "exact_release_observed",
        Ok(evidence) if evidence.allocation.is_some() => "allocation_observed_release_unconfirmed",
        Ok(_) => "allocation_not_observed",
        Err(_) => "cleanup_failed",
    };
    Err(format!(
        "served_synthetic_http_failure; scenario={scenario}; cancellation={cancellation_class}; gateway={gateway_class}; process={}; private_fixture={}; model={model_class}",
        if service_result.is_ok() { "settled" } else { "settlement_failed" },
        if fixture_cleanup.is_ok() { "removed" } else { "cleanup_failed" }
    )
    .into())
}
