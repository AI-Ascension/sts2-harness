// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::{
    Arc, Barrier,
    atomic::{AtomicU64, Ordering},
};
use sts2_harness::management::{
    AuthContext, ErrorBody, ErrorClass, FileWorkflowStore, ManagementService, MemoryWorkflowStore,
    SqliteWorkflowStore, SubmissionClaim, SubmissionClaimAcceptance, SubmissionClaimOutcome,
    SubmissionRecoveryState, WorkflowStore, digest_value,
};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct DatabasePath(PathBuf);
impl DatabasePath {
    fn new() -> Self {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/submission-claim-tests");
        std::fs::create_dir_all(&root).expect("fixture directory");
        Self(root.join(format!(
            "claims-{}-{}.db",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for DatabasePath {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", self.0.display()));
            let _ = std::fs::remove_file(path);
        }
    }
}

fn claim(actor: &str, request: &str) -> SubmissionClaim {
    SubmissionClaim {
        actor_digest: digest_value(&serde_json::json!({"subject": actor})).expect("actor digest"),
        request_digest: digest_value(&serde_json::json!({"request": request}))
            .expect("request digest"),
        outcome: None,
    }
}

fn check_store(store: &dyn WorkflowStore) {
    let original = claim("owner", "one");
    assert_eq!(
        store
            .claim_submission("request-1", original.clone())
            .expect("claim"),
        SubmissionClaimAcceptance::Acquired
    );
    assert_eq!(
        store
            .claim_submission("request-1", original.clone())
            .expect("repeat"),
        SubmissionClaimAcceptance::Existing(original.clone())
    );
    for conflicting in [claim("foreign", "one"), claim("owner", "changed")] {
        assert_eq!(
            store
                .claim_submission("request-1", conflicting.clone())
                .expect("conflict"),
            SubmissionClaimAcceptance::Conflict
        );
        let mut altered = conflicting;
        altered.outcome = Some(SubmissionClaimOutcome::Failed {
            workflow_run_id: None,
            error: ErrorBody {
                class: ErrorClass::Unavailable,
                code: "refused".into(),
                message: "refused".into(),
            },
        });
        assert!(store.finish_submission_claim("request-1", altered).is_err());
    }
    assert_eq!(
        store.submission_claim("request-1").expect("unchanged"),
        Some(original.clone())
    );
    let mut finished = original;
    finished.outcome = Some(SubmissionClaimOutcome::Failed {
        workflow_run_id: None,
        error: ErrorBody {
            class: ErrorClass::Capability,
            code: "capability_missing".into(),
            message: "capability absent".into(),
        },
    });
    store
        .finish_submission_claim("request-1", finished.clone())
        .expect("finish");
    store
        .finish_submission_claim("request-1", finished.clone())
        .expect("same finish is idempotent");
    let mut changed = finished.clone();
    changed.outcome = None;
    assert!(
        store.finish_submission_claim("request-1", changed).is_err(),
        "completed claims cannot be cleared for retry"
    );
    assert_eq!(
        store.submission_claim("request-1").expect("retained"),
        Some(finished)
    );
    let mut oversized = claim("owner", "oversized");
    store
        .claim_submission("oversized", oversized.clone())
        .expect("pending claim");
    oversized.outcome = Some(SubmissionClaimOutcome::Failed {
        workflow_run_id: None,
        error: ErrorBody {
            class: ErrorClass::Store,
            code: "oversized".into(),
            message: "x".repeat(16_384),
        },
    });
    assert_eq!(
        store
            .finish_submission_claim("oversized", oversized)
            .expect_err("record bound")
            .code,
        "submission_claim_too_large"
    );
    assert!(
        store
            .submission_claim("oversized")
            .expect("retained pending")
            .expect("claim")
            .outcome
            .is_none()
    );
}

#[test]
fn capacity_refuses_only_new_identities_and_retains_existing_claims() {
    let store = MemoryWorkflowStore::new();
    let original = claim("owner", "same");
    for index in 0..sts2_harness::management::MAX_SUBMISSION_CLAIMS {
        assert_eq!(
            store
                .claim_submission(&format!("capacity-{index}"), original.clone())
                .expect("bounded admission"),
            SubmissionClaimAcceptance::Acquired
        );
    }
    assert_eq!(
        store
            .claim_submission("capacity-new", original.clone())
            .expect_err("capacity")
            .code,
        "submission_claim_capacity"
    );
    assert_eq!(
        store
            .claim_submission("capacity-0", original.clone())
            .expect("existing at capacity"),
        SubmissionClaimAcceptance::Existing(original)
    );
}

#[test]
fn all_store_adapters_bind_actor_payload_and_immutable_outcome() {
    check_store(&MemoryWorkflowStore::new());
    let file = DatabasePath::new();
    check_store(&FileWorkflowStore::open(&file.0).expect("file store"));
    let sqlite = DatabasePath::new();
    check_store(&SqliteWorkflowStore::open(&sqlite.0).expect("sqlite store"));
}

#[test]
fn pending_claims_survive_both_durable_store_restarts() {
    let pending = claim("owner", "pending");
    let file = DatabasePath::new();
    {
        let store = FileWorkflowStore::open(&file.0).expect("file store");
        store
            .claim_submission("request-1", pending.clone())
            .expect("claim");
    }
    let reopened = FileWorkflowStore::open(&file.0).expect("reopen file");
    assert_eq!(
        reopened
            .claim_submission("request-1", pending.clone())
            .expect("not reacquired"),
        SubmissionClaimAcceptance::Existing(pending.clone())
    );
    let sqlite = DatabasePath::new();
    {
        let store = SqliteWorkflowStore::open(&sqlite.0).expect("sqlite store");
        store
            .claim_submission("request-1", pending.clone())
            .expect("claim");
    }
    let reopened = SqliteWorkflowStore::open(&sqlite.0).expect("reopen sqlite");
    assert_eq!(
        reopened
            .claim_submission("request-1", pending.clone())
            .expect("not reacquired"),
        SubmissionClaimAcceptance::Existing(pending)
    );
}

#[test]
fn independent_sqlite_connections_have_exactly_one_claim_winner() {
    let path = DatabasePath::new();
    let stores = [
        SqliteWorkflowStore::open(&path.0).expect("first"),
        SqliteWorkflowStore::open(&path.0).expect("second"),
    ];
    let barrier = Arc::new(Barrier::new(2));
    let handles = stores
        .into_iter()
        .map(|store| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                store
                    .claim_submission("request-race", claim("owner", "same"))
                    .expect("arbitrate")
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().expect("join"))
        .collect::<Vec<_>>();
    assert_eq!(
        results
            .iter()
            .filter(|value| **value == SubmissionClaimAcceptance::Acquired)
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|value| matches!(value, SubmissionClaimAcceptance::Existing(_)))
            .count(),
        1
    );
}

#[test]
fn recovery_never_discloses_another_actors_claim_or_error() {
    let store = Arc::new(MemoryWorkflowStore::new());
    store
        .claim_submission("request-private", claim("owner", "private-payload"))
        .expect("claim");
    let service = ManagementService::new(store);
    let owner = AuthContext::new("owner", ["workflow:read".into()]).expect("owner");
    let foreign = AuthContext::new("foreign", ["workflow:read".into()]).expect("foreign");
    let hidden = service
        .submission_recovery(&foreign, "request-private")
        .expect("hidden");
    let mut absent = service
        .submission_recovery(&foreign, "request-missing")
        .expect("missing");
    absent.request_id = "request-private".into();
    assert_eq!(
        hidden, absent,
        "foreign existence must be indistinguishable from absence"
    );
    assert_eq!(
        service
            .submission_recovery(&owner, "request-private")
            .expect("owner read")
            .state,
        SubmissionRecoveryState::Pending
    );
    let unscoped = AuthContext::new("owner", Vec::new()).expect("unscoped");
    assert_eq!(
        service
            .submission_recovery(&unscoped, "request-private")
            .expect_err("scope")
            .class,
        ErrorClass::Forbidden
    );
}

#[test]
fn a_legacy_submission_cannot_be_adopted_by_guessing_its_identity() {
    let path = DatabasePath::new();
    let store = SqliteWorkflowStore::open(&path.0).expect("sqlite store");
    let connection = rusqlite::Connection::open(&path.0).expect("legacy connection");
    let proposed = claim("owner", "legacy");
    connection.execute("INSERT INTO management_submissions(request_id, request_digest, workflow_run_id) VALUES (?1, ?2, ?3)",
        rusqlite::params!["legacy-request", proposed.request_digest, "legacy-run"]).expect("old submission row");
    assert_eq!(
        store
            .claim_submission("legacy-request", proposed)
            .expect("refusal"),
        SubmissionClaimAcceptance::Conflict
    );
    assert!(
        store
            .submission_claim("legacy-request")
            .expect("no owner invented")
            .is_none()
    );
}
