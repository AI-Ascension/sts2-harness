// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::cell::Cell;

use serde_json::json;
use sts2_harness::workflow::{
    ActionId, BoundedText, CompiledWorkflow, DecisionProposal, Digest, DynamicExecutorPort,
    DynamicRuntime, EdgeOutcome, Generation, NodeDefinition, NodeExecutor, NodeKind, NodeOutcome,
    RuntimeContext, RuntimeFault, RuntimeStatus, TypedValue, WorkflowDefinition, decode_strict,
};

const VALID: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn dynamic_workflow() -> CompiledWorkflow {
    let mut value: serde_json::Value = serde_json::from_slice(VALID).expect("fixture JSON");
    value["mode"] = json!("dynamic");
    value["graphs"][0]["nodes"][1] = json!({
        "id": "decide",
        "kind": "adaptive_region",
        "config": {
            "region_id": "region-1",
            "planner_profile_ref": "planner-1",
            "allowed_operations": ["map.inspect"],
            "max_plan_nodes": 4,
            "max_plan_edges": 4,
            "max_replans": 2,
            "output_type": "DecisionProposal"
        }
    });
    let bytes = serde_json::to_vec(&value).expect("dynamic fixture JSON");
    let definition: WorkflowDefinition = decode_strict(&bytes).expect("dynamic fixture decodes");
    CompiledWorkflow::compile(definition).expect("dynamic fixture compiles")
}

#[derive(Default)]
struct DynamicFixtureExecutor {
    adaptive_calls: Cell<u32>,
}

impl NodeExecutor for DynamicFixtureExecutor {
    fn execute(
        &mut self,
        node: &NodeDefinition,
        _context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        let value = match node.kind() {
            NodeKind::Observe => {
                TypedValue::Observation(sts2_harness::workflow::ObservationValue {
                    state_id: BoundedText::new("state-0").expect("state id"),
                    generation: Generation::new(0).expect("generation"),
                    fields: Default::default(),
                })
            }
            _ => TypedValue::Null,
        };
        Ok(NodeOutcome::new(EdgeOutcome::Ok, value))
    }
}

impl DynamicExecutorPort for DynamicFixtureExecutor {
    fn execute_adaptive(
        &mut self,
        _config: &sts2_harness::workflow::AdaptiveRegionConfig,
        _context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        self.adaptive_calls
            .set(self.adaptive_calls.get().saturating_add(1));
        Ok(NodeOutcome::new(
            EdgeOutcome::Ok,
            TypedValue::DecisionProposal(Box::new(DecisionProposal {
                state_id: BoundedText::new("state-0").expect("state id"),
                generation: Generation::new(0).expect("generation"),
                catalog_digest: Digest::sha256(b"catalog"),
                action_id: ActionId::new("end-turn").expect("action id"),
                provider_execution_id: sts2_harness::workflow::ProviderExecutionId::new(
                    "provider-0",
                )
                .expect("provider id"),
                reason_code: None,
            })),
        ))
    }
}

#[test]
fn dynamic_runtime_routes_adaptive_output_through_the_same_action_boundary() {
    let mut runtime = DynamicRuntime::new(dynamic_workflow(), DynamicFixtureExecutor::default())
        .expect("runtime starts");
    let report = runtime.run_to_completion().expect("runtime completes");
    assert_eq!(report.status, RuntimeStatus::Completed);
    assert_eq!(report.steps, 4);
    assert_eq!(runtime.status(), RuntimeStatus::Completed);
    assert!(runtime.executor().adaptive_calls.get() >= 1);
}
