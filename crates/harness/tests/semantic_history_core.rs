// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryAppend, SemanticHistoryCaptureWindow, SemanticHistoryCausalParent,
    SemanticHistoryCoverage, SemanticHistoryCoverageInterval, SemanticHistoryCoverageStatus,
    SemanticHistoryKind, SemanticHistoryNamespace, SemanticHistoryOrigin, SemanticHistoryStore,
    SemanticHistorySubject, SemanticHistorySubjectRole, SemanticHistoryValue,
    is_opaque_history_identity,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[test]
fn an_appended_event_is_read_back_with_its_order_and_identity() {
    let mut store = fixture::store();
    let appended = store
        .append(
            fixture::ROOT,
            fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("append");
    assert_eq!(appended, SemanticHistoryAppend::Recorded);
    let events = store.events(fixture::ROOT).expect("events");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].input.event_id, "event_1");
    assert_eq!(events[0].input.sequence, 1);
    assert_eq!(events[0].branch_id, fixture::ROOT);
}

#[test]
fn a_sequence_that_does_not_advance_is_refused_rather_than_reordered() {
    let mut store = fixture::store();
    store
        .append(
            fixture::ROOT,
            fixture::event("event_1", SemanticHistoryKind::CardPlayed, 4, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("first");
    // A backwards move would make stored order ambiguous.
    assert!(
        store
            .append(
                fixture::ROOT,
                fixture::event("event_2", SemanticHistoryKind::CardPlayed, 3, None),
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );
    assert!(
        store
            .append(
                fixture::ROOT,
                fixture::event("event_3", SemanticHistoryKind::CardPlayed, 4, None),
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );
}

#[test]
fn an_undeclared_sequence_jump_is_refused_and_a_declared_one_is_admitted() {
    let mut store = fixture::store();
    store
        .append(
            fixture::ROOT,
            fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("first");
    // Jumping over 2..3 without declaring a gap would close it silently.
    assert!(
        store
            .append(
                fixture::ROOT,
                fixture::event("event_2", SemanticHistoryKind::CardPlayed, 4, None),
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );

    let mut declared = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 3),
    )
    .expect("store opens");
    declared
        .append(
            fixture::ROOT,
            fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("first");
    declared
        .append(
            fixture::ROOT,
            fixture::event("event_2", SemanticHistoryKind::CardPlayed, 4, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("declared jump");
}

#[test]
fn a_captured_event_may_not_close_a_declared_gap() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 2),
    )
    .expect("store opens");
    store
        .append(
            fixture::ROOT,
            fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("first");
    // A captured event inside the declared dropped span would report an observation the capture
    // explicitly said it could not make.
    assert!(
        store
            .append(
                fixture::ROOT,
                fixture::event("event_2", SemanticHistoryKind::CardPlayed, 2, None),
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );
}

#[test]
fn a_gap_event_is_admitted_only_when_its_status_matches_the_declared_span() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 2),
    )
    .expect("store opens");
    store
        .append(
            fixture::ROOT,
            fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("first");
    let mut mismatched = fixture::gap_event("event_2", 2);
    mismatched.coverage = SemanticHistoryCoverage::gap(
        SemanticHistoryCoverageStatus::Unsupported,
        "capture_dropped",
    );
    assert!(
        store
            .append(
                fixture::ROOT,
                mismatched,
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );
    assert_eq!(
        store
            .append(
                fixture::ROOT,
                fixture::gap_event("event_3", 2),
                SemanticHistoryCausalParent::NotStated,
            )
            .expect("matching gap"),
        SemanticHistoryAppend::Recorded
    );
}

#[test]
fn a_quantity_change_must_state_a_value_rather_than_omit_it() {
    let mut store = fixture::store();
    // Omitting the value would make a damage event indistinguishable from one that changed nothing.
    assert!(
        store
            .append(
                fixture::ROOT,
                fixture::event("event_1", SemanticHistoryKind::Damage, 1, None),
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );
    // An explicit `Unavailable` is the honest way to say the boundary cannot state it.
    store
        .append(
            fixture::ROOT,
            fixture::event(
                "event_2",
                SemanticHistoryKind::Damage,
                1,
                Some(SemanticHistoryValue::Unavailable {
                    reason: "not_observed".to_owned(),
                }),
            ),
            SemanticHistoryCausalParent::NotStated,
        )
        .expect("unavailable is stated");
}

#[test]
fn only_a_live_instance_may_be_a_subject() {
    let mut store = fixture::store();
    for namespace in [
        SemanticHistoryNamespace::Definition,
        SemanticHistoryNamespace::Action,
        SemanticHistoryNamespace::Event,
    ] {
        let mut input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
        input.subjects = vec![SemanticHistorySubject {
            role: SemanticHistorySubjectRole::Actor,
            namespace,
            identity: "thing_1".to_owned(),
        }];
        assert!(
            store
                .append(fixture::ROOT, input, SemanticHistoryCausalParent::NotStated)
                .is_err(),
            "a {} subject is refused",
            namespace.name()
        );
    }
}

#[test]
fn a_path_shaped_identity_is_refused() {
    for candidate in ["/etc/passwd", "a/b", "..", "file://x", "a\\b", "with:colon"] {
        assert!(
            !is_opaque_history_identity(candidate),
            "{candidate} is not opaque"
        );
    }
    assert!(is_opaque_history_identity("instance_hero"));
    let mut store = fixture::store();
    let mut input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    input.event_id = "/etc/passwd".to_owned();
    assert!(
        store
            .append(fixture::ROOT, input, SemanticHistoryCausalParent::NotStated)
            .is_err()
    );
}

#[test]
fn an_epoch_mismatch_is_refused_and_advancing_resets_sequence_expectations() {
    let mut store = fixture::store();
    let mut stale = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    stale.authority_epoch = 2;
    assert!(
        store
            .append(fixture::ROOT, stale, SemanticHistoryCausalParent::NotStated)
            .is_err()
    );
    store.advance_epoch(2).expect("advance");
    // A new epoch may legitimately restart host sequencing.
    let mut fresh = fixture::event("event_2", SemanticHistoryKind::CardPlayed, 1, None);
    fresh.authority_epoch = 2;
    store
        .append(fixture::ROOT, fresh, SemanticHistoryCausalParent::NotStated)
        .expect("new epoch restarts sequencing");
}

#[test]
fn a_captured_event_carrying_a_gap_label_is_refused() {
    let mut store = fixture::store();
    let mut input = fixture::event("event_1", SemanticHistoryKind::CardPlayed, 1, None);
    input.coverage = SemanticHistoryCoverage {
        status: SemanticHistoryCoverageStatus::Captured,
        label: Some("looks_bad".to_owned()),
    };
    // A captured event's label would let a reason stand in for a value the boundary observed.
    assert!(
        store
            .append(fixture::ROOT, input, SemanticHistoryCausalParent::NotStated)
            .is_err()
    );
}

#[test]
fn a_before_capture_sequence_is_outside_this_capture_rather_than_stored() {
    let mut store =
        SemanticHistoryStore::open(fixture::binding(), fixture::ROOT, fixture::window(5))
            .expect("store opens");
    assert!(
        store.capture_window().is_before_capture(4),
        "sequence 4 predates capture start 5"
    );
    assert!(
        store
            .append(
                fixture::ROOT,
                fixture::event("event_1", SemanticHistoryKind::CardPlayed, 4, None),
                SemanticHistoryCausalParent::NotStated,
            )
            .is_err()
    );
}

#[test]
fn a_window_declaring_a_gap_before_capture_is_refused() {
    let mut window = SemanticHistoryCaptureWindow::complete(5);
    window.intervals.push(SemanticHistoryCoverageInterval {
        from_sequence: 2,
        to_sequence: 3,
        status: SemanticHistoryCoverageStatus::Dropped,
        label: "stale".to_owned(),
    });
    // A span before capture began is unknown history, not a gap in this capture.
    assert!(window.validate().is_err());
}

#[test]
fn every_kind_and_origin_reports_a_stable_name() {
    for kind in SemanticHistoryKind::ALL {
        assert!(!kind.name().is_empty());
    }
    for origin in SemanticHistoryOrigin::ALL {
        assert!(!origin.name().is_empty());
    }
    for namespace in SemanticHistoryNamespace::ALL {
        assert!(!namespace.name().is_empty());
    }
    assert_eq!(SemanticHistoryKind::ALL.len(), 14);
    assert_eq!(SemanticHistoryOrigin::ALL.len(), 3);
    assert_eq!(SemanticHistoryNamespace::ALL.len(), 4);
}
