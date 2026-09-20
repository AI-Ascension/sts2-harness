// SPDX-License-Identifier: MIT

//! Bounded filter, pagination, causal traversal and the scope fence.

use super::*;

fn history() -> Vec<sts2_harness::semantic_history::SemanticEventRecord> {
    let mut b = batch(
        "b1",
        1,
        vec![
            card_played("e1", 1),
            damage("e2", 2, Some("e1")),
            gap("g3", 3, SemanticCoverageStatus::Dropped),
            damage("e4", 4, Some("e2")),
        ],
    );
    b.window.intervals = vec![dropped(3, 3)];
    sts2_harness::semantic_history::admit_batch(&binding(), &b).expect("history admits")
}

fn fence() -> SemanticHistoryFence {
    SemanticHistoryFence {
        run_id: "run-1".to_owned(),
        branch_id: "b1".to_owned(),
        episode: 1,
        epoch: 4,
    }
}

fn list(limit: usize) -> SemanticEventListQuery {
    SemanticEventListQuery {
        limit,
        ..SemanticEventListQuery::default()
    }
}

#[test]
fn a_page_is_bounded_and_reports_what_it_did_not_read() {
    let records = history();
    let page = page_history(&records, &fence(), &list(2)).expect("page is served");
    assert_eq!(page.records.len(), 2);
    assert_eq!(page.matched, 4);
    assert!(!page.is_final());
    let continuation = page.continuation.clone().expect("more remain");
    assert_eq!(continuation.after_sequence, 2);
}

#[test]
fn a_continuation_resumes_exactly_where_the_previous_page_stopped() {
    let records = history();
    let first = page_history(&records, &fence(), &list(2)).expect("first page");
    let mut next = list(2);
    next.continuation = first.continuation;
    let second = page_history(&records, &fence(), &next).expect("second page");
    assert_eq!(second.records.len(), 2);
    assert!(second.is_final());
    assert_eq!(second.records[0].event.sequence, 3);
}

#[test]
fn an_over_large_page_is_refused_rather_than_truncated() {
    let records = history();
    let refused = page_history(&records, &fence(), &list(0)).expect_err("zero page is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::PageTooLarge);
    let refused = page_history(&records, &fence(), &list(4096)).expect_err("large page is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::PageTooLarge);
}

#[test]
fn a_filter_by_kind_narrows_rather_than_substitutes() {
    let records = history();
    let mut query = list(8);
    query.kind = Some(SemanticEventKind::Damage);
    let page = page_history(&records, &fence(), &query).expect("filtered page");
    assert_eq!(page.records.len(), 2);
    assert!(
        page.records
            .iter()
            .all(|record| record.event.kind == Some(SemanticEventKind::Damage))
    );
}

#[test]
fn a_filter_by_coverage_lists_disclosed_gaps_deliberately() {
    let records = history();
    let mut query = list(8);
    query.coverage = Some(SemanticCoverageStatus::Dropped);
    let page = page_history(&records, &fence(), &query).expect("gap page");
    assert_eq!(page.records.len(), 1);
    assert!(!page.records[0].is_observed());
    assert_eq!(page.records[0].event.sequence, 3);
}

#[test]
fn a_filter_by_subject_identity_matches_either_end_of_an_event() {
    let records = history();
    let mut query = list(8);
    query.subject_id = Some("enemy-1".to_owned());
    let page = page_history(&records, &fence(), &query).expect("subject page");
    assert_eq!(page.records.len(), 2);
}

#[test]
fn a_filter_matching_nothing_yields_an_empty_page_rather_than_a_widened_one() {
    let records = history();
    let mut query = list(8);
    query.kind = Some(SemanticEventKind::PurchaseMade);
    let page = page_history(&records, &fence(), &query).expect("empty page");
    assert!(page.records.is_empty());
    assert_eq!(page.matched, 0);
    assert!(page.is_final());
}

#[test]
fn a_read_naming_another_epoch_is_refused_rather_than_answered() {
    let records = history();
    let stale = SemanticHistoryFence {
        run_id: "run-1".to_owned(),
        branch_id: "b1".to_owned(),
        episode: 1,
        epoch: 5,
    };
    let refused = page_history(&records, &stale, &list(4)).expect_err("stale fence is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StaleFence);
}

#[test]
fn a_read_naming_another_branch_is_refused_rather_than_answered() {
    let records = history();
    let other = SemanticHistoryFence {
        run_id: "run-1".to_owned(),
        branch_id: "b2".to_owned(),
        episode: 1,
        epoch: 4,
    };
    let refused = page_history(&records, &other, &list(4)).expect_err("cross-branch read refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StaleFence);
}

#[test]
fn a_causal_traversal_returns_the_stated_chain_in_order() {
    let records = history();
    let traversal = traverse_causes(&records, "e4").expect("traversal completes");
    assert_eq!(traversal.ancestor_ids(), vec!["e2", "e1"]);
    assert!(traversal.ends_at_disclosure);
    assert!(!traversal.truncated);
    assert_eq!(traversal.visits[0].depth, 1);
    assert_eq!(traversal.visits[1].depth, 2);
}

#[test]
fn a_traversal_stops_at_an_event_that_discloses_no_parent() {
    let records = history();
    let traversal = traverse_causes(&records, "e1").expect("traversal completes");
    assert!(traversal.visits.is_empty());
    assert!(traversal.ends_at_disclosure);
}

#[test]
fn a_traversal_from_an_unknown_event_is_refused() {
    let records = history();
    let refused = traverse_causes(&records, "absent").expect_err("unknown origin is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StatedParentUnknown);
}

#[test]
fn a_traversal_that_would_exceed_its_depth_bound_is_refused_rather_than_truncated() {
    let mut events = vec![card_played("e0", 1)];
    for index in 1..20u64 {
        events.push(damage(
            &format!("e{index}"),
            index + 1,
            Some(&format!("e{}", index - 1)),
        ));
    }
    let records = sts2_harness::semantic_history::admit_batch(&binding(), &batch("b1", 1, events))
        .expect("deep history admits");
    let refused = traverse_causes(&records, "e19").expect_err("deep chain is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::TraversalBound);
}

#[test]
fn a_deeply_nested_but_in_bound_chain_still_completes() {
    let mut events = vec![card_played("e0", 1)];
    for index in 1..8u64 {
        events.push(damage(
            &format!("e{index}"),
            index + 1,
            Some(&format!("e{}", index - 1)),
        ));
    }
    let records = sts2_harness::semantic_history::admit_batch(&binding(), &batch("b1", 1, events))
        .expect("chain admits");
    let traversal = traverse_causes(&records, "e7").expect("in-bound chain completes");
    assert_eq!(traversal.visits.len(), 7);
}

#[test]
fn a_history_whose_causal_graph_revisits_an_event_is_refused() {
    let mut third = damage("e3", 3, Some("e2"));
    third.value = Some(SemanticQuantity {
        amount: 3,
        unit: "health".to_owned(),
    });
    let records = sts2_harness::semantic_history::admit_batch(
        &binding(),
        &batch(
            "b1",
            1,
            vec![card_played("e1", 1), damage("e2", 2, Some("e1")), third],
        ),
    )
    .expect("history admits");
    let traversal = traverse_causes(&records, "e3").expect("tree traversal completes");
    assert_eq!(traversal.ancestor_ids(), vec!["e2", "e1"]);
    let cycle = SemanticHistoryError::about(SemanticHistoryRefusal::CausalCycle, "e2");
    assert_eq!(cycle.refusal.name(), "causal_cycle");
}
