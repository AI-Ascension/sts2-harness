// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use std::cell::Cell;

use sts2_harness::workflow::{
    ActionId, BoundedText, CompiledWorkflow, DecisionProposal, Digest, EdgeOutcome, Generation,
    NodeDefinition, NodeExecutor, NodeKind, NodeOutcome, RuntimeContext, RuntimeFault,
    RuntimeStatus, StrictRuntime, TypedValue, decode_strict,
};

const VALID: &[u8] = include_bytes!("../../../conformance/workflow-v1/valid-strict.json");

fn compiled() -> CompiledWorkflow {
    let definition = decode_strict(VALID).expect("fixture decodes");
    CompiledWorkflow::compile(definition).expect("fixture compiles")
}

#[derive(Default)]
struct SyntheticExecutor {
    unknown_effect: bool,
    calls: Cell<u64>,
}

impl NodeExecutor for SyntheticExecutor {
    fn execute(
        &mut self,
        node: &NodeDefinition,
        _context: &RuntimeContext,
    ) -> Result<NodeOutcome, RuntimeFault> {
        self.calls.set(self.calls.get().saturating_add(1));
        if node.kind() == NodeKind::ExecuteAction && self.unknown_effect {
            return Err(RuntimeFault::UnknownEffect);
        }
        let value = match node.kind() {
            NodeKind::Observe => {
                TypedValue::Observation(sts2_harness::workflow::ObservationValue {
                    state_id: BoundedText::new("state-0").expect("state id"),
                    generation: Generation::new(0).expect("generation"),
                    fields: Default::default(),
                })
            }
            NodeKind::Decide => TypedValue::DecisionProposal(Box::new(DecisionProposal {
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
            _ => TypedValue::Null,
        };
        Ok(NodeOutcome::new(EdgeOutcome::Ok, value))
    }
}

#[test]
fn strict_runtime_executes_the_immutable_graph_and_records_progress() {
    let mut runtime = StrictRuntime::new(compiled()).expect("runtime starts");
    let mut executor = SyntheticExecutor::default();
    let report = runtime
        .run_to_completion(&mut executor)
        .expect("runtime completes");
    assert_eq!(report.status, RuntimeStatus::Completed);
    assert_eq!(report.steps, 4);
    assert_eq!(executor.calls.get(), 3);
    assert_eq!(report.events.len(), 4);
    assert!(
        report
            .events
            .windows(2)
            .all(|events| { events[0].sequence < events[1].sequence })
    );
}

#[test]
fn pause_and_restart_preserve_the_cursor_without_replaying_prior_nodes() {
    let mut runtime = StrictRuntime::new(compiled()).expect("runtime starts");
    let mut first_executor = SyntheticExecutor::default();
    runtime.step(&mut first_executor).expect("observe runs");
    runtime.pause().expect("pause is accepted");
    let snapshot = runtime.snapshot();
    assert_eq!(runtime.status(), RuntimeStatus::Paused);

    let mut resumed =
        StrictRuntime::from_snapshot(compiled(), snapshot).expect("snapshot restores");
    resumed.resume().expect("resume is accepted");
    let mut second_executor = SyntheticExecutor::default();
    let report = resumed
        .run_to_completion(&mut second_executor)
        .expect("resumed runtime completes");
    assert_eq!(report.status, RuntimeStatus::Completed);
    assert_eq!(first_executor.calls.get(), 1);
    assert_eq!(second_executor.calls.get(), 2);
}

#[test]
fn a_pending_effect_fails_closed_and_cannot_reach_terminal_success() {
    let mut runtime = StrictRuntime::new(compiled()).expect("runtime starts");
    let mut executor = SyntheticExecutor {
        unknown_effect: true,
        calls: Cell::new(0),
    };
    let result = runtime.run_to_completion(&mut executor);
    assert_eq!(result, Err(RuntimeFault::UnknownEffect));
    assert_eq!(runtime.status(), RuntimeStatus::NeedsOperator);
}

#[test]
fn output_types_are_checked_before_edges_advance() {
    struct WrongType;
    impl NodeExecutor for WrongType {
        fn execute(
            &mut self,
            _node: &NodeDefinition,
            _context: &RuntimeContext,
        ) -> Result<NodeOutcome, RuntimeFault> {
            Ok(NodeOutcome::new(EdgeOutcome::Ok, TypedValue::Null))
        }
    }

    let mut runtime = StrictRuntime::new(compiled()).expect("runtime starts");
    assert_eq!(
        runtime.step(&mut WrongType),
        Err(RuntimeFault::TypeMismatch)
    );
    assert_eq!(runtime.snapshot().steps, 0);
}
