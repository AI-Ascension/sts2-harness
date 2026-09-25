// SPDX-License-Identifier: MIT

//! Stale-base/concurrency conflicts and single-operation honesty for the
//! proposal-only authoring-inference endpoint (`sts2-harness#105`, AC 3 and 4).
//!
//! Every fixture is synthetic and in-memory. A pass proves a stale or
//! concurrently-changed base can never overwrite a draft, and that budget
//! exhaustion, cancellation and restart each preserve exactly one operation
//! with an honest cost and outcome.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use serde_json::json;
use sts2_harness::management::{
    AuthoringInferenceCost, AuthoringInferenceJournal, AuthoringInferenceOperationState,
    AuthoringStore, ErrorClass, ManagementError, MemoryAuthoringStore,
    SqliteAuthoringInferenceJournal, SqliteWorkflowStore, authoring_inference_operation_id,
    digest_value,
};

#[path = "support/authoring_inference_harness.rs"]
mod harness;
use harness::*;

fn definition() -> serde_json::Value {
    two_node_definition(DECIDE_PROFILE, "planner.synthetic.v1")
}

fn refusal(
    outcome: Result<impl std::fmt::Debug, ManagementError>,
) -> Result<ManagementError, Box<dyn std::error::Error>> {
    match outcome {
        Ok(value) => Err(format!("expected refusal, got {value:?}").into()),
        Err(error) => Ok(error),
    }
}

#[test]
fn a_stale_base_is_refused_before_any_reservation_and_never_overwrites_the_draft()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = RecordingAuthoringProvider::returning(candidate(definition(), 1, 512));
    let suite = Suite::build(provider)?;

    // Wrong revision.
    let mut revision = suite.request("mutation-stale-revision")?;
    revision.base.revision = 99;
    let error = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, revision),
    )?;
    assert_eq!(error.code, "authoring_inference_base_conflict");

    // Wrong etag.
    let mut etag = suite.request("mutation-stale-etag")?;
    etag.base.etag = digest_value(&json!({"stale": "etag"}))?;
    let error = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, etag),
    )?;
    assert_eq!(error.code, "authoring_inference_base_conflict");

    // Wrong definition digest.
    let mut digest = suite.request("mutation-stale-digest")?;
    digest.base.definition_digest = digest_value(&json!({"stale": "digest"}))?;
    let error = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, digest),
    )?;
    assert_eq!(error.code, "authoring_inference_base_conflict");

    // The fence runs before the reservation and before the provider.
    assert_eq!(suite.provider.calls(), 0);
    assert!(
        suite
            .service
            .authoring_inference_operation(&suite.actor, DRAFT_ID, "mutation-stale-revision")
            .is_err(),
        "a stale base must not reserve an operation"
    );
    suite.assert_no_effect()?;
    Ok(())
}

#[test]
fn a_concurrent_draft_change_wins_and_the_proposal_loses_to_an_actionable_conflict()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = RecordingAuthoringProvider::returning(candidate(definition(), 1, 512));
    let suite = Suite::build(provider)?;
    let store = Arc::clone(&suite.authoring);
    suite.provider.on_call(move || {
        let current = store
            .get_draft(DRAFT_ID)
            .expect("draft read")
            .expect("draft present");
        store
            .save_draft(
                DRAFT_ID,
                current.revision,
                &current.etag,
                "mutation-winner",
                json!({"workflow_id": "winner", "version": "0.2.0"}),
                json!({"nodes": [], "edges": []}),
            )
            .expect("the concurrent winner must save");
    });

    let request = suite.request("mutation-race")?;
    let error = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, request),
    )?;
    assert_eq!(error.code, "authoring_inference_base_conflict");
    assert!(error.message.contains("served revision"));

    // The concurrent winner is untouched by the losing proposal.
    let winner = suite.service.studio_draft(&suite.actor, DRAFT_ID)?;
    assert_eq!(winner.revision, suite.draft.revision + 1);
    assert_eq!(winner.document["workflow_id"], json!("winner"));

    // The operation is recorded honestly as refused, and no run was started.
    let recorded =
        suite
            .service
            .authoring_inference_operation(&suite.actor, DRAFT_ID, "mutation-race")?;
    assert_eq!(recorded.state, AuthoringInferenceOperationState::Refused);
    assert_eq!(suite.provider.calls(), 1);
    assert_eq!(
        suite
            .execution
            .submissions
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    Ok(())
}

#[test]
fn budget_exhaustion_and_cancellation_preserve_one_honest_operation()
-> Result<(), Box<dyn std::error::Error>> {
    // Budget exhaustion: the candidate over-spends its reservation.
    let over = RecordingAuthoringProvider::returning(candidate(definition(), 3, 4096));
    let suite = Suite::build(over)?;
    let error = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, suite.request("mutation-budget")?),
    )?;
    assert_eq!(error.code, "authoring_inference_budget_exhausted");
    assert_eq!(error.class, ErrorClass::Budget);
    let recorded =
        suite
            .service
            .authoring_inference_operation(&suite.actor, DRAFT_ID, "mutation-budget")?;
    assert_eq!(
        recorded.state,
        AuthoringInferenceOperationState::BudgetExhausted
    );
    assert_eq!(recorded.cost.provider_calls, 3, "the cost must be honest");
    // A replay is a typed error, never a silent second provider call.
    let replay = suite
        .service
        .authoring_inference_proposal(&suite.actor, suite.request("mutation-budget")?);
    assert!(replay.is_err());
    assert_eq!(suite.provider.calls(), 1);
    suite.assert_no_effect()?;

    // Cancellation: the provider signals before a proposal exists.
    let cancelled = RecordingAuthoringProvider::failing(ManagementError::conflict(
        "authoring_inference_cancelled",
        "the provider cancelled this operation",
    ));
    let suite = Suite::build(cancelled)?;
    let error = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, suite.request("mutation-cancel")?),
    )?;
    assert_eq!(error.code, "authoring_inference_cancelled");
    let recorded =
        suite
            .service
            .authoring_inference_operation(&suite.actor, DRAFT_ID, "mutation-cancel")?;
    assert_eq!(recorded.state, AuthoringInferenceOperationState::Cancelled);
    assert_eq!(
        recorded.cost,
        AuthoringInferenceCost {
            provider_calls: 1,
            output_tokens: 0
        }
    );
    suite.assert_no_effect()?;
    Ok(())
}

#[test]
fn restart_replays_the_one_recorded_operation_without_a_second_provider_call()
-> Result<(), Box<dyn std::error::Error>> {
    let authoring = Arc::new(sts2_harness::management::MemoryAuthoringStore::new());
    let catalog = baseline_catalog();
    let execution = RecordingExecutionPort::new();
    let journal = Arc::new(sts2_harness::management::MemoryAuthoringInferenceJournal::new());
    let provider = RecordingAuthoringProvider::returning(candidate(definition(), 1, 512));
    let actor = actor()?;

    let first_service = service_with(&authoring, &provider, &journal, &catalog, &execution);
    let draft = first_service.studio_create_draft(&actor, create_request(base_document()))?;
    let request = request_for(&draft, &catalog, "mutation-restart")?;
    let first = first_service.authoring_inference_proposal(&actor, request.clone())?;
    assert_eq!(provider.calls(), 1);

    // A restarted owner shares the same durable store and journal.
    let restarted = service_with(&authoring, &provider, &journal, &catalog, &execution);
    let second = restarted.authoring_inference_proposal(&actor, request)?;
    assert_eq!(second, first);
    assert_eq!(
        provider.calls(),
        1,
        "a restart must replay the recorded proposal, not regenerate it"
    );
    let recorded = restarted.authoring_inference_operation(&actor, DRAFT_ID, "mutation-restart")?;
    assert_eq!(recorded.state, AuthoringInferenceOperationState::Proposed);
    assert_eq!(recorded.cost, first.cost);
    assert_eq!(
        recorded.proposal_id.as_deref(),
        Some(first.proposal_id.as_str())
    );
    assert_eq!(
        recorded.operation_id,
        authoring_inference_operation_id(DRAFT_ID, "mutation-restart")
    );
    Ok(())
}

#[test]
fn an_unresolved_reservation_and_a_reused_identity_are_actionable_conflicts()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = RecordingAuthoringProvider::returning(candidate(definition(), 1, 512));
    let suite = Suite::build(provider)?;
    let request = suite.request("mutation-pending")?;

    // A reservation with the same identity is still pending: refuse rather than
    // silently generating a second operation.
    let digest = digest_value(&serde_json::to_value(&request)?)?;
    suite.journal.begin(
        &authoring_inference_operation_id(DRAFT_ID, "mutation-pending"),
        DRAFT_ID,
        "mutation-pending",
        &digest,
        AuthoringInferenceCost {
            provider_calls: 1,
            output_tokens: 0,
        },
    )?;
    let pending = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, request.clone()),
    )?;
    assert_eq!(pending.code, "authoring_inference_operation_in_progress");
    assert_eq!(suite.provider.calls(), 0);
    suite.assert_no_effect()?;

    // The same identity with a different request body is a conflict.
    let mut different = request.clone();
    different.requirement.max_stages = 3;
    let conflict = refusal(
        suite
            .service
            .authoring_inference_proposal(&suite.actor, different),
    )?;
    assert_eq!(conflict.code, "authoring_inference_operation_conflict");
    assert_eq!(suite.provider.calls(), 0);
    Ok(())
}

/// AC4 says a *restart* preserves one operation and an honest cost/outcome
/// state, and requirement 4 says the reservation is *persisted*. The in-memory
/// restart test above passes the same `Arc<MemoryAuthoringInferenceJournal>` to
/// both services, so it proves an in-process re-composition. This test closes
/// the store, reopens it from disk, and re-composes over a *new* journal
/// instance: the replay below can only succeed if the reservation and its
/// terminal outcome actually survived the process-local map.
#[test]
fn a_reopened_database_replays_the_one_recorded_operation_without_a_second_provider_call()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = std::env::temp_dir().join(format!(
        "sts2-authoring-inference-journal-{}-{}",
        std::process::id(),
        line!()
    ));
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("management.sqlite3");

    let authoring = Arc::new(MemoryAuthoringStore::new());
    let catalog = baseline_catalog();
    let execution = RecordingExecutionPort::new();
    let provider = RecordingAuthoringProvider::returning(candidate(definition(), 1, 512));
    let actor = actor()?;

    let first_service = service_with_journal(
        &authoring,
        &provider,
        Arc::new(SqliteAuthoringInferenceJournal::new(Arc::new(
            SqliteWorkflowStore::open(&path)?,
        ))),
        &catalog,
        &execution,
    );
    let draft = first_service.studio_create_draft(&actor, create_request(base_document()))?;
    let request = request_for(&draft, &catalog, "mutation-reopen")?;
    let first = first_service.authoring_inference_proposal(&actor, request.clone())?;
    assert_eq!(provider.calls(), 1);
    let recorded =
        first_service.authoring_inference_operation(&actor, DRAFT_ID, "mutation-reopen")?;
    assert_eq!(recorded.state, AuthoringInferenceOperationState::Proposed);

    // Every handle onto the first store is gone before the reopen, so nothing
    // can be replayed from a live in-process journal.
    drop(first_service);
    let reopened = Arc::new(SqliteWorkflowStore::open(&path)?);
    let restarted = service_with_journal(
        &authoring,
        &provider,
        Arc::new(SqliteAuthoringInferenceJournal::new(reopened.clone())),
        &catalog,
        &execution,
    );

    let second = restarted.authoring_inference_proposal(&actor, request)?;
    assert_eq!(second, first);
    assert_eq!(
        provider.calls(),
        1,
        "a restart across a reopened store must replay the recorded proposal, not regenerate it"
    );
    let recovered = restarted.authoring_inference_operation(&actor, DRAFT_ID, "mutation-reopen")?;
    assert_eq!(recovered, recorded);
    assert_eq!(recovered.cost, first.cost);
    assert_eq!(
        recovered.proposal_id.as_deref(),
        Some(first.proposal_id.as_str())
    );
    Ok(())
}
