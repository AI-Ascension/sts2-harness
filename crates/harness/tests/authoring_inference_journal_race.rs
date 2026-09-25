// SPDX-License-Identifier: MIT

//! Cross-connection reservation races for the durable authoring-inference
//! journal (`sts2-harness#509`).
//!
//! Requirement 4 of `sts2-harness#105` says a reservation is *persisted* so a
//! repeated identity cannot reach the provider twice. The store's `Mutex` makes
//! that true for callers sharing one `SqliteWorkflowStore`, but the durable
//! journal exists precisely for the shape where two processes (or two stores
//! over one file) hold *different* connections, and there the `Mutex` is not a
//! boundary.
//!
//! These tests therefore open **two** stores over one database file. Two
//! callers race for one identity, and the assertion is that exactly one is told
//! `Started`: the loser must be classified from the row that already existed,
//! never assumed to have won, because `Started` is what authorizes a provider
//! call.

#![allow(clippy::expect_used)]

use std::sync::{Arc, Barrier};

use sts2_harness::management::{
    AuthoringInferenceBegin, AuthoringInferenceCost, AuthoringInferenceJournal,
    AuthoringInferenceOperationState, SqliteAuthoringInferenceJournal, SqliteWorkflowStore,
    authoring_inference_operation_id,
};

const DIGEST: &str = "0b6a1d5c4f3e2a1908172635445362718290a1b2c3d4e5f60718293a4b5c6d7e";

fn reserved() -> AuthoringInferenceCost {
    AuthoringInferenceCost {
        provider_calls: 1,
        output_tokens: 256,
    }
}

fn scratch_path(tag: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "sts2-authoring-inference-race-{}-{tag}",
        std::process::id()
    ));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory.join("management.sqlite3")
}

fn begin(
    journal: &SqliteAuthoringInferenceJournal,
    operation_id: &str,
) -> Result<AuthoringInferenceBegin, sts2_harness::management::StoreError> {
    journal.begin(
        operation_id,
        "draft-cross-connection",
        "mutation-cross-connection",
        DIGEST,
        reserved(),
    )
}

/// The reported defect, as a regression test: two connections, one identity, one
/// `Started`.
///
/// Before the fix the insert rowcount was discarded and the read-back tested the
/// digest only, so the caller that lost `INSERT OR IGNORE` read the winner's
/// pending row, saw an equal digest, and was told `Started` as well — measured
/// at 200/200 rounds for this topology. `Started` is the variant that sends the
/// caller on to `authoring_inference_provider.propose(...)`, so both callers
/// would contact the provider for one identity, which is the re-contact the
/// reservation exists to prevent.
#[test]
fn two_connections_racing_one_identity_yield_exactly_one_started()
-> Result<(), Box<dyn std::error::Error>> {
    let path = scratch_path("begin");
    let rounds = 32;
    let mut started = 0;
    let mut in_progress = 0;

    for round in 0..rounds {
        let operation_id = authoring_inference_operation_id(
            "draft-cross-connection",
            &format!("mutation-round-{round}"),
        );
        let first = Arc::new(SqliteAuthoringInferenceJournal::new(Arc::new(
            SqliteWorkflowStore::open(&path)?,
        )));
        let second = Arc::new(SqliteAuthoringInferenceJournal::new(Arc::new(
            SqliteWorkflowStore::open(&path)?,
        )));
        // Both threads are released together, so both are inside `begin` before
        // either has inserted: the loser cannot be classified by the pre-check,
        // which is what makes this the racy path rather than a plain replay.
        let gate = Arc::new(Barrier::new(2));

        let handle = {
            let journal = Arc::clone(&second);
            let gate = Arc::clone(&gate);
            let operation_id = operation_id.clone();
            std::thread::spawn(move || {
                gate.wait();
                begin(&journal, &operation_id)
            })
        };
        gate.wait();
        let mine = begin(&first, &operation_id)?;
        let theirs = handle.join().expect("racing thread must not panic")?;

        for outcome in [&mine, &theirs] {
            match outcome {
                AuthoringInferenceBegin::Started(_) => started += 1,
                AuthoringInferenceBegin::InProgress(_) => in_progress += 1,
                other => {
                    return Err(format!("unexpected begin outcome: {other:?}").into());
                }
            }
        }
        assert_eq!(
            started,
            round + 1,
            "round {round}: exactly one caller may be authorized to contact the provider"
        );
        assert_eq!(
            in_progress,
            round + 1,
            "round {round}: the caller that lost the reservation must be told InProgress"
        );
    }

    assert_eq!(started, rounds);
    assert_eq!(in_progress, rounds);
    Ok(())
}

/// The same boundary one step later: a reservation that reaches a terminal
/// state on one connection must be `Replayed` — never `Started` — on another.
///
/// This is the digest-equal, already-terminal case that the pre-check handles
/// correctly today; it is asserted here because the rowcount branch added for
/// the race must not disturb it.
#[test]
fn a_terminal_state_on_another_connection_replays_rather_than_starts()
-> Result<(), Box<dyn std::error::Error>> {
    let path = scratch_path("terminal");
    let winning = SqliteAuthoringInferenceJournal::new(Arc::new(SqliteWorkflowStore::open(&path)?));
    let other = SqliteAuthoringInferenceJournal::new(Arc::new(SqliteWorkflowStore::open(&path)?));
    let operation_id =
        authoring_inference_operation_id("draft-cross-connection", "mutation-terminal");

    match begin(&winning, &operation_id)? {
        AuthoringInferenceBegin::Started(_) => {}
        other => return Err(format!("the first reservation must start, got {other:?}").into()),
    }
    winning.complete(
        &operation_id,
        AuthoringInferenceOperationState::Proposed,
        reserved(),
        None,
        "cross-connection terminal outcome",
    )?;

    match begin(&other, &operation_id)? {
        AuthoringInferenceBegin::Replayed(record) => {
            assert_eq!(record.state, AuthoringInferenceOperationState::Proposed);
        }
        other => {
            return Err(format!("a terminal identity must replay, not start: {other:?}").into());
        }
    }
    Ok(())
}
