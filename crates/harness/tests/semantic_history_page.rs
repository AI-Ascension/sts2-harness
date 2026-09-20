// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryCoverageStatus, SemanticHistoryError, SemanticHistoryKind, SemanticHistoryQuery,
    SemanticHistoryReader, SemanticHistoryStore,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn a_page_discloses_the_declared_gaps_it_cannot_show() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 3),
    )
    .expect("open");
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
        4,
    );
    let reader = SemanticHistoryReader::open(&store, fixture::ROOT).expect("reader");
    let query = SemanticHistoryQuery {
        from_sequence: Some(1),
        to_sequence: Some(4),
        ..SemanticHistoryQuery::branch(fixture::ROOT, 10)
    };
    let page = reader.page(&query, None).expect("page");
    assert_eq!(page.events.len(), 2);
    // A short page is never read as a quiet run: the declared gap is disclosed with it.
    assert_eq!(page.gaps, vec![SemanticHistoryCoverageStatus::Dropped]);
    assert!(!page.before_capture);
}
#[test]
fn a_page_that_reaches_before_capture_says_so() {
    let store = SemanticHistoryStore::open(fixture::binding(), fixture::ROOT, fixture::window(5))
        .expect("open");
    let reader = SemanticHistoryReader::open(&store, fixture::ROOT).expect("reader");
    let query = SemanticHistoryQuery {
        from_sequence: Some(4),
        ..SemanticHistoryQuery::branch(fixture::ROOT, 10)
    };
    let page = reader.page(&query, None).expect("page");
    assert!(page.before_capture);
    assert!(page.events.is_empty());
}
#[test]
fn a_continuation_is_bound_to_its_query_and_generation() {
    let store = fixture::store_with_events(3);
    let reader = SemanticHistoryReader::open(&store, fixture::ROOT).expect("reader");
    assert_eq!(reader.generation(), 3);
    let query = SemanticHistoryQuery::branch(fixture::ROOT, 2);
    let first = reader.page(&query, None).expect("first page");
    assert_eq!(first.events.len(), 2);
    assert_eq!(first.events[1].input.sequence, 2);
    let continuation = first.continuation.clone().expect("continuation");
    assert_eq!(continuation.cursor.next_sequence, 3);
    let second = reader
        .page(&query, Some(&continuation))
        .expect("second page");
    assert_eq!(second.events.len(), 1);
    assert!(second.continuation.is_none());
    // The same cursor cannot answer a different question.
    let mut other = SemanticHistoryQuery::branch(fixture::ROOT, 2);
    other.kind = Some(SemanticHistoryKind::Damage);
    assert_eq!(
        reader.page(&other, Some(&continuation)),
        Err(SemanticHistoryError::Continuation)
    );
    // Nor can it be replayed against a history that has since advanced.
    let mut stale = continuation;
    stale.cursor.generation = 99;
    assert_eq!(
        reader.page(&query, Some(&stale)),
        Err(SemanticHistoryError::Continuation)
    );
}
#[test]
fn a_page_that_covers_every_match_reports_no_continuation() {
    let store = fixture::store_with_events(3);
    let reader = SemanticHistoryReader::open(&store, fixture::ROOT).expect("reader");
    let page = reader
        .page(&SemanticHistoryQuery::branch(fixture::ROOT, 3), None)
        .expect("page");
    assert_eq!(page.events.len(), 3);
    assert!(page.continuation.is_none());
}
#[test]
fn a_summary_counts_gaps_separately_from_observations() {
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
            sts2_harness::semantic_history::SemanticHistoryCausalParent::NotStated,
        )
        .expect("declared gap");
    let reader = SemanticHistoryReader::open(&store, fixture::ROOT).expect("reader");
    let summary = reader.summary(fixture::ROOT).expect("summary");
    assert_eq!(summary.branch_id, fixture::ROOT);
    assert_eq!(summary.total, 2);
    assert_eq!(summary.captured, 1);
    assert_eq!(summary.gaps, 1);
    assert_eq!(summary.first_sequence, Some(1));
    assert_eq!(summary.last_sequence, Some(2));
}
