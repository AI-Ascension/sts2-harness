// SPDX-License-Identifier: MIT

//! Authenticated, redacted provider-session policy owner projection and durable reopen.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use sts2_harness::management::MemoryWorkflowStore;
use sts2_harness::management::{
    AuthContext, DurableProviderSessionPolicyCommandPort, StaticAuthenticator, synthetic_store,
};
use sts2_harness::provider_session::{
    MAX_COMPLETED_TURNS, NativeCapabilities, ProviderSessionMetadataStore,
    ProviderSessionPolicyOwner,
};

#[path = "support/provider_session_policy_http.rs"]
mod support;

use support::*;

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
