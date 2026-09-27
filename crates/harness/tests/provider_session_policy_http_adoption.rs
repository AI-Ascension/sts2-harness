// SPDX-License-Identifier: MIT

//! Provider-session policy adoption identity and admitted-run binding over HTTP.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use sts2_harness::management::{
    AuthContext, LiveProviderPolicyPort, ProviderSessionPolicyOwnerPort,
};
use sts2_harness::provider_session::{
    MAX_COMPLETED_TURNS, NativeCapabilities, ProviderSessionMetadataStore,
    ProviderSessionPolicyOwner,
};
use sts2_harness::workflow::WorkflowDefinition;

#[path = "support/provider_session_policy_http.rs"]
mod support;

use support::*;

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
