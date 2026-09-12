// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::{Value, json};
use sts2_harness::management::{
    AuthContext, CommandKind, CommandParameters, CommandRequest, ContextAssociationContext,
    ContextAvailability, ContextCaptureEvidence, ContextCaptureMode, ContextCaptureState,
    ContextInspectionCapabilities, ContextInspectionPort, ContextInspectionResult,
    MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementError, ManagementServer,
    MemoryWorkflowStore, RunRequest, ServerConfig, StaticAuthenticator, ValidateRequest,
    synthetic_store,
};

const VALID_WORKFLOW: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn actor() -> AuthContext {
    AuthContext::new("integration.tester", ["workflow:*".to_owned()]).expect("valid actor")
}

fn request() -> RunRequest {
    RunRequest {
        schema_version: "ascension.management/v1".to_owned(),
        request_id: "context-association-request".to_owned(),
        definition: Some(serde_json::from_slice::<Value>(VALID_WORKFLOW).expect("valid fixture")),
        artifact_id: None,
        instance_id: "integration-instance".to_owned(),
        profile: "synthetic".to_owned(),
    }
}

#[test]
fn synthetic_owner_exposes_a_scoped_unavailable_context_association() {
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()));
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request())
        .expect("run is admitted");

    let association = service
        .context_association(&actor, &submitted.workflow_run_id)
        .expect("context association is returned");

    assert_eq!(
        association.schema_version,
        "ascension.workflow-context-association/v1"
    );
    assert_eq!(
        association.workflow.workflow_run_id,
        submitted.workflow_run_id
    );
    assert_eq!(
        association.context.availability,
        ContextAvailability::Unavailable
    );
    assert!(association.context.context_ref.is_none());
    assert!(association.context.snapshot_id.is_none());
    assert_eq!(association.capture.state, ContextCaptureState::Unavailable);
    assert!(association.capabilities.inspect_metadata);
    assert!(!association.capabilities.control_context);
}

struct IncompleteAvailableContext;

impl ContextInspectionPort for IncompleteAvailableContext {
    fn inspect(
        &self,
        _actor: &AuthContext,
        _snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ContextInspectionResult, ManagementError> {
        Ok(ContextInspectionResult {
            context: ContextAssociationContext {
                availability: ContextAvailability::Available,
                context_ref: Some("context.fixture".to_owned()),
                run_id: None,
                episode_id: None,
                agent_id: None,
                snapshot_id: None,
                approved_revision_id: None,
                plan_epoch: None,
                reason_code: None,
            },
            capture: ContextCaptureEvidence {
                mode: ContextCaptureMode::Metadata,
                state: ContextCaptureState::Prepared,
                attempt_id: Some("attempt.fixture".to_owned()),
                reason_code: None,
            },
            capabilities: ContextInspectionCapabilities::default(),
        })
    }
}

#[test]
fn owner_rejects_an_available_context_with_missing_bound_identities() {
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()))
        .with_context_inspection_port(Arc::new(IncompleteAvailableContext));
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request())
        .expect("run is admitted");

    let error = service
        .context_association(&actor, &submitted.workflow_run_id)
        .expect_err("partial association is rejected");
    assert_eq!(error.code, "context_association_incomplete");
}

#[test]
fn http_route_returns_only_the_scoped_context_metadata() {
    let service = Arc::new(synthetic_store(Arc::new(MemoryWorkflowStore::new())));
    let actor = actor();
    let submitted = service
        .submit_run(&actor, request())
        .expect("run is admitted");
    let authenticator =
        StaticAuthenticator::single("context-test-token", actor).expect("authenticator");
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback address"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, Arc::clone(&service)).expect("server starts");
    let client = ManagementClient::new(server.address(), "context-test-token").expect("client");
    let response = client
        .request_json(
            "GET",
            &format!("/v1/workflow-runs/{}/context", submitted.workflow_run_id),
            None,
        )
        .expect("context response");
    server.shutdown().expect("server shuts down");

    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    assert_eq!(response.status, 200);
    assert_eq!(
        value.get("schema_version").and_then(Value::as_str),
        Some("ascension.workflow-context-association/v1")
    );
    assert!(
        value
            .pointer("/context/snapshot_id")
            .is_some_and(Value::is_null)
    );
    assert!(
        value
            .pointer("/capture/attempt_id")
            .is_some_and(Value::is_null)
    );
}

#[test]
fn disclosed_context_binding_catalog_rejects_unresolved_and_incompatible_refs() {
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()));
    let actor = actor();
    let definition: Value = serde_json::from_slice(VALID_WORKFLOW).expect("valid fixture");
    let validate = |definition: Value, bindings: Value| {
        service
            .validate(
                &actor,
                ValidateRequest {
                    schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
                    definition,
                    capabilities: json!({
                        "capabilities": [
                            "observe.fair-play.v1",
                            "actions.catalog.v1",
                            "actions.settlement.v1"
                        ],
                        "context_bindings": bindings
                    }),
                },
            )
            .expect("validation is a read-only response")
    };

    let valid = validate(
        definition.clone(),
        json!([{"context_ref": "context.synthetic.v1", "node_kinds": ["decide"]}]),
    );
    assert!(valid.valid);

    let unresolved = validate(definition.clone(), json!([]));
    assert!(!unresolved.valid);
    assert!(
        unresolved
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "context_ref_unresolved")
    );

    let incompatible = validate(
        definition,
        json!([{"context_ref": "context.synthetic.v1", "node_kinds": ["analyze"]}]),
    );
    assert!(!incompatible.valid);
    assert!(
        incompatible
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "context_ref_incompatible")
    );
}

#[test]
fn synthetic_workflow_pause_and_resume_are_fenced_by_harness_context_authority() {
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()));
    let actor = actor();
    let submitted = service.submit_run(&actor, request()).expect("run admitted");
    let command = |id: &str, revision: u64, kind: CommandKind| CommandRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        command_id: id.to_owned(),
        run_id: submitted.workflow_run_id.clone(),
        expected_revision: revision,
        actor_scope: "integration.tester".to_owned(),
        kind,
        parameters: CommandParameters::default(),
    };
    let paused = service
        .command(&actor, command("context-gate-pause", 1, CommandKind::Pause))
        .expect("context-authority pause succeeds");
    assert_eq!(paused.run_revision, 2);
    let duplicate_pause = service
        .command(
            &actor,
            command("context-gate-pause-second", 2, CommandKind::Pause),
        )
        .expect_err("authority rejects a second pause while latched");
    assert_eq!(duplicate_pause.code, "context_control_conflict");
    let resumed = service
        .command(
            &actor,
            command("context-gate-resume", 2, CommandKind::Resume),
        )
        .expect("context-authority resume succeeds");
    assert_eq!(resumed.run_revision, 3);
}
