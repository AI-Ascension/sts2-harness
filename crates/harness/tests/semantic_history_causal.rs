// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryError, SemanticHistoryKind, SemanticHistoryOrigin, SemanticHistoryStore,
    SemanticHistoryTraversalLimits, SemanticHistoryValue,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

fn chained(branch: &str, depth: u64) -> SemanticHistoryStore {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        branch,
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        1,
    );
    for sequence in 2..=depth {
        let event_id = format!("event_{sequence}");
        let parent = format!("event_{}", sequence - 1);
        fixture::append_linked(
            &mut store,
            branch,
            &event_id,
            SemanticHistoryKind::ChoiceMade,
            sequence,
            &parent,
        );
    }
    store
}

#[test]
fn a_stated_chain_is_walked_backwards_to_its_root() {
    let store = chained(fixture::ROOT, 3);
    let explanation = store
        .explain(
            fixture::ROOT,
            "event_3",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    assert!(explanation.is_explained());
    assert_eq!(explanation.event_id, "event_3");
    assert!(!explanation.traversal.root_cause_unstated);
    assert!(!explanation.traversal.truncated);
    assert_eq!(explanation.traversal.links.len(), 2);
    assert_eq!(explanation.traversal.links[0].from_event_id, "event_2");
    assert_eq!(explanation.traversal.links[0].to_event_id, "event_3");
    assert_eq!(explanation.traversal.links[0].depth, 1);
    assert_eq!(explanation.traversal.links[1].from_event_id, "event_1");
    assert_eq!(explanation.traversal.links[1].depth, 2);
    assert_eq!(
        explanation.traversal.visited,
        vec![
            "event_3".to_owned(),
            "event_2".to_owned(),
            "event_1".to_owned()
        ]
    );
}

#[test]
fn an_event_with_no_stated_cause_reports_that_it_is_unexplained() {
    let store = chained(fixture::ROOT, 2);
    let explanation = store
        .explain(
            fixture::ROOT,
            "event_1",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    // Nothing is inferred from a difference between snapshots: no link, and the answer says so.
    assert!(!explanation.is_explained());
    assert!(explanation.traversal.root_cause_unstated);
    assert!(explanation.traversal.links.is_empty());
    assert_eq!(explanation.traversal.visited, vec!["event_1".to_owned()]);
}

#[test]
fn the_answer_carries_the_value_and_kind_it_explains() {
    let mut store = fixture::store();
    fixture::append_quantity(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::Damage,
        1,
        7,
    );
    let explanation = store
        .explain(
            fixture::ROOT,
            "event_1",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    assert_eq!(explanation.kind, SemanticHistoryKind::Damage);
    assert_eq!(explanation.value, Some(fixture::quantity(7, "hp")));
    assert_eq!(
        explanation.value.clone().expect("stated"),
        SemanticHistoryValue::Quantity {
            amount: 7,
            unit: "hp".to_owned()
        }
    );
}

#[test]
fn a_traversal_that_stops_at_the_depth_bound_reports_truncation() {
    let store = chained(fixture::ROOT, 4);
    let limits = SemanticHistoryTraversalLimits {
        max_depth: 1,
        max_visits: 8,
    };
    let explanation = store
        .explain(fixture::ROOT, "event_4", limits)
        .expect("explain");
    assert_eq!(explanation.traversal.links.len(), 1);
    // There is a stated cause beyond the bound; a bounded answer says so rather than ending quietly.
    assert!(explanation.traversal.truncated);
}

#[test]
fn a_traversal_that_stops_at_the_visit_bound_reports_truncation() {
    let store = chained(fixture::ROOT, 4);
    let limits = SemanticHistoryTraversalLimits {
        max_depth: 8,
        max_visits: 2,
    };
    let explanation = store
        .explain(fixture::ROOT, "event_4", limits)
        .expect("explain");
    assert_eq!(explanation.traversal.visited.len(), 2);
    assert_eq!(explanation.traversal.links.len(), 1);
    assert!(explanation.traversal.truncated);
}

#[test]
fn zero_or_oversized_traversal_limits_are_refused() {
    let store = chained(fixture::ROOT, 2);
    for limits in [
        SemanticHistoryTraversalLimits {
            max_depth: 0,
            max_visits: 8,
        },
        SemanticHistoryTraversalLimits {
            max_depth: 17,
            max_visits: 8,
        },
        SemanticHistoryTraversalLimits {
            max_depth: 8,
            max_visits: 0,
        },
        SemanticHistoryTraversalLimits {
            max_depth: 8,
            max_visits: 257,
        },
    ] {
        assert_eq!(limits.validate(), Err(SemanticHistoryError::Bounds));
        assert_eq!(
            store.explain(fixture::ROOT, "event_2", limits),
            Err(SemanticHistoryError::Bounds)
        );
    }
    assert!(SemanticHistoryTraversalLimits::bounded().validate().is_ok());
}

#[test]
fn a_stated_parent_that_is_not_in_this_branch_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    store.fork("branch_child", fixture::ROOT, 1).expect("fork");
    // The parent lives in the root branch; ancestry is followed through the lineage, not copied.
    let err = store
        .append(
            "branch_child",
            fixture::event("event_9", SemanticHistoryKind::ChoiceMade, 1, None),
            fixture::stated_parent("event_1"),
        )
        .expect_err("a parent from another branch is refused");
    assert_eq!(err, SemanticHistoryError::Causality);
}

#[test]
fn a_stated_parent_that_does_not_precede_its_child_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        5,
    );
    store.advance_epoch(2).expect("advance");
    let mut input = fixture::event("event_2", SemanticHistoryKind::ChoiceMade, 3, None);
    input.authority_epoch = 2;
    // A new epoch may restart host sequencing, but an event that sits later than its child is
    // still not its cause.
    let err = store
        .append(fixture::ROOT, input, fixture::stated_parent("event_1"))
        .expect_err("a parent must strictly precede its child");
    assert_eq!(err, SemanticHistoryError::ParentNotPreceding);
}

#[test]
fn a_stated_parent_from_another_epoch_is_refused() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        1,
    );
    store.advance_epoch(2).expect("advance");
    let mut input = fixture::event("event_2", SemanticHistoryKind::ChoiceMade, 2, None);
    input.authority_epoch = 2;
    let err = store
        .append(fixture::ROOT, input, fixture::stated_parent("event_1"))
        .expect_err("a parent from the previous epoch is refused");
    assert_eq!(err, SemanticHistoryError::Causality);
}

#[test]
fn an_unknown_event_cannot_be_explained() {
    let store = chained(fixture::ROOT, 2);
    assert_eq!(
        store.explain(
            fixture::ROOT,
            "event_absent",
            SemanticHistoryTraversalLimits::bounded()
        ),
        Err(SemanticHistoryError::Causality)
    );
    assert_eq!(
        store.explain(
            "branch_absent",
            "event_1",
            SemanticHistoryTraversalLimits::bounded()
        ),
        Err(SemanticHistoryError::Branch)
    );
}

#[test]
fn an_imported_event_may_not_state_a_causal_parent() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::ChoiceMade,
        1,
    );
    let mut input = fixture::event("event_2", SemanticHistoryKind::ChoiceMade, 2, None);
    input.origin = SemanticHistoryOrigin::Imported;
    // An imported event's causality was settled when it was captured; stating one now would invent it.
    assert_eq!(
        store
            .append(fixture::ROOT, input, fixture::stated_parent("event_1"))
            .expect_err("imported history cannot state a parent"),
        SemanticHistoryError::ImportedStatesParent
    );
}

#[test]
fn only_native_and_derived_origins_admit_a_stated_parent() {
    assert!(SemanticHistoryOrigin::Native.admits_stated_parent());
    assert!(SemanticHistoryOrigin::Derived.admits_stated_parent());
    assert!(!SemanticHistoryOrigin::Imported.admits_stated_parent());
    for origin in SemanticHistoryOrigin::ALL {
        assert_eq!(
            SemanticHistoryStore::origin_admits_parent(origin),
            origin != SemanticHistoryOrigin::Imported
        );
    }
}

#[test]
fn a_deep_chain_is_bounded_rather_than_followed_forever() {
    let store = chained(fixture::ROOT, 20);
    let explanation = store
        .explain(
            fixture::ROOT,
            "event_20",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    assert_eq!(explanation.traversal.links.len(), 16);
    assert_eq!(explanation.traversal.visited.len(), 17);
    assert!(explanation.traversal.truncated);
    assert!(explanation.is_explained());
}

#[test]
fn a_traversal_walks_only_the_ancestors_of_its_root() {
    let store = chained(fixture::ROOT, 5);
    let explanation = store
        .explain(
            fixture::ROOT,
            "event_2",
            SemanticHistoryTraversalLimits::bounded(),
        )
        .expect("explain");
    // Later events name this event as their parent; they are not part of its own explanation.
    assert_eq!(explanation.traversal.links.len(), 1);
    assert_eq!(
        explanation.traversal.visited,
        vec!["event_2".to_owned(), "event_1".to_owned()]
    );
    assert!(explanation.is_explained());
    assert!(explanation.value.is_none());
}
