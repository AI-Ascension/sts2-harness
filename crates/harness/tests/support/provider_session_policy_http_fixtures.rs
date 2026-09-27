// SPDX-License-Identifier: MIT

//! Authenticated, redacted provider-session policy owner projection.

// Everything below is `pub(super)` so the split test crates can reach it with `use fixtures::*`:
// the helpers, the constants, and the crate paths those helpers name. A module's own `use` is
// private, so without this the tests would compile with the helpers in scope but not with the
// types they mention. Refs sts2-harness#564.
pub(super) use std::sync::Arc;

pub(super) use serde_json::Value;
pub(super) use sts2_harness::management::MemoryWorkflowStore;
// The split files use different subsets of these, so the union is re-exported and the unused
// remainder is allowed: a per-file import list would have to be kept in step with both.
#[allow(unused_imports)]
pub(super) use sts2_harness::management::{
    AuthContext, DurableProviderSessionPolicyCommandPort, LiveProviderPolicyPort,
    MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementServer, ManagementService,
    PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION, ProviderSessionPolicyOwnerPort, RunRequest,
    ServerConfig, StaticAuthenticator, synthetic_store,
};
pub(super) use sts2_harness::provider_session::{
    MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS, NativeCapabilities, ProviderSessionMetadataStore,
    ProviderSessionMode, ProviderSessionPolicy, ProviderSessionPolicyOwner, SessionScope,
};
#[allow(unused_imports)]
pub(super) use sts2_harness::workflow::WorkflowDefinition;

pub(super) const VALID_WORKFLOW: &[u8] =
    include_bytes!("../../../../conformance/workflow-v1/valid-strict.json");
pub(super) const RUN_ID: &str = "run.synthetic.523ea08ab60d2c1fd48285b1b006de10";

pub(super) fn scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run.synthetic.523ea08ab60d2c1fd48285b1b006de10",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

pub(super) fn create_private_test_directory(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;

        let mut builder = std::fs::DirBuilder::new();
        builder.mode(0o700).create(path)
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir(path)
    }
}

pub(super) fn policy() -> ProviderSessionPolicy {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    policy.max_completed_turns = MAX_COMPLETED_TURNS;
    policy.history_ttl_seconds = MAX_HISTORY_TTL_SECONDS;
    policy
}

pub(super) fn over_limit_policy() -> ProviderSessionPolicy {
    let mut value = policy();
    value.version = 2;
    value.epoch = 2;
    value.max_completed_turns = MAX_COMPLETED_TURNS + 1;
    value
}

pub(super) fn actor() -> AuthContext {
    AuthContext::new("policy-reader", ["workflow:read".to_owned()]).expect("actor")
}

pub(super) fn submitter() -> AuthContext {
    AuthContext::new(
        "policy-owner",
        [
            "workflow:control".to_owned(),
            "workflow:content:write".to_owned(),
        ],
    )
    .expect("submitter")
}

pub(super) fn run_request() -> RunRequest {
    RunRequest {
        schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
        request_id: "run-fixture".to_owned(),
        definition: Some(serde_json::from_slice(VALID_WORKFLOW).expect("workflow")),
        artifact_id: None,
        instance_id: "fixture-instance".to_owned(),
        profile: "synthetic".to_owned(),
        admission: None,
    }
}

pub(super) fn service_and_path() -> (Arc<ManagementService>, std::path::PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "sts2-provider-policy-http-{}",
        uuid::Uuid::new_v4()
    ));
    create_private_test_directory(&directory).expect("private directory");
    let path = directory.join("owner.bin");
    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let owner = Arc::new(
        ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
            .expect("owner"),
    );
    let bytes = serde_json::to_vec(&policy()).expect("policy bytes");
    let digest = owner.import(bytes).expect("import");
    assert_eq!(owner.metadata().expect("metadata").revision, 2);
    assert_eq!(
        owner
            .import(serde_json::to_vec(&policy()).expect("same policy"))
            .expect("idempotent import"),
        digest
    );
    assert_eq!(owner.metadata().expect("metadata").revision, 2);
    owner.adopt_imported(&digest, 2).expect("adopt");
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()))
        .with_provider_session_policy_command_port(Arc::new(
            DurableProviderSessionPolicyCommandPort::new(owner),
        ));
    let run = service
        .submit_run(&submitter(), run_request())
        .expect("run");
    assert_eq!(
        run.workflow_run_id,
        "run.synthetic.523ea08ab60d2c1fd48285b1b006de10"
    );
    (Arc::new(service), path)
}

pub(super) fn fresh_service_and_path() -> (Arc<ManagementService>, std::path::PathBuf) {
    let directory = std::env::temp_dir().join(format!(
        "sts2-provider-policy-http-fresh-{}",
        uuid::Uuid::new_v4()
    ));
    create_private_test_directory(&directory).expect("private directory");
    let path = directory.join("owner.bin");
    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let owner = Arc::new(
        ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
            .expect("owner"),
    );
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()))
        .with_provider_session_policy_command_port(Arc::new(
            DurableProviderSessionPolicyCommandPort::new(owner),
        ));
    service
        .submit_run(&submitter(), run_request())
        .expect("run");
    (Arc::new(service), path)
}

pub(super) fn get(
    service: &Arc<ManagementService>,
    authenticator: StaticAuthenticator,
) -> (u16, Value) {
    http_request(
        service,
        authenticator,
        "GET",
        &format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy"),
        None,
    )
}

pub(super) fn http_request(
    service: &Arc<ManagementService>,
    authenticator: StaticAuthenticator,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
) -> (u16, Value) {
    let config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("loopback"),
        Arc::new(authenticator),
    )
    .expect("config");
    let server = ManagementServer::start(config, Arc::clone(service)).expect("server");
    let client = ManagementClient::new(server.address(), "token").expect("client");
    let response = client.request_json(method, path, body).expect("response");
    server.shutdown().expect("shutdown");
    (
        response.status,
        serde_json::from_slice(&response.body).expect("json"),
    )
}
