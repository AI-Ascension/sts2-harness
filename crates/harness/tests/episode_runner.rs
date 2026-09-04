// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use serde_json::{Value, json};
use sts2_harness::{
    ActionIdentity, ActionKind, Decision, DecisionInput, DecisionSource, DispatchStatus,
    BarrierError, BarrierPort, EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation,
    EpisodeRunner, EpisodeRunnerConfig, EpisodeRunnerError, EpisodeRuntimePort, EpisodeStage,
    PolicyError, PortError, RecoveryController, RecoveryError, RecoveryPort, RecoveryResult,
    ShutdownError, ShutdownPort, StabilityBarrier, TransitionReceipt, WaitOutcome, WaitSample,
};

#[derive(Clone)]
struct State {
    observation: EpisodeObservation,
    actions: EpisodeLegalActionSet,
}

struct PendingTransition {
    operation_id: String,
    action: EpisodeLegalAction,
    after: EpisodeObservation,
}

struct FakeRuntime {
    states: Vec<State>,
    index: usize,
    pending: Option<PendingTransition>,
    fail_first_dispatch: bool,
    dispatches: usize,
    reconciles: usize,
    launched: bool,
    released: bool,
    mcp_closed: bool,
    gateway_closed: bool,
}

impl FakeRuntime {
    fn new(states: Vec<State>) -> Self {
        Self {
            states,
            index: 0,
            pending: None,
            fail_first_dispatch: false,
            dispatches: 0,
            reconciles: 0,
            launched: false,
            released: false,
            mcp_closed: false,
            gateway_closed: false,
        }
    }

    fn current(&self) -> &State {
        &self.states[self.index]
    }

    fn next_observation(&self) -> Option<EpisodeObservation> {
        self.states
            .get(self.index + 1)
            .map(|state| state.observation.clone())
    }
}

impl EpisodeRuntimePort for FakeRuntime {
    fn launch(&mut self) -> Result<(), PortError> {
        self.launched = true;
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        Ok(self.current().observation.clone())
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        let state = self.current();
        if state.observation.state_id() != state_id
            || state.observation.generation() != generation
        {
            return Err(PortError::new(
                "stale_catalog",
                "fake catalog identity does not match observation",
                false,
            ));
        }
        Ok(state.actions.clone())
    }

    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        if self.pending.is_some() {
            return Err(PortError::new(
                "pending_transition",
                "fake runtime already has a pending transition",
                false,
            ));
        }
        let after = self.next_observation().ok_or_else(|| {
            PortError::new("terminal_runtime", "fake runtime has no successor", false)
        })?;
        self.pending = Some(PendingTransition {
            operation_id: identity.operation_id.clone(),
            action: action.clone(),
            after,
        });
        self.dispatches += 1;
        if self.fail_first_dispatch && self.dispatches == 1 {
            return Err(PortError::new(
                "transport_uncertain",
                "fake transport failed after admission",
                true,
            ));
        }
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

impl BarrierPort for FakeRuntime {
    fn wait_for_transition(
        &mut self,
        operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        let Some(pending) = self.pending.take() else {
            return Ok(WaitSample::new(WaitOutcome::Timeout, None));
        };
        if pending.operation_id != operation_id {
            return Err(BarrierError::InvalidOperation);
        }
        self.index += 1;
        Ok(WaitSample::new(WaitOutcome::Successor, Some(pending.after))
            .with_effect_kind("host.semantic.settled"))
    }
}

impl RecoveryPort for FakeRuntime {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        Ok(self.current().observation.clone())
    }

    fn reconcile(&mut self, operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        let Some(pending) = self.pending.take() else {
            return Err(RecoveryError::PortFailure);
        };
        if pending.operation_id != operation_id {
            return Err(RecoveryError::InvalidOperation);
        }
        self.index += 1;
        self.reconciles += 1;
        Ok(TransitionReceipt::new(
            pending.operation_id,
            pending.action,
            DispatchStatus::Settled,
            Some(pending.after),
            Some(String::from("host.semantic.reconciled")),
            None,
        ))
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        self.released = true;
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }
}

impl ShutdownPort for FakeRuntime {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.released = true;
        Ok(())
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        self.mcp_closed = true;
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        self.gateway_closed = true;
        Ok(())
    }
}

#[derive(Default)]
struct FakeModel {
    calls: usize,
    unavailable: bool,
}

impl DecisionSource for FakeModel {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.calls += 1;
        if self.unavailable {
            return Err(PolicyError::ProviderUnavailable);
        }
        let action = input
            .legal_actions
            .actions()
            .first()
            .ok_or(PolicyError::IllegalAction)?;
        Ok(Decision::Action {
            action_id: action.action_id().to_owned(),
            rationale: String::from("bounded fake provider decision"),
            confidence: Some(75),
        })
    }
}

fn state(stage: EpisodeStage, generation: u64) -> State {
    let state_id = format!("{}-{generation}", stage_name(stage));
    let action_id = format!("{}-action", stage_name(stage));
    let kind = match stage {
        EpisodeStage::Setup => ActionKind::StartRun,
        EpisodeStage::Map => ActionKind::SelectMapNode,
        EpisodeStage::Combat => ActionKind::EndTurn,
        EpisodeStage::Reward => ActionKind::ChooseReward,
        EpisodeStage::Shop => ActionKind::ShopPurchase,
        EpisodeStage::Event => ActionKind::EventChoice,
        EpisodeStage::Rest => ActionKind::Rest,
        EpisodeStage::Selection => ActionKind::SelectCard,
        EpisodeStage::Victory | EpisodeStage::Defeat => ActionKind::SaveQuit,
        EpisodeStage::Unknown | EpisodeStage::Recovery => ActionKind::SaveQuit,
    };
    let observation = EpisodeObservation::new(
        state_id.clone(),
        generation,
        stage,
        !stage.is_terminal(),
        false,
        !stage.is_terminal(),
        projection(&state_id, generation, stage),
    )
    .expect("state projection is valid");
    let action = EpisodeLegalAction::new(action_id, kind).expect("action is valid");
    let actions = EpisodeLegalActionSet::new(state_id, generation, vec![action])
        .expect("action set is valid");
    State {
        observation,
        actions,
    }
}

fn projection(state_id: &str, generation: u64, stage: EpisodeStage) -> Value {
    let state = match stage {
        EpisodeStage::Setup => json!({"state":"setup","characters":[]}),
        EpisodeStage::Map => json!({"state":"map","node_id":"node-1","options":[]}),
        EpisodeStage::Combat => json!({"state":"combat","turn_index":1,"enemies":[]}),
        EpisodeStage::Reward | EpisodeStage::Rest => json!({"state":stage_name(stage),"options":[]}),
        EpisodeStage::Shop => json!({"state":"shop","items":[]}),
        EpisodeStage::Event | EpisodeStage::Selection => {
            json!({"state":stage_name(stage),"choices":[]})
        }
        EpisodeStage::Victory => json!({"state":"victory"}),
        EpisodeStage::Defeat => json!({"state":"defeat","reason":"test"}),
        EpisodeStage::Unknown | EpisodeStage::Recovery => {
            json!({"state":"recovery","code":"test"})
        }
    };
    json!({
        "state_id": state_id,
        "generation": generation,
        "visible_seed": "visible-seed-only",
        "player": {"hp":50,"max_hp":50,"energy":3,"gold":99,"hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state": state,
        "legal_actions": [{"action_id": format!("{}-action", stage_name(stage)), "action": {"kind":"end_turn"}}]
    })
}

fn stage_name(stage: EpisodeStage) -> &'static str {
    match stage {
        EpisodeStage::Setup => "setup",
        EpisodeStage::Map => "map",
        EpisodeStage::Combat => "combat",
        EpisodeStage::Reward => "reward",
        EpisodeStage::Shop => "shop",
        EpisodeStage::Event => "event",
        EpisodeStage::Rest => "rest",
        EpisodeStage::Selection => "selection",
        EpisodeStage::Victory => "victory",
        EpisodeStage::Defeat => "defeat",
        EpisodeStage::Recovery => "recovery",
        EpisodeStage::Unknown => "unknown",
    }
}

fn runner() -> EpisodeRunner {
    let barrier = StabilityBarrier::new(2, 1).expect("barrier is valid");
    let recovery = RecoveryController::new(1).expect("recovery is valid");
    EpisodeRunner::new(
        EpisodeRunnerConfig::new(
            16,
            barrier,
            recovery,
            "complete the run",
            vec![String::from("use only current host legal actions")],
        )
        .expect("runner configuration is valid"),
    )
}

fn complete_states() -> Vec<State> {
    [
        EpisodeStage::Setup,
        EpisodeStage::Map,
        EpisodeStage::Combat,
        EpisodeStage::Reward,
        EpisodeStage::Shop,
        EpisodeStage::Event,
        EpisodeStage::Rest,
        EpisodeStage::Selection,
        EpisodeStage::Victory,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, stage)| state(stage, index as u64))
    .collect()
}

#[test]
fn runner_routes_every_playable_surface_and_verifies_terminal_transition() {
    let mut runtime = FakeRuntime::new(complete_states());
    let mut model = FakeModel::default();
    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("fake run should complete");
    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(report.transitions(), 8);
    assert_eq!(report.steps(), 8);
    assert_eq!(report.recoveries(), 0);
    assert_eq!(model.calls, 8);
    assert_eq!(runtime.dispatches, 8);
    assert!(runtime.launched);
    assert!(runtime.released);
    assert!(runtime.mcp_closed);
    assert!(runtime.gateway_closed);
}

#[test]
fn provider_failure_is_fail_closed_and_never_dispatches() {
    let mut runtime = FakeRuntime::new(complete_states());
    let mut model = FakeModel {
        calls: 0,
        unavailable: true,
    };
    let error = runner()
        .run(&mut runtime, &mut model)
        .expect_err("provider failure must stop the run");
    assert!(matches!(error, EpisodeRunnerError::Policy(PolicyError::ProviderUnavailable)));
    assert_eq!(runtime.dispatches, 0);
    assert!(runtime.released);
    assert!(runtime.mcp_closed);
    assert!(runtime.gateway_closed);
}

#[test]
fn uncertain_dispatch_is_reconciled_without_a_strategic_retry() {
    let mut runtime = FakeRuntime::new(complete_states());
    runtime.fail_first_dispatch = true;
    let mut model = FakeModel::default();
    let report = runner()
        .run(&mut runtime, &mut model)
        .expect("reconciliation should settle the admitted operation");
    assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
    assert_eq!(runtime.dispatches, 8);
    assert_eq!(runtime.reconciles, 1);
    assert_eq!(report.recoveries(), 1);
}
