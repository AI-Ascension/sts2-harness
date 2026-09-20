// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryAppend, SemanticHistoryCausalParent, SemanticHistoryCoverageStatus,
    SemanticHistoryError, SemanticHistoryKind, SemanticHistoryRetention, SemanticHistoryStore,
    SemanticHistoryValue,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

const MAX_LINEAGE_DEPTH: usize = 16;

#[test]
fn re_appending_identical_content_replays_without_writing() {
    let mut store = fixture::store();
    let input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    assert_eq!(
        store
            .append(
                fixture::ROOT,
                input.clone(),
                SemanticHistoryCausalParent::NotStated
            )
            .expect("first append"),
        SemanticHistoryAppend::Recorded
    );
    // The same identity with the same content is the same event, so nothing is written twice.
    assert_eq!(
        store
            .append(fixture::ROOT, input, SemanticHistoryCausalParent::NotStated)
            .expect("replay"),
        SemanticHistoryAppend::Replayed
    );
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
}

#[test]
fn re_appending_one_identity_with_different_content_is_a_conflict() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut conflicting = fixture::event(
        "event_1",
        SemanticHistoryKind::Damage,
        1,
        Some(fixture::quantity(3, "hp")),
    );
    conflicting.origin = sts2_harness::semantic_history::SemanticHistoryOrigin::Derived;
    // A history that can be edited in place cannot be trusted to explain a run.
    assert_eq!(
        store
            .append(
                fixture::ROOT,
                conflicting,
                SemanticHistoryCausalParent::NotStated
            )
            .expect_err("a conflicting re-append is refused"),
        SemanticHistoryError::MixedGeneration
    );
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
}

#[test]
fn a_forked_branch_starts_empty_and_does_not_inherit_its_parent() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store.fork("branch_child", fixture::ROOT, 1).expect("fork");
    assert!(store.events("branch_child").expect("events").is_empty());
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
    assert_eq!(store.lineage().len(), 2);
    assert_eq!(
        store.lineage()[1].parent_branch_id.as_deref(),
        Some(fixture::ROOT)
    );
    fixture::append_plain(
        &mut store,
        "branch_child",
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    assert_eq!(store.len("branch_child").expect("len"), 1);
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
}

#[test]
fn forking_from_an_unknown_branch_or_onto_an_existing_branch_is_refused() {
    let mut store = fixture::store();
    assert_eq!(
        store.fork("branch_child", "branch_absent", 1),
        Err(SemanticHistoryError::Branch)
    );
    store
        .fork("branch_child", fixture::ROOT, 1)
        .expect("first fork");
    assert_eq!(
        store.fork("branch_child", fixture::ROOT, 1),
        Err(SemanticHistoryError::Branch)
    );
    assert_eq!(
        store.fork(fixture::ROOT, fixture::ROOT, 1),
        Err(SemanticHistoryError::Branch)
    );
}

#[test]
fn a_lineage_deeper_than_the_bound_is_refused() {
    let mut store = fixture::store();
    let mut previous = fixture::ROOT.to_owned();
    for depth in 1..MAX_LINEAGE_DEPTH {
        let branch = format!("branch_{depth:02}");
        store
            .fork(&branch, &previous, depth as u64)
            .expect("a fork inside the bound");
        previous = branch;
    }
    // One more edge would make ancestry deeper than this boundary will follow.
    assert_eq!(
        store.fork("branch_16", &previous, 16),
        Err(SemanticHistoryError::Lineage)
    );
}

#[test]
fn retention_redacts_the_value_and_keeps_the_event() {
    let mut store = fixture::store();
    let input = fixture::event(
        "event_1",
        SemanticHistoryKind::Damage,
        1,
        Some(fixture::quantity(9, "hp")),
    );
    store
        .append(
            fixture::ROOT,
            input.clone(),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("append");
    let digest = store
        .event(fixture::ROOT, "event_1")
        .expect("event")
        .content_digest
        .clone();
    assert_eq!(
        store
            .apply_retention(fixture::ROOT, "event_1", true)
            .expect("redact"),
        SemanticHistoryRetention::Redacted
    );
    let stored = store.event(fixture::ROOT, "event_1").expect("event");
    // The event, its coverage and its identity survive; only the payload is replaced.
    assert_eq!(stored.content_digest, digest);
    assert_eq!(
        stored.input.value,
        Some(SemanticHistoryValue::Unavailable {
            reason: "retention".to_owned()
        })
    );
    assert!(stored.input.coverage.status.is_observed());
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
    // The digest still identifies the event, so the original content is recognised as a replay.
    assert_eq!(
        store
            .append(fixture::ROOT, input, SemanticHistoryCausalParent::NotStated)
            .expect("replay after redaction"),
        SemanticHistoryAppend::Replayed
    );
}

#[test]
fn retention_without_redaction_changes_nothing() {
    let mut store = fixture::store();
    fixture::append_quantity(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::Damage,
        1,
        9,
    );
    assert_eq!(
        store
            .apply_retention(fixture::ROOT, "event_1", false)
            .expect("retain"),
        SemanticHistoryRetention::Retained
    );
    assert_eq!(
        store
            .event(fixture::ROOT, "event_1")
            .expect("event")
            .input
            .value,
        Some(fixture::quantity(9, "hp"))
    );
}

#[test]
fn retention_on_an_unknown_event_or_branch_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    assert_eq!(
        store.apply_retention(fixture::ROOT, "event_absent", true),
        Err(SemanticHistoryError::Causality)
    );
    assert_eq!(
        store.apply_retention("branch_absent", "event_1", true),
        Err(SemanticHistoryError::Branch)
    );
}

#[test]
fn advancing_the_epoch_keeps_each_event_in_the_epoch_it_was_recorded_under() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store.advance_epoch(2).expect("advance");
    let mut fresh = fixture::event("event_2", SemanticHistoryKind::CardPlayed, 1, None);
    fresh.authority_epoch = 2;
    store
        .append(fixture::ROOT, fresh, SemanticHistoryCausalParent::NotStated)
        .expect("a new epoch may restart host sequencing");
    let events = store.events(fixture::ROOT).expect("events");
    assert_eq!(events[0].input.authority_epoch, 1);
    assert_eq!(events[1].input.authority_epoch, 2);
    assert_eq!(store.binding().authority_epoch, 2);
}

#[test]
fn advancing_to_the_same_or_an_earlier_epoch_is_refused() {
    let mut store = fixture::store();
    assert_eq!(store.advance_epoch(0), Err(SemanticHistoryError::Epoch));
    assert_eq!(store.advance_epoch(1), Err(SemanticHistoryError::Epoch));
    assert_eq!(store.advance_epoch(2), Ok(()));
    assert_eq!(store.advance_epoch(2), Err(SemanticHistoryError::Epoch));
}

#[test]
fn coverage_at_reports_a_recorded_gap_and_nothing_for_an_undeclared_sequence() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 2),
    )
    .expect("open");
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
        .expect("declared gap");
    assert_eq!(
        store.coverage_at(fixture::ROOT, 1).expect("coverage"),
        Some(SemanticHistoryCoverageStatus::Captured)
    );
    assert_eq!(
        store.coverage_at(fixture::ROOT, 2).expect("coverage"),
        Some(SemanticHistoryCoverageStatus::Dropped)
    );
    assert_eq!(store.coverage_at(fixture::ROOT, 3).expect("coverage"), None);
    assert_eq!(
        store.coverage_at("branch_absent", 1),
        Err(SemanticHistoryError::Branch)
    );
}

#[test]
fn length_and_emptiness_are_tracked_per_branch() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store.fork("branch_child", fixture::ROOT, 1).expect("fork");
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
    assert!(!store.is_empty(fixture::ROOT).expect("empty"));
    assert_eq!(store.len("branch_child").expect("len"), 0);
    assert!(store.is_empty("branch_child").expect("empty"));
    assert_eq!(
        store.len("branch_absent"),
        Err(SemanticHistoryError::Branch)
    );
    assert_eq!(
        store.is_empty("branch_absent"),
        Err(SemanticHistoryError::Branch)
    );
}
