// SPDX-License-Identifier: MIT

//! Authoritative context-owner current association over authenticated management HTTP.
//! Synthetic owner and in-memory store only; no provider or game is launched.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA, CONTEXT_OWNER_BINDING_SCHEMA_VERSION,
    ContextBindingCatalog, ContextBindingContinuity, ContextBindingGrants, ContextBindingRequest,
    ContextBindingState, ContextOwnerBinding, ContextOwnerPort, MANAGEMENT_SCHEMA_VERSION,
    ManagementClient, ManagementError, ManagementServer, ManagementService, MemoryWorkflowStore,
    RunRequest, ServerConfig, StaticAuthenticator, synthetic_store,
};

const VALID_WORKFLOW: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn actor() -> AuthContext {
    AuthContext::new("integration.tester", ["workflow:*".to_owned()]).expect("actor")
}

fn request() -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "owner-association-request".to_owned(),
        definition: Some(serde_json::from_slice::<Value>(VALID_WORKFLOW).expect("fixture")),
        artifact_id: None,
        instance_id: "integration-instance".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    }
}

fn boundary(run_id: &str) -> ContextBoundary {
    ContextBoundary {
        run_id: run_id.to_owned(),
        episode_id: "test.episode.1".to_owned(),
        agent_id: "test.agent.1".to_owned(),
        state_id: "test.state.1".to_owned(),
        generation: 1,
        observation_sha256: "a".repeat(64),
        catalog_sha256: "b".repeat(64),
        adapter_revision: "test.adapter.v1".to_owned(),
        model_revision: "test.model.v1".to_owned(),
        configuration_sha256: "c".repeat(64),
        output_schema_sha256: "d".repeat(64),
        controller_epoch: 1,
        gate_epoch: 1,
        control_version: 1,
    }
}

fn binding_for(run_id: &str) -> ContextOwnerBinding {
    ContextOwnerBinding {
        schema_version: CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
        owner_id: "test.context-owner".to_owned(),
        owner_version: "1.0.0".to_owned(),
        invocation_id: "test.invocation.1".to_owned(),
        binding_id: "test.binding.1".to_owned(),
        binding_version: 1,
        binding_digest: "e".repeat(64),
        context_ref: "context.test.v1".to_owned(),
        instance_id: "sts2-test-1".to_owned(),
        node_kind: "decide".to_owned(),
        state: ContextBindingState::Available,
        workflow_run_id: run_id.to_owned(),
        definition_digest: "f".repeat(64),
        graph_id: "main".to_owned(),
        node_id: "decide".to_owned(),
        node_execution_id: "test.node.1".to_owned(),
        boundary: boundary(run_id),
        lease_epoch: 1,
        snapshot_id: "test.snapshot.1".to_owned(),
        approved_revision_id: "test.revision.1".to_owned(),
        plan_epoch: 1,
        grants: ContextBindingGrants {
            metadata_read: true,
            content_read: false,
            edit: false,
            control: false,
        },
        continuity: ContextBindingContinuity {
            survives_controller_restart: true,
            receipt_recovery: true,
            provider_session_continuity: false,
        },
    }
}

/// Association owner that answers either for the requested run or for a foreign one.
struct AssociationOwner {
    answer: Answer,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    Matching,
    ForeignRun,
}

impl ContextOwnerPort for AssociationOwner {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        Err(ManagementError::unavailable(
            "test_catalog_unused",
            "catalog is not used by the association path",
        ))
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        _request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "test_bind_unused",
            "bind is not used by the association path",
        ))
    }

    fn association(
        &self,
        _actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        let run = match self.answer {
            Answer::Matching => snapshot.workflow_run_id.clone(),
            Answer::ForeignRun => "run.somewhere.else".to_owned(),
        };
        Ok(binding_for(&run))
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn service_with(owner: Answer) -> Arc<ManagementService> {
    Arc::new(
        synthetic_store(Arc::new(MemoryWorkflowStore::new()))
            .with_context_owner_port(Arc::new(AssociationOwner { answer: owner })),
    )
}

fn run_id(service: &ManagementService) -> String {
    service
        .submit_run(&actor(), request())
        .expect("run is admitted")
        .workflow_run_id
}

fn get(
    service: &Arc<ManagementService>,
    token: &str,
    authenticator: StaticAuthenticator,
    path: &str,
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server starts");
    let client = ManagementClient::new(server.address(), token).expect("client");
    let response = client.request_json("GET", path, None).expect("response");
    server.shutdown().expect("server shuts down");
    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    (response.status, value)
}

fn owner_authenticator() -> StaticAuthenticator {
    StaticAuthenticator::single("owner-token", actor()).expect("authenticator")
}

fn path_for(run_id: &str) -> String {
    format!("/v1/workflow-runs/{run_id}/context-owner-association")
}

#[test]
fn current_association_is_projected_for_the_run() {
    let service = service_with(Answer::Matching);
    let run = run_id(&service);
    let (status, value) = get(
        &service,
        "owner-token",
        owner_authenticator(),
        &path_for(&run),
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value.get("schema_version").and_then(Value::as_str),
        Some(CONTEXT_OWNER_ASSOCIATION_VIEW_SCHEMA)
    );
    let binding = value.get("binding").expect("binding projection");
    assert_eq!(
        binding.get("owner_id").and_then(Value::as_str),
        Some("test.context-owner")
    );
    assert_eq!(
        binding.get("binding_id").and_then(Value::as_str),
        Some("test.binding.1")
    );
    assert_eq!(
        binding.get("workflow_run_id").and_then(Value::as_str),
        Some(run.as_str())
    );
    assert_eq!(
        binding.get("state").and_then(Value::as_str),
        Some("available")
    );
}

#[test]
fn a_binding_for_another_run_is_rejected_not_returned() {
    let service = service_with(Answer::ForeignRun);
    let run = run_id(&service);
    let (status, value) = get(
        &service,
        "owner-token",
        owner_authenticator(),
        &path_for(&run),
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_binding_mismatch")
    );
}

#[test]
fn an_unattached_owner_reports_unavailable() {
    let service = Arc::new(synthetic_store(Arc::new(MemoryWorkflowStore::new())));
    let run = run_id(&service);
    let (status, value) = get(
        &service,
        "owner-token",
        owner_authenticator(),
        &path_for(&run),
    );
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_owner_association_unavailable")
    );
}

#[test]
fn association_requires_the_scoped_workflow_read_grant() {
    let service = service_with(Answer::Matching);
    let run = run_id(&service);
    let authenticator = StaticAuthenticator::single(
        "limited-token",
        AuthContext::new("integration.tester", ["context:read".to_owned()]).expect("actor"),
    )
    .expect("authenticator");
    let (status, value) = get(&service, "limited-token", authenticator, &path_for(&run));
    assert_eq!(status, 403, "body: {value}");
}

#[test]
fn an_unknown_run_is_reported_as_not_found_input() {
    let service = service_with(Answer::Matching);
    let (status, value) = get(
        &service,
        "owner-token",
        owner_authenticator(),
        &path_for("run.does.not.exist"),
    );
    assert_eq!(status, 400, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("run_not_found")
    );
}
