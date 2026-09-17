// SPDX-License-Identifier: MIT

use super::*;
use session::{WorkflowServiceConfig, response, submit_policy_gate};
use sts2_harness::context_control::{
    ContextDraft, ContextItem, ContextItemRef, ContextSourceDocument, context_source_digest,
};
use sts2_harness::management::{
    CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION, CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION, CommandKind,
    CommandParameters, CommandRequest, CommandResponse, ContextBindingCatalog,
    ContextBindingRequest, ContextOwnerBinding, ContextOwnerSourceStatus,
    ContextSourceAdoptionRequest, ContextSourceUpload, MANAGEMENT_SCHEMA_VERSION,
};

pub(crate) fn run_served_context_source_adoption(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    run_context_source_scenario(gateway_binary, mcp_binary, harness_binary, true)?;
    run_context_source_scenario(gateway_binary, mcp_binary, harness_binary, false)
}

fn run_context_source_scenario(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    adopt_before_decision: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = TempDir::new()?;
    let provider_capture = temporary.path.join("provider-request.json");
    let bridge = temporary.bridge_capturing(&provider_capture)?;
    let mod_server = ModServer::new(FixtureMode::Success)?;
    let gateway_address = free_address()?;
    let workflow_address = free_address()?;
    let policy_store = temporary.path.join("served-provider-policy.sqlite3");
    let context_store = temporary.path.join("served-context.sqlite3");
    let execution_store = temporary.path.join("served-execution.sqlite3");
    let workflow_store = temporary.path.join("served-workflow.sqlite3");
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
    let result: Result<bool, Box<dyn std::error::Error>> = (|| {
        ready(&mut gateway_process, gateway_address)?;
        let mut service = workflow_service_command(&service_config)?.spawn()?;
        let attempt: Result<bool, Box<dyn std::error::Error>> = (|| {
            let client = wait_for_workflow_service(&mut service, workflow_address)?;
            let submission = submit_policy_gate(&client)?;
            let run_id = submission.run_id.as_str();

            let status_path = format!("/v1/workflow-runs/{run_id}/context-owner-source-status");
            let source_status = response::<ContextOwnerSourceStatus>(client.request_json(
                "GET",
                &status_path,
                None,
            )?)?;
            if source_status.workflow_run_id != run_id || source_status.active_source.is_some() {
                return Err("served owner did not report the fresh inactive source state".into());
            }

            if adopt_before_decision {
                let upload = ContextSourceUpload {
                    schema_version: CONTEXT_SOURCE_UPLOAD_SCHEMA_VERSION.to_owned(),
                    document: document.clone(),
                };
                let published = client.request_json(
                    "PUT",
                    &format!("/v1/workflow-runs/{run_id}/context-sources/strategy"),
                    Some(&serde_json::to_vec(&upload)?),
                )?;
                if published.status != 200 {
                    return Err(format!(
                        "served source publication failed: HTTP {} {}",
                        published.status,
                        String::from_utf8_lossy(&published.body)
                    )
                    .into());
                }
            }

            let observed = step(&client, run_id, submission.revision, "source-observe")?;
            if observed.outcome != sts2_harness::management::CommandOutcome::Applied {
                return Err("served observation step was not applied".into());
            }
            let current: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}"),
                None,
            )?)?;
            let run = &current["run"];
            if run["status"] != "running" || run["cursor"]["node_id"] != "decide" {
                return Err(
                    format!("served run did not reach its decide cursor: {current}").into(),
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
                .ok_or("served source owner omitted its decide descriptor")?;
            let cursor = &run["cursor"];
            let binding_request = ContextBindingRequest {
                workflow_run_id: run_id.to_owned(),
                definition_digest: run["definition_digest"]
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
                return Err("served context owner bound a different current cursor".into());
            }
            let association: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}/context-owner-association"),
                None,
            )?)?;
            if association["binding"]["node_execution_id"] != binding_request.node_execution_id {
                return Err("served association did not expose the exact bound cursor".into());
            }

            let next_revision = observed.run_revision;
            if adopt_before_decision {
                let current_source_status: ContextOwnerSourceStatus =
                    response(client.request_json("GET", &status_path, None)?)?;
                let adoption = ContextSourceAdoptionRequest {
                    schema_version: CONTEXT_SOURCE_ADOPTION_SCHEMA_VERSION.to_owned(),
                    idempotency_key: "served-source-adoption.1".to_owned(),
                    expected_control_version: current_source_status.boundary.control_version,
                    expected_revision_id: current_source_status.active_revision_id.clone(),
                    expected_boundary: current_source_status.boundary.clone(),
                };
                let adopted = client.request_json(
                    "POST",
                    &format!("/v1/workflow-runs/{run_id}/context-sources/strategy/adopt"),
                    Some(&serde_json::to_vec(&adoption)?),
                )?;
                if adopted.status != 200 {
                    return Err(format!(
                        "served source adoption failed: HTTP {} {}",
                        adopted.status,
                        String::from_utf8_lossy(&adopted.body)
                    )
                    .into());
                }
                let updated: ContextOwnerSourceStatus =
                    response(client.request_json("GET", &status_path, None)?)?;
                if updated.active_source.as_ref().is_none_or(|active| {
                    active.source_id != "strategy"
                        || active.digest != source_digest
                        || active.active_revision_id != updated.active_revision_id
                }) {
                    return Err(
                        "served source adoption did not activate the advertised source".into(),
                    );
                }
            }

            let decided = step(&client, run_id, next_revision, "source-decide")?;
            if decided.outcome != sts2_harness::management::CommandOutcome::Applied {
                return Err("served provider decision step was not applied".into());
            }
            let after_decision: Value = response(client.request_json(
                "GET",
                &format!("/v1/workflow-runs/{run_id}"),
                None,
            )?)?;
            if adopt_before_decision {
                if after_decision["run"]["status"] != "running" {
                    let events: Value = response(client.request_json(
                        "GET",
                        &format!("/v1/workflow-runs/{run_id}/events?after_sequence=0&limit=128"),
                        None,
                    )?)?;
                    return Err(format!(
                        "managed decision failed after source adoption: status={after_decision}; events={events}"
                    )
                    .into());
                }
                if after_decision["run"]["cursor"]["node_id"] != "execute" {
                    return Err(format!(
                        "served managed decision did not reach execute cursor: {after_decision}"
                    )
                    .into());
                }
            } else if after_decision["run"]["status"] != "failed" {
                return Err(format!(
                    "missing active source did not fail the run before provider exchange: {after_decision}"
                )
                .into());
            }
            Ok(adopt_before_decision)
        })();
        let output = stop(service)?;
        let adopted = attempt.map_err(|error| {
            format!(
                "served context-source attempt failed: {error}; stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            )
        })?;
        assert_killed(&output, "served context-source workflow")?;
        Ok(adopted)
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    if gateway_output.status.code() != Some(0) && gateway_output.status.signal().is_none() {
        return Err(format!("gateway cleanup failed: {}", gateway_output.status).into());
    }
    let adopted = result?;
    if !ledger.errors.is_empty() {
        return Err(format!("context-source gateway fixture failed: {:?}", ledger.errors).into());
    }
    let actions: Vec<_> = ledger
        .requests
        .iter()
        .filter(|request| request.path == "/api/v4/runtime/expert-action")
        .collect();
    if adopted {
        if !actions.is_empty() {
            return Err(format!(
                "source-render fixture crossed the game boundary before execute: {actions:?}"
            )
            .into());
        }
        let exchange_count = fs::read(provider_capture.with_extension("count"))?;
        if exchange_count != b"x" {
            return Err(format!(
                "managed decision did not perform exactly one provider exchange: {exchange_count:?}"
            )
            .into());
        }
        let request_bytes = fs::read(&provider_capture)?;
        let provider_request: Value = serde_json::from_slice(&request_bytes)?;
        if provider_request["management_profile"] != "management-enabled"
            || provider_request["management_context"]["selected_items"][0]["content"]
                != "served retained strategy"
        {
            return Err("actual served Exo request omitted the adopted managed source".into());
        }
    } else if !actions.is_empty()
        || provider_capture.exists()
        || provider_capture.with_extension("count").exists()
    {
        return Err(format!(
            "missing-source step crossed a provider or game boundary: {}",
            paths(&ledger).join(", ")
        )
        .into());
    }
    Ok(())
}

fn source_document() -> Result<(ContextSourceDocument, String), Box<dyn std::error::Error>> {
    let bytes = b"served retained strategy".to_vec();
    let item = ContextItem {
        reference: ContextItemRef {
            item_id: "served-strategy".to_owned(),
            version: 1,
            sha256: sts2_harness::sha256_hex(&bytes),
        },
        kind: "strategy".to_owned(),
        bytes,
        protected: false,
        expires_at: 4_000_000_000,
    };
    let mut draft = ContextDraft::new("served-draft", "context.revision.1");
    draft.selected_items.push(item.reference.clone());
    let mut items = std::collections::BTreeMap::new();
    items.insert("served-strategy:1".to_owned(), item);
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
