// SPDX-License-Identifier: MIT

//! Served-live acceptance evidence for `#94` AC1 and AC4.
//!
//! The fixtures here drive the *real* served composition — the same
//! `ProductionLiveWorkflowSession` the factory admits, the same
//! `LiveNodeExecutor`, and the same `reconcile_pending` the cancellation path
//! calls — over a runtime double that only supplies the transport outcomes.
//! Nothing but the gateway/MCP answers is faked, so the assertions below pin
//! exactly the two behaviours the acceptance criteria name:
//!
//! * an accepted `execute_action` may not advance the run until an independent
//!   settlement receipt is observed and verified, and
//! * an accepted operation that cannot be settled must stay unresolved and must
//!   never be replaced by a second dispatch.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::*;
use crate::episode::{DispatchStatus, WaitOutcome};
use crate::management::{ErrorClass, LiveWorkflowOptions, PendingOperationState};
use crate::workflow::{
    ActionId, BoundedText, DecisionProposal, EdgeOutcome, ExecuteActionConfig, Generation, GraphId,
    NodeDefinition, NodeExecutor, NodeId, OutputId, ProposalBinding, ProviderExecutionId,
    RuntimeContext, RuntimeFault, TypedValue,
};
use std::collections::BTreeMap;

use super::super::super::execution_state::{LiveNodeState, PendingDispatch};
use super::super::super::node::LiveNodeExecutor;
use super::super::super::node_projection::catalog_digest;
use super::super::super::node_recovery::reconcile_pending;

/// Bounded transport log for the runtime double.
#[derive(Default)]
struct SettlementLog {
    dispatches: Vec<(String, String)>,
    waits: Vec<String>,
    reconciles: Vec<String>,
    stops: usize,
    releases: usize,
}

#[derive(Clone, Copy)]
enum WaitMode {
    Settled,
    Timeout,
}

#[derive(Clone, Copy)]
enum ReconcileMode {
    Settled,
    Accepted,
}

struct SettlementRuntime {
    before: EpisodeObservation,
    after: EpisodeObservation,
    wait_mode: WaitMode,
    reconcile_mode: ReconcileMode,
    log: Arc<Mutex<SettlementLog>>,
}

impl SettlementRuntime {
    fn new(
        before: EpisodeObservation,
        after: EpisodeObservation,
        wait_mode: WaitMode,
        reconcile_mode: ReconcileMode,
        log: Arc<Mutex<SettlementLog>>,
    ) -> Self {
        Self {
            before,
            after,
            wait_mode,
            reconcile_mode,
            log,
        }
    }
}

fn end_turn() -> EpisodeLegalAction {
    EpisodeLegalAction::new("combat.end-turn", crate::ActionKind::EndTurn).expect("end turn")
}

impl EpisodeRuntimePort for SettlementRuntime {
    fn launch(&mut self) -> Result<(), PortError> {
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        Ok(self.before.clone())
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        EpisodeLegalActionSet::new(state_id, generation, vec![end_turn()])
            .map_err(|error| PortError::new("test_catalog", error.to_string(), false))
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        self.log
            .lock()
            .expect("log lock")
            .dispatches
            .push((identity.operation_id.clone(), action.action_id().to_owned()));
        Ok(TransitionReceipt::new(
            identity.operation_id.clone(),
            action.clone(),
            DispatchStatus::Accepted,
            None,
            None,
            None,
        ))
    }
}

impl BarrierPort for SettlementRuntime {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        self.log
            .lock()
            .expect("log lock")
            .waits
            .push(operation_id.to_owned());
        Ok(match self.wait_mode {
            WaitMode::Settled => WaitSample::new(WaitOutcome::Successor, Some(self.after.clone()))
                .with_effect_kind("end_turn"),
            WaitMode::Timeout => WaitSample::new(WaitOutcome::Timeout, None),
        })
    }
}

impl RecoveryPort for SettlementRuntime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        Ok(self.before.clone())
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        self.log
            .lock()
            .expect("log lock")
            .reconciles
            .push(operation_id.to_owned());
        Ok(match self.reconcile_mode {
            ReconcileMode::Settled => TransitionReceipt::new(
                operation_id.to_owned(),
                end_turn(),
                DispatchStatus::Settled,
                Some(self.after.clone()),
                Some("end_turn".to_owned()),
                None,
            ),
            ReconcileMode::Accepted => TransitionReceipt::new(
                operation_id.to_owned(),
                end_turn(),
                DispatchStatus::Accepted,
                None,
                None,
                None,
            ),
        })
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.log.lock().expect("log lock").releases += 1;
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        self.log.lock().expect("log lock").stops += 1;
        Ok(())
    }
}

impl ShutdownPort for SettlementRuntime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
}

fn observation(generation: u64) -> EpisodeObservation {
    EpisodeObservation::new(
        "combat-1",
        generation,
        EpisodeStage::Combat,
        true,
        false,
        true,
        serde_json::json!({
            "state_id":"combat-1",
            "generation":generation,
            "visible_seed":"fixture",
            "player":{"hp":1,"max_hp":1,"energy":1,"gold":0,"hand":[],"deck":[],"discard":[],"exhaust":[]},
            "state":{"state":"combat","turn_index":1,"enemies":[]},
            "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}]
        }),
    )
    .expect("observation fixture")
}

fn catalog() -> EpisodeLegalActionSet {
    EpisodeLegalActionSet::new("combat-1", 1, vec![end_turn()]).expect("catalog fixture")
}

/// Builds the served session over a runtime double and returns the state the
/// node executor mutates, plus the transport log.
fn served_state(
    wait_mode: WaitMode,
    reconcile_mode: ReconcileMode,
) -> (LiveNodeState, Arc<Mutex<SettlementLog>>) {
    let (mut session, _, _) = make_session(Change::HistoryOnlyDuringInference);
    let log = Arc::new(Mutex::new(SettlementLog::default()));
    session.runtime = Box::new(SettlementRuntime::new(
        observation(1),
        observation(2),
        wait_mode,
        reconcile_mode,
        Arc::clone(&log),
    ));
    let state = LiveNodeState {
        session: Box::new(session),
        instance_id: "test-instance".to_owned(),
        observation: Some(observation(1)),
        actions: Some(catalog()),
        pending: None,
        pending_decision: None,
        provider_calls: 0,
        max_provider_calls: 8,
        options: LiveWorkflowOptions::default(),
    };
    (state, log)
}

fn proposal() -> DecisionProposal {
    let actions = catalog();
    DecisionProposal {
        state_id: BoundedText::new("combat-1").expect("state"),
        generation: Generation::new(1).expect("generation"),
        catalog_digest: catalog_digest(&actions).expect("catalog digest"),
        action_id: ActionId::new("combat.end-turn").expect("action"),
        provider_execution_id: ProviderExecutionId::new("execution-1").expect("execution"),
        reason_code: None,
    }
}

fn execute_context() -> RuntimeContext {
    let proposal_node = NodeId::new("decide-1").expect("decide node");
    let mut outputs = BTreeMap::new();
    outputs.insert(
        format!("{proposal_node}/proposal"),
        TypedValue::DecisionProposal(Box::new(proposal())),
    );
    RuntimeContext {
        graph_id: GraphId::new("graph-1").expect("graph"),
        node_id: NodeId::new("execute-1").expect("node"),
        step: 1,
        outputs,
        guard_fields: BTreeMap::new(),
    }
}

fn execute_node() -> NodeDefinition {
    NodeDefinition::ExecuteAction {
        id: NodeId::new("execute-1").expect("node"),
        config: ExecuteActionConfig {
            proposal_from: ProposalBinding {
                node_id: NodeId::new("decide-1").expect("decide node"),
                output: OutputId::new("proposal").expect("output"),
            },
        },
    }
}

/// `#94` AC1: an accepted dispatch advances only after the independent
/// settlement receipt is observed and verified, and it dispatches exactly once.
#[test]
fn served_live_execute_action_advances_only_after_verified_settlement() {
    let (mut state, log) = served_state(WaitMode::Settled, ReconcileMode::Settled);

    let outcome = LiveNodeExecutor {
        state: &mut state,
        intent_recorder: None,
    }
    .execute(&execute_node(), &execute_context())
    .expect("a settled accepted action advances");

    assert_eq!(outcome.outcome, EdgeOutcome::Ok);
    let log = log.lock().expect("log lock");
    assert_eq!(log.dispatches.len(), 1, "exactly one mutating dispatch");
    assert_eq!(log.dispatches[0].1, "combat.end-turn");
    assert_eq!(log.waits.len(), 1, "exactly one settlement wait");
    assert_eq!(
        log.waits[0], log.dispatches[0].0,
        "the wait observes the same operation the dispatch installed"
    );
    drop(log);
    assert!(state.pending.is_none(), "the settled operation is released");
    assert_eq!(
        state
            .observation
            .as_ref()
            .expect("advanced observation")
            .generation(),
        2,
        "the run advanced only to the settled observation"
    );
}

/// `#94` AC1: an accepted dispatch whose settlement cannot be observed must not
/// advance the run and must not be settled synthetically.
#[test]
fn served_live_execute_action_refuses_to_advance_without_settlement() {
    let (mut state, log) = served_state(WaitMode::Timeout, ReconcileMode::Settled);

    let fault = LiveNodeExecutor {
        state: &mut state,
        intent_recorder: None,
    }
    .execute(&execute_node(), &execute_context())
    .expect_err("an unobserved settlement is unknown, never success");

    assert_eq!(fault, RuntimeFault::UnknownEffect);
    let log = log.lock().expect("log lock");
    assert_eq!(log.dispatches.len(), 1, "the mutation was attempted once");
    assert!(
        log.reconciles.is_empty(),
        "no replacement or reconciliation effect is fabricated at the node"
    );
    drop(log);
    assert_eq!(
        state
            .observation
            .as_ref()
            .expect("retained observation")
            .generation(),
        1,
        "the run did not advance past the accepted dispatch"
    );
    let pending = state.pending.as_ref().expect("accepted operation retained");
    assert_eq!(pending.state, PendingOperationState::Accepted);
    assert!(pending.resolved.is_none());
}

/// `#94` AC4: cancellation reconciles the retained accepted operation by its
/// exact identity and settles it without dispatching a replacement.
#[test]
fn served_live_cancel_reconciles_the_accepted_operation_once() {
    let (mut state, log) = served_state(WaitMode::Timeout, ReconcileMode::Settled);
    let identity =
        ActionIdentity::new("op-accepted-1", "combat-1", 1, "combat.end-turn").expect("identity");
    state.pending = Some(PendingDispatch {
        identity: identity.clone(),
        action: end_turn(),
        state: PendingOperationState::Accepted,
        resolved: None,
    });

    reconcile_pending(&mut state).expect("a served settlement resolves the operation");

    let log = log.lock().expect("log lock");
    assert_eq!(log.reconciles, ["op-accepted-1"]);
    assert!(
        log.dispatches.is_empty(),
        "cancellation never re-dispatches"
    );
    drop(log);
    let pending = state.pending.as_ref().expect("retained operation");
    let resolved = pending.resolved.as_ref().expect("resolved settlement");
    assert_eq!(resolved.status(), DispatchStatus::Settled);
    assert_eq!(resolved.operation_id(), "op-accepted-1");
}

/// `#94` AC4: an accepted operation that reconciliation still cannot settle is
/// reported unresolved, retained as unknown, and never replaced.
#[test]
fn served_live_cancel_retains_an_unresolved_accepted_operation() {
    let (mut state, log) = served_state(WaitMode::Timeout, ReconcileMode::Accepted);
    let identity =
        ActionIdentity::new("op-accepted-2", "combat-1", 1, "combat.end-turn").expect("identity");
    state.pending = Some(PendingDispatch {
        identity: identity.clone(),
        action: end_turn(),
        state: PendingOperationState::Accepted,
        resolved: None,
    });

    let error = reconcile_pending(&mut state).expect_err("still-accepted stays unresolved");

    assert_eq!(error.class, ErrorClass::Unresolved);
    assert_eq!(error.code, "live_operation_unknown");
    let log = log.lock().expect("log lock");
    assert_eq!(log.reconciles, ["op-accepted-2"]);
    assert!(
        log.dispatches.is_empty(),
        "an unresolved operation is never replaced"
    );
    drop(log);
    let pending = state
        .pending
        .as_ref()
        .expect("operation retained for the operator");
    assert_eq!(pending.state, PendingOperationState::Unknown);
    assert!(pending.resolved.is_none());
}
