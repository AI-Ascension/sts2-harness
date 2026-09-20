// SPDX-License-Identifier: MIT

//! Retention: what an explicit policy may disclose, and what it may never silently destroy.

use super::*;
use sts2_harness::semantic_history::{
    SEMANTIC_RETENTION_LABEL, SemanticPruneRequest, SemanticRetentionPolicy,
};

fn prune_request(branch: &str, policy: SemanticRetentionPolicy) -> SemanticPruneRequest {
    SemanticPruneRequest {
        branch_id: branch.to_owned(),
        policy,
    }
}

fn permitting(retain_latest: usize) -> SemanticRetentionPolicy {
    SemanticRetentionPolicy {
        retain_observed: false,
        retain_latest,
    }
}

fn fence(branch: &str) -> SemanticHistoryFence {
    SemanticHistoryFence {
        run_id: "run-1".to_owned(),
        branch_id: branch.to_owned(),
        episode: 1,
        epoch: 4,
    }
}

fn retained(
    mut store: SemanticHistoryStore,
    events: Vec<SemanticEventInput>,
) -> SemanticHistoryStore {
    store
        .append(&append("op-1", "b1", 1, events))
        .expect("history lands");
    store
}

#[test]
fn the_default_policy_selects_nothing_and_an_empty_prune_is_refused() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-default")),
        vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
    );
    assert_eq!(
        SemanticRetentionPolicy::default(),
        SemanticRetentionPolicy::protective()
    );
    let request = prune_request("b1", SemanticRetentionPolicy::default());
    let plan = store.prune_preview(&request).expect("preview computes");
    assert!(plan.is_empty());
    assert_eq!(plan.retained_sequences, vec![1, 2]);
    let refused = store
        .prune("prune-1", &request, &plan)
        .expect_err("nothing to prune is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::NothingPrunable);
    assert_eq!(store.records("b1").unwrap().len(), 2);
    assert!(store.records("b1").unwrap()[0].is_observed());
}

#[test]
fn a_pruned_value_becomes_a_declared_gap_rather_than_a_zero() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-disclose")),
        vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
    );
    let request = prune_request("b1", permitting(0));
    let plan = store.prune_preview(&request).expect("preview computes");
    assert_eq!(plan.prunable_sequences, vec![1, 2]);
    assert_eq!(plan.pinned_sequences, Vec::<u64>::new());
    store
        .prune("prune-1", &request, &plan)
        .expect("prune applies");
    let records = store.records("b1").unwrap();
    assert_eq!(
        records.len(),
        2,
        "a pruned record keeps its sequence number"
    );
    for record in records {
        assert!(!record.is_observed());
        assert_eq!(
            record.event.coverage.status,
            SemanticCoverageStatus::Dropped
        );
        assert_eq!(
            record.event.coverage.label.as_deref(),
            Some(SEMANTIC_RETENTION_LABEL)
        );
        assert!(record.event.kind.is_none());
        assert!(record.event.value.is_none(), "no amount survives as a zero");
        assert!(record.event.subjects.is_empty());
        assert!(record.event.reference.is_none());
    }
    let window = store.window("b1").unwrap();
    assert_eq!(window.intervals.len(), 1);
    assert_eq!(window.intervals[0].first_sequence, 1);
    assert_eq!(window.intervals[0].last_sequence, 2);
    assert_eq!(
        store.retention_intervals("b1").unwrap(),
        window.intervals.as_slice()
    );
}

#[test]
fn a_record_a_surviving_record_still_names_as_its_cause_is_kept() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-pin")),
        vec![
            card_played("e1", 1),
            card_played("e2", 2),
            card_played("e3", 3),
            damage("e4", 4, Some("e3")),
        ],
    );
    let request = prune_request("b1", permitting(1));
    let plan = store.prune_preview(&request).expect("preview computes");
    assert_eq!(
        plan.prunable_sequences,
        vec![1, 2],
        "an unreferenced pair is selected"
    );
    assert_eq!(
        plan.pinned_sequences,
        vec![3],
        "the named cause is kept even though the policy selected it"
    );
    assert_eq!(plan.retained_sequences, vec![4]);
    store
        .prune("prune-1", &request, &plan)
        .expect("prune applies");
    let records = store.records("b1").unwrap();
    assert!(records[2].is_observed(), "the pinned cause survives intact");
    assert_eq!(
        records[3]
            .event
            .causal_parent
            .as_ref()
            .and_then(|parent| parent.stated_parent()),
        Some("e3"),
        "the surviving child still reaches its stated cause"
    );
    assert!(records[..2].iter().all(|record| !record.is_observed()));
}

#[test]
fn pinning_closes_over_the_whole_ancestor_chain() {
    let store = retained(
        SemanticHistoryStore::in_memory(path("ret-closure")),
        vec![
            damage("e1", 1, None),
            damage("e2", 2, Some("e1")),
            damage("e3", 3, Some("e2")),
        ],
    );
    let request = prune_request("b1", permitting(1));
    let plan = store.prune_preview(&request).expect("preview computes");
    assert!(
        plan.prunable_sequences.is_empty(),
        "the whole chain is still reachable from the survivor"
    );
    assert_eq!(plan.pinned_sequences, vec![1, 2]);
    assert_eq!(plan.retained_sequences, vec![3]);
}

#[test]
fn a_prune_plan_that_does_not_describe_the_history_is_refused() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-stale")),
        vec![damage("e1", 1, None), damage("e2", 2, None)],
    );
    let request = prune_request("b1", permitting(0));
    let plan = store.prune_preview(&request).expect("preview computes");
    store
        .append(&append("op-2", "b1", 3, vec![damage("e3", 3, None)]))
        .expect("the history moves on");
    let refused = store
        .prune("prune-1", &request, &plan)
        .expect_err("a stale plan is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StalePrunePlan);
    assert_eq!(store.records("b1").unwrap().len(), 3);
    assert!(store.records("b1").unwrap()[0].is_observed());
}

#[test]
fn a_plan_applied_under_another_policy_is_refused() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-otherpolicy")),
        vec![damage("e1", 1, None), damage("e2", 2, None)],
    );
    let previewed = prune_request("b1", permitting(1));
    let plan = store.prune_preview(&previewed).expect("preview computes");
    assert_eq!(plan.prunable_sequences, vec![1]);
    let widened = prune_request("b1", permitting(0));
    let refused = store
        .prune("prune-1", &widened, &plan)
        .expect_err("a plan from another policy is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::StalePrunePlan);
    assert_eq!(store.records("b1").unwrap().len(), 2);
    assert!(store.records("b1").unwrap()[0].is_observed());
}

#[test]
fn a_re_delivered_prune_returns_its_plan_rather_than_pruning_a_second_time() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-replay")),
        vec![card_played("e1", 1), damage("e2", 2, None)],
    );
    let request = prune_request("b1", permitting(0));
    let plan = store.prune_preview(&request).expect("preview computes");
    let applied = store
        .prune("prune-1", &request, &plan)
        .expect("prune applies");
    let replayed = store
        .prune("prune-1", &request, &plan)
        .expect("re-delivery returns the applied plan");
    assert_eq!(replayed, applied);
    assert_eq!(store.records("b1").unwrap().len(), 2);
    let conflict = prune_request("b1", permitting(1));
    let plan_under_other_policy = store.prune_preview(&conflict).expect("preview computes");
    let refused = store
        .prune("prune-1", &conflict, &plan_under_other_policy)
        .expect_err("a reused identity is a conflict");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::IdempotencyConflict);
}

#[test]
fn a_prune_and_its_disclosure_survive_a_restart() {
    let file = path("ret-restart");
    let request = prune_request("b1", permitting(0));
    {
        let mut store = SemanticHistoryStore::open(file.clone()).expect("store opens");
        store
            .append(&append(
                "op-1",
                "b1",
                1,
                vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
            ))
            .expect("history lands");
        let plan = store.prune_preview(&request).expect("preview computes");
        store
            .prune("prune-1", &request, &plan)
            .expect("prune applies");
    }
    let reopened = SemanticHistoryStore::open(file).expect("store reopens");
    assert_eq!(reopened.records("b1").unwrap().len(), 2);
    assert!(!reopened.records("b1").unwrap()[0].is_observed());
    assert_eq!(reopened.retention_intervals("b1").unwrap().len(), 1);
    let replayed = reopened.clone().prune_preview(&request).expect("preview");
    assert!(replayed.is_empty(), "the span is already disclosed");
}

#[test]
fn an_append_after_a_prune_still_declares_the_disclosed_span() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-append")),
        vec![damage("e1", 1, None), damage("e2", 2, None)],
    );
    let request = prune_request("b1", permitting(0));
    let plan = store.prune_preview(&request).expect("preview computes");
    store
        .prune("prune-1", &request, &plan)
        .expect("prune applies");
    store
        .append(&append("op-2", "b1", 3, vec![damage("e3", 3, None)]))
        .expect("later events still land");
    let window = store.window("b1").unwrap();
    assert_eq!(
        window.intervals.len(),
        1,
        "the declared span a producer cannot restate is inherited, not dropped"
    );
    assert_eq!(window.intervals[0].last_sequence, 2);
    assert_eq!(store.records("b1").unwrap().len(), 3);
    assert!(store.records("b1").unwrap()[2].is_observed());
}

#[test]
fn a_page_over_a_pruned_span_discloses_the_gap_rather_than_an_empty_result() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-page")),
        vec![
            damage("e1", 1, None),
            card_played("e2", 2),
            damage("e3", 3, None),
        ],
    );
    let request = prune_request("b1", permitting(0));
    let plan = store.prune_preview(&request).expect("preview computes");
    assert_eq!(plan.prunable_sequences, vec![1, 2, 3]);
    store
        .prune("prune-1", &request, &plan)
        .expect("prune applies");
    let all = page_history(
        store.records("b1").unwrap(),
        &fence("b1"),
        &SemanticEventListQuery {
            limit: 8,
            ..SemanticEventListQuery::default()
        },
    )
    .expect("an unfiltered page still shows the disclosed span");
    assert_eq!(all.matched, 3);
    assert!(all.records.iter().all(|record| !record.is_observed()));
    let disclosed = page_history(
        store.records("b1").unwrap(),
        &fence("b1"),
        &SemanticEventListQuery {
            coverage: Some(SemanticCoverageStatus::Dropped),
            limit: 8,
            ..SemanticEventListQuery::default()
        },
    )
    .expect("gaps can be listed deliberately");
    assert_eq!(disclosed.matched, 3);
    assert_eq!(
        disclosed.records[0].event.coverage.label.as_deref(),
        Some(SEMANTIC_RETENTION_LABEL)
    );
    let observed = page_history(
        store.records("b1").unwrap(),
        &fence("b1"),
        &SemanticEventListQuery {
            coverage: Some(SemanticCoverageStatus::Captured),
            limit: 8,
            ..SemanticEventListQuery::default()
        },
    )
    .expect("observed events are listed");
    assert_eq!(observed.matched, 0, "nothing is reported as measured");
}

#[test]
fn a_fork_inherits_the_disclosure_it_copied() {
    let mut store = retained(
        SemanticHistoryStore::in_memory(path("ret-fork")),
        vec![damage("e1", 1, None), damage("e2", 2, None)],
    );
    let request = prune_request("b1", permitting(0));
    let plan = store.prune_preview(&request).expect("preview computes");
    store
        .prune("prune-1", &request, &plan)
        .expect("prune applies");
    let outcome = store
        .fork(&SemanticHistoryFork {
            operation_id: "fork-1".to_owned(),
            parent_branch_id: "b1".to_owned(),
            child: scope("b2"),
        })
        .expect("fork lands");
    assert_eq!(outcome.total, 2);
    assert!(!store.records("b2").unwrap()[0].is_observed());
    assert_eq!(store.retention_intervals("b2").unwrap().len(), 1);
    assert_eq!(store.window("b2").unwrap().intervals.len(), 1);
}

#[test]
fn an_unknown_branch_and_a_path_carrying_identity_are_refused() {
    let store = SemanticHistoryStore::in_memory(path("ret-unknown"));
    let missing = store
        .prune_preview(&prune_request("absent", permitting(0)))
        .expect_err("an unknown branch is refused");
    assert_eq!(missing.refusal, SemanticHistoryRefusal::UnknownBranch);
    let mut store = retained(store, vec![damage("e1", 1, None)]);
    let plan = store
        .prune_preview(&prune_request("b1", permitting(0)))
        .expect("preview computes");
    let refused = store
        .prune("../escape", &prune_request("b1", permitting(0)), &plan)
        .expect_err("an operation identity that could be a path is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::Identity);
    assert!(store.records("b1").unwrap()[0].is_observed());
}

#[test]
fn a_prune_that_would_exceed_the_span_bound_is_refused_without_touching_the_history() {
    let mut events = Vec::new();
    let parents = 65u64;
    for index in 0..parents {
        let parent = index * 2 + 1;
        events.push(damage(&format!("e{parent}"), parent, None));
        let single = parent + 1;
        events.push(damage(&format!("e{single}"), single, None));
    }
    for index in 0..parents {
        let sequence = parents * 2 + index + 1;
        events.push(damage(
            &format!("e{sequence}"),
            sequence,
            Some(&format!("e{}", index * 2 + 1)),
        ));
    }
    let mut store = retained(SemanticHistoryStore::in_memory(path("ret-bound")), events);
    let request = prune_request("b1", permitting(usize::try_from(parents).expect("fits")));
    let plan = store.prune_preview(&request).expect("preview computes");
    assert_eq!(
        plan.prunable_sequences.len(),
        usize::try_from(parents).expect("fits"),
        "each selected record is isolated, so each needs its own declared span"
    );
    let refused = store
        .prune("prune-1", &request, &plan)
        .expect_err("more spans than a window may declare is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::TooManyIntervals);
    let records = store.records("b1").unwrap();
    assert_eq!(records.len(), usize::try_from(parents * 3).expect("fits"));
    assert!(
        records.iter().all(|record| record.is_observed()),
        "a refused prune changes nothing"
    );
    assert!(store.retention_intervals("b1").unwrap().is_empty());
}
