// SPDX-License-Identifier: MIT

//! Owner control-receipt recovery over the authenticated management HTTP surface.
//! Synthetic owner and in-memory store only; no provider or game is launched.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::context_control::ContextBoundary;
use sts2_harness::management::{
    AuthContext, CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION, ContextBindingCatalog,
    ContextBindingContinuity, ContextBindingGrants, ContextBindingRequest, ContextBindingState,
    ContextControlCommand, ContextControlCommandKind, ContextControlReceipt, ContextOwnerBinding,
    ContextOwnerPort, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementError,
    ManagementServer, ManagementService, MemoryWorkflowStore, RunRequest, ServerConfig,
    StaticAuthenticator, synthetic_store,
};

const VALID_WORKFLOW: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn actor() -> AuthContext {
    AuthContext::new("integration.tester", ["workflow:*".to_owned()]).expect("actor")
}

fn request() -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "receipt-recovery-request".to_owned(),
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

fn binding_for(run_id: &str, receipt_recovery: bool) -> ContextOwnerBinding {
    ContextOwnerBinding {
        schema_version: sts2_harness::management::CONTEXT_OWNER_BINDING_SCHEMA_VERSION.to_owned(),
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
            control: true,
        },
        continuity: ContextBindingContinuity {
            survives_controller_restart: true,
            receipt_recovery,
            provider_session_continuity: false,
        },
    }
}

fn pause_command() -> ContextControlCommand {
    ContextControlCommand::Pause {
        idempotency_key: "control.pause.key.1".to_owned(),
        expected_control_version: 1,
    }
}

/// The receipt the owner would have issued when it accepted `pause_command`.
fn pause_receipt(binding: &ContextOwnerBinding) -> ContextControlReceipt {
    let mut resulting = binding.boundary.clone();
    // `Pause` advances both the control version and the gate epoch.
    resulting.control_version += 1;
    resulting.gate_epoch += 1;
    ContextControlReceipt {
        schema_version: CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION.to_owned(),
        owner_id: binding.owner_id.clone(),
        invocation_id: binding.invocation_id.clone(),
        binding_id: binding.binding_id.clone(),
        binding_digest: binding.binding_digest.clone(),
        command: ContextControlCommandKind::Pause,
        command_id: "control.pause.1".to_owned(),
        idempotency_key: "control.pause.key.1".to_owned(),
        effect: "pause_requested".to_owned(),
        control_version: resulting.control_version,
        plan_epoch: binding.plan_epoch,
        controller_epoch: resulting.controller_epoch,
        gate_epoch: resulting.gate_epoch,
        boundary: resulting,
        revision_id: None,
        preview_manifest_digest: None,
        approved_manifest_digest: None,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Recorded,
    Absent,
    Tampered,
}

struct RecoveringOwner {
    receipt_recovery: bool,
    outcome: Outcome,
}

impl ContextOwnerPort for RecoveringOwner {
    fn catalog(&self, _actor: &AuthContext) -> Result<ContextBindingCatalog, ManagementError> {
        Err(ManagementError::unavailable(
            "test_catalog_unused",
            "catalog is not used by the recovery path",
        ))
    }

    fn bind(
        &self,
        _actor: &AuthContext,
        _request: &ContextBindingRequest,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Err(ManagementError::unavailable(
            "test_bind_unused",
            "bind is not used by the recovery path",
        ))
    }

    fn association(
        &self,
        _actor: &AuthContext,
        snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextOwnerBinding, ManagementError> {
        Ok(binding_for(
            &snapshot.workflow_run_id,
            self.receipt_recovery,
        ))
    }

    fn control_receipt(
        &self,
        _actor: &AuthContext,
        binding: &ContextOwnerBinding,
        _command: &ContextControlCommand,
    ) -> Result<Option<ContextControlReceipt>, ManagementError> {
        match self.outcome {
            Outcome::Absent => Ok(None),
            Outcome::Recorded => Ok(Some(pause_receipt(binding))),
            Outcome::Tampered => {
                let mut receipt = pause_receipt(binding);
                receipt.binding_digest = "0".repeat(64);
                Ok(Some(receipt))
            }
        }
    }

    fn is_available(&self) -> bool {
        true
    }
}

fn service_with_owner(owner: RecoveringOwner) -> Arc<ManagementService> {
    Arc::new(
        synthetic_store(Arc::new(MemoryWorkflowStore::new()))
            .with_context_owner_port(Arc::new(owner)),
    )
}

fn run_id(service: &ManagementService) -> String {
    service
        .submit_run(&actor(), request())
        .expect("run is admitted")
        .workflow_run_id
}

fn post(
    service: &Arc<ManagementService>,
    token: &str,
    authenticator: StaticAuthenticator,
    path: &str,
    body: &[u8],
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server starts");
    let client = ManagementClient::new(server.address(), token).expect("client");
    let response = client
        .request_json("POST", path, Some(body))
        .expect("response");
    server.shutdown().expect("server shuts down");
    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    (response.status, value)
}

fn owner_authenticator() -> StaticAuthenticator {
    StaticAuthenticator::single("owner-token", actor()).expect("authenticator")
}

fn lookup_path(run_id: &str) -> String {
    format!("/v1/workflow-runs/{run_id}/context-control-receipts/lookup")
}

#[test]
fn recorded_control_receipt_is_recovered_for_its_own_subject() {
    let service = service_with_owner(RecoveringOwner {
        receipt_recovery: true,
        outcome: Outcome::Recorded,
    });
    let run = run_id(&service);
    let body = serde_json::to_vec(&pause_command()).expect("command");
    let (status, value) = post(
        &service,
        "owner-token",
        owner_authenticator(),
        &lookup_path(&run),
        &body,
    );
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value.get("schema_version").and_then(Value::as_str),
        Some(CONTEXT_OWNER_RECEIPT_SCHEMA_VERSION)
    );
    assert_eq!(value.get("command").and_then(Value::as_str), Some("pause"));
    assert_eq!(
        value.get("effect").and_then(Value::as_str),
        Some("pause_requested")
    );
    assert_eq!(
        value.get("idempotency_key").and_then(Value::as_str),
        Some("control.pause.key.1")
    );
    assert_eq!(
        value.get("control_version").and_then(Value::as_u64),
        Some(2)
    );
}

#[test]
fn owner_without_receipt_recovery_capability_is_unsupported() {
    let service = service_with_owner(RecoveringOwner {
        receipt_recovery: false,
        outcome: Outcome::Recorded,
    });
    let run = run_id(&service);
    let body = serde_json::to_vec(&pause_command()).expect("command");
    let (status, value) = post(
        &service,
        "owner-token",
        owner_authenticator(),
        &lookup_path(&run),
        &body,
    );
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_control_receipt_recovery_unsupported")
    );
}

#[test]
fn unrecorded_command_is_not_reported_as_a_recovered_effect() {
    let service = service_with_owner(RecoveringOwner {
        receipt_recovery: true,
        outcome: Outcome::Absent,
    });
    let run = run_id(&service);
    let body = serde_json::to_vec(&pause_command()).expect("command");
    let (status, value) = post(
        &service,
        "owner-token",
        owner_authenticator(),
        &lookup_path(&run),
        &body,
    );
    assert_eq!(status, 404, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_control_receipt_not_recorded")
    );
}

#[test]
fn a_receipt_for_another_binding_is_rejected_not_returned() {
    let service = service_with_owner(RecoveringOwner {
        receipt_recovery: true,
        outcome: Outcome::Tampered,
    });
    let run = run_id(&service);
    let body = serde_json::to_vec(&pause_command()).expect("command");
    let (status, value) = post(
        &service,
        "owner-token",
        owner_authenticator(),
        &lookup_path(&run),
        &body,
    );
    assert_eq!(status, 409, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_control_receipt_mismatch")
    );
}

#[test]
fn recovery_requires_the_scoped_workflow_read_grant() {
    let service = service_with_owner(RecoveringOwner {
        receipt_recovery: true,
        outcome: Outcome::Recorded,
    });
    let run = run_id(&service);
    let authenticator = StaticAuthenticator::single(
        "limited-token",
        AuthContext::new("integration.tester", ["context:read".to_owned()]).expect("actor"),
    )
    .expect("authenticator");
    let body = serde_json::to_vec(&pause_command()).expect("command");
    let (status, value) = post(
        &service,
        "limited-token",
        authenticator,
        &lookup_path(&run),
        &body,
    );
    assert_eq!(status, 403, "body: {value}");
}

#[test]
fn an_unattached_owner_reports_unavailable_rather_than_no_receipt() {
    let service = Arc::new(synthetic_store(Arc::new(MemoryWorkflowStore::new())));
    let run = run_id(&service);
    let body = serde_json::to_vec(&pause_command()).expect("command");
    let (status, value) = post(
        &service,
        "owner-token",
        owner_authenticator(),
        &lookup_path(&run),
        &body,
    );
    assert_eq!(status, 503, "body: {value}");
    assert_eq!(
        value.pointer("/error/code").and_then(Value::as_str),
        Some("context_owner_association_unavailable")
    );
}
