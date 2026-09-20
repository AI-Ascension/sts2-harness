// SPDX-License-Identifier: MIT

//! Backfilling saved history through the owned import port.
//!
//! A port can only hand back encoded bytes, so these tests build those bytes the way an
//! owner-side importer would and check that the harness re-derives every rule about a saved event
//! instead of trusting it. An imported record keeps the origin it was captured under and never
//! becomes a native observation, a re-import replays instead of duplicating, a batch that fails
//! partway writes nothing at all, and a dropped span travels with the history it was captured in.

#![allow(clippy::expect_used, dead_code)]

use sts2_harness::semantic_history::{
    SemanticHistoryCaptureWindow, SemanticHistoryCausalParent, SemanticHistoryCoverage,
    SemanticHistoryCoverageStatus, SemanticHistoryError, SemanticHistoryImportEvent,
    SemanticHistoryImportOutcome, SemanticHistoryKind, SemanticHistoryOrigin, SemanticHistoryQuery,
    SemanticHistoryReader, SemanticHistoryStore, import_saved_history,
};

#[path = "support/semantic_history_fixture.rs"]
mod fixture;

#[path = "support/semantic_history_import_doubles.rs"]
mod import_doubles;

use import_doubles::*;

#[test]
fn a_saved_batch_is_appended_and_served_as_imported_history() {
    let mut store = fixture::store();
    let outcomes =
        imported_into(&mut store, &batch("capture_0001", fixture::ROOT, 3)).expect("import");
    assert_eq!(
        outcomes,
        vec![SemanticHistoryImportOutcome {
            batch_id: "batch_0001".to_owned(),
            capture_id: "capture_0001".to_owned(),
            branch_id: fixture::ROOT.to_owned(),
            recorded: 3,
            replayed: 0,
        }]
    );
    let events = store.events(fixture::ROOT).expect("events");
    assert_eq!(events.len(), 3);
    // Imported history keeps the source label it was captured under; it is never native history.
    assert!(
        events
            .iter()
            .all(|event| event.input.origin == SemanticHistoryOrigin::Imported)
    );
    let page = SemanticHistoryReader::open(&store, fixture::ROOT)
        .expect("reader")
        .page(&SemanticHistoryQuery::branch(fixture::ROOT, 10), None)
        .expect("page");
    assert_eq!(page.events.len(), 3);
    assert!(page.gaps.is_empty());
}

#[test]
fn a_re_import_replays_the_whole_batch_without_duplicating_it() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 2);
    imported_into(&mut store, &saved).expect("first import");
    let second = imported_into(&mut store, &saved).expect("second import");
    assert_eq!(second[0].recorded, 0);
    assert_eq!(second[0].replayed, 2);
    assert_eq!(store.len(fixture::ROOT).expect("len"), 2);
}

#[test]
fn a_re_import_that_changes_a_saved_event_is_refused() {
    let mut store = fixture::store();
    imported_into(&mut store, &batch("capture_0001", fixture::ROOT, 2)).expect("import");
    let mut changed = batch("capture_0001", fixture::ROOT, 2);
    // The same saved identity now describes a different event, which is a conflict rather than a
    // correction: a backfill may not rewrite history it already imported.
    changed.events[0].input.episode = 2;
    assert_eq!(
        refused(&mut store, &changed),
        SemanticHistoryError::MixedGeneration
    );
    assert_eq!(store.len(fixture::ROOT).expect("len"), 2);
}

#[test]
fn a_batch_that_fails_partway_writes_nothing_at_all() {
    let mut store = fixture::store();
    fixture::append_plain(
        &mut store,
        fixture::ROOT,
        "event_1",
        SemanticHistoryKind::CardPlayed,
        1,
    );
    let mut conflicting = imported(
        "event_1",
        SemanticHistoryKind::Damage,
        2,
        Some(fixture::quantity(5, "hp")),
    );
    conflicting.input.origin = SemanticHistoryOrigin::Imported;
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    saved.events = vec![
        imported("event_2", SemanticHistoryKind::CardPlayed, 2, None),
        conflicting,
    ];
    assert_eq!(
        refused(&mut store, &saved),
        SemanticHistoryError::MixedGeneration
    );
    // The first event of the batch was admitted before the second was refused; neither is kept.
    assert_eq!(store.len(fixture::ROOT).expect("len"), 1);
    assert!(store.event(fixture::ROOT, "event_2").is_err());
}

#[test]
fn a_saved_event_that_claims_a_native_origin_is_refused() {
    let mut store = fixture::store();
    let mut saved = batch("capture_0001", fixture::ROOT, 1);
    saved.events[0].input.origin = SemanticHistoryOrigin::Native;
    assert_eq!(
        refused(&mut store, &saved),
        SemanticHistoryError::ImportedOrigin
    );
    saved.events[0].input.origin = SemanticHistoryOrigin::Derived;
    assert_eq!(
        refused(&mut store, &saved),
        SemanticHistoryError::ImportedOrigin
    );
    assert_eq!(store.len(fixture::ROOT).expect("len"), 0);
}

#[test]
fn an_imported_event_may_not_state_a_causal_parent() {
    let mut store = fixture::store();
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    let mut child = imported("event_2", SemanticHistoryKind::ChoiceMade, 2, None);
    child.causal_parent = fixture::stated_parent("event_1");
    saved.events = vec![
        imported("event_1", SemanticHistoryKind::ChoiceMade, 1, None),
        child,
    ];
    // An imported event's causality was settled when it was captured; stating one at import time
    // would be exactly the snapshot-difference inference this boundary refuses.
    assert_eq!(
        refused(&mut store, &saved),
        SemanticHistoryError::ImportedStatesParent
    );
}

#[test]
fn an_empty_batch_is_admitted_and_changes_nothing() {
    let mut store = fixture::store();
    let outcomes =
        imported_into(&mut store, &batch("capture_0001", fixture::ROOT, 0)).expect("import");
    assert_eq!(outcomes[0].recorded, 0);
    assert_eq!(outcomes[0].replayed, 0);
    assert_eq!(store.len(fixture::ROOT).expect("len"), 0);
}

#[test]
fn a_port_that_refuses_a_batch_leaves_the_store_untouched() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 1);
    let port = Saved {
        fail: Some(SemanticHistoryError::Port),
        ..Saved::new(fixture::binding()).holding("batch_0001", &saved)
    };
    assert_eq!(
        import_saved_history(&mut store, &port).expect_err("the port refused"),
        SemanticHistoryError::Port
    );
    assert_eq!(store.len(fixture::ROOT).expect("len"), 0);
}

#[test]
fn imported_history_survives_a_restart_with_its_source_label() {
    let mut store = fixture::store();
    let saved = batch("capture_0001", fixture::ROOT, 2);
    imported_into(&mut store, &saved).expect("import");
    let mut restored = SemanticHistoryStore::restore(&store.encode().expect("encode"))
        .expect("a restored history");
    let events = restored.events(fixture::ROOT).expect("events");
    assert!(
        events
            .iter()
            .all(|event| event.input.origin == SemanticHistoryOrigin::Imported)
    );
    // A rejoin after the restart still replays the saved batch rather than writing it twice.
    let after = imported_into(&mut restored, &saved).expect("re-import");
    assert_eq!(after[0].recorded, 0);
    assert_eq!(after[0].replayed, 2);
    assert_eq!(restored.len(fixture::ROOT).expect("len"), 2);
}

#[test]
fn an_imported_event_that_omits_the_detail_its_kind_requires_is_refused() {
    let mut store = fixture::store();
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    let mut damage = imported("event_1", SemanticHistoryKind::Damage, 1, None);
    damage.input.value = None;
    saved.events = vec![damage];
    // A saved capture that omitted the amount is refused rather than read as a zero.
    assert_eq!(
        refused(&mut store, &saved),
        SemanticHistoryError::InvalidField("value")
    );
}

#[test]
fn an_imported_gap_keeps_its_coverage_and_is_never_read_as_a_quiet_run() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 3),
    )
    .expect("open");
    let mut gap = fixture::gap_event("event_2", 2);
    gap.origin = SemanticHistoryOrigin::Imported;
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    saved.events = vec![
        imported("event_1", SemanticHistoryKind::CardPlayed, 1, None),
        SemanticHistoryImportEvent {
            input: gap,
            causal_parent: SemanticHistoryCausalParent::NotStated,
        },
        imported("event_3", SemanticHistoryKind::CardPlayed, 4, None),
    ];
    let outcomes = imported_into(&mut store, &saved).expect("import");
    assert_eq!(outcomes[0].recorded, 3);
    // The dropped span travelled with the saved history: it is disclosed, not closed.
    assert_eq!(
        store.coverage_at(fixture::ROOT, 2).expect("coverage"),
        Some(SemanticHistoryCoverageStatus::Dropped)
    );
    let page = SemanticHistoryReader::open(&store, fixture::ROOT)
        .expect("reader")
        .page(
            &SemanticHistoryQuery {
                from_sequence: Some(1),
                to_sequence: Some(4),
                ..SemanticHistoryQuery::branch(fixture::ROOT, 10)
            },
            None,
        )
        .expect("page");
    assert_eq!(page.gaps, vec![SemanticHistoryCoverageStatus::Dropped]);
}

#[test]
fn an_imported_event_inside_a_declared_gap_that_claims_capture_is_refused() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 3),
    )
    .expect("open");
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    saved.events = vec![imported(
        "event_2",
        SemanticHistoryKind::CardPlayed,
        2,
        None,
    )];
    assert_eq!(refused(&mut store, &saved), SemanticHistoryError::Coverage);
}

#[test]
fn an_imported_event_before_capture_began_is_refused() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        SemanticHistoryCaptureWindow::complete(5),
    )
    .expect("open");
    let mut saved = batch("capture_0001", fixture::ROOT, 1);
    saved.events[0].input.sequence = 4;
    assert_eq!(refused(&mut store, &saved), SemanticHistoryError::Coverage);
}

#[test]
fn an_imported_gap_whose_coverage_contradicts_the_window_is_refused() {
    let mut store = SemanticHistoryStore::open(
        fixture::binding(),
        fixture::ROOT,
        fixture::window_with_gap(1, 2, 3),
    )
    .expect("open");
    let mut saved = batch("capture_0001", fixture::ROOT, 0);
    let mut unsupported = fixture::gap_event("event_2", 2);
    unsupported.coverage = SemanticHistoryCoverage::gap(
        SemanticHistoryCoverageStatus::Unsupported,
        "capture_dropped",
    );
    unsupported.origin = SemanticHistoryOrigin::Imported;
    saved.events = vec![SemanticHistoryImportEvent {
        input: unsupported,
        causal_parent: SemanticHistoryCausalParent::NotStated,
    }];
    assert_eq!(refused(&mut store, &saved), SemanticHistoryError::Coverage);
}
