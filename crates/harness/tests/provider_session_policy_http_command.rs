// SPDX-License-Identifier: MIT

//! Provider-session policy command lifecycle: CAS, idempotency, restart and control grants.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use sts2_harness::management::MemoryWorkflowStore;
use sts2_harness::management::{
    AuthContext, DurableProviderSessionPolicyCommandPort,
    PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION, StaticAuthenticator, synthetic_store,
};
use sts2_harness::provider_session::{
    MAX_COMPLETED_TURNS, NativeCapabilities, ProviderSessionMetadataStore,
    ProviderSessionPolicyOwner,
};

#[path = "support/provider_session_policy_http.rs"]
mod support;

use support::*;

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
