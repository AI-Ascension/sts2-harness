// SPDX-License-Identifier: MIT

//! Backfill: the owned mod port, the labels restored history keeps, and its span rule.

use super::*;
use sts2_harness::semantic_history::{
    SemanticBackfillAuthority, SemanticBackfillOutcome, SemanticBackfillPort,
    SemanticBackfillRequest, SemanticCaptureWindow, SemanticEventBatch, restore_saved_history,
};

/// A mod port that returns whatever it was handed, so the tests exercise the harness's own rules.
struct ModPort {
    batch: Option<SemanticEventBatch>,
    calls: usize,
}

impl ModPort {
    fn of(batch: Option<SemanticEventBatch>) -> Self {
        Self { batch, calls: 0 }
    }

    fn yielding(batch: SemanticEventBatch) -> Self {
        Self::of(Some(batch))
    }
}

impl SemanticBackfillPort for ModPort {
    fn restore(
        &mut self,
        _request: &SemanticBackfillRequest,
    ) -> Result<SemanticEventBatch, SemanticHistoryError> {
        self.calls += 1;
        self.batch
            .clone()
            .ok_or_else(|| SemanticHistoryError::new(SemanticHistoryRefusal::Storage))
    }
}

/// Restores through a granted port; the ungranted path is asserted where it is the subject.
fn granted(
    store: &mut SemanticHistoryStore,
    port: &mut ModPort,
    request: &SemanticBackfillRequest,
) -> Result<SemanticBackfillOutcome, SemanticHistoryError> {
    restore_saved_history(store, port, SemanticBackfillAuthority::Granted, request)
}

/// The refusal a granted restore produced, keeping its subject so callers can assert on it.
fn refusal(
    store: &mut SemanticHistoryStore,
    port: &mut ModPort,
    request: &SemanticBackfillRequest,
) -> SemanticHistoryError {
    granted(store, port, request).expect_err("the restore must be refused")
}

/// Restores `span` through a granted port that yields it, for the rules under test.
fn landed(
    store: &mut SemanticHistoryStore,
    span: SemanticEventBatch,
    request: &SemanticBackfillRequest,
) -> SemanticBackfillOutcome {
    granted(store, &mut ModPort::yielding(span), request).expect("restore lands")
}

/// The whole refusal a granted restore of `span` produced, so callers can assert on its subject.
fn denial(
    store: &mut SemanticHistoryStore,
    span: SemanticEventBatch,
    request: &SemanticBackfillRequest,
) -> SemanticHistoryError {
    refusal(store, &mut ModPort::yielding(span), request)
}

fn restored(event_id: &str, sequence: u64) -> SemanticEventInput {
    let mut event = damage(event_id, sequence, None);
    event.origin = Some(SemanticEventOrigin::Imported);
    event
}

fn restored_gap(event_id: &str, sequence: u64, label: Option<&str>) -> SemanticEventInput {
    let mut event = gap(event_id, sequence, SemanticCoverageStatus::Dropped);
    event.coverage.label = label.map(str::to_owned);
    event
}

/// A restored span declared by its own source, to sit ahead of a capture starting at 4.
fn saved_span(events: Vec<SemanticEventInput>, start: u64) -> SemanticEventBatch {
    SemanticEventBatch {
        scope: scope("b1"),
        window: SemanticCaptureWindow {
            capture_start_sequence: start,
            history_before_capture: start != 1,
            intervals: Vec::new(),
        },
        events,
    }
}

/// A history whose capture began at 4, so a saved span ending at 3 can be restored ahead of it.
fn late_capture(name: &str) -> SemanticHistoryStore {
    let mut store = SemanticHistoryStore::in_memory(path(name));
    let mut b = append("op-live", "b1", 4, vec![damage("e4", 4, None)]);
    b.batch.window.history_before_capture = true;
    store.append(&b).expect("live capture lands");
    store
}

/// A history whose capture began at 2, so a saved span holding only sequence 1 reaches the beginning.
fn capture_after_one_saved_event(name: &str) -> SemanticHistoryStore {
    let mut store = SemanticHistoryStore::in_memory(path(name));
    let mut b = append("op-live", "b1", 2, vec![damage("e2", 2, None)]);
    b.batch.window.history_before_capture = true;
    store.append(&b).expect("live capture lands");
    store
}

fn request(start: u64, last: u64) -> SemanticBackfillRequest {
    SemanticBackfillRequest {
        operation_id: "restore-1".to_owned(),
        fence: SemanticHistoryFence {
            run_id: "run-1".to_owned(),
            branch_id: "b1".to_owned(),
            episode: 1,
            epoch: 4,
        },
        binding: binding(),
        first_sequence: start,
        last_sequence: last,
    }
}

/// The two-record saved span the tests restore ahead of a capture starting at 4.
fn two_saved() -> SemanticEventBatch {
    saved_span(vec![restored("r2", 2), restored("r3", 3)], 2)
}

#[test]
fn saved_history_is_not_restored_until_the_harness_grants_the_owned_mod_port() {
    let mut store = late_capture("bf-ungranted");
    let mut port = ModPort::yielding(two_saved());
    let refused = restore_saved_history(
        &mut store,
        &mut port,
        SemanticBackfillAuthority::NotGranted,
        &request(2, 3),
    )
    .expect_err("an ungranted port restores nothing");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::BackfillNotGranted);
    assert_eq!(
        port.calls, 0,
        "the mod is never asked for native history the harness has not authorized"
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(store.window("b1").unwrap().capture_start_sequence, 4);
}

#[test]
fn a_granted_restore_puts_saved_history_ahead_of_the_retained_capture() {
    let mut store = late_capture("bf-applies");
    let outcome = landed(&mut store, two_saved(), &request(2, 3));
    assert_eq!(outcome.restored, 2);
    assert_eq!(outcome.total, 3);
    assert_eq!(outcome.capture_start_sequence, 2);
    let window = store.window("b1").unwrap();
    assert_eq!(window.capture_start_sequence, 2);
    assert!(window.history_before_capture);
    assert_eq!(
        store
            .records("b1")
            .unwrap()
            .iter()
            .map(|record| record.event.sequence)
            .collect::<Vec<_>>(),
        vec![2, 3, 4]
    );
}

#[test]
fn restored_history_keeps_the_imported_origin_it_was_restored_with() {
    let mut store = late_capture("bf-labels");
    landed(&mut store, two_saved(), &request(2, 3));
    let records = store.records("b1").unwrap();
    assert_eq!(
        records[0].event.origin,
        Some(SemanticEventOrigin::Imported),
        "restored history can never read as an event this harness observed"
    );
    assert_eq!(
        records[2].event.origin,
        Some(SemanticEventOrigin::Native),
        "live capture keeps its own origin"
    );
}

#[test]
fn a_restore_that_reaches_the_first_sequence_leaves_nothing_before_capture() {
    let mut store = capture_after_one_saved_event("bf-from-start");
    let outcome = landed(
        &mut store,
        saved_span(vec![restored("r1", 1)], 1),
        &request(1, 1),
    );
    assert_eq!(outcome.restored, 1);
    assert_eq!(outcome.capture_start_sequence, 1);
    let window = store.window("b1").unwrap();
    assert_eq!(window.capture_start_sequence, 1);
    assert!(
        !window.history_before_capture,
        "once the restore reaches the first sequence there is no history before the capture"
    );
    assert_eq!(
        store
            .records("b1")
            .unwrap()
            .iter()
            .map(|record| record.event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn a_restored_record_claiming_a_native_origin_is_refused_rather_than_relabelled() {
    let mut store = late_capture("bf-native-claim");
    let mut claimed = damage("r3", 3, None);
    claimed.origin = Some(SemanticEventOrigin::Native);
    let refused = denial(&mut store, saved_span(vec![claimed], 3), &request(3, 3));
    assert_eq!(refused.refusal, SemanticHistoryRefusal::BackfillLabel);
    assert_eq!(refused.subject.as_deref(), Some("r3"));
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(store.window("b1").unwrap().capture_start_sequence, 4);
}

#[test]
fn a_restored_gap_keeps_the_coverage_label_its_source_stated() {
    let mut store = late_capture("bf-gap-label");
    let mut span = saved_span(vec![restored_gap("r3", 3, Some("save truncated"))], 3);
    span.window.intervals = vec![dropped(3, 3)];
    landed(&mut store, span, &request(3, 3));
    let records = store.records("b1").unwrap();
    assert!(!records[0].is_observed());
    assert_eq!(
        records[0].event.coverage.label.as_deref(),
        Some("save truncated"),
        "a restored gap keeps the label naming what its source could not see"
    );
    assert_eq!(store.window("b1").unwrap().intervals, vec![dropped(3, 3)]);
}

#[test]
fn a_restored_gap_that_lost_its_coverage_label_is_refused() {
    let mut store = late_capture("bf-gap-unlabelled");
    let mut span = saved_span(vec![restored_gap("r3", 3, None)], 3);
    span.window.intervals = vec![dropped(3, 3)];
    assert_eq!(
        denial(&mut store, span, &request(3, 3)).refusal,
        SemanticHistoryRefusal::BackfillLabel
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
}

#[test]
fn a_restored_span_that_does_not_abut_the_retained_start_is_refused() {
    let mut store = late_capture("bf-hole");
    let refused = denial(
        &mut store,
        saved_span(vec![restored("r2", 2)], 2),
        &request(2, 2),
    );
    assert_eq!(refused.refusal, SemanticHistoryRefusal::BackfillSpan);
    assert_eq!(
        refused.subject.as_deref(),
        Some("b1"),
        "sequence 3 would be an undeclared hole ahead of the retained capture"
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(store.window("b1").unwrap().capture_start_sequence, 4);
}

#[test]
fn a_restored_span_that_overruns_the_retained_capture_is_refused() {
    let mut store = late_capture("bf-overrun");
    let span = saved_span(
        vec![restored("r3", 3), restored("r4", 4), restored("r5", 5)],
        3,
    );
    assert_eq!(
        denial(&mut store, span, &request(3, 5)).refusal,
        SemanticHistoryRefusal::BackfillSpan
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
}

#[test]
fn a_restored_span_carrying_fewer_records_than_it_declares_is_refused() {
    let mut store = late_capture("bf-short-span");
    // The span ends at 3, so it abuts the retained start at 4; only the declared length is wrong.
    let refused = denial(
        &mut store,
        saved_span(vec![restored("r2", 2)], 2),
        &request(2, 3),
    );
    assert_eq!(refused.refusal, SemanticHistoryRefusal::BackfillSpan);
    assert_eq!(refused.subject.as_deref(), Some("b1"));
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(store.window("b1").unwrap().capture_start_sequence, 4);
}

#[test]
fn a_restored_span_naming_another_scope_is_refused() {
    let mut store = late_capture("bf-scope");
    let mut span = saved_span(vec![restored("r3", 3)], 3);
    span.scope = scope("b2");
    assert_eq!(
        denial(&mut store, span, &request(3, 3)).refusal,
        SemanticHistoryRefusal::ScopeMismatch
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(store.window("b1").unwrap().capture_start_sequence, 4);
}

#[test]
fn a_restored_span_naming_another_binding_or_fence_is_refused() {
    let mut store = late_capture("bf-mismatch");
    let span = saved_span(vec![restored("r3", 3)], 3);
    let mut other_binding = request(3, 3);
    other_binding.binding.manifest_digest = "manifest-digest-b".to_owned();
    assert_eq!(
        denial(&mut store, span.clone(), &other_binding).refusal,
        SemanticHistoryRefusal::BindingMismatch
    );

    let mut stale = request(3, 3);
    stale.fence.epoch = 9;
    assert_eq!(
        denial(&mut store, span.clone(), &stale).refusal,
        SemanticHistoryRefusal::StaleFence
    );

    let mut unknown = request(3, 3);
    unknown.fence.branch_id = "b9".to_owned();
    assert_eq!(
        denial(&mut store, span, &unknown).refusal,
        SemanticHistoryRefusal::UnknownBranch
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
}

#[test]
fn a_restored_batch_is_admitted_under_the_same_rules_a_live_batch_faces() {
    let mut store = late_capture("bf-vocabulary");
    let mut unbounded = restored("r3", 3);
    unbounded.value = None;
    assert_eq!(
        denial(&mut store, saved_span(vec![unbounded], 3), &request(3, 3)).refusal,
        SemanticHistoryRefusal::MissingDetail
    );

    let mut stating = restored("r3", 3);
    stating.causal_parent = Some(SemanticCausalParent {
        parent_event_id: Some("r2".to_owned()),
        provenance: SemanticCausalProvenance::Stated,
    });
    assert_eq!(
        denial(&mut store, saved_span(vec![stating], 3), &request(3, 3)).refusal,
        SemanticHistoryRefusal::ImportedCausality,
        "restored history states no cause"
    );
    assert_eq!(store.records("b1").unwrap().len(), 1);
}

#[test]
fn a_re_delivered_restore_changes_nothing_and_a_reused_identity_is_refused() {
    let mut store = late_capture("bf-idempotent");
    let mut port = ModPort::yielding(two_saved());
    let first = granted(&mut store, &mut port, &request(2, 3)).expect("restore lands");
    assert_eq!(first.restored, 2);
    let replay = granted(&mut store, &mut port, &request(2, 3)).expect("re-delivery is recognised");
    assert_eq!(replay.restored, 0);
    assert_eq!(replay.total, 3);
    assert_eq!(store.records("b1").unwrap().len(), 3);

    let mut conflict = request(2, 3);
    conflict.binding.manifest_digest = "manifest-digest-c".to_owned();
    assert_eq!(
        refusal(&mut store, &mut port, &conflict).refusal,
        SemanticHistoryRefusal::IdempotencyConflict,
        "a reused identity with another binding is refused"
    );
    assert_eq!(store.records("b1").unwrap().len(), 3);
}

#[test]
fn a_restored_span_is_readable_through_the_retained_history_after_a_restart() {
    let file = path("bf-restart");
    {
        let mut store = SemanticHistoryStore::open(file.clone()).expect("opens empty");
        let mut live = append("op-live", "b1", 4, vec![damage("e4", 4, None)]);
        live.batch.window.history_before_capture = true;
        store.append(&live).expect("live capture lands");
        landed(
            &mut store,
            saved_span(vec![restored("r3", 3)], 3),
            &request(3, 3),
        );
        let mut later = append("op-live-2", "b1", 5, vec![damage("e5", 5, None)]);
        later.batch.window.history_before_capture = true;
        store.append(&later).expect("capture continues");
    }
    let reopened = SemanticHistoryStore::open(file).expect("store reopens");
    let records = reopened.records("b1").unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].event.origin, Some(SemanticEventOrigin::Imported));
    assert_eq!(records[0].event.sequence, 3);
    let window = reopened.window("b1").unwrap();
    assert_eq!(
        window.capture_start_sequence, 3,
        "the restored span is still where capture begins"
    );
    assert!(window.history_before_capture);
}

#[test]
fn a_port_that_cannot_read_saved_history_refuses_rather_than_returning_an_empty_span() {
    let mut store = late_capture("bf-unavailable");
    let mut port = ModPort::of(None);
    let refused = refusal(&mut store, &mut port, &request(2, 3));
    assert_eq!(refused.refusal, SemanticHistoryRefusal::Storage);
    assert_eq!(port.calls, 1);
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(store.window("b1").unwrap().capture_start_sequence, 4);
}
