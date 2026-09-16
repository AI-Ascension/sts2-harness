// SPDX-License-Identifier: MIT

use super::*;
use serde_json::Value;
use std::sync::Arc;
use sts2_harness::management::{
    Budget, CleanupState, Cursor, EventClassification, EventPayload, EventType, ExecutionMode,
    GameOutcome, ManagementClient, ManagementServer, ManagementService, MemoryWorkflowStore,
    RUN_SCHEMA_VERSION, RunEvent, RunSnapshot, RunTargetConfiguration, ServerConfig,
    StaticAuthenticator, TARGET_ADMISSION_SCHEMA_VERSION, TargetAdmissionBinding,
    WorkflowRunStatus, WorkflowStore,
};

#[path = "association_receipt_commit_test.rs"]
mod commit_test;

fn admitted_snapshot(
    request: &RunRequest,
    digest: &str,
    binding_request: &ContextBindingRequest,
) -> RunSnapshot {
    RunSnapshot {
        schema_version: RUN_SCHEMA_VERSION.into(),
        workflow_run_id: run_id(request, digest).expect("run"),
        definition_digest: digest.into(),
        run_revision: 1,
        status: WorkflowRunStatus::Running,
        game_outcome: GameOutcome::NotTerminal,
        cursor: Cursor {
            graph_id: binding_request.graph_id.clone(),
            node_id: binding_request.node_id.clone(),
            node_execution_id: binding_request.node_execution_id.clone(),
        },
        pending_operation: None,
        budget: Budget::default(),
        cleanup: CleanupState::NotStarted,
        admission: Some(TargetAdmissionBinding {
            schema_version: TARGET_ADMISSION_SCHEMA_VERSION.into(),
            request_id: request.request_id.clone(),
            workflow_definition_digest: digest.into(),
            target: RunTargetConfiguration {
                instance_id: request.instance_id.clone(),
                execution_profile: "live".into(),
                execution_mode: ExecutionMode::Live,
                workflow_revision: "workflow-v1".into(),
                compatibility_revision: "compatibility-v1".into(),
                capability_revision: "capability-v1".into(),
                game_profile: "game-profile".into(),
                save_profile: None,
                inference_profile: None,
                context_capability: None,
                provider_capability: None,
            },
            descriptor_digest: "e".repeat(64),
            catalog_revision: "catalog-v1".into(),
        }),
        execution_mode: Some(sts2_harness::management::ExecutionMode::Live),
    }
}

fn management_service(owner: Arc<Owner>, snapshot: &RunSnapshot) -> Arc<ManagementService> {
    let store = Arc::new(MemoryWorkflowStore::new());
    let event = RunEvent {
        schema_version: sts2_harness::management::EVENT_SCHEMA_VERSION.into(),
        workflow_run_id: snapshot.workflow_run_id.clone(),
        sequence: 1,
        run_revision: snapshot.run_revision,
        event_type: EventType::RunStarted,
        definition_digest: snapshot.definition_digest.clone(),
        node_execution_id: snapshot.cursor.node_execution_id.clone(),
        payload: EventPayload {
            operation_id: None,
            classification: Some(EventClassification::Accepted),
            reason_code: "fixture_started".into(),
        },
        integrity_digest: None,
    };
    store
        .create_run("request", &"f".repeat(64), snapshot.clone(), vec![event])
        .expect("seed admitted run snapshot");
    Arc::new(ManagementService::new(store).with_context_owner_port(owner))
}

fn management_http(
    service: &Arc<ManagementService>,
    token: &str,
    authenticator: StaticAuthenticator,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server starts");
    let client = ManagementClient::new(server.address(), token).expect("client");
    let response = client
        .request_json(method, path, body)
        .expect("HTTP response");
    server.shutdown().expect("server shuts down");
    (
        response.status,
        serde_json::from_slice(&response.body).expect("JSON response"),
    )
}

fn owner_authenticator(actor: &AuthContext) -> StaticAuthenticator {
    StaticAuthenticator::single("owner-token", actor.clone()).expect("authenticator")
}

#[test]
fn authenticated_http_exposes_fresh_association_and_recovers_receipt_after_restart() {
    let (owner, actor, request, mut runtime_binding, digest, selected) = setup();
    runtime_binding.lease_epoch = 7;
    owner
        .record_observation(
            &actor,
            &request,
            &digest,
            &runtime_binding,
            &observation(1),
            &selected,
        )
        .expect("runtime observation");
    let actions = EpisodeLegalActionSet::new(
        "combat-1",
        1,
        vec![EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn).expect("action")],
    )
    .expect("actions");
    owner
        .record_legal_actions(&actor, &request, &digest, &runtime_binding, &actions)
        .expect("current legal-action catalog");
    let catalog = owner.catalog(&actor).expect("catalog");
    let descriptor = &catalog.descriptors[0];
    let binding_request = ContextBindingRequest {
        workflow_run_id: runtime_binding.run_id.clone(),
        definition_digest: digest.clone(),
        instance_id: request.instance_id.clone(),
        graph_id: "graph-1".into(),
        node_id: "node-1".into(),
        node_execution_id: "execution-1".into(),
        node_kind: "decide".into(),
        context_ref: descriptor.context_ref.clone(),
        binding_id: descriptor.binding_id.clone(),
        binding_version: descriptor.version,
        binding_digest: descriptor.digest.clone(),
    };
    let binding = owner
        .bind(&actor, &binding_request)
        .expect("current invocation binding");
    let active_owner = Arc::new(owner);
    let snapshot = admitted_snapshot(&request, &digest, &binding_request);
    let service = management_service(Arc::clone(&active_owner), &snapshot);
    let mut foreign_instance_request = binding_request.clone();
    foreign_instance_request.instance_id = "foreign-instance".into();
    assert_eq!(
        active_owner
            .bind(&actor, &foreign_instance_request)
            .expect_err("owner must reject an untrusted instance")
            .code,
        "context_owner_instance"
    );
    let body = serde_json::to_vec(&foreign_instance_request).expect("foreign binding JSON");
    let (status, value) = management_http(
        &service,
        "owner-token",
        owner_authenticator(&actor),
        "POST",
        "/v1/context-bindings/bind",
        Some(&body),
    );
    assert_eq!(
        status, 409,
        "foreign instance bind must be refused: {value}"
    );
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_binding_instance_mismatch")
    );

    let association_path = format!(
        "/v1/workflow-runs/{}/context-owner-association",
        snapshot.workflow_run_id
    );
    let (status, association) = management_http(
        &service,
        "owner-token",
        owner_authenticator(&actor),
        "GET",
        &association_path,
        None,
    );
    assert_eq!(
        status, 200,
        "true association remains available: {association}"
    );
    assert_eq!(
        association
            .pointer("/binding/instance_id")
            .and_then(Value::as_str),
        Some(request.instance_id.as_str())
    );
    assert_eq!(
        binding.lease_epoch, 7,
        "binding must carry gateway lease epoch"
    );
    let command = ContextControlCommand::Pause {
        idempotency_key: "pause.restart.case".into(),
        expected_control_version: binding.boundary.control_version,
    };
    let receipt = active_owner
        .control(&actor, &binding, &command)
        .expect("accepted control command");
    assert_eq!(receipt.effect, "pause_requested");

    let (status, value) = management_http(
        &service,
        "owner-token",
        owner_authenticator(&actor),
        "GET",
        &association_path,
        None,
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value
            .pointer("/binding/lease_epoch")
            .and_then(Value::as_u64),
        Some(7)
    );
    let limits_path = format!(
        "/v1/workflow-runs/{}/context-owner-effective-limits",
        snapshot.workflow_run_id
    );
    let (status, value) = management_http(
        &service,
        "owner-token",
        owner_authenticator(&actor),
        "GET",
        &limits_path,
        None,
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value
            .pointer("/effective_limits/max_control_events")
            .and_then(Value::as_u64),
        Some(selected.max_control_events)
    );
    let foreign = AuthContext::new("other.actor", ["workflow:*".into()]).expect("foreign actor");
    let (status, _) = management_http(
        &service,
        "foreign-token",
        StaticAuthenticator::single("foreign-token", foreign.clone()).expect("authenticator"),
        "GET",
        &association_path,
        None,
    );
    assert_eq!(status, 403, "association belongs to the original actor");

    // A new process has no current LiveRun association. Its receipt port reads
    // only encrypted historical evidence and must not claim the live writer.
    let restarted = Arc::new(Owner {
        configuration: active_owner.configuration.clone(),
        key: active_owner.key,
        current: Mutex::new(BTreeMap::new()),
    });
    let restarted_service = management_service(Arc::clone(&restarted), &snapshot);
    let (status, value) = management_http(
        &restarted_service,
        "owner-token",
        owner_authenticator(&actor),
        "GET",
        &association_path,
        None,
    );
    assert_eq!(status, 503, "association must not survive restart: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_owner_association_unavailable")
    );

    let receipt_path = format!(
        "/v1/workflow-runs/{}/context-control-receipts/lookup",
        snapshot.workflow_run_id
    );
    let body = serde_json::to_vec(&command).expect("command JSON");
    let (status, value) = management_http(
        &restarted_service,
        "owner-token",
        owner_authenticator(&actor),
        "POST",
        &receipt_path,
        Some(&body),
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value.get("effect").and_then(Value::as_str),
        Some("pause_requested")
    );
    assert_eq!(
        value.get("idempotency_key").and_then(Value::as_str),
        Some("pause.restart.case")
    );
    assert!(restarted.current.lock().expect("current lock").is_empty());

    let (status, value) = management_http(
        &restarted_service,
        "foreign-token",
        StaticAuthenticator::single("foreign-token", foreign.clone()).expect("authenticator"),
        "POST",
        &receipt_path,
        Some(&body),
    );
    assert_eq!(status, 404, "foreign actor must not learn receipt: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_control_receipt_not_recorded")
    );

    let changed = ContextControlCommand::Pause {
        idempotency_key: "pause.restart.case".into(),
        expected_control_version: binding.boundary.control_version + 1,
    };
    let changed_body = serde_json::to_vec(&changed).expect("changed command JSON");
    let (status, value) = management_http(
        &restarted_service,
        "owner-token",
        owner_authenticator(&actor),
        "POST",
        &receipt_path,
        Some(&changed_body),
    );
    assert_eq!(
        status, 404,
        "changed command must not reuse receipt: {value}"
    );

    // The historical lookup did not steal the active process's single-writer
    // token; it can still issue the next binding and persist through the store.
    let rebound = active_owner
        .bind(&actor, &binding_request)
        .expect("live writer remains unfenced");
    assert_eq!(rebound.boundary.controller_epoch, receipt.controller_epoch);
}
