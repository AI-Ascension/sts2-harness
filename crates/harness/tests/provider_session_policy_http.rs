// SPDX-License-Identifier: MIT

//! Authenticated, redacted provider-session policy owner projection.
#![allow(clippy::expect_used, clippy::unwrap_used)]

// The shared service, policy and request fixtures live in their own module so this file and its
// companion stay inside the repository's 600-line test budget. Refs sts2-harness#564.
#[path = "support/provider_session_policy_http_fixtures.rs"]
mod fixtures;

use fixtures::*;

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
