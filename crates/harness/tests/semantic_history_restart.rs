// SPDX-License-Identifier: MIT

//! One run's history, written and restored.
//!
//! The store keeps a run's records, its lineage and its capture window in process. These tests
//! write that history out and read it back, and check that the restored history is the one the
//! writer had: the same records in the same order, the same stated chain, the same answers, and
//! the same refusals for the appends and rejoins that follow a restart.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryAppend, SemanticHistoryCausalParent, SemanticHistoryError, SemanticHistoryKind,
    SemanticHistoryQuery, SemanticHistoryReader, SemanticHistoryStore,
    SemanticHistoryTraversalLimits,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn a_restarted_history_serves_the_same_records_in_the_same_order() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_quantity(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::Damage,
        2,
        7,
    );
    fixture::append_linked(
        &mut store,
        fixture::ROOT,
        "event_3",
        SemanticHistoryKind::ChoiceMade,
        3,
        "event_2",
    );
    let restored = fixture::restore(&fixture::document(&store)).expect("restore");
    assert_eq!(restored.binding(), store.binding());
    assert_eq!(restored.capture_window(), store.capture_window());
    assert_eq!(restored.lineage(), store.lineage());
    assert_eq!(
        restored.events(fixture::ROOT).expect("events"),
        store.events(fixture::ROOT).expect("events")
    );
    assert_eq!(restored.len(fixture::ROOT).expect("len"), 3);
}

#[test]
fn a_restarted_history_still_explains_a_stated_chain() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_linked(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::ChoiceMade,
        2,
        "event_1",
    );
    let restored = fixture::restore(&fixture::document(&store)).expect("restore");
    let explanation = restored
        .explain(
            fixture::ROOT,
            "event_2",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    assert!(explanation.is_explained());
    assert_eq!(explanation.traversal.links.len(), 1);
    assert_eq!(explanation.traversal.links[0].from_event_id, "event_1");
}

#[test]
fn a_restarted_history_answers_the_same_bounded_query() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::ChoiceMade,
        2,
    );
    let restored = fixture::restore(&fixture::document(&store)).expect("restore");
    let query = SemanticHistoryQuery::branch(fixture::ROOT, 10);
    let before = SemanticHistoryReader::open(&store, fixture::ROOT)
        .expect("reader")
        .page(&query, None)
        .expect("page");
    let after = SemanticHistoryReader::open(&restored, fixture::ROOT)
        .expect("reader")
        .page(&query, None)
        .expect("page");
    assert_eq!(after.events, before.events);
    assert_eq!(after.gaps, before.gaps);
}

#[test]
fn re_appending_after_a_restart_replays_without_writing_a_second_event() {
    let mut store = fixture::store();
    let input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    assert_eq!(
        store
            .append(
                fixture::ROOT,
                input.clone(),
                SemanticHistoryCausalParent::NotStated
            )
            .expect("append"),
        SemanticHistoryAppend::Recorded
    );
    // The process ends here, and the history is read back by a new one.
    let mut restored = fixture::restore(&fixture::document(&store)).expect("restore");
    assert_eq!(
        restored
            .append(fixture::ROOT, input, SemanticHistoryCausalParent::NotStated)
            .expect("replay"),
        SemanticHistoryAppend::Replayed
    );
    assert_eq!(restored.len(fixture::ROOT).expect("len"), 1);
}

#[test]
fn a_rejoin_that_changes_a_recorded_event_is_refused_after_a_restart() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut restored = fixture::restore(&fixture::document(&store)).expect("restore");
    let conflicting = fixture::event("event_1", SemanticHistoryKind::ChoiceMade, 1, None);
    assert_eq!(
        restored
            .append(
                fixture::ROOT,
                conflicting,
                SemanticHistoryCausalParent::NotStated
            )
            .expect_err("a changed rejoin is refused"),
        SemanticHistoryError::MixedGeneration
    );
    assert_eq!(restored.len(fixture::ROOT).expect("len"), 1);
}

#[test]
fn an_append_that_moves_sequence_backwards_is_refused_after_a_restart() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_2",
        SemanticHistoryKind::CardPlayed,
        2,
    );
    let mut restored = fixture::restore(&fixture::document(&store)).expect("restore");
    let backwards = fixture::event("event_9", SemanticHistoryKind::CardPlayed, 2, None);
    assert_eq!(
        restored
            .append(
                fixture::ROOT,
                backwards,
                SemanticHistoryCausalParent::NotStated
            )
            .expect_err("a backwards sequence is refused"),
        SemanticHistoryError::Sequence
    );
}

#[test]
fn an_epoch_advance_that_restarts_sequencing_survives_a_restart() {
    let mut store = fixture::store();
    // The old epoch's last sequence is deliberately higher than the new epoch's first, so a
    // restoration that remembered it would refuse an honest restarted sequence.
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        5,
    );
    store.advance_epoch(2).expect("advance");
    let mut restarted = fixture::event("event_2", SemanticHistoryKind::CardPlayed, 1, None);
    restarted.authority_epoch = 2;
    store
        .append(
            fixture::ROOT,
            restarted,
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("a restarted sequence is admitted in a new epoch");
    let mut restored = fixture::restore(&fixture::document(&store)).expect("restore");
    let events = restored.events(fixture::ROOT).expect("events");
    assert_eq!(events[0].input.authority_epoch, 1);
    assert_eq!(events[1].input.authority_epoch, 2);
    // The restored branch expects the next sequence of the epoch now in force.
    let mut next = fixture::event("event_3", SemanticHistoryKind::CardPlayed, 2, None);
    next.authority_epoch = 2;
    assert_eq!(
        restored
            .append(fixture::ROOT, next, SemanticHistoryCausalParent::NotStated)
            .expect("the next sequence is admitted"),
        SemanticHistoryAppend::Recorded
    );
}

#[test]
fn a_branch_whose_last_record_is_in_an_older_epoch_restarts_its_sequence() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        5,
    );
    store.advance_epoch(2).expect("advance");
    store.fork("branch_child", fixture::ROOT, 5).expect("fork");
    let mut restored = fixture::restore(&fixture::document(&store)).expect("restore");
    // The root recorded nothing in the epoch now in force, so it is waiting for the first sequence of
    // that epoch rather than for the next one after the epoch it left behind.
    let mut next = fixture::event("event_2", SemanticHistoryKind::CardPlayed, 1, None);
    next.authority_epoch = 2;
    assert_eq!(
        restored
            .append(fixture::ROOT, next, SemanticHistoryCausalParent::NotStated)
            .expect("the restarted sequence is admitted"),
        SemanticHistoryAppend::Recorded
    );
}

#[test]
fn a_declared_gap_travels_with_the_history_across_a_restart() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 2),
    )
    .expect("store opens");
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store
        .append(
            fixture::ROOT,
            fixture::gap_event("event_2", 2),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("a declared gap event is admitted");
    let restored = fixture::restore(&fixture::document(&store)).expect("restore");
    assert_eq!(restored.capture_window(), store.capture_window());
    assert_eq!(
        restored.events(fixture::ROOT).expect("events"),
        store.events(fixture::ROOT).expect("events")
    );
    let query = SemanticHistoryQuery::branch(fixture::ROOT, 10);
    let page = SemanticHistoryReader::open(&restored, fixture::ROOT)
        .expect("reader")
        .page(&query, None)
        .expect("page");
    assert_eq!(page.gaps.len(), 1);
    assert_eq!(page.gaps[0], store.capture_window().intervals[0].status);
}

#[test]
fn a_forked_branch_survives_a_restart_without_inheriting_its_parent() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store.fork("branch_child", fixture::ROOT, 5).expect("fork");
    fixture::append_plain(
        &mut store,
        "branch_child",
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        5,
    );
    let restored = fixture::restore(&fixture::document(&store)).expect("restore");
    assert_eq!(restored.lineage().len(), 2);
    assert_eq!(restored.len(fixture::ROOT).expect("len"), 1);
    assert_eq!(restored.len("branch_child").expect("len"), 1);
    let child = restored.events("branch_child").expect("events");
    assert_eq!(child[0].input.kind, SemanticHistoryKind::ChoiceMade);
}

#[test]
fn forking_is_still_bounded_after_a_restart() {
    let mut store = fixture::store();
    let mut parent = fixture::ROOT.to_owned();
    let mut accepted = 0_usize;
    for depth in 1..=32_u64 {
        let child = format!("branch_{depth}");
        if let Some(error) = store.fork(&child, &parent, depth).err() {
            // Only the depth bound may stop this chain; any other refusal would mean the bound was
            // never the thing under test.
            assert_eq!(
                error,
                SemanticHistoryError::Lineage,
                "an unexpected refusal"
            );
            break;
        }
        accepted += 1;
        parent = child;
    }
    // The chain is genuinely bounded, and it got deep enough for the bound to be the thing that
    // stopped it rather than a first refusal.
    assert!(accepted > 1, "the chain stopped after {accepted} forks");
    let mut restored = fixture::restore(&fixture::document(&store)).expect("restore");
    assert_eq!(restored.lineage().len(), accepted + 1);
    // The restored lineage carries the depth, so the deepest chain cannot be extended here either.
    assert_eq!(
        restored
            .fork("branch_beyond", &parent, 99)
            .expect_err("a fork past the bound is refused"),
        SemanticHistoryError::Lineage
    );
    assert_eq!(
        restored
            .fork("branch_other", "branch_unknown", 1)
            .expect_err("a fork from an unknown branch is refused"),
        SemanticHistoryError::Branch
    );
}
