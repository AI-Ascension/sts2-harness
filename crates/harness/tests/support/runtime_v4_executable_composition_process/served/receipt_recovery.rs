// SPDX-License-Identifier: MIT

use super::*;
use serde_json::Value;
use session::{
    WorkflowServiceConfig, response, served_runtime_run_id, submit_policy_gate,
    wait_for_workflow_service, workflow_service_command,
};
use sts2_harness::context_control::{
    ContextDraft, ContextItem, ContextItemRef, ContextSourceDocument, context_source_digest,
};
use sts2_harness::management::{
    CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION, CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingRequest, ContextControlCommand, ContextControlReceipt,
    ContextOwnerBinding, ContextOwnerSourceStatus, ContextSourceAdoptionRequest,
    ContextSourceUpload,
};

#[path = "receipt_boundary_negatives.rs"]
mod boundary_negatives;

pub(crate) fn run_served_context_receipt_recovery(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    boundary_negatives::run_receipt_boundary_negatives(gateway_binary, mcp_binary, harness_binary)?;
    let temporary = TempDir::new()?;
    let provider_capture = temporary.path.join("receipt-provider-request.json");
    let bridge = temporary.bridge_capturing(&provider_capture)?;
    let mod_server = ModServer::new(FixtureMode::Success)?;
    let gateway_address = free_address()?;
    let workflow_address = free_address()?;
    let policy_store = temporary.path.join("receipt-provider-policy.sqlite3");
    let context_store = temporary.path.join("receipt-context.sqlite3");
    let execution_store = temporary.path.join("receipt-execution.sqlite3");
    let workflow_store = temporary.path.join("receipt-workflow.sqlite3");
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
    let mut gateway = gateway(gateway_binary, gateway_address, mod_server.address)?;
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway, gateway_address)?;
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let write_result: Result<
            (String, ContextControlReceipt, ContextControlCommand),
            Box<dyn std::error::Error>,
        > = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            let submission = submit_policy_gate(&client)?;
            let run_id = submission.run_id.clone();
            let status_path = format!("/v1/workflow-runs/{run_id}/context-owner-source-status");
            let status: ContextOwnerSourceStatus =
                response(client.request_json("GET", &status_path, None)?)?;
            if status.active_source.is_some() {
                return Err("new served owner unexpectedly had an active source".into());
            }
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
                return Err(format!(
                    "source publication failed: HTTP {} {}",
                    uploaded.status,
                    String::from_utf8_lossy(&uploaded.body)
                )
                .into());
            }
            step(&client, &run_id, submission.revision, "receipt-observe")?;
            let current: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}"),
                None,
            )?)?;
            if current["run"]["status"] != "running"
                || current["run"]["cursor"]["node_id"] != "decide"
            {
                return Err(
                    format!("run did not reach its current decide cursor: {current}").into(),
                );
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
                .ok_or("served owner omitted its decide descriptor")?;
            let cursor = &current["run"]["cursor"];
            let binding_request = ContextBindingRequest {
                workflow_run_id: run_id.clone(),
                definition_digest: current["run"]["definition_digest"]
                    .as_str()
                    .ok_or("run omitted definition digest")?
                    .to_owned(),
                instance_id: INSTANCE_ID.to_owned(),
                graph_id: cursor["graph_id"]
                    .as_str()
                    .ok_or("run omitted graph identity")?
                    .to_owned(),
                node_id: cursor["node_id"]
                    .as_str()
                    .ok_or("run omitted node identity")?
                    .to_owned(),
                node_execution_id: cursor["node_execution_id"]
                    .as_str()
                    .ok_or("run omitted node execution identity")?
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
            if bound.node_execution_id != binding_request.node_execution_id
                || bound.node_id != binding_request.node_id
                || bound.instance_id != INSTANCE_ID
            {
                return Err("owner bound a different current invocation".into());
            }
            let current_status: ContextOwnerSourceStatus =
                response(client.request_json("GET", &status_path, None)?)?;
            let adoption = ContextSourceAdoptionRequest {
                schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
                idempotency_key: "receipt-process-restart.1".to_owned(),
                expected_control_version: current_status.boundary.control_version,
                expected_revision_id: current_status.active_revision_id.clone(),
                expected_boundary: current_status.boundary.clone(),
            };
            let command = ContextControlCommand::Commit {
                idempotency_key: adoption.idempotency_key.clone(),
                expected_control_version: adoption.expected_control_version,
                expected_revision_id: adoption.expected_revision_id.clone(),
                expected_boundary: adoption.expected_boundary.clone(),
                preview_manifest_digest: source_digest.clone(),
                approved_manifest_digest: source_digest.clone(),
            };
            let adopted = client.request_json(
                "POST",
                &format!("/v1/workflow-runs/{run_id}/context-sources/strategy/adopt"),
                Some(&serde_json::to_vec(&adoption)?),
            )?;
            if adopted.status != 200 {
                return Err(format!(
                    "source adoption failed: HTTP {} {}",
                    adopted.status,
                    String::from_utf8_lossy(&adopted.body)
                )
                .into());
            }
            let receipt: ContextControlReceipt = serde_json::from_slice(&adopted.body)?;
            if receipt.command != sts2_harness::management::ContextControlCommandKind::Commit
                || receipt.binding_id != bound.binding_id
                || receipt.invocation_id != bound.invocation_id
                || receipt.idempotency_key != adoption.idempotency_key
            {
                return Err("adoption receipt was not bound to the current invocation".into());
            }
            Ok((run_id, receipt, command))
        })();
        let first_output = stop(service)?;
        let (run_id, receipt, command) =
            write_result.map_err(|error| format!("receipt writer phase: {error}"))?;
        assert_killed(&first_output, "receipt writer workflow")?;

        let mut restarted_service = workflow_service_command(&service_config)?.spawn()?;
        let restart_result: Result<(), Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut restarted_service, workflow_address)?;
            let association_path = format!("/v1/workflow-runs/{run_id}/context-owner-association");
            let association = client.request_json("GET", &association_path, None)?;
            let association_body: Value = serde_json::from_slice(&association.body)?;
            if association.status != 503
                || association_body
                    .pointer("/error/code")
                    .and_then(Value::as_str)
                    != Some("context_owner_association_unavailable")
            {
                return Err(format!(
                    "restart fabricated current owner association: HTTP {} {association_body}",
                    association.status
                )
                .into());
            }
            let receipt_path =
                format!("/v1/workflow-runs/{run_id}/context-control-receipts/lookup");
            let recovered_response =
                client.request_json("POST", &receipt_path, Some(&serde_json::to_vec(&command)?))?;
            if recovered_response.status != 200 {
                return Err(format!(
                    "historical receipt lookup failed: HTTP {} {}",
                    recovered_response.status,
                    String::from_utf8_lossy(&recovered_response.body)
                )
                .into());
            }
            let recovered: ContextControlReceipt =
                serde_json::from_slice(&recovered_response.body)?;
            if recovered != receipt {
                return Err("historical lookup returned a different owner receipt".into());
            }
            let (expected_control_version, expected_revision_id, expected_boundary) = match &command
            {
                ContextControlCommand::Commit {
                    expected_control_version,
                    expected_revision_id,
                    expected_boundary,
                    ..
                } => (
                    *expected_control_version,
                    expected_revision_id.clone(),
                    expected_boundary.clone(),
                ),
                ContextControlCommand::Pause { .. } | ContextControlCommand::Resume { .. } => {
                    return Err("receipt command unexpectedly was not a commit".into());
                }
            };
            let changed = ContextControlCommand::Commit {
                idempotency_key: "receipt-process-restart.1".to_owned(),
                expected_control_version: expected_control_version + 1,
                expected_revision_id,
                expected_boundary,
                preview_manifest_digest: source_digest.clone(),
                approved_manifest_digest: source_digest.clone(),
            };
            let changed_response =
                client.request_json("POST", &receipt_path, Some(&serde_json::to_vec(&changed)?))?;
            assert_not_recorded(changed_response, "changed command")?;
            let metadata_only = ContextControlCommand::Pause {
                idempotency_key: "receipt-process-restart.1".to_owned(),
                expected_control_version,
            };
            let metadata_response = client.request_json(
                "POST",
                &receipt_path,
                Some(&serde_json::to_vec(&metadata_only)?),
            )?;
            assert_not_recorded(metadata_response, "metadata-only command")?;
            Ok(())
        })();
        let restarted_output = stop(restarted_service)?;
        restart_result.map_err(|error| format!("restart recovery phase: {error}"))?;
        assert_killed(&restarted_output, "receipt recovery workflow")?;

        let mut foreign_service = workflow_service_command(&service_config)?;
        foreign_service
            .env("STS2_WORKFLOW_AUTH_PROFILE", "foreign")
            .env("STS2_WORKFLOW_TOKEN_FOREIGN", "foreign-workflow-token");
        let mut foreign_service = foreign_service.spawn()?;
        let foreign_result: Result<(), Box<dyn std::error::Error>> = (|| {
            let _ = wait_for_workflow_service(&mut foreign_service, workflow_address)?;
            let client = ManagementClient::new(workflow_address, "foreign-workflow-token")?;
            let receipt_path =
                format!("/v1/workflow-runs/{run_id}/context-control-receipts/lookup");
            let denied =
                client.request_json("POST", &receipt_path, Some(&serde_json::to_vec(&command)?))?;
            let body: Value = serde_json::from_slice(&denied.body)?;
            if denied.status != 404
                || body.pointer("/error/code").and_then(Value::as_str)
                    != Some("context_control_receipt_not_recorded")
            {
                return Err(format!(
                    "foreign actor learned or changed the historical receipt: HTTP {} {body}",
                    denied.status
                )
                .into());
            }
            Ok(())
        })();
        let foreign_output = stop(foreign_service)?;
        foreign_result.map_err(|error| format!("foreign actor phase: {error}"))?;
        assert_killed(&foreign_output, "foreign receipt reader")?;
        Ok(())
    })();
    let gateway_output = stop(gateway)?;
    let ledger = mod_server.finish();
    result.map_err(|error| {
        gateway_failure_evidence(
            "receipt-recovery",
            &format!("served context receipt recovery: {error}"),
            &gateway_output,
        )
    })?;
    if gateway_output.status.code() != Some(0) && gateway_output.status.signal().is_none() {
        return Err(format!("gateway cleanup failed: {}", gateway_output.status).into());
    }
    if !ledger.errors.is_empty() {
        return Err(format!("receipt recovery fixture failed: {:?}", ledger.errors).into());
    }
    if ledger
        .requests
        .iter()
        .any(|request| request.path == "/api/v4/runtime/expert-action")
        || provider_capture.exists()
        || provider_capture.with_extension("count").exists()
    {
        return Err("receipt recovery crossed provider or game action boundary".into());
    }
    if !ledger
        .requests
        .iter()
        .any(|request| request.path == "/api/v4/runtime/expert-state")
    {
        return Err("receipt recovery did not observe the authoritative current invocation".into());
    }
    Ok(())
}

fn source_document() -> Result<(ContextSourceDocument, String), Box<dyn std::error::Error>> {
    let bytes = b"receipt retained strategy".to_vec();
    let item = ContextItem {
        reference: ContextItemRef {
            item_id: "receipt-strategy".to_owned(),
            version: 1,
            sha256: sts2_harness::sha256_hex(&bytes),
        },
        kind: "strategy".to_owned(),
        bytes,
        protected: false,
        expires_at: 4_000_000_000,
    };
    let mut draft = ContextDraft::new("receipt-draft", "context.revision.1");
    draft.selected_items.push(item.reference.clone());
    let mut items = std::collections::BTreeMap::new();
    items.insert("receipt-strategy:1".to_owned(), item);
    let document = ContextSourceDocument { draft, items };
    let digest = context_source_digest(&document)?;
    Ok((document, digest))
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
            "{label} was incorrectly accepted: HTTP {} {body}",
            response.status
        )
        .into());
    }
    Ok(())
}
