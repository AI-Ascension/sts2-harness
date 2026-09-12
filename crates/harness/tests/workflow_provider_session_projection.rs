// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::sync::{Arc, Mutex};

use serde_json::Value;
use sts2_harness::management::{
    AuthContext, ManagementClient, ManagementError, ManagementServer, MemoryWorkflowStore,
    ProviderSessionBindingSummary, ProviderSessionBrokerInspectionPort,
    ProviderSessionInspectionPort, ProviderSessionInspectionResult,
    ProviderSessionOperationSummary, RunRequest, ServerConfig, StaticAuthenticator,
    synthetic_store,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionBroker, ProviderSessionMode, ProviderSessionPolicy,
    SessionScope,
};
use sts2_harness::sha256_hex;

const VALID_WORKFLOW: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn actor() -> AuthContext {
    AuthContext::new("integration.tester", ["workflow:*".to_owned()]).expect("valid actor")
}

fn request() -> RunRequest {
    RunRequest {
        schema_version: "ascension.management/v1".to_owned(),
        request_id: "provider-session-projection-request".to_owned(),
        definition: Some(serde_json::from_slice(VALID_WORKFLOW).expect("valid fixture")),
        artifact_id: None,
        instance_id: "integration-instance".to_owned(),
        profile: "synthetic".to_owned(),
    }
}

struct SessionInspectionDouble {
    run_id: String,
    effectful: bool,
}

impl ProviderSessionInspectionPort for SessionInspectionDouble {
    fn list(
        &self,
        _actor: &AuthContext,
        _snapshot: &sts2_harness::management::RunSnapshot,
    ) -> Result<ProviderSessionInspectionResult, ManagementError> {
        Ok(ProviderSessionInspectionResult {
            workflow_run_id: self.run_id.clone(),
            bindings: vec![ProviderSessionBindingSummary {
                binding_id: "binding.fixture.1".to_owned(),
                state: "held".to_owned(),
                history_coverage: "unknown".to_owned(),
                game_dispatch_capability: false,
            }],
            operations: vec![ProviderSessionOperationSummary {
                operation_id: "operation.fixture.1".to_owned(),
                state: "intent_persisted".to_owned(),
                game_effects: u64::from(self.effectful),
                auto_resume: self.effectful,
            }],
            next_cursor: None,
        })
    }
}

#[test]
fn projection_is_scoped_and_metadata_only() {
    let actor = actor();
    let store = Arc::new(MemoryWorkflowStore::new());
    let submitted = synthetic_store(store.clone())
        .submit_run(&actor, request())
        .expect("run is admitted");
    let service = synthetic_store(store).with_provider_session_inspection_port(Arc::new(
        SessionInspectionDouble {
            run_id: submitted.workflow_run_id.clone(),
            effectful: false,
        },
    ));
    let projection = service
        .provider_sessions(&actor, &submitted.workflow_run_id)
        .expect("scoped metadata is returned");
    assert_eq!(
        projection.schema,
        "ascension.provider-session.api-result.v1"
    );
    assert_eq!(projection.operation, "list");
    assert_eq!(projection.value.run_id, submitted.workflow_run_id);
    assert_eq!(projection.value.bindings.len(), 1);
    assert_eq!(projection.value.operations.len(), 1);
    assert_eq!(projection.effect_class, "local_metadata_only");
    assert_eq!(projection.inference_calls, 0);
    assert_eq!(projection.game_effects, 0);
}

#[test]
fn projection_rejects_cross_run_or_effectful_adapter_results() {
    let actor = actor();
    let store = Arc::new(MemoryWorkflowStore::new());
    let submitted = synthetic_store(store.clone())
        .submit_run(&actor, request())
        .expect("run is admitted");
    let mismatch = synthetic_store(store.clone()).with_provider_session_inspection_port(Arc::new(
        SessionInspectionDouble {
            run_id: "different.workflow.run".to_owned(),
            effectful: false,
        },
    ));
    assert_eq!(
        mismatch
            .provider_sessions(&actor, &submitted.workflow_run_id)
            .expect_err("cross-run result is rejected")
            .code,
        "provider_session_run_mismatch"
    );
    let effectful = synthetic_store(store).with_provider_session_inspection_port(Arc::new(
        SessionInspectionDouble {
            run_id: submitted.workflow_run_id.clone(),
            effectful: true,
        },
    ));
    assert_eq!(
        effectful
            .provider_sessions(&actor, &submitted.workflow_run_id)
            .expect_err("effectful result is rejected")
            .code,
        "provider_session_projection_effectful"
    );
}

#[test]
fn http_route_preserves_workflow_scope_and_redacts_native_fields() {
    let actor = actor();
    let store = Arc::new(MemoryWorkflowStore::new());
    let submitted = synthetic_store(store.clone())
        .submit_run(&actor, request())
        .expect("run is admitted");
    let service = Arc::new(
        synthetic_store(store).with_provider_session_inspection_port(Arc::new(
            SessionInspectionDouble {
                run_id: submitted.workflow_run_id.clone(),
                effectful: false,
            },
        )),
    );
    let authenticator =
        StaticAuthenticator::single("session-test-token", actor).expect("authenticator");
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback address"),
        Arc::new(authenticator),
    )
    .expect("server config");
    let server = ManagementServer::start(config, service).expect("server starts");
    let client = ManagementClient::new(server.address(), "session-test-token").expect("client");
    let response = client
        .request_json(
            "GET",
            &format!(
                "/v1/workflow-runs/{}/provider-sessions",
                submitted.workflow_run_id
            ),
            None,
        )
        .expect("provider-session response");
    server.shutdown().expect("server shuts down");
    let value: Value = serde_json::from_slice(&response.body).expect("JSON response");
    assert_eq!(response.status, 200);
    assert_eq!(
        value.pointer("/value/run_id").and_then(Value::as_str),
        Some(submitted.workflow_run_id.as_str())
    );
    assert!(
        value
            .pointer("/value/bindings/0/native_thread_ref")
            .is_none()
    );
    assert!(
        value
            .pointer("/value/operations/0/request_sha256")
            .is_none()
    );
}

#[test]
fn broker_projection_requires_an_explicit_workflow_mapping() {
    let actor = actor();
    let store = Arc::new(MemoryWorkflowStore::new());
    let submitted = synthetic_store(store.clone())
        .submit_run(&actor, request())
        .expect("run is admitted");
    let scope = SessionScope::new(
        "context.project.1",
        "context.run.1",
        "context.episode.1",
        "context.agent.1",
    )
    .expect("valid independent provider-session scope");
    let mut policy = ProviderSessionPolicy::disabled(scope.clone());
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture.realm.1".to_owned();
    policy.profile_sha256 = sha256_hex("codex-app-server-fixture-v1");
    let broker = ProviderSessionBroker::new(
        scope,
        policy,
        NativeCapabilities::fixture(),
        "owner.fixture.1",
    )
    .expect("fixture broker");
    let port = ProviderSessionBrokerInspectionPort::new(vec![(
        submitted.workflow_run_id.clone(),
        Arc::new(Mutex::new(broker)),
    )])
    .expect("explicit mapping");
    let service = synthetic_store(store).with_provider_session_inspection_port(Arc::new(port));
    let projection = service
        .provider_sessions(&actor, &submitted.workflow_run_id)
        .expect("mapped broker projection");
    assert_eq!(projection.value.run_id, submitted.workflow_run_id);
    assert!(projection.value.bindings.is_empty());
    assert!(projection.value.operations.is_empty());
    assert_eq!(projection.effect_class, "local_metadata_only");
}
