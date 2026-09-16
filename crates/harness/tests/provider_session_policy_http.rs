// SPDX-License-Identifier: MIT

//! Authenticated, redacted provider-session policy owner projection.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use serde_json::Value;
use sts2_harness::management::MemoryWorkflowStore;
use sts2_harness::management::{
    AuthContext, DurableProviderSessionPolicyCommandPort, LiveProviderPolicyPort,
    MANAGEMENT_SCHEMA_VERSION, ManagementClient, ManagementServer, ManagementService,
    PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION, ProviderSessionPolicyOwnerPort, RunRequest,
    ServerConfig, StaticAuthenticator, synthetic_store,
};
use sts2_harness::provider_session::{
    MAX_COMPLETED_TURNS, MAX_HISTORY_TTL_SECONDS, NativeCapabilities, ProviderSessionMetadataStore,
    ProviderSessionMode, ProviderSessionPolicy, ProviderSessionPolicyOwner, SessionScope,
};
use sts2_harness::workflow::WorkflowDefinition;

const VALID_WORKFLOW: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");
const RUN_ID: &str = "run.synthetic.523ea08ab60d2c1fd48285b1b006de10";

fn scope() -> SessionScope {
    SessionScope::new(
        "project-fixture",
        "run.synthetic.523ea08ab60d2c1fd48285b1b006de10",
        "episode-fixture",
        "agent-fixture",
    )
    .expect("scope")
}

fn create_private_test_directory(path: &std::path::Path) -> std::io::Result<()> {
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

fn policy() -> ProviderSessionPolicy {
    let scope = scope();
    let mut policy = ProviderSessionPolicy::disabled(scope);
    policy.mode = ProviderSessionMode::FixtureOnly;
    policy.credential_realm_ref = "fixture-realm".to_owned();
    policy.profile_sha256 = sts2_harness::sha256_hex("codex-app-server-fixture-v1");
    policy.max_completed_turns = MAX_COMPLETED_TURNS;
    policy.history_ttl_seconds = MAX_HISTORY_TTL_SECONDS;
    policy
}

fn over_limit_policy() -> ProviderSessionPolicy {
    let mut value = policy();
    value.version = 2;
    value.epoch = 2;
    value.max_completed_turns = MAX_COMPLETED_TURNS + 1;
    value
}

fn actor() -> AuthContext {
    AuthContext::new("policy-reader", ["workflow:read".to_owned()]).expect("actor")
}

fn submitter() -> AuthContext {
    AuthContext::new(
        "policy-owner",
        [
            "workflow:control".to_owned(),
            "workflow:content:write".to_owned(),
        ],
    )
    .expect("submitter")
}

fn run_request() -> RunRequest {
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

fn service_and_path() -> (Arc<ManagementService>, std::path::PathBuf) {
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

fn fresh_service_and_path() -> (Arc<ManagementService>, std::path::PathBuf) {
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

fn get(service: &Arc<ManagementService>, authenticator: StaticAuthenticator) -> (u16, Value) {
    http_request(
        service,
        authenticator,
        "GET",
        &format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy"),
        None,
    )
}

fn http_request(
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

#[test]
fn policy_route_returns_redacted_current_binding_and_history() {
    let (service, path) = service_and_path();
    let auth = StaticAuthenticator::single("token", actor()).expect("auth");
    let (status, value) = get(&service, auth);
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(
        value["schema_version"],
        "ascension.provider-session.policy-owner-view.v1"
    );
    assert_eq!(value["value"]["run_id"], RUN_ID);
    assert_eq!(value["value"]["revision"], 3);
    assert_eq!(value["value"]["history"][0]["active"], true);
    assert_eq!(value["value"]["active"]["mode"], "fixture_only");
    assert!(
        value["value"]["active"]
            .get("credential_realm_ref")
            .is_none()
    );
    assert!(value["value"]["active"].get("policy_bytes").is_none());
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn policy_route_rejects_foreign_run_scope_and_missing_grant() {
    let (service, path) = service_and_path();
    let foreign = AuthContext::with_run_prefix(
        "foreign",
        ["workflow:read".to_owned()],
        Some("another-run".to_owned()),
    )
    .expect("foreign");
    let no_grant = AuthContext::new("no-grant", std::iter::empty::<String>()).expect("grant");
    let foreign_auth = StaticAuthenticator::single("token", foreign).expect("auth");
    let (status, value) = get(&service, foreign_auth);
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(value["error"]["code"], "run_scope_denied");
    let no_grant_auth = StaticAuthenticator::single("token", no_grant).expect("auth");
    let (status, value) = get(&service, no_grant_auth);
    assert_eq!(status, 403, "body: {value}");
    assert_eq!(value["error"]["code"], "missing_scope");
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn reopened_encrypted_owner_preserves_route_metadata() {
    let (service, path) = service_and_path();
    drop(service);
    let reopened_store =
        ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let reopened = Arc::new(
        ProviderSessionPolicyOwner::open(reopened_store, scope(), NativeCapabilities::fixture())
            .expect("reopen"),
    );
    let service = synthetic_store(Arc::new(MemoryWorkflowStore::new()))
        .with_provider_session_policy_command_port(Arc::new(
            DurableProviderSessionPolicyCommandPort::new(reopened),
        ));
    service
        .submit_run(&submitter(), run_request())
        .expect("run");
    let auth = StaticAuthenticator::single("token", actor()).expect("auth");
    let (status, value) = get(&Arc::new(service), auth);
    assert_eq!(status, 200, "body: {value}");
    assert_eq!(value["value"]["revision"], 3);
    assert_eq!(
        value["value"]["history"][0]["sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn reopened_owner_preserves_mutable_migration_history() {
    let (service, path) = service_and_path();
    drop(service);
    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let owner = ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
        .expect("owner");
    let mut source = policy();
    source.max_completed_turns = MAX_COMPLETED_TURNS + 1;
    source.version = 2;
    source.epoch = 2;
    let source_bytes = serde_json::to_vec(&source).expect("source");
    let source_sha256 = owner.import(source_bytes).expect("source import");
    let mut target = source.clone();
    target.version = 3;
    target.epoch = 3;
    target.max_completed_turns = MAX_COMPLETED_TURNS;
    let target_bytes = serde_json::to_vec(&target).expect("target");
    let revision = owner.metadata().expect("metadata").revision;
    let proposal_sha256 = owner
        .propose("proposal-one", &source_sha256, target_bytes, revision)
        .expect("proposal");
    owner
        .approve("proposal-one", &proposal_sha256, "approval-one")
        .expect("approval");
    owner
        .adopt(
            "proposal-one",
            &proposal_sha256,
            "approval-one",
            revision + 2,
        )
        .expect("adoption");
    drop(owner);

    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let reopened = ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
        .expect("reopen after adopted proposal");
    let metadata = reopened.metadata().expect("metadata");
    assert_eq!(metadata.proposals.len(), 1);
    assert!(metadata.proposals[0].approval_recorded);
    assert_eq!(
        metadata.proposals[0].adopted_policy_sha256.as_deref(),
        Some(metadata.active.as_ref().expect("active").sha256.as_str())
    );
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn pretty_printed_target_adopts_and_reopens_with_exact_identity() {
    let (service, path) = service_and_path();
    drop(service);

    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let owner = Arc::new(
        ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
            .expect("owner"),
    );
    let mut source = over_limit_policy();
    source.version = 2;
    source.epoch = 2;
    let source_bytes = serde_json::to_vec_pretty(&source).expect("source");
    let source_sha256 = owner.import(source_bytes).expect("source import");

    let mut target = source;
    target.version = 3;
    target.epoch = 3;
    target.max_completed_turns = MAX_COMPLETED_TURNS;
    let target_bytes = serde_json::to_vec_pretty(&target).expect("pretty target");
    let target_sha256 = sts2_harness::sha256_hex(&target_bytes);
    let revision = owner.metadata().expect("metadata").revision;
    let proposal_sha256 = owner
        .propose(
            "proposal-pretty-target",
            &source_sha256,
            target_bytes,
            revision,
        )
        .expect("proposal");
    owner
        .approve(
            "proposal-pretty-target",
            &proposal_sha256,
            "approval-pretty-target",
        )
        .expect("approval");
    owner
        .adopt(
            "proposal-pretty-target",
            &proposal_sha256,
            "approval-pretty-target",
            revision + 2,
        )
        .expect("adoption");
    let metadata = owner.metadata().expect("adopted metadata");
    assert_eq!(
        metadata.proposals[0].adopted_policy_sha256.as_deref(),
        Some(target_sha256.as_str())
    );
    assert_eq!(
        metadata.active.as_ref().expect("active").sha256,
        target_sha256
    );
    drop(owner);

    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let reopened = ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
        .expect("reopen exact-byte adoption");
    let (_, active_sha256, _) = reopened.active().expect("active after reopen");
    assert_eq!(active_sha256, target_sha256);
    let metadata = reopened.metadata().expect("reopened metadata");
    assert_eq!(
        metadata.proposals[0].adopted_policy_sha256.as_deref(),
        Some(target_sha256.as_str())
    );
    assert_eq!(
        metadata.active.as_ref().expect("active").sha256,
        target_sha256
    );
    drop(reopened);
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn live_policy_uses_admitted_workflow_run_id_not_submission_request_id() {
    let (service, path) = fresh_service_and_path();
    drop(service);

    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let owner = Arc::new(
        ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture())
            .expect("owner"),
    );
    let bytes = serde_json::to_vec(&policy()).expect("policy");
    let sha256 = owner.import(bytes).expect("import");
    owner.adopt_imported(&sha256, 2).expect("adopt");

    let port = ProviderSessionPolicyOwnerPort::new(Arc::clone(&owner));
    let request = run_request();
    assert_ne!(request.request_id, RUN_ID);
    let definition: WorkflowDefinition =
        serde_json::from_slice(VALID_WORKFLOW).expect("workflow definition");
    let actor = AuthContext::with_run_prefix(
        "live-policy-owner",
        ["workflow:control".to_owned()],
        Some(RUN_ID.to_owned()),
    )
    .expect("actor");
    let binding = port
        .load_active_policy(
            &actor,
            &request,
            RUN_ID,
            &definition,
            &NativeCapabilities::fixture(),
        )
        .expect("load policy for admitted workflow run");
    assert_eq!(binding.policy_sha256, sha256);

    let request_scoped_actor = AuthContext::with_run_prefix(
        "request-only-policy-owner",
        ["workflow:control".to_owned()],
        Some(request.request_id.clone()),
    )
    .expect("request scoped actor");
    let denied = port.load_active_policy(
        &request_scoped_actor,
        &request,
        RUN_ID,
        &definition,
        &NativeCapabilities::fixture(),
    );
    assert_eq!(
        denied
            .expect_err("request ID must not grant workflow-run access")
            .code,
        "provider_session_policy_forbidden"
    );

    let foreign = port.load_active_policy(
        &actor,
        &request,
        "run.foreign",
        &definition,
        &NativeCapabilities::fixture(),
    );
    assert_eq!(
        foreign
            .expect_err("foreign workflow run is outside actor scope")
            .code,
        "provider_session_policy_forbidden"
    );
    drop(port);
    drop(owner);
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn corrupt_existing_owner_journal_is_never_reinitialized() {
    let (service, path) = service_and_path();
    drop(service);
    std::fs::write(&path, b"corrupt encrypted owner state").expect("corrupt fixture");
    let store = ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    assert!(
        ProviderSessionPolicyOwner::open(store, scope(), NativeCapabilities::fixture()).is_err(),
        "corrupt retained policy history must fail closed"
    );
    assert_eq!(
        std::fs::read(&path).expect("still present"),
        b"corrupt encrypted owner state"
    );
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn policy_command_http_lifecycle_is_cas_idempotent_and_survives_restart() {
    let (service, path) = service_and_path();
    let control_auth = StaticAuthenticator::single("token", submitter()).expect("auth");

    let source_bytes = serde_json::to_vec(&over_limit_policy()).expect("source bytes");
    let import_path =
        format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy/import?expected_revision=3");
    let (status, imported) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &import_path,
        Some(&source_bytes),
    );
    assert_eq!(status, 200, "body: {imported}");
    let source_sha256 = sts2_harness::sha256_hex(&source_bytes);
    assert_eq!(imported["policy_sha256"], source_sha256);
    assert_eq!(imported["revision"], 4);

    let (status, imported_retry) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &import_path,
        Some(&source_bytes),
    );
    assert_eq!(status, 200, "body: {imported_retry}");
    assert_eq!(imported_retry["policy_sha256"], source_sha256);
    assert_eq!(imported_retry["revision"], 4);

    let target = {
        let mut value = over_limit_policy();
        value.version = 3;
        value.epoch = 3;
        value.max_completed_turns = MAX_COMPLETED_TURNS;
        value
    };
    let target_bytes = serde_json::to_vec(&target).expect("target bytes");
    let target_sha256 = sts2_harness::sha256_hex(&target_bytes);
    let proposal_path = format!(
        "/v1/workflow-runs/{RUN_ID}/provider-session-policy/proposals/proposal-http?source_sha256={source_sha256}&expected_revision=4"
    );
    let (status, proposed) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &proposal_path,
        Some(&target_bytes),
    );
    assert_eq!(status, 200, "body: {proposed}");
    let proposal_sha256 = proposed["proposal_sha256"]
        .as_str()
        .expect("proposal digest")
        .to_owned();
    assert_eq!(proposed["revision"], 5);

    let (status, proposed_retry) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &proposal_path,
        Some(&target_bytes),
    );
    assert_eq!(status, 200, "body: {proposed_retry}");
    assert_eq!(proposed_retry["proposal_sha256"], proposal_sha256);
    assert_eq!(proposed_retry["revision"], 5);

    let approval = serde_json::json!({
        "schema_version": PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION,
        "proposal_sha256": proposal_sha256,
        "approval_ref": "approval-http"
    });
    let approval_bytes = serde_json::to_vec(&approval).expect("approval request");
    let approve_path = format!(
        "/v1/workflow-runs/{RUN_ID}/provider-session-policy/proposals/proposal-http/approve?expected_revision=5"
    );
    let (status, approved) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &approve_path,
        Some(&approval_bytes),
    );
    assert_eq!(status, 200, "body: {approved}");
    assert_eq!(approved["revision"], 6);

    let (status, approved_retry) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &approve_path,
        Some(&approval_bytes),
    );
    assert_eq!(status, 200, "body: {approved_retry}");
    assert_eq!(approved_retry["revision"], 6);

    let stale_adopt_path = format!(
        "/v1/workflow-runs/{RUN_ID}/provider-session-policy/proposals/proposal-http/adopt?expected_revision=5"
    );
    let (status, stale) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &stale_adopt_path,
        Some(&approval_bytes),
    );
    assert_eq!(status, 409, "body: {stale}");
    assert_eq!(
        stale["error"]["code"],
        "provider_session_policy_owner_conflict"
    );

    let adopt_path = format!(
        "/v1/workflow-runs/{RUN_ID}/provider-session-policy/proposals/proposal-http/adopt?expected_revision=6"
    );
    let (status, adopted) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &adopt_path,
        Some(&approval_bytes),
    );
    assert_eq!(status, 200, "body: {adopted}");
    assert_eq!(adopted["revision"], 7);
    assert_eq!(adopted["policy_sha256"], target_sha256);

    let (status, view) = get(&service, control_auth.clone());
    assert_eq!(status, 200, "body: {view}");
    assert_eq!(view["value"]["revision"], 7);
    assert_eq!(view["value"]["active"]["sha256"], target_sha256);
    assert_eq!(view["value"]["proposals"][0]["state"], "adopted");
    assert_eq!(
        view["value"]["proposals"][0]["proposal_sha256"],
        proposal_sha256
    );
    assert_eq!(view["value"]["proposals"][0]["approval_recorded"], true);
    assert!(view["value"]["proposals"][0].get("approval_ref").is_none());
    assert_eq!(
        view["value"]["history"].as_array().expect("history").len(),
        3
    );
    assert_eq!(view["value"]["history"][2]["sha256"], target_sha256);
    drop(service);

    let reopened_store =
        ProviderSessionMetadataStore::encrypted(&path, [7; 32], scope()).expect("store");
    let reopened = Arc::new(
        ProviderSessionPolicyOwner::open(reopened_store, scope(), NativeCapabilities::fixture())
            .expect("reopen"),
    );
    let reopened_service = synthetic_store(Arc::new(MemoryWorkflowStore::new()))
        .with_provider_session_policy_command_port(Arc::new(
            DurableProviderSessionPolicyCommandPort::new(reopened),
        ));
    reopened_service
        .submit_run(&submitter(), run_request())
        .expect("run");
    let (status, reopened_view) = get(&Arc::new(reopened_service), control_auth);
    assert_eq!(status, 200, "body: {reopened_view}");
    assert_eq!(reopened_view["value"]["revision"], 7);
    assert_eq!(reopened_view["value"]["active"]["sha256"], target_sha256);
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn policy_command_http_requires_control_grant_and_rejects_stale_import() {
    let (service, path) = service_and_path();
    let policy_bytes = serde_json::to_vec(&over_limit_policy()).expect("policy");
    let import_path =
        format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy/import?expected_revision=3");

    let read_only = StaticAuthenticator::single("token", actor()).expect("auth");
    let (status, denied) = http_request(
        &service,
        read_only,
        "POST",
        &import_path,
        Some(&policy_bytes),
    );
    assert_eq!(status, 403, "body: {denied}");
    assert_eq!(denied["error"]["code"], "missing_scope");

    let foreign = AuthContext::with_run_prefix(
        "foreign-policy-owner",
        [
            "workflow:control".to_owned(),
            "workflow:content:write".to_owned(),
        ],
        Some("another-run".to_owned()),
    )
    .expect("foreign actor");
    let foreign_auth = StaticAuthenticator::single("token", foreign).expect("auth");
    let (status, foreign_denied) = http_request(
        &service,
        foreign_auth,
        "POST",
        &import_path,
        Some(&policy_bytes),
    );
    assert_eq!(status, 403, "body: {foreign_denied}");
    assert_eq!(foreign_denied["error"]["code"], "run_scope_denied");

    let control_only =
        AuthContext::new("control-only-policy-owner", ["workflow:control".to_owned()])
            .expect("control-only actor");
    let control_only_auth = StaticAuthenticator::single("token", control_only).expect("auth");
    let (status, content_denied) = http_request(
        &service,
        control_only_auth,
        "POST",
        &import_path,
        Some(&policy_bytes),
    );
    assert_eq!(status, 403, "body: {content_denied}");
    assert_eq!(content_denied["error"]["code"], "missing_scope");

    let control_auth = StaticAuthenticator::single("token", submitter()).expect("auth");
    let stale_path =
        format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy/import?expected_revision=2");
    let (status, stale) = http_request(
        &service,
        control_auth,
        "POST",
        &stale_path,
        Some(&policy_bytes),
    );
    assert_eq!(status, 409, "body: {stale}");
    assert_eq!(
        stale["error"]["code"],
        "provider_session_policy_owner_conflict"
    );
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}

#[test]
fn valid_initial_policy_can_be_imported_and_explicitly_adopted_over_http() {
    let (service, path) = fresh_service_and_path();
    let control_auth = StaticAuthenticator::single("token", submitter()).expect("auth");
    let policy_bytes = serde_json::to_vec(&policy()).expect("policy bytes");
    let policy_sha256 = sts2_harness::sha256_hex(&policy_bytes);
    let import_path =
        format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy/import?expected_revision=1");
    let (status, imported) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &import_path,
        Some(&policy_bytes),
    );
    assert_eq!(status, 200, "body: {imported}");
    assert_eq!(imported["revision"], 2);
    assert_eq!(imported["policy_sha256"], policy_sha256);

    let adoption = serde_json::json!({
        "schema_version": PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION,
        "policy_sha256": policy_sha256
    });
    let adoption_bytes = serde_json::to_vec(&adoption).expect("adoption request");
    let adoption_path =
        format!("/v1/workflow-runs/{RUN_ID}/provider-session-policy/adoptions?expected_revision=2");
    let (status, adopted) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &adoption_path,
        Some(&adoption_bytes),
    );
    assert_eq!(status, 200, "body: {adopted}");
    assert_eq!(adopted["revision"], 3);
    assert_eq!(adopted["policy_sha256"], policy_sha256);

    let (status, adopted_retry) = http_request(
        &service,
        control_auth.clone(),
        "POST",
        &adoption_path,
        Some(&adoption_bytes),
    );
    assert_eq!(status, 200, "body: {adopted_retry}");
    assert_eq!(adopted_retry["revision"], 3);

    let (status, view) = get(&service, control_auth);
    assert_eq!(status, 200, "body: {view}");
    assert_eq!(view["value"]["active"]["sha256"], policy_sha256);
    assert_eq!(
        view["value"]["history"].as_array().expect("history").len(),
        1
    );
    std::fs::remove_dir_all(path.parent().expect("parent")).expect("cleanup");
}
