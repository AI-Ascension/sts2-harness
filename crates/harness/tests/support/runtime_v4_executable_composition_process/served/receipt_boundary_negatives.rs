// SPDX-License-Identifier: MIT

use super::*;
use serde_json::Value;
use session::{
    WorkflowServiceConfig, response, served_runtime_run_id, submit_policy_gate,
    wait_for_workflow_service, workflow_service_command,
};
use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION, CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION, CommandKind,
    CommandParameters, CommandRequest, CommandResponse, ContextBindingCatalog,
    ContextBindingRequest, ContextControlCommand, ContextControlReceipt, ContextOwnerBinding,
    ContextOwnerSourceStatus, ContextSourceAdoptionRequest, ContextSourceUpload,
    MANAGEMENT_SCHEMA_VERSION,
};

pub(super) fn run_receipt_boundary_negatives(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let provider_capture = temporary.path.join("boundary-provider-request.json");
    let bridge = temporary.bridge_capturing(&provider_capture)?;
    let mod_server = ModServer::new(FixtureMode::Success)?;
    let gateway_address = free_address()?;
    let workflow_address = free_address()?;
    let policy_store = temporary.path.join("boundary-provider-policy.sqlite3");
    let context_store = temporary.path.join("boundary-context.sqlite3");
    let execution_store = temporary.path.join("boundary-execution.sqlite3");
    let workflow_store = temporary.path.join("boundary-workflow.sqlite3");
    let runtime_run_id = served_runtime_run_id()?;
    seed_adopted_runtime_policy(&policy_store, &runtime_run_id)?;
    let (document, source_digest) = source_document()?;
    let context_owner_config = serde_json::to_string(&json!({
        "schema_version":"ascension.workflow-context-owner-config.v1",
        "store_path":context_store,
        "key_reference":"STS2_SERVED_CONTEXT_OWNER_KEY",
        "owner_id":"served-context-owner",
        "owner_version":"v1",
        "context_ref":"context.live.v1",
        "limits":{"max_items":2,"max_notes":2,"max_context_bytes":65536,"max_objective_bytes":128,"max_control_events":64},
        "render_required":true,
        "sources":[{"source_id":"strategy","version":1,"digest":source_digest}]
    }))?;
    let service_config = WorkflowServiceConfig {
        harness_binary,
        mcp_binary,
        bridge: &bridge,
        gateway_address,
        workflow_address,
        policy_store: &policy_store,
        context_store: &context_store,
        execution_store: &execution_store,
        workflow_store: &workflow_store,
        runtime_run_id: &runtime_run_id,
        context_owner_config: Some(&context_owner_config),
        instance_id: INSTANCE_ID,
        lease_id: LEASE_ID,
        lease_epoch: LEASE_EPOCH,
    };
    let mut gateway_process = gateway(gateway_binary, gateway_address, mod_server.address)?;
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway_process, gateway_address)?;
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let attempt: Result<(), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            let submission = submit_policy_gate(&client)?;
            let run_id = submission.run_id.clone();
            let status_path = format!("/v1/workflow-runs/{run_id}/context-owner-source-status");
            let upload = ContextSourceUpload {
                schema_version: CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION.to_owned(),
                document,
            };
            let uploaded = client.request_json(
                "PUT",
                &format!("/v1/workflow-runs/{run_id}/context-sources/strategy"),
                Some(&serde_json::to_vec(&upload)?),
            )?;
            if uploaded.status != 200 {
                return Err(
                    format!("boundary source publication failed: {}", uploaded.status).into(),
                );
            }
            step(
                &client,
                &run_id,
                submission.revision,
                "boundary-negative-observe",
            )?;
            let current: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}"),
                None,
            )?)?;
            if current["run"]["cursor"]["node_id"] != "decide" {
                return Err(format!("boundary fixture did not reach decide: {current}").into());
            }
            let catalog: ContextBindingCatalog =
                response(client.request_json("GET", "/v1/context-bindings", None)?)?;
            let descriptor = catalog
                .descriptors
                .iter()
                .find(|descriptor| {
                    descriptor.context_ref == "context.live.v1"
                        && descriptor.node_kinds.iter().any(|kind| kind == "decide")
                })
                .ok_or("boundary fixture omitted decide descriptor")?;
            let cursor = &current["run"]["cursor"];
            let binding_request = ContextBindingRequest {
                workflow_run_id: run_id.clone(),
                definition_digest: current["run"]["definition_digest"]
                    .as_str()
                    .ok_or("boundary run omitted definition digest")?
                    .to_owned(),
                instance_id: INSTANCE_ID.to_owned(),
                graph_id: cursor["graph_id"]
                    .as_str()
                    .ok_or("boundary run omitted graph id")?
                    .to_owned(),
                node_id: cursor["node_id"]
                    .as_str()
                    .ok_or("boundary run omitted node id")?
                    .to_owned(),
                node_execution_id: cursor["node_execution_id"]
                    .as_str()
                    .ok_or("boundary run omitted node execution id")?
                    .to_owned(),
                node_kind: "decide".to_owned(),
                context_ref: "context.live.v1".to_owned(),
                binding_id: descriptor.binding_id.clone(),
                binding_version: descriptor.version,
                binding_digest: descriptor.digest.clone(),
            };
            let bound: ContextOwnerBinding = response(client.request_json(
                "POST",
                "/v1/context-bindings/bind",
                Some(&serde_json::to_vec(&binding_request)?),
            )?)?;
            let status: ContextOwnerSourceStatus =
                response(client.request_json("GET", &status_path, None)?)?;
            if status.active_source.is_some()
                || bound.node_execution_id != binding_request.node_execution_id
            {
                return Err("boundary setup activated an unexpected source or invocation".into());
            }
            let baseline_status = status.clone();
            let valid = ContextSourceAdoptionRequest {
                schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
                idempotency_key: "boundary-negative-valid".to_owned(),
                expected_control_version: status.boundary.control_version,
                expected_revision_id: status.active_revision_id.clone(),
                expected_boundary: status.boundary.clone(),
            };
            for case in BoundaryCase::all() {
                let mut invalid = valid.clone();
                invalid.idempotency_key = format!("boundary-negative-{}", case.label());
                invalid.expected_boundary = case.mutate(&valid.expected_boundary);
                let refused = client.request_json(
                    "POST",
                    &format!("/v1/workflow-runs/{run_id}/context-sources/strategy/adopt"),
                    Some(&serde_json::to_vec(&invalid)?),
                )?;
                let body: Value = serde_json::from_slice(&refused.body)?;
                if refused.status != 409
                    || body.pointer("/error/code").and_then(Value::as_str)
                        != Some("context_source_adoption_stale")
                {
                    return Err(format!(
                        "{} did not fail with a typed 409: HTTP {} {body}",
                        case.label(),
                        refused.status
                    )
                    .into());
                }
                let unchanged: ContextOwnerSourceStatus =
                    response(client.request_json("GET", &status_path, None)?)?;
                if unchanged.active_source.is_some()
                    || unchanged.boundary != baseline_status.boundary
                    || unchanged.active_revision_id != baseline_status.active_revision_id
                {
                    return Err(format!(
                        "{} changed source status before its fence passed",
                        case.label()
                    )
                    .into());
                }
                let lookup = client.request_json(
                    "POST",
                    &format!("/v1/workflow-runs/{run_id}/context-control-receipts/lookup"),
                    Some(&serde_json::to_vec(&ContextControlCommand::Commit {
                        idempotency_key: invalid.idempotency_key.clone(),
                        expected_control_version: invalid.expected_control_version,
                        expected_revision_id: invalid.expected_revision_id.clone(),
                        expected_boundary: invalid.expected_boundary.clone(),
                        preview_manifest_digest: source_digest.clone(),
                        approved_manifest_digest: source_digest.clone(),
                    })?),
                )?;
                assert_not_recorded(lookup, case.label())?;
            }
            let adopted = client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{run_id}/context-sources/strategy/adopt"),
                Some(&serde_json::to_vec(&valid)?),
            )?;
            if adopted.status != 200 {
                return Err(format!("valid boundary adoption failed: {}", adopted.status).into());
            }
            let receipt: ContextControlReceipt = serde_json::from_slice(&adopted.body)?;
            if receipt.idempotency_key != valid.idempotency_key {
                return Err("valid boundary adoption returned the wrong receipt".into());
            }
            let activated: ContextOwnerSourceStatus =
                response(client.request_json("GET", &status_path, None)?)?;
            if activated.active_source.is_none() {
                return Err("valid boundary adoption did not activate its source".into());
            }
            let stale = ContextSourceAdoptionRequest {
                idempotency_key: "boundary-negative-stale".to_owned(),
                ..valid.clone()
            };
            let refused = client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{run_id}/context-sources/strategy/adopt"),
                Some(&serde_json::to_vec(&stale)?),
            )?;
            let body: Value = serde_json::from_slice(&refused.body)?;
            if refused.status != 409
                || body.pointer("/error/code").and_then(Value::as_str)
                    != Some("context_source_adoption_stale")
            {
                return Err(format!(
                    "old boundary was not rejected as stale: HTTP {} {body}",
                    refused.status
                )
                .into());
            }
            let unchanged: ContextOwnerSourceStatus =
                response(client.request_json("GET", &status_path, None)?)?;
            if unchanged != activated {
                return Err("stale adoption changed the active source status".into());
            }
            Ok(())
        })();
        let output = stop(service)?;
        attempt.map_err(|error| {
            format!(
                "boundary-negative workflow failed: {error}; service_stdout={}; service_stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        })?;
        assert_killed(&output, "boundary-negative workflow")?;
        Ok(())
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    result.map_err(|error| {
        gateway_failure_evidence(
            &format!("receipt boundary negatives: {error}"),
            &gateway_output,
        )
    })?;
    if gateway_output.status.code() != Some(0) && gateway_output.status.signal().is_none() {
        return Err(format!("boundary gateway cleanup failed: {}", gateway_output.status).into());
    }
    if !ledger.errors.is_empty()
        || ledger
            .requests
            .iter()
            .any(|request| request.path == "/api/v4/runtime/expert-action")
        || provider_capture.exists()
        || provider_capture.with_extension("count").exists()
    {
        return Err(format!(
            "boundary negatives crossed an effect boundary: errors={:?}, paths={:?}",
            ledger.errors,
            paths(&ledger)
        )
        .into());
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum BoundaryCase {
    Run,
    Episode,
    Agent,
    Generation,
    Controller,
    Gate,
    Control,
    Observation,
    Catalog,
    Configuration,
    Output,
}

impl BoundaryCase {
    fn all() -> [Self; 11] {
        [
            Self::Run,
            Self::Episode,
            Self::Agent,
            Self::Generation,
            Self::Controller,
            Self::Gate,
            Self::Control,
            Self::Observation,
            Self::Catalog,
            Self::Configuration,
            Self::Output,
        ]
    }

    fn label(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Episode => "episode",
            Self::Agent => "agent",
            Self::Generation => "generation",
            Self::Controller => "controller-epoch",
            Self::Gate => "gate-epoch",
            Self::Control => "control-version",
            Self::Observation => "observation-digest",
            Self::Catalog => "catalog-digest",
            Self::Configuration => "configuration-digest",
            Self::Output => "output-digest",
        }
    }

    fn mutate(self, source: &ContextBoundary) -> ContextBoundary {
        let mut value = source.clone();
        match self {
            Self::Run => value.run_id = "wrong-workflow-run".to_owned(),
            Self::Episode => value.episode_id = "wrong-episode".to_owned(),
            Self::Agent => value.agent_id = "wrong-agent".to_owned(),
            Self::Generation => value.generation += 1,
            Self::Controller => value.controller_epoch += 1,
            Self::Gate => value.gate_epoch += 1,
            Self::Control => value.control_version += 1,
            Self::Observation => value.observation_sha256 = "0".repeat(64),
            Self::Catalog => value.catalog_sha256 = "1".repeat(64),
            Self::Configuration => value.configuration_sha256 = "2".repeat(64),
            Self::Output => value.output_schema_sha256 = "3".repeat(64),
        }
        value
    }
}

fn step(
    client: &ManagementClient,
    run_id: &str,
    revision: u64,
    command_id: &str,
) -> Result<CommandResponse, Box<dyn std::error::Error>> {
    let command = CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: command_id.to_owned(),
        run_id: run_id.to_owned(),
        expected_revision: revision,
        actor_scope: "profile:served".to_owned(),
        kind: CommandKind::Step,
        parameters: CommandParameters::default(),
    };
    response(client.request_json(
        "POST",
        &format!("/v1/workflow-runs/{run_id}/commands"),
        Some(&serde_json::to_vec(&command)?),
    )?)
}

fn assert_not_recorded(
    response: sts2_harness::management::ClientResponse,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let body: Value = serde_json::from_slice(&response.body)?;
    if response.status != 404
        || body.pointer("/error/code").and_then(Value::as_str)
            != Some("context_control_receipt_not_recorded")
    {
        return Err(format!(
            "{label} unexpectedly produced a receipt: HTTP {} {body}",
            response.status
        )
        .into());
    }
    Ok(())
}
