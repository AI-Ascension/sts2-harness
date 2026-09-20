// SPDX-License-Identifier: MIT

//! The repository's existing branch retention, applied to the retained history.

use super::*;
use sts2_harness::semantic_history::{
    SemanticBranchRetentionOutcome, SemanticBranchRetentionRequest,
};
use sts2_harness::{BranchPrunePlan, BranchRetentionPolicy};

/// The existing branch policy an operator runs the branch store's prune under.
fn branch_policy() -> BranchRetentionPolicy {
    BranchRetentionPolicy {
        retain_completed: false,
        minimum_age_millis: 0,
    }
}

/// The plan the existing branch prune produced, taken here as it would be handed over.
fn plan(branches: &[&str]) -> BranchPrunePlan {
    BranchPrunePlan {
        branch_ids: branches.iter().map(|value| (*value).to_owned()).collect(),
        retained_artifacts: Vec::new(),
        collectable_artifacts: Vec::new(),
    }
}

fn request(operation: &str, branches: &[&str]) -> SemanticBranchRetentionRequest {
    SemanticBranchRetentionRequest {
        operation_id: operation.to_owned(),
        experiment_id: "experiment-1".to_owned(),
        policy: branch_policy(),
        plan: plan(branches),
    }
}

/// A store holding two branches of observed detail, as a live capture would have left them.
fn captured(name: &str) -> SemanticHistoryStore {
    let mut store = SemanticHistoryStore::in_memory(path(name));
    let first = append(
        "op-live-a",
        "b1",
        1,
        vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
    );
    store.append(&first).expect("first branch captures");
    let second = append("op-live-b", "b2", 1, vec![damage("f1", 1, None)]);
    store.append(&second).expect("second branch captures");
    store
}

/// Applies the existing branch prune's selection.
fn applied(
    store: &mut SemanticHistoryStore,
    request: &SemanticBranchRetentionRequest,
) -> SemanticBranchRetentionOutcome {
    store
        .apply_branch_retention(request)
        .expect("the application lands")
}

/// The observed values one branch still serves, so a test can show detail was there before it went.
fn served_detail(store: &SemanticHistoryStore, branch: &str) -> usize {
    store
        .records(branch)
        .expect("branch is retained")
        .iter()
        .filter(|record| record.is_observed())
        .count()
}

#[test]
fn a_branch_the_existing_policy_pruned_is_disclosed_rather_than_left_readable() {
    let mut store = captured("br-discloses");
    assert_eq!(
        served_detail(&store, "b1"),
        2,
        "the branch serves observed detail before the existing policy is applied"
    );
    let outcome = applied(&mut store, &request("retire-1", &["b1"]));
    assert_eq!(outcome.disclosed_branches, 1);
    assert_eq!(outcome.disclosed_records, 2);
    assert_eq!(served_detail(&store, "b1"), 0);
    let records = store.records("b1").unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2],
        "a disclosed record keeps its sequence number"
    );
    for record in records {
        assert!(!record.is_observed());
        assert_eq!(record.event.kind, None);
        assert_eq!(record.event.origin, None);
        assert_eq!(record.event.value, None);
        assert_eq!(record.event.reference, None);
        assert_eq!(
            record.event.coverage.label.as_deref(),
            Some("retention"),
            "the disclosure names retention rather than reading as a measured zero"
        );
    }
}

#[test]
fn the_disclosure_declares_the_span_every_pruned_sequence_needs() {
    let mut store = captured("br-span");
    applied(&mut store, &request("retire-1", &["b1"]));
    let window = store.window("b1").unwrap();
    assert_eq!(window.capture_start_sequence, 1);
    assert_eq!(window.intervals, vec![dropped(1, 2)]);
    assert_eq!(
        store.retention_intervals("b1"),
        Some([dropped(1, 2)].as_slice())
    );
}

#[test]
fn a_branch_the_plan_does_not_name_keeps_its_observed_detail() {
    let mut store = captured("br-untouched");
    let outcome = applied(&mut store, &request("retire-1", &["b1"]));
    assert_eq!(outcome.disclosed_branches, 1);
    assert_eq!(served_detail(&store, "b2"), 1);
    assert_eq!(
        store.records("b2").unwrap()[0]
            .event
            .value
            .as_ref()
            .unwrap()
            .amount,
        6,
        "a branch the existing plan did not select is untouched"
    );
}

#[test]
fn a_re_delivered_application_discloses_nothing_a_second_time() {
    let mut store = captured("br-idempotent");
    let first = applied(&mut store, &request("retire-1", &["b1"]));
    assert_eq!(first.disclosed_records, 2);
    let replay = applied(&mut store, &request("retire-1", &["b1"]));
    assert_eq!(replay.disclosed_records, 0);
    assert_eq!(replay.already_applied, 1);
    assert_eq!(store.window("b1").unwrap().intervals, vec![dropped(1, 2)]);
}

#[test]
fn reusing_the_operation_identity_under_another_selection_is_refused() {
    let mut store = captured("br-conflict");
    applied(&mut store, &request("retire-1", &["b1"]));
    assert_eq!(
        store
            .apply_branch_retention(&request("retire-1", &["b1", "b2"]))
            .expect_err("another selection under one identity is refused")
            .refusal,
        SemanticHistoryRefusal::IdempotencyConflict
    );
    assert_eq!(served_detail(&store, "b2"), 1);
}

#[test]
fn an_empty_selection_is_refused_rather_than_applied() {
    let mut store = captured("br-empty");
    assert_eq!(
        store
            .apply_branch_retention(&request("retire-1", &[]))
            .expect_err("an empty selection is refused")
            .refusal,
        SemanticHistoryRefusal::NothingPrunable
    );
    assert_eq!(served_detail(&store, "b1"), 2);
}

#[test]
fn a_branch_the_store_never_captured_is_counted_rather_than_refused() {
    let mut store = captured("br-absent");
    let outcome = applied(&mut store, &request("retire-1", &["b1", "b9"]));
    assert_eq!(outcome.disclosed_branches, 1);
    assert_eq!(outcome.absent_branches, 1);
    assert_eq!(outcome.disclosed_records, 2);
}

#[test]
fn a_cause_is_disclosed_along_with_the_value_it_explained() {
    let mut store = captured("br-cause");
    applied(&mut store, &request("retire-1", &["b1"]));
    let records = store.records("b1").unwrap();
    assert_eq!(records[1].event.causal_parent, None);
    assert!(records[1].event.subjects.is_empty());
    assert_eq!(records[0].event.reference, None);
}

#[test]
fn a_branch_without_observed_detail_still_records_the_application_identity() {
    let mut store = SemanticHistoryStore::in_memory(path("br-gaps-only"));
    let mut only_gaps = append(
        "op-live",
        "b1",
        1,
        vec![gap("e1", 1, SemanticCoverageStatus::Dropped)],
    );
    only_gaps.batch.window.intervals = vec![dropped(1, 1)];
    store.append(&only_gaps).expect("a gap-only history lands");
    let outcome = applied(&mut store, &request("retire-1", &["b1"]));
    assert_eq!(outcome.disclosed_records, 0);
    assert_eq!(outcome.disclosed_branches, 1);
    assert_eq!(store.records("b1").unwrap().len(), 1);
    assert_eq!(
        applied(&mut store, &request("retire-1", &["b1"])).already_applied,
        1,
        "a gap-only branch is not re-examined on a re-delivery"
    );
}

#[test]
fn a_branch_pruned_by_the_existing_policy_discloses_the_gap_after_a_restart() {
    let file = path("br-restart");
    {
        let mut store = SemanticHistoryStore::open(file.clone()).expect("opens empty");
        let live = append("op-live", "b1", 1, vec![damage("e1", 1, None)]);
        store.append(&live).expect("capture lands");
        applied(&mut store, &request("retire-1", &["b1"]));
    }
    let reopened = SemanticHistoryStore::open(file).expect("store reopens");
    assert_eq!(served_detail(&reopened, "b1"), 0);
    assert_eq!(
        reopened.window("b1").unwrap().intervals,
        vec![dropped(1, 1)]
    );
    assert_eq!(reopened.records("b1").unwrap().len(), 1);
}
