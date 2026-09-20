// SPDX-License-Identifier: MIT

//! Durability: idempotent append, restart, and branch-fork lineage.

use super::*;

#[test]
fn a_re_delivered_batch_is_a_no_op_rather_than_a_duplicate() {
    let mut store = SemanticHistoryStore::in_memory(path("replay"));
    let request = append(
        "op-1",
        "b1",
        1,
        vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
    );
    let first = store.append(&request).expect("first append lands");
    assert_eq!(
        first,
        SemanticAppendOutcome::Appended { added: 2, total: 2 }
    );
    let second = store.append(&request).expect("replay is accepted");
    assert_eq!(second, SemanticAppendOutcome::AlreadyPresent { total: 2 });
    assert!(!second.changed());
    assert_eq!(store.records("b1").unwrap().len(), 2);
}

#[test]
fn reusing_an_append_identity_with_a_different_payload_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("conflict"));
    store
        .append(&append("op-1", "b1", 1, vec![card_played("e1", 1)]))
        .expect("first append lands");
    let changed = append("op-1", "b1", 1, vec![damage("e1", 1, None)]);
    let refused = store.append(&changed).expect_err("conflict is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::IdempotencyConflict);
    assert_eq!(store.records("b1").unwrap().len(), 1);
}

#[test]
fn an_append_that_is_not_contiguous_with_the_history_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("contiguous"));
    store
        .append(&append("op-1", "b1", 1, vec![card_played("e1", 1)]))
        .expect("first append lands");
    let skipped = append("op-2", "b1", 3, vec![card_played("e3", 3)]);
    let refused = store.append(&skipped).expect_err("gap is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::NotContiguous);
}

#[test]
fn a_batch_naming_another_scope_than_the_history_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("scope"));
    store
        .append(&append("op-1", "b1", 1, vec![card_played("e1", 1)]))
        .expect("first append lands");
    let mut other = append("op-2", "b1", 2, vec![card_played("e2", 2)]);
    other.batch.scope.epoch = 5;
    let refused = store.append(&other).expect_err("scope change is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::ScopeMismatch);
}

#[test]
fn a_batch_naming_another_catalog_binding_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("binding"));
    store
        .append(&append("op-1", "b1", 1, vec![card_played("e1", 1)]))
        .expect("first append lands");
    let mut other = append("op-2", "b1", 2, vec![card_played("e2", 2)]);
    other.binding.manifest_digest = "manifest-digest-b".to_owned();
    let refused = store.append(&other).expect_err("binding change is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::BindingMismatch);
}

#[test]
fn a_history_survives_a_restart_and_a_replay_after_it() {
    let file = path("restart");
    let request = append(
        "op-1",
        "b1",
        1,
        vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
    );
    {
        let mut store = SemanticHistoryStore::open(file.clone()).expect("store opens");
        store.append(&request).expect("append lands");
    }
    let mut reopened = SemanticHistoryStore::open(file.clone()).expect("store reopens");
    assert_eq!(reopened.records("b1").unwrap().len(), 2);
    assert_eq!(reopened.binding("b1").unwrap(), &binding());
    let replay = reopened.append(&request).expect("replay after restart");
    assert_eq!(replay, SemanticAppendOutcome::AlreadyPresent { total: 2 });
    let _ = std::fs::remove_file(file);
}

#[test]
fn a_fork_inherits_its_ancestor_by_lineage_and_appends_only_what_is_new() {
    let mut store = SemanticHistoryStore::in_memory(path("fork"));
    store
        .append(&append(
            "op-1",
            "b1",
            1,
            vec![card_played("e1", 1), damage("e2", 2, Some("e1"))],
        ))
        .expect("ancestor append lands");
    let fork = SemanticHistoryFork {
        operation_id: "fork-1".to_owned(),
        parent_branch_id: "b1".to_owned(),
        child: scope("b2"),
    };
    let outcome = store.fork(&fork).expect("fork lands");
    assert_eq!(outcome.inherited, 2);
    assert!(outcome.changed());
    assert_eq!(store.parent_branch("b2"), Some("b1"));
    assert_eq!(store.records("b2").unwrap().len(), 2);
    let repeated = store.fork(&fork).expect("repeated fork is a no-op");
    assert!(repeated.already_present);
    assert_eq!(store.records("b2").unwrap().len(), 2);
}

#[test]
fn a_fork_naming_its_own_branch_as_parent_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("selfparent"));
    store
        .append(&append("op-1", "b1", 1, vec![card_played("e1", 1)]))
        .expect("append lands");
    let fork = SemanticHistoryFork {
        operation_id: "fork-1".to_owned(),
        parent_branch_id: "b1".to_owned(),
        child: scope("b1"),
    };
    let refused = store.fork(&fork).expect_err("self-parent is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::SelfParentBranch);
}

#[test]
fn a_fork_naming_an_unretained_parent_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("noparent"));
    let fork = SemanticHistoryFork {
        operation_id: "fork-1".to_owned(),
        parent_branch_id: "absent".to_owned(),
        child: scope("b2"),
    };
    let refused = store.fork(&fork).expect_err("unknown parent is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::UnknownParentBranch);
    assert_eq!(store.branch_count(), 0);
}

#[test]
fn a_refused_append_retains_nothing_at_all() {
    let mut store = SemanticHistoryStore::in_memory(path("atomic"));
    let mut request = append(
        "op-1",
        "b1",
        1,
        vec![card_played("e1", 1), card_played("e2", 2)],
    );
    request.batch.events[1].subjects = Vec::new();
    let refused = store
        .append(&request)
        .expect_err("invalid batch is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::SubjectRole);
    assert!(store.records("b1").is_none());
    assert_eq!(store.branch_count(), 0);
}

#[test]
fn the_store_reports_a_storage_refusal_for_a_corrupt_document() {
    let file = path("corrupt");
    std::fs::write(&file, b"{not json").expect("fixture writes");
    let refused = SemanticHistoryStore::open(file.clone()).expect_err("corrupt store is refused");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::Storage);
    let _ = std::fs::remove_file(file);
}

#[test]
fn an_open_of_a_missing_store_starts_empty_rather_than_failing() {
    let file = path("absent");
    let store = SemanticHistoryStore::open(file).expect("a missing store opens empty");
    assert_eq!(store.branch_count(), 0);
}

#[test]
fn a_history_whose_capture_began_late_retains_where_capture_began() {
    let mut store = SemanticHistoryStore::in_memory(path("late-capture"));
    let mut late = append("op-1", "b1", 3, vec![card_played("e3", 3)]);
    late.batch.window.history_before_capture = true;
    store
        .append(&late)
        .expect("a history that declares the span it does not hold is retainable");
    let window = store.window("b1").expect("retained");
    assert_eq!(window.capture_start_sequence, 3);
    assert!(window.history_before_capture);
    assert_eq!(store.records("b1").unwrap().len(), 1);
}

#[test]
fn the_retained_capture_start_is_where_the_history_began_not_where_it_last_appended() {
    let mut store = SemanticHistoryStore::in_memory(path("first-start"));
    store
        .append(&append("op-1", "b1", 1, vec![card_played("e1", 1)]))
        .expect("first append lands");
    store
        .append(&append("op-2", "b1", 2, vec![card_played("e2", 2)]))
        .expect("second append lands");
    let window = store.window("b1").expect("retained");
    assert_eq!(
        window.capture_start_sequence, 1,
        "the history began at sequence 1, so that is where its capture began"
    );
    assert!(!window.history_before_capture);
    assert_eq!(store.records("b1").unwrap().len(), 2);
}

#[test]
fn a_span_an_earlier_batch_declared_is_still_declared_after_a_later_append() {
    let mut store = SemanticHistoryStore::in_memory(path("spans-kept"));
    let mut first = append(
        "op-1",
        "b1",
        1,
        vec![gap("g1", 1, SemanticCoverageStatus::Dropped)],
    );
    first.batch.window.intervals = vec![dropped(1, 1)];
    store.append(&first).expect("first append lands");
    store
        .append(&append("op-2", "b1", 2, vec![card_played("e2", 2)]))
        .expect("second append lands");
    let window = store.window("b1").expect("retained");
    assert_eq!(
        window.intervals,
        vec![dropped(1, 1)],
        "a span the first batch declared is not dropped when a later batch states its own"
    );
    assert_eq!(window.capture_start_sequence, 1);
    assert_eq!(store.records("b1").unwrap().len(), 2);
}

#[test]
fn a_batch_that_restates_a_span_the_history_already_declares_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("spans-restated"));
    let mut first = append("op-1", "b1", 1, vec![card_played("e1", 1)]);
    first.batch.window.intervals = Vec::new();
    store.append(&first).expect("first append lands");
    let mut declared = append(
        "op-2",
        "b1",
        2,
        vec![gap("g2", 2, SemanticCoverageStatus::Dropped)],
    );
    declared.batch.window.intervals = vec![dropped(2, 2), dropped(5, 5)];
    store.append(&declared).expect("a declared span lands");
    let mut restated = append("op-3", "b1", 3, vec![card_played("e3", 3)]);
    restated.batch.window.intervals = vec![dropped(5, 5)];
    let refused = store.append(&restated).expect_err("restatement is refused");
    assert_eq!(
        refused.refusal,
        SemanticHistoryRefusal::OverlappingIntervals
    );
    assert_eq!(store.records("b1").unwrap().len(), 2);
}

#[test]
fn a_window_that_denies_the_history_it_starts_after_is_refused() {
    let mut store = SemanticHistoryStore::in_memory(path("window-denies"));
    let mut denied = append("op-1", "b1", 3, vec![card_played("e3", 3)]);
    denied.batch.window.history_before_capture = false;
    let refused = store
        .append(&denied)
        .expect_err("a late start must declare its prefix");
    assert_eq!(refused.refusal, SemanticHistoryRefusal::WindowContradiction);
    assert_eq!(store.branch_count(), 0);
}
