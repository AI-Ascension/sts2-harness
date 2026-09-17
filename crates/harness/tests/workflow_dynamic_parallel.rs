// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

#[path = "workflow_dynamic_parallel/controlled.rs"]
mod controlled;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sts2_harness::workflow::{
    AnalysisValue, BoundedText, BranchOutcome, DynamicPlanError, ParallelCap, execute_plan,
    execute_plan_bounded, validate_plan,
};

use controlled::{
    Controlled, Panics, Serial, allowed, analyze, decide, edge, index_of, limits, node, plan,
};

#[test]
fn two_independent_analyses_overlap_at_cap_two_and_serialize_at_cap_one() {
    let plan = plan(vec![analyze("a"), analyze("b")], Vec::new());
    validate_plan(&plan, &allowed(&plan), 4, 4).expect("plan validates");

    let overlapping = Controlled {
        rendezvous: Some(2),
        ..Controlled::default()
    };
    let cap_two = ParallelCap::from_limits(&limits(2)).expect("cap 2");
    let joined = execute_plan_bounded(&plan, cap_two, &overlapping).expect("cap 2 executes");
    assert_eq!(overlapping.peak(), 2, "both branches were live together");
    assert!(joined.outcomes.values().all(BranchOutcome::is_settled));

    let serial = Controlled::default();
    let cap_one = ParallelCap::from_limits(&limits(1)).expect("cap 1");
    assert_eq!(cap_one, ParallelCap::SERIAL);
    let joined_serial = execute_plan_bounded(&plan, cap_one, &serial).expect("cap 1 executes");
    assert!(
        joined_serial
            .outcomes
            .values()
            .all(BranchOutcome::is_settled)
    );
    assert_eq!(serial.peak(), 1, "cap=1 never overlaps");
    let log = serial.log();
    assert!(index_of(&log, "end a") < index_of(&log, "start b"));

    let chain = controlled::plan(
        vec![analyze("a"), analyze("b"), decide("d", &["a", "b"])],
        vec![edge("a", "b"), edge("a", "d"), edge("b", "d")],
    );
    let order = validate_plan(&chain, &allowed(&chain), 4, 4).expect("order");
    let legacy = execute_plan(&chain, &order, &mut Serial).expect("serial route");
    let bounded = execute_plan_bounded(&chain, cap_one, &Controlled::default())
        .expect("cap 1 chain executes")
        .into_plan_result()
        .expect("all settled");
    assert_eq!(
        bounded, legacy,
        "cap=1 bounded route reproduces the serial route for declared inputs"
    );
    assert_eq!(
        bounded
            .analyses
            .get(&node("d"))
            .map(|value| value.code.as_str()),
        Some("profile<-a,b")
    );

    assert_eq!(ParallelCap::new(0), Err(DynamicPlanError::Capacity));
    assert_eq!(ParallelCap::new(5), Err(DynamicPlanError::Capacity));
}

#[test]
fn in_flight_never_exceeds_cap_under_failure_and_retry() {
    let plan = plan(
        vec![
            analyze("a"),
            analyze("b"),
            analyze("c"),
            analyze("d"),
            analyze("e"),
            analyze("f"),
        ],
        Vec::new(),
    );
    let executor = Controlled {
        fail_once: Mutex::new(BTreeSet::from(["b".to_owned(), "e".to_owned()])),
        unknown: BTreeSet::from(["c".to_owned()]),
        ..Controlled::default()
    };
    let cap = ParallelCap::new(2).expect("cap 2");

    let first = execute_plan_bounded(&plan, cap, &executor).expect("first attempt joins");
    assert_eq!(
        first.outcomes.get(&node("b")),
        Some(&BranchOutcome::Failed(DynamicPlanError::UnknownOperation))
    );
    assert_eq!(
        first.outcomes.get(&node("c")),
        Some(&BranchOutcome::Unknown)
    );
    assert!(
        executor.peak() <= 2,
        "peak {} exceeded cap",
        executor.peak()
    );
    assert!(executor.peak() >= 2, "load produced no overlap at all");

    let retry = execute_plan_bounded(&plan, cap, &executor).expect("retry joins");
    assert!(
        retry
            .outcomes
            .get(&node("b"))
            .is_some_and(BranchOutcome::is_settled)
    );
    assert!(
        retry
            .outcomes
            .get(&node("e"))
            .is_some_and(BranchOutcome::is_settled)
    );
    assert_eq!(
        retry.outcomes.get(&node("c")),
        Some(&BranchOutcome::Unknown)
    );
    assert_eq!(executor.peak(), 2, "retry never exceeded the cap either");
    assert_eq!(executor.calls.load(Ordering::SeqCst), 12);
}

#[test]
fn dependent_node_waits_until_all_declared_inputs_settle() {
    let plan = plan(
        vec![
            analyze("a"),
            analyze("b"),
            analyze("c"),
            decide("d", &["a", "c"]),
        ],
        vec![edge("a", "c"), edge("b", "c")],
    );
    validate_plan(&plan, &allowed(&plan), 4, 4).expect("plan validates");
    let executor = Controlled {
        delays: BTreeMap::from([
            ("a".to_owned(), Duration::from_millis(60)),
            ("b".to_owned(), Duration::from_millis(5)),
        ]),
        ..Controlled::default()
    };
    let joined =
        execute_plan_bounded(&plan, ParallelCap::new(4).expect("cap"), &executor).expect("joins");
    let log = executor.log();
    assert!(index_of(&log, "end a") < index_of(&log, "start c"));
    assert!(index_of(&log, "end b") < index_of(&log, "start c"));
    assert!(index_of(&log, "end c") < index_of(&log, "start profile"));
    assert_eq!(
        joined.outcomes.get(&node("c")),
        Some(&BranchOutcome::Settled(AnalysisValue {
            code: BoundedText::new("c<-a,b").expect("code"),
            fields: BTreeMap::new(),
        })),
        "a dependent branch sees exactly its declared inputs"
    );
    assert_eq!(
        joined.outcomes.get(&node("d")),
        Some(&BranchOutcome::Settled(AnalysisValue {
            code: BoundedText::new("profile<-a,c").expect("code"),
            fields: BTreeMap::new(),
        }))
    );
}

#[test]
fn permuted_completion_order_yields_identical_join_order_and_digest() {
    let plan = plan(
        vec![
            analyze("a"),
            analyze("b"),
            analyze("c"),
            decide("d", &["a", "b", "c"]),
        ],
        Vec::new(),
    );
    let slow_first = Controlled {
        delays: BTreeMap::from([
            ("a".to_owned(), Duration::from_millis(60)),
            ("b".to_owned(), Duration::from_millis(30)),
            ("c".to_owned(), Duration::from_millis(1)),
        ]),
        ..Controlled::default()
    };
    let slow_last = Controlled {
        delays: BTreeMap::from([
            ("a".to_owned(), Duration::from_millis(1)),
            ("b".to_owned(), Duration::from_millis(30)),
            ("c".to_owned(), Duration::from_millis(60)),
        ]),
        ..Controlled::default()
    };
    let cap = ParallelCap::new(3).expect("cap 3");
    let first = execute_plan_bounded(&plan, cap, &slow_first).expect("first joins");
    let second = execute_plan_bounded(&plan, cap, &slow_last).expect("second joins");

    let ends = |log: &[String]| -> Vec<String> {
        log.iter()
            .filter(|line| line.starts_with("end "))
            .take(3)
            .cloned()
            .collect()
    };
    assert_ne!(
        ends(&slow_first.log()),
        ends(&slow_last.log()),
        "the two runs really completed in different orders"
    );
    assert_eq!(
        first.outcomes.keys().collect::<Vec<_>>(),
        vec![&node("a"), &node("b"), &node("c"), &node("d")]
    );
    assert_eq!(first.outcomes, second.outcomes);
    assert_eq!(first.join_digest, second.join_digest);
    assert_eq!(first, second);
}

#[test]
fn failed_or_unknown_branch_never_feeds_a_successful_decision() {
    let plan = plan(
        vec![
            analyze("a"),
            analyze("b"),
            analyze("c"),
            analyze("e"),
            decide("d", &["a", "b", "c"]),
            decide("g", &["c"]),
        ],
        vec![edge("a", "e")],
    );
    let executor = Controlled {
        fail_once: Mutex::new(BTreeSet::from(["a".to_owned()])),
        unknown: BTreeSet::from(["b".to_owned()]),
        ..Controlled::default()
    };
    let joined =
        execute_plan_bounded(&plan, ParallelCap::new(2).expect("cap"), &executor).expect("joins");

    assert_eq!(
        joined.outcomes.get(&node("a")),
        Some(&BranchOutcome::Failed(DynamicPlanError::UnknownOperation))
    );
    assert_eq!(
        joined.outcomes.get(&node("b")),
        Some(&BranchOutcome::Unknown)
    );
    assert!(
        joined
            .outcomes
            .get(&node("c"))
            .is_some_and(BranchOutcome::is_settled)
    );
    assert_eq!(
        joined.outcomes.get(&node("d")),
        Some(&BranchOutcome::Failed(DynamicPlanError::UnsettledInput)),
        "a decision with a failed or unknown input is refused, not executed"
    );
    assert_eq!(
        joined.outcomes.get(&node("e")),
        Some(&BranchOutcome::Failed(DynamicPlanError::UnsettledInput)),
        "a dependent analysis of a failed branch is not dispatched"
    );
    assert!(
        joined
            .outcomes
            .get(&node("g"))
            .is_some_and(BranchOutcome::is_settled)
    );
    let log = executor.log();
    assert_eq!(
        log.iter().filter(|line| *line == "start profile").count(),
        1,
        "only the decision whose inputs all settled reached the executor"
    );
    assert!(!log.iter().any(|line| line == "start e"));
    assert_eq!(
        joined.into_plan_result(),
        Err(DynamicPlanError::UnknownOperation),
        "the compatibility projection refuses the plan on the first unsettled node"
    );
}

#[test]
fn a_plan_that_never_becomes_ready_is_refused_as_a_cycle() {
    let plan = plan(
        vec![analyze("a"), analyze("b")],
        vec![edge("a", "b"), edge("b", "a")],
    );
    let executor = Controlled::default();
    assert_eq!(
        execute_plan_bounded(&plan, ParallelCap::SERIAL, &executor),
        Err(DynamicPlanError::Cycle)
    );
    assert!(executor.log().is_empty());
}

#[test]
fn an_unwinding_branch_reports_branch_lost_instead_of_stalling_the_join() {
    let plan = plan(
        vec![analyze("a"), analyze("b"), decide("d", &["a", "b"])],
        Vec::new(),
    );
    let executor = Panics {
        unwound: BTreeSet::from(["a".to_owned()]),
    };
    let joined = execute_plan_bounded(&plan, ParallelCap::new(2).expect("cap"), &executor)
        .expect("the join still completes");

    assert_eq!(
        joined.outcomes.get(&node("a")),
        Some(&BranchOutcome::Failed(DynamicPlanError::BranchLost)),
        "a branch that ended without reporting is BranchLost, not a stall"
    );
    assert!(
        joined
            .outcomes
            .get(&node("b"))
            .is_some_and(BranchOutcome::is_settled),
        "the other branch still ran"
    );
    assert_eq!(
        joined.outcomes.get(&node("d")),
        Some(&BranchOutcome::Failed(DynamicPlanError::UnsettledInput)),
        "an unwound branch never feeds a decision"
    );
    assert_eq!(joined.into_plan_result(), Err(DynamicPlanError::BranchLost));
}
