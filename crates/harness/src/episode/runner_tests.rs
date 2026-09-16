// SPDX-License-Identifier: MIT

use super::*;
use crate::Decision;
use crate::episode::{
    BarrierError, DecisionInput, PolicyError, RecoveryError, ShutdownError, WaitSample,
};

struct FailingBindingPort {
    calls: Vec<&'static str>,
}

impl BarrierPort for FailingBindingPort {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        unreachable!("binding preparation must run before an episode observation")
    }
}

impl RecoveryPort for FailingBindingPort {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        unreachable!("binding preparation must run before an episode observation")
    }

    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        unreachable!("binding preparation must run before an episode observation")
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        unreachable!("the shutdown port owns cleanup in this test")
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        unreachable!("binding preparation must run before recovery")
    }
}

impl ShutdownPort for FailingBindingPort {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.calls.push("release");
        Ok(())
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        self.calls.push("mcp");
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        self.calls.push("gateway");
        Ok(())
    }
}

impl EpisodeRuntimePort for FailingBindingPort {
    fn launch(&mut self) -> Result<(), PortError> {
        self.calls.push("launch");
        Ok(())
    }

    fn prepare_game_information_binding(&mut self) -> Result<(), PortError> {
        self.calls.push("binding");
        Err(PortError::new("binding", "unavailable", false))
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        unreachable!("binding preparation must run before an episode observation")
    }

    fn legal_actions(
        &mut self,
        _state_id: &str,
        _generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        unreachable!("binding preparation must run before legal-action discovery")
    }

    fn dispatch_action(
        &mut self,
        _identity: &ActionIdentity,
        _action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        unreachable!("binding preparation must run before action dispatch")
    }
}

struct UnusedSource;

impl super::super::policy_router::DecisionSource for UnusedSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        unreachable!("binding preparation must run before provider delivery")
    }
}

#[test]
fn binding_preparation_is_before_observation_and_cleanup_is_retained()
-> Result<(), Box<dyn std::error::Error>> {
    let config = EpisodeRunnerConfig::new(
        1,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "test",
        Vec::new(),
    )?;
    let mut port = FailingBindingPort { calls: Vec::new() };
    let result = EpisodeRunner::new(config).run(&mut port, &mut UnusedSource);

    assert!(matches!(
        result,
        Err(EpisodeRunnerError::GameInformationBinding(error)) if error.code() == "binding"
    ));
    assert_eq!(
        port.calls,
        ["launch", "binding", "release", "mcp", "gateway"]
    );
    Ok(())
}

struct FailingDecisionBindingPort {
    calls: Vec<&'static str>,
}

impl BarrierPort for FailingDecisionBindingPort {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        unreachable!("the actionable fixture does not need a barrier")
    }
}

impl RecoveryPort for FailingDecisionBindingPort {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        unreachable!("a failed lookup fence must stop before recovery")
    }

    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        unreachable!("the lookup fence must stop before dispatch")
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }
}

impl ShutdownPort for FailingDecisionBindingPort {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        self.calls.push("release");
        Ok(())
    }

    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        self.calls.push("mcp");
        Ok(())
    }

    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        self.calls.push("gateway");
        Ok(())
    }
}

impl EpisodeRuntimePort for FailingDecisionBindingPort {
    fn launch(&mut self) -> Result<(), PortError> {
        self.calls.push("launch");
        Ok(())
    }

    fn prepare_game_information_binding(&mut self) -> Result<(), PortError> {
        self.calls.push("binding");
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        self.calls.push("observe");
        EpisodeObservation::new(
            "state-1",
            1,
            super::super::observation::EpisodeStage::Combat,
            true,
            false,
            true,
            serde_json::json!({
                "state_id":"state-1",
                "generation":1,
                "visible_seed":null,
                "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,
                    "hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"combat","turn_index":1,"enemies":[]},
                "legal_actions":[{"action_id":"end-turn","action":{"kind":"end_turn"}}]
            }),
        )
        .map_err(|error| PortError::new("observation", error.to_string(), false))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        self.calls.push("legal");
        EpisodeLegalActionSet::new(
            state_id,
            generation,
            vec![
                crate::EpisodeLegalAction::new("end-turn", crate::ActionKind::EndTurn)
                    .map_err(|error| PortError::new("action_set", error.to_string(), false))?,
            ],
        )
        .map_err(|error| PortError::new("action_set", error.to_string(), false))
    }

    fn refresh_game_information_binding(
        &mut self,
        _state_id: &str,
        _generation: u64,
    ) -> Result<(), PortError> {
        self.calls.push("refresh");
        Err(PortError::new(
            "game_information_binding_unavailable",
            "fixture refuses stale lookup authority",
            false,
        ))
    }

    fn dispatch_action(
        &mut self,
        _identity: &ActionIdentity,
        _action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        unreachable!("a failed lookup fence must stop before dispatch")
    }
}

#[test]
fn per_decision_binding_refresh_precedes_provider_and_action_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let config = EpisodeRunnerConfig::new(
        1,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "test",
        Vec::new(),
    )?;
    let mut port = FailingDecisionBindingPort { calls: Vec::new() };
    let result = EpisodeRunner::new(config).run(&mut port, &mut UnusedSource);
    assert!(matches!(
        result,
        Err(EpisodeRunnerError::LegalActions(error))
            if error.code() == "game_information_binding_unavailable"
    ));
    assert_eq!(
        port.calls,
        [
            "launch", "binding", "observe", "legal", "refresh", "release", "mcp", "gateway"
        ]
    );
    Ok(())
}
