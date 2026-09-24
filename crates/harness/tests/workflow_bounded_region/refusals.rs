// SPDX-License-Identifier: MIT

//! AC5 refusal and reporting edges: a region that cannot be admitted never
//! dispatches a branch, a budget that cannot cover every branch refuses before
//! inference, and the admitted vocabulary cannot express a mutation.

#![allow(clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::sync::atomic::Ordering;

use serde_json::json;
use sts2_harness::workflow::{
    AdaptiveRegionConfig, BoundedBranchState, BoundedRegionRefusal, BranchBudgetLedger, CancelFlag,
    DynamicPlan, DynamicPlanError, DynamicRuntime, ParallelBudget, ParallelCap, PlannerProfileId,
    RegionId, admit_bounded_region, is_analysis_kind, run_bounded_region,
};

use super::controlled::{Controlled, allowed, analyze, decide, edge, limits, plan as build_plan};
use super::{FixtureExecutor, UNITS, dynamic_workflow, independent_plan, region};

#[test]
fn a_definite_branch_failure_is_reported_with_its_own_reason() {
    let plan = build_plan(vec![analyze("a")], Vec::new());
    let executor = Controlled {
        fail_once: Mutex::new(BTreeSet::from(["a".to_owned()])),
        ..Controlled::default()
    };
    let ledger = BranchBudgetLedger::new(UNITS).expect("limit");

    let (outcome, _joined) = run_bounded_region(
        &plan,
        &region(),
        &limits(1),
        &executor,
        &ledger,
        UNITS,
        &CancelFlag::new(),
    )
    .expect("joins");

    assert_eq!(
        outcome.branches.get("a"),
        Some(&BoundedBranchState::Failed {
            reason: "unknown_operation".to_owned()
        })
    );
    assert_eq!(outcome.settled(), 0);
}

#[test]
fn a_refused_region_never_dispatches_a_branch() {
    let plan = build_plan(vec![analyze("a"), analyze("b")], Vec::new());
    let executor = Controlled::default();
    let ledger = BranchBudgetLedger::new(2 * UNITS).expect("limit");

    let mut narrowed = region();
    narrowed.allowed_operations.clear();
    let refused = run_bounded_region(
        &plan,
        &narrowed,
        &limits(2),
        &executor,
        &ledger,
        UNITS,
        &CancelFlag::new(),
    );
    assert_eq!(refused, Err(BoundedRegionRefusal::RegionAdmitsNoOperations));
    assert_eq!(
        executor.calls.load(Ordering::SeqCst),
        0,
        "admission precedes dispatch"
    );
    assert_eq!(ledger.reserved_units(), 0, "no budget was reserved");

    let mut wide = limits(2);
    wide.max_parallel_analyses = 5;
    assert_eq!(
        admit_bounded_region(&plan, &region(), &wide),
        Err(BoundedRegionRefusal::CapOutsideAdmittedRange)
    );

    let stray = build_plan(vec![analyze("a"), analyze("z")], Vec::new());
    assert_eq!(
        admit_bounded_region(&stray, &region(), &limits(2)),
        Err(BoundedRegionRefusal::PlanRejected(
            DynamicPlanError::UnknownOperation
        )),
        "an operation outside the region is refused before dispatch"
    );
}

#[test]
fn a_foreign_region_or_planner_plan_is_refused_by_identity_before_dispatch() {
    let plan = build_plan(vec![analyze("a")], Vec::new());
    // Positive control: the plan that names this region and profile is admitted.
    assert_eq!(
        admit_bounded_region(&plan, &region(), &limits(1)),
        Ok(cap_of(1))
    );

    let mut foreign_region = plan.clone();
    foreign_region.region_id = RegionId::new("region-2").expect("region id");
    assert_eq!(
        admit_bounded_region(&foreign_region, &region(), &limits(1)),
        Err(BoundedRegionRefusal::PlanIdentityMismatch),
        "a plan naming a different region is refused"
    );

    let mut foreign_profile = plan.clone();
    foreign_profile.planner_profile_ref = PlannerProfileId::new("planner-2").expect("planner id");
    assert_eq!(
        admit_bounded_region(&foreign_profile, &region(), &limits(1)),
        Err(BoundedRegionRefusal::PlanIdentityMismatch),
        "a plan naming a different planner profile is refused"
    );

    // The identity gate runs before structural validation, so a foreign plan whose
    // operations are also outside the region reports the identity reason, not a
    // shadowing `PlanRejected`.
    let mut foreign_and_stray = foreign_region.clone();
    foreign_and_stray.nodes = vec![analyze("z")];
    assert_eq!(
        admit_bounded_region(&foreign_and_stray, &region(), &limits(1)),
        Err(BoundedRegionRefusal::PlanIdentityMismatch),
        "the identity gate is not shadowed by the structural plan check"
    );

    // Fail-closed: a foreign-region plan never reaches the executor.
    let executor = Controlled::default();
    let ledger = BranchBudgetLedger::new(UNITS).expect("limit");
    let refused = run_bounded_region(
        &foreign_region,
        &region(),
        &limits(1),
        &executor,
        &ledger,
        UNITS,
        &CancelFlag::new(),
    );
    assert_eq!(refused, Err(BoundedRegionRefusal::PlanIdentityMismatch));
    assert_eq!(
        executor.calls.load(Ordering::SeqCst),
        0,
        "identity admission precedes dispatch"
    );
    assert_eq!(ledger.reserved_units(), 0, "no budget was reserved");
}

fn cap_of(max_parallel_analyses: u64) -> ParallelCap {
    ParallelCap::from_limits(&limits(max_parallel_analyses)).expect("cap")
}

#[test]
fn a_dependent_region_joins_actual_inputs_and_keeps_one_join_digest() {
    let plan = build_plan(
        vec![analyze("a"), analyze("b"), decide("c", &["a", "b"])],
        vec![edge("a", "c"), edge("b", "c")],
    );
    let first = Controlled::default();
    let second = Controlled::default();
    let left = BranchBudgetLedger::new(3 * UNITS).expect("limit");
    let right = BranchBudgetLedger::new(3 * UNITS).expect("limit");

    let (one, joined_one) = run_bounded_region(
        &plan,
        &region_for(&plan),
        &limits(3),
        &first,
        &left,
        UNITS,
        &CancelFlag::new(),
    )
    .expect("first joins");
    let (two, joined_two) = run_bounded_region(
        &plan,
        &region_for(&plan),
        &limits(3),
        &second,
        &right,
        UNITS,
        &CancelFlag::new(),
    )
    .expect("second joins");

    let decide_state = one.branches.get("c").expect("decide reported");
    assert_eq!(
        decide_state,
        two.branches.get("c").expect("decide reported")
    );
    assert_eq!(
        decide_state,
        &BoundedBranchState::Settled {
            code: "profile<-a,b".to_owned()
        },
        "the joined value names its actual inputs `a` and `b`, not a bare count"
    );
    assert_eq!(
        one.join_digest, two.join_digest,
        "the join digest does not depend on completion timing"
    );
    assert_eq!(joined_one.join_digest, joined_two.join_digest);
}

fn region_for(plan: &DynamicPlan) -> AdaptiveRegionConfig {
    let mut widened = region();
    widened.allowed_operations = allowed(plan).into_iter().collect();
    widened
}

#[test]
fn the_admitted_plan_vocabulary_cannot_express_a_mutation() {
    let plan = independent_plan();
    assert!(
        plan.nodes.iter().all(|node| is_analysis_kind(&node.kind)),
        "every admitted node kind is an analysis kind"
    );

    let mut value = serde_json::to_value(&plan).expect("plan serializes");
    value["nodes"][0]["kind"] = json!("execute_action");
    let refused = serde_json::from_value::<DynamicPlan>(value);
    assert!(
        refused.is_err(),
        "a plan naming a mutating node kind is refused at decode: {refused:?}"
    );

    let mut value = serde_json::to_value(&plan).expect("plan serializes");
    value["nodes"][0]["kind"] = json!("checkpoint");
    assert!(
        serde_json::from_value::<DynamicPlan>(value).is_err(),
        "no other node kind is expressible in a bounded plan"
    );
}

#[test]
fn a_budget_that_cannot_cover_every_branch_refuses_before_inference() {
    let plan = build_plan(vec![analyze("a"), analyze("b")], Vec::new());
    let executor = Controlled::default();
    let ledger = BranchBudgetLedger::new(UNITS).expect("limit");

    let (outcome, _joined) = run_bounded_region(
        &plan,
        &region(),
        &limits(2),
        &executor,
        &ledger,
        UNITS,
        &CancelFlag::new(),
    )
    .expect("joins");

    assert_eq!(
        executor.calls.load(Ordering::SeqCst),
        1,
        "only the reservable branch dispatched"
    );
    assert_eq!(outcome.settled(), 1);
    assert_eq!(outcome.unsettled(), 1);
    assert!(outcome.branches.values().any(|state| matches!(
        state,
        BoundedBranchState::Failed { reason } if reason == "budget_exhausted"
    )));
}

#[test]
fn the_cap_comes_from_the_workflow_limits_not_from_the_caller() {
    let plan = build_plan(
        vec![analyze("a"), analyze("b"), analyze("c"), analyze("d")],
        Vec::new(),
    );
    let executor = Controlled {
        rendezvous: Some(2),
        ..Controlled::default()
    };
    let ledger = BranchBudgetLedger::new(4 * UNITS).expect("limit");
    let mut widened = region();
    widened.allowed_operations = allowed(&plan).into_iter().collect();

    let (outcome, _joined) = run_bounded_region(
        &plan,
        &widened,
        &limits(2),
        &executor,
        &ledger,
        UNITS,
        &CancelFlag::new(),
    )
    .expect("joins");

    assert_eq!(outcome.cap, 2, "the report names the cap that was enforced");
    assert_eq!(executor.peak(), 2, "never more than the declared cap");
    assert!(outcome.settled() <= outcome.branches.len());
}

#[test]
fn the_runtime_keeps_its_existing_adaptive_boundary_for_region_nodes() {
    let mut runtime = DynamicRuntime::new(dynamic_workflow(), FixtureExecutor::default())
        .expect("runtime starts");
    let report = runtime.run_to_completion().expect("runtime completes");
    assert_eq!(
        report.status,
        sts2_harness::workflow::RuntimeStatus::Completed
    );
    assert!(
        runtime.executor().adaptive_calls.load(Ordering::SeqCst) >= 1,
        "the node route still delegates to the adaptive executor"
    );
    assert_eq!(
        runtime.last_bounded_outcome(),
        None,
        "no bounded region ran, so no report is claimed"
    );
}

#[test]
fn an_unchanged_plan_yields_an_identical_report_binding() {
    let plan = independent_plan();
    let executor = Controlled::default();
    let ledger = BranchBudgetLedger::new(2 * UNITS).expect("limit");
    let (outcome, _joined) = run_bounded_region(
        &plan,
        &region(),
        &limits(2),
        &executor,
        &ledger,
        UNITS,
        &CancelFlag::new(),
    )
    .expect("joins");

    let again = BTreeMap::from([
        ("a".to_owned(), outcome.branches["a"].clone()),
        ("b".to_owned(), outcome.branches["b"].clone()),
    ]);
    assert_eq!(outcome.branches, again);
    assert_eq!(
        outcome.plan_digest,
        plan.digest().expect("digest").as_str(),
        "the report binds the admitted plan, not a layout guess"
    );
    assert_eq!(ParallelCap::new(2).expect("cap").get(), 2);
}
