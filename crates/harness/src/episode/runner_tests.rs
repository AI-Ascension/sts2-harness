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
