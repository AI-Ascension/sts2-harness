// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::management::workflow_ports::SyntheticDefinitionPort;
use crate::management::{
    CapabilityPort, DefinitionPort, DiffResult, InspectionResult, TargetCatalogResponse,
    ValidationResult,
};
use serde_json::{Value, json};

struct StalledRead(Arc<Stall>);

impl DefinitionPort for StalledRead {
    fn validate(
        &self,
        definition: &Value,
        capabilities: &Value,
    ) -> Result<ValidationResult, ManagementError> {
        self.0.wait();
        SyntheticDefinitionPort.validate(definition, capabilities)
    }
    fn inspect(&self, definition: &Value) -> Result<InspectionResult, ManagementError> {
        self.0.wait();
        SyntheticDefinitionPort.inspect(definition)
    }
    fn diff(&self, before: &Value, after: &Value) -> Result<DiffResult, ManagementError> {
        self.0.wait();
        SyntheticDefinitionPort.diff(before, after)
    }
}

impl CapabilityPort for StalledRead {
    fn capabilities(&self) -> Result<Value, ManagementError> {
        self.0.wait();
        Ok(json!({"capabilities": []}))
    }
    fn target_catalog(&self, _: &AuthContext) -> Result<TargetCatalogResponse, ManagementError> {
        self.0.wait();
        Ok(TargetCatalogResponse {
            schema_version: "ascension.workflow-targets/v1".to_owned(),
            catalog_revision: "fixture-revision".to_owned(),
            targets: vec![],
        })
    }
}

#[test]
fn stalled_read_only_post_handlers_and_get_report_unavailable_without_write_uncertainty() {
    let definition: Value = serde_json::from_slice(include_bytes!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))
    .expect("definition");
    let schema = "ascension.management/v1";
    let cases = [
        (
            "POST",
            "/v1/workflow-definitions/validate",
            Some(json!({
                "schema_version": schema, "definition": definition, "capabilities": {"capabilities": []},
            })),
        ),
        (
            "POST",
            "/v1/workflow-definitions/inspect",
            Some(json!({
                "schema_version": schema, "definition": definition, "format": "json",
            })),
        ),
        (
            "POST",
            "/v1/workflow-definitions/diff",
            Some(json!({
                "schema_version": schema, "old_definition": definition, "new_definition": definition,
                "format": "json",
            })),
        ),
        (
            "POST",
            "/v1/workflow-targets/preflight",
            Some(json!({
                "schema_version": "ascension.workflow-admission/v1", "request_id": "preflight-request",
                "workflow_definition_digest": "a".repeat(64),
                "target": {"instance_id": "fixture-instance", "execution_profile": "synthetic",
                    "execution_mode": "synthetic", "workflow_revision": "1",
                    "compatibility_revision": "fixture-compatibility", "capability_revision": "fixture-capability",
                    "game_profile": "fixture-game", "save_profile": null, "inference_profile": null,
                    "context_capability": null, "provider_capability": null},
            })),
        ),
        ("GET", "/v1/capabilities", None),
    ];
    for (method, path, body) in cases {
        let stall = ReleaseOnDrop(Arc::new(Stall::default()));
        let port = Arc::new(StalledRead(Arc::clone(&stall.0)));
        let actor = AuthContext::new("read-owner", ["workflow:*".to_owned()]).expect("actor");
        let auth = StaticAuthenticator::single("read-token", actor.clone()).expect("auth");
        // No execution or provider port is installed. These actual routes reach only their
        // intentionally stalled definition/catalog port, then terminate after explicit release.
        let service = Arc::new(
            ManagementService::new(Arc::new(MemoryWorkflowStore::new()))
                .with_definition_port(port.clone())
                .with_capability_port(port),
        );
        let mut config = ServerConfig::new("127.0.0.1:0".parse().expect("address"), Arc::new(auth))
            .expect("config");
        config.limits.dispatch_deadline = Duration::from_millis(100);
        let server = ManagementServer::start(config, Arc::clone(&service)).expect("server");
        let client = ManagementClient::new(server.address(), "read-token").expect("client");
        let bytes = body.map(|value| serde_json::to_vec(&value).expect("body"));
        let response = client
            .request_json(method, path, bytes.as_deref())
            .expect("bounded response");
        assert_eq!(response.status, 503, "{path}");
        let value = json(response);
        assert_eq!(value["schema_version"], schema, "{path}");
        assert_eq!(value["error"]["class"], "unavailable", "{path}");
        assert_eq!(
            value["error"]["code"], "dispatch_deadline_exceeded",
            "{path}"
        );
        assert_eq!(
            stall.0.entered.load(Ordering::SeqCst),
            1,
            "actual port stalled for {path}"
        );
        assert_eq!(server.pending_dispatches().expect("held capacity"), 1);
        assert_eq!(
            service
                .submission_recovery(&actor, "preflight-request")
                .expect("read claim")
                .state,
            SubmissionRecoveryState::Missing,
            "read-only request never claimed a mutation"
        );
        stall.0.release();
        server.shutdown().expect("released read handler joins");
        assert_eq!(server.pending_dispatches().expect("joined capacity"), 0);
    }
}
