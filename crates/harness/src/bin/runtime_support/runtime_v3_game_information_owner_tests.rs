// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[path = "../../../tests/support/memory_policy_owner.rs"]
mod fixture;

use super::*;
use std::sync::Arc;
use sts2_harness::context_memory::policy_owner::{MemoryPolicyOwner, PolicyStoreConsent};
use sts2_harness::game_information::LookupError;
use sts2_harness::management::ManagementClient;

#[cfg(unix)]
#[path = "runtime_v3_game_information_entry_tests.rs"]
mod entry_tests;

pub(in crate::runtime_support::runtime_v3) struct AdoptedRuntimeOwnerFixture {
    _fixture: fixture::Fixture,
    pub(in crate::runtime_support::runtime_v3) owner: Arc<RuntimeGameInformationOwner>,
    pub(in crate::runtime_support::runtime_v3) authority: Arc<MemoryPolicyAuthority>,
    pub(in crate::runtime_support::runtime_v3) binding:
        sts2_harness::context_memory::policy_owner::ActivePolicyBinding,
}

pub(in crate::runtime_support::runtime_v3) fn adopted_runtime_owner() -> AdoptedRuntimeOwnerFixture
{
    let fixture = fixture::Fixture::new();
    let initial_review = fixture.adopt();
    let source = fixture
        .owner
        .inspect_policy(fixture::access(), &initial_review.target)
        .expect("source policy");
    let scope = fixture::scope();
    let reopened = Arc::new(
        MemoryPolicyOwner::open(
            &fixture.path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly,
        )
        .expect("reopen durable owner"),
    );
    let corpus_path = fixture.directory.join("runtime-corpus.sqlite");
    let mut corpus_store = DurableMemoryStore::open(
        corpus_path.to_str().expect("UTF-8 path"),
        scope.clone(),
        [9; 32],
    )
    .expect("runtime corpus store");
    let trusted_corpus = fixture
        .authority
        .inspect(|state| Ok(state.corpus.clone()))
        .expect("trusted corpus");
    for entry in trusted_corpus.entries() {
        corpus_store
            .publish(entry.clone())
            .expect("persist trusted corpus");
    }
    let authenticator: Arc<dyn Authenticator> = fixture::authenticator();
    let owner = Arc::new(RuntimeGameInformationOwner {
        scope: scope.clone(),
        owner: reopened.clone(),
        corpus_store: Mutex::new(corpus_store),
        archive_store: Mutex::new(
            DurableMemoryStore::open(":memory:", scope, [11; 32]).expect("archive store"),
        ),
        authenticator,
        selector_grant_id: String::from("policy-grant"),
        operator_grant_id: String::from("policy-grant"),
        bearer: Zeroizing::new(String::from("synthetic-owner-token")),
        deployment_sha256: sts2_harness::sha256_hex(b"synthetic test deployment"),
        management_listen: "127.0.0.1:0".parse().expect("loopback"),
        preflight_timeout_seconds: 3,
        replay_archive: true,
        archive_retention_seconds: 86_400,
    });

    reopened
        .execute(
            fixture::access(),
            PolicyCommand::ProposeRevalidation {
                key: String::from("test-revalidation"),
                review_id: String::from("test-revalidation"),
                source: initial_review.target,
                target_raw: source.raw_bytes().to_vec(),
                expected_active_version: 1,
            },
        )
        .expect("propose explicit revalidation");
    let review = reopened
        .inspect_review(fixture::access(), "test-revalidation")
        .expect("review");
    reopened
        .execute(
            fixture::access(),
            PolicyCommand::Approve {
                key: String::from("test-approval"),
                review_id: review.review_id.clone(),
                review_sha256: review.review_sha256.clone(),
            },
        )
        .expect("approve revalidation");
    reopened
        .execute(
            fixture::access(),
            PolicyCommand::Adopt {
                key: String::from("test-adoption"),
                review_id: review.review_id,
                review_sha256: review.review_sha256,
            },
        )
        .expect("adopt revalidated policy");
    let selected = owner
        .lookup_snapshot(None)
        .expect("fresh policy and matching corpus");
    let authority = fixture.authority.clone();
    AdoptedRuntimeOwnerFixture {
        _fixture: fixture,
        owner,
        authority,
        binding: selected.binding,
    }
}

#[test]
fn blocked_mcp_callback_releases_owner_lease_and_discards_stale_reply() {
    use std::sync::mpsc;
    use std::time::Duration;

    let fixture = adopted_runtime_owner();
    let (started_tx, started_rx) = mpsc::sync_channel(1);
    let (release_tx, release_rx) = mpsc::sync_channel(1);
    let owner = Arc::clone(&fixture.owner);
    let binding = fixture.binding.clone();
    let mcp_call = std::thread::spawn(move || {
        owner.call_with_lookup_revalidation(&binding, || {
            started_tx.send(()).map_err(|_| LookupError::Transport)?;
            release_rx
                .recv_timeout(Duration::from_secs(3))
                .map_err(|_| LookupError::Transport)?;
            Ok(vec![1, 2, 3])
        })
    });
    started_rx
        .recv_timeout(Duration::from_secs(3))
        .expect("fake MCP call reached its blocked external boundary");

    let (revoked_tx, revoked_rx) = mpsc::sync_channel(1);
    let authority = Arc::clone(&fixture.authority);
    let revoke = std::thread::spawn(move || {
        let result = authority.update(|state| {
            state.control_epoch = state.control_epoch.saturating_add(1);
            Ok(())
        });
        let _ = revoked_tx.send(result);
    });
    let revocation = revoked_rx.recv_timeout(Duration::from_millis(500));
    let _ = release_tx.send(());
    let call_result = mcp_call.join().expect("fake MCP thread");
    revoke.join().expect("revocation thread");
    assert!(
        revocation.is_ok(),
        "owner revocation must proceed while the external MCP call is blocked"
    );
    assert!(revocation.expect("revocation completed").is_ok());
    assert_eq!(
        call_result,
        Err(LookupError::Scope),
        "the MCP response must be discarded after authority changes in flight"
    );
}

#[test]
fn authenticated_loopback_preflight_requires_explicit_revalidation_approval_and_adoption() {
    let fixture = fixture::Fixture::new();
    let initial_review = fixture.adopt();
    let source = fixture
        .owner
        .inspect_policy(fixture::access(), &initial_review.target)
        .expect("current exact policy bytes");
    let scope = fixture::scope();
    let policy_path = fixture.path.clone();
    let reopened_owner = Arc::new(
        MemoryPolicyOwner::open(
            &policy_path,
            [7; 32],
            fixture.authority.clone(),
            PolicyStoreConsent::SyntheticOnly,
        )
        .expect("reopen bumps durable owner fence"),
    );
    let corpus_path = fixture.directory.join("runtime-corpus.sqlite");
    let mut corpus_store = DurableMemoryStore::open(
        corpus_path.to_str().expect("UTF-8 path"),
        scope.clone(),
        [9; 32],
    )
    .expect("runtime corpus store");
    let trusted_corpus = fixture
        .authority
        .inspect(|state| Ok(state.corpus.clone()))
        .expect("trusted corpus");
    for entry in trusted_corpus.entries() {
        corpus_store
            .publish(entry.clone())
            .expect("persist selected corpus fixture");
    }
    let trusted_authenticator: Arc<dyn Authenticator> = fixture::authenticator();
    let owner = Arc::new(RuntimeGameInformationOwner {
        scope: scope.clone(),
        owner: reopened_owner,
        corpus_store: Mutex::new(corpus_store),
        archive_store: Mutex::new(
            DurableMemoryStore::open(":memory:", scope.clone(), [11; 32]).expect("archive store"),
        ),
        authenticator: trusted_authenticator,
        selector_grant_id: String::from("policy-grant"),
        operator_grant_id: String::from("policy-grant"),
        bearer: Zeroizing::new(String::from("synthetic-owner-token")),
        deployment_sha256: sts2_harness::sha256_hex(b"pinned fixture config"),
        management_listen: "127.0.0.1:0".parse().expect("loopback"),
        preflight_timeout_seconds: 3,
        replay_archive: false,
        archive_retention_seconds: 86_400,
    });
    assert!(
        owner.lookup_snapshot(None).is_err(),
        "opening the durable owner must leave the old adoption stale"
    );

    let server = start_management_server(&owner).expect("authenticated loopback server");
    let client =
        ManagementClient::new(server.address(), "synthetic-owner-token").expect("operator client");
    let initial_status = client
        .request_json("GET", "/v1/memory-policy-owner", None)
        .expect("initial status");
    assert_eq!(initial_status.status, 200);
    let initial_status: Value = serde_json::from_slice(&initial_status.body).expect("status JSON");
    assert_eq!(initial_status["lookup_ready"], false);

    let unauthorized = ManagementClient::new(server.address(), "other-token")
        .expect("different principal client")
        .request_json("GET", "/v1/memory-policy-owner", None)
        .expect("denied owner status");
    assert_eq!(unauthorized.status, 403);

    let raw_sha256 = &initial_review.target.raw_sha256;
    let content_path = format!(
        "/v1/memory-policy-owner/policies/{}/{}/{}",
        initial_review.target.policy_id, initial_review.target.version, raw_sha256
    );
    let content = client
        .request_json("GET", &content_path, None)
        .expect("exact source policy content");
    assert_eq!(content.status, 200);
    let content: Value = serde_json::from_slice(&content.body).expect("policy JSON");
    assert_eq!(
        hex_decode(content["raw_hex"].as_str().expect("hex content")),
        source.raw_bytes()
    );

    let review_id = "runtime-revalidation";
    let proposal_path = format!(
        "/v1/memory-policy-owner/proposals/{review_id}?source_policy_id={}&source_version={}&source_raw_sha256={}&expected_active_version=1",
        initial_review.target.policy_id,
        initial_review.target.version,
        initial_review.target.raw_sha256,
    );
    let proposal = client
        .request_json_with_idempotency_key(
            "POST",
            &proposal_path,
            source.raw_bytes(),
            "runtime-propose",
        )
        .expect("revalidation proposal");
    assert_eq!(proposal.status, 200);
    let first_receipt: Value = serde_json::from_slice(&proposal.body).expect("proposal receipt");
    assert_eq!(first_receipt["operation"], "propose_revalidation");
    let duplicate_proposal = client
        .request_json_with_idempotency_key(
            "POST",
            &proposal_path,
            source.raw_bytes(),
            "runtime-propose",
        )
        .expect("idempotent proposal retry");
    assert_eq!(duplicate_proposal.status, 200);
    assert_eq!(duplicate_proposal.body, proposal.body);

    let review_response = client
        .request_json(
            "GET",
            &format!("/v1/memory-policy-owner/reviews/{review_id}"),
            None,
        )
        .expect("review metadata");
    assert_eq!(review_response.status, 200);
    let review: Value = serde_json::from_slice(&review_response.body).expect("review JSON");
    assert_eq!(review["kind"], "revalidation");
    assert_eq!(review["violations"], serde_json::json!([]));
    let digest = review["review_sha256"].as_str().expect("review digest");
    let digest_body =
        serde_json::to_vec(&json!({ "review_sha256": digest })).expect("command JSON");

    let premature_adopt = client
        .request_json_with_idempotency_key(
            "POST",
            &format!("/v1/memory-policy-owner/proposals/{review_id}/adopt"),
            &digest_body,
            "runtime-adopt-too-early",
        )
        .expect("premature adoption response");
    assert_eq!(premature_adopt.status, 403);

    let approval = client
        .request_json_with_idempotency_key(
            "POST",
            &format!("/v1/memory-policy-owner/proposals/{review_id}/approve"),
            &digest_body,
            "runtime-approve",
        )
        .expect("approval response");
    assert_eq!(approval.status, 200);
    let adoption = client
        .request_json_with_idempotency_key(
            "POST",
            &format!("/v1/memory-policy-owner/proposals/{review_id}/adopt"),
            &digest_body,
            "runtime-adopt",
        )
        .expect("adoption response");
    assert_eq!(adoption.status, 200);
    wait_for_owner_ready(&owner, 1).expect("fresh adopted policy is ready");
    let final_status = client
        .request_json("GET", "/v1/memory-policy-owner", None)
        .expect("final status");
    let final_status: Value = serde_json::from_slice(&final_status.body).expect("status JSON");
    assert_eq!(final_status["lookup_ready"], true);
    assert_eq!(final_status["active_binding"]["version"].as_u64(), Some(2));
    let selected = owner
        .lookup_snapshot(None)
        .expect("selected owner and persisted corpus match");
    let held = owner
        .lock_lookup_snapshot(&selected.binding)
        .expect("current selection yields a held authority guard");
    assert_eq!(held.snapshot().binding, selected.binding);
    drop(held);
    owner
        .corpus_store
        .lock()
        .expect("corpus handle")
        .publish(fixture::entry("changed-corpus", 1))
        .expect("persist test corpus change");
    assert_eq!(
        owner.lookup_snapshot(Some(&selected.binding)).unwrap_err(),
        PolicyOwnerError::OwnerFenced,
        "a reopened or changed corpus cannot use the previous selected-policy snapshot"
    );
    server.shutdown().expect("management listener closes");
}

fn hex_decode(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let digit = |byte| match byte {
                b'0'..=b'9' => Some(byte - b'0'),
                b'a'..=b'f' => Some(byte - b'a' + 10),
                _ => None,
            };
            Some((digit(pair[0])? << 4) | digit(pair[1])?)
        })
        .collect::<Option<Vec<_>>>()
        .expect("well-formed lowercase hex")
}
