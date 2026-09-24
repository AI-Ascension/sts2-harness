// SPDX-License-Identifier: MIT

//! AC5: the production dynamic runtime reaches the bounded parallel analysis
//! route, reports actual branch states, and cannot reach a game mutation.

#![allow(clippy::expect_used)]

#[path = "workflow_dynamic_parallel/controlled.rs"]
#[allow(dead_code)]
mod controlled;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::json;
use sts2_harness::workflow::{
    AdaptiveRegionConfig, BOUNDED_ANALYSIS_REPORT_SCHEMA, BoundedBranchState, BranchBudgetLedger,
    BranchOutcome, CancelFlag, CompiledWorkflow, Digest, DynamicExecutorPort, DynamicPlan,
    DynamicRuntime, EdgeOutcome, Generation, NodeDefinition, NodeExecutor, NodeOutcome,
    RuntimeContext, RuntimeFault, TypedValue, WorkflowDefinition, decode_strict,
    run_bounded_region,
};

use controlled::{Controlled, analyze, limits, node, plan};

#[path = "workflow_bounded_region/refusals.rs"]
mod refusals;

const VALID: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");
pub(crate) const UNITS: u64 = 3;

pub(crate) fn region() -> AdaptiveRegionConfig {
    AdaptiveRegionConfig {
        region_id: sts2_harness::workflow::RegionId::new("region-1").expect("region"),
        planner_profile_ref: sts2_harness::workflow::PlannerProfileId::new("planner-1")
            .expect("planner"),
        allowed_operations: vec![
            sts2_harness::workflow::OperationRef::new("map.inspect.a").expect("op a"),
            sts2_harness::workflow::OperationRef::new("map.inspect.b").expect("op b"),
        ],
        max_plan_nodes: 4,
        max_plan_edges: 4,
        max_replans: 2,
        output_type: sts2_harness::workflow::AdaptiveOutputType::DecisionProposal,
    }
}

pub(crate) fn independent_plan() -> DynamicPlan {
    plan(vec![analyze("a"), analyze("b")], Vec::new())
}

#[derive(Default)]
pub(crate) struct FixtureExecutor {
    adaptive_calls: AtomicUsize,
}

impl NodeExecutor for FixtureExecutor {
    fn execute(
        &mut self,
        node: &NodeDefinition,
        _context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        let value = match node.kind() {
            sts2_harness::workflow::NodeKind::Observe => {
                TypedValue::Observation(sts2_harness::workflow::ObservationValue {
                    state_id: sts2_harness::workflow::BoundedText::new("state-0")
                        .expect("state id"),
                    generation: Generation::new(0).expect("generation"),
                    fields: Default::default(),
                })
            }
            _ => TypedValue::Null,
        };
        Ok(NodeOutcome::new(EdgeOutcome::Ok, value))
    }
}

impl DynamicExecutorPort for FixtureExecutor {
    fn execute_adaptive(
        &mut self,
        _config: &AdaptiveRegionConfig,
        _context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        self.adaptive_calls.fetch_add(1, Ordering::SeqCst);
        Ok(NodeOutcome::new(
            EdgeOutcome::Ok,
            TypedValue::DecisionProposal(Box::new(sts2_harness::workflow::DecisionProposal {
                state_id: sts2_harness::workflow::BoundedText::new("state-0").expect("state id"),
                generation: Generation::new(0).expect("generation"),
                catalog_digest: Digest::sha256(b"catalog"),
                action_id: sts2_harness::workflow::ActionId::new("end-turn").expect("action"),
                provider_execution_id: sts2_harness::workflow::ProviderExecutionId::new(
                    "provider-0",
                )
                .expect("provider"),
                reason_code: None,
            })),
        ))
    }
}

pub(crate) fn dynamic_workflow() -> CompiledWorkflow {
    let mut value: serde_json::Value = serde_json::from_slice(VALID).expect("fixture JSON");
    value["mode"] = json!("dynamic");
    value["graphs"][0]["nodes"][1] = json!({
        "id": "decide",
        "kind": "adaptive_region",
        "config": {
            "region_id": "region-1",
            "planner_profile_ref": "planner-1",
            "allowed_operations": ["map.inspect.a", "map.inspect.b"],
            "max_plan_nodes": 4,
            "max_plan_edges": 4,
            "max_replans": 2,
            "output_type": "DecisionProposal"
        }
    });
    let bytes = serde_json::to_vec(&value).expect("dynamic fixture JSON");
    let definition: WorkflowDefinition = decode_strict(&bytes).expect("decodes");
    CompiledWorkflow::compile(definition).expect("compiles")
}

#[test]
fn the_production_runtime_executes_a_declared_region_on_the_bounded_route() {
    let mut runtime = DynamicRuntime::new(dynamic_workflow(), FixtureExecutor::default())
        .expect("runtime starts");
    let plan = independent_plan();
    let executor = Controlled::default();
    let ledger = BranchBudgetLedger::new(2 * UNITS).expect("limit");

    let outcome = runtime
        .execute_bounded_region(
            &plan,
            &region(),
            &executor,
            &ledger,
            UNITS,
            &CancelFlag::new(),
        )
        .expect("bounded region executes");

    assert_eq!(outcome.schema, BOUNDED_ANALYSIS_REPORT_SCHEMA);
    assert_eq!(outcome.settled(), 2);
    assert_eq!(outcome.unsettled(), 0);
    assert_eq!(executor.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        runtime.last_bounded_outcome().map(|last| last.settled()),
        Some(2),
        "the runtime retains the report for a consumer"
    );
}

#[test]
fn a_report_names_each_branch_state_instead_of_a_count() {
    let plan = plan(vec![analyze("a"), analyze("b")], Vec::new());
    let executor = Controlled {
        unknown: BTreeSet::from(["b".to_owned()]),
        ..Controlled::default()
    };
    let ledger = BranchBudgetLedger::new(2 * UNITS).expect("limit");

    let (outcome, joined) = run_bounded_region(
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
        outcome.branches.get("a"),
        Some(&BoundedBranchState::Settled {
            code: "a<-".to_owned()
        })
    );
    assert_eq!(
        outcome.branches.get("b"),
        Some(&BoundedBranchState::Unknown)
    );
    assert_eq!(outcome.settled(), 1);
    assert_eq!(outcome.unsettled(), 1);
    assert_eq!(
        outcome.plan_digest,
        joined.plan_digest.as_str(),
        "the report is bound to the exact plan"
    );
    assert_eq!(outcome.join_digest, joined.join_digest.as_str());
    assert_eq!(
        joined.outcomes.get(&node("b")),
        Some(&BranchOutcome::Unknown)
    );
}
