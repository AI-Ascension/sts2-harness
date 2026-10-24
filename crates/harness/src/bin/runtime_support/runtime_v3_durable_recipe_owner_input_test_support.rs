// SPDX-License-Identifier: MIT

use serde_json::Value;
use sts2_harness::{
    ActionIdentity, ActionKind, BarrierError, BarrierPort, Decision, DecisionInput, DecisionSource,
    EpisodeLegalAction, EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner,
    EpisodeRunnerConfig, EpisodeRunnerError, EpisodeRuntimePort, EpisodeStage, ModelExecutionId,
    PolicyError, PortError, RecoveryController, RecoveryError, RecoveryPort, ShutdownError,
    ShutdownPort, StabilityBarrier, TransitionReceipt, WaitSample,
};

struct OwnerMapPort {
    response: Value,
}

impl BarrierPort for OwnerMapPort {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}

impl RecoveryPort for OwnerMapPort {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        Err(RecoveryError::PortFailure)
    }

    fn reconcile(&mut self, _operation_id: &str) -> Result<TransitionReceipt, RecoveryError> {
        Err(RecoveryError::PortFailure)
    }

    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }

    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        Ok(())
    }
}

impl ShutdownPort for OwnerMapPort {
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

impl EpisodeRuntimePort for OwnerMapPort {
    fn launch(&mut self) -> Result<(), PortError> {
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        EpisodeObservation::new(
            "state-1",
            1,
            EpisodeStage::Map,
            true,
            false,
            true,
            serde_json::json!({
                "state_id":"state-1","generation":1,"visible_seed":null,
                "player":{"hp":10,"max_hp":10,"energy":3,"gold":0,
                    "hand":[],"deck":[],"discard":[],"exhaust":[]},
                "state":{"state":"map","node_id":"start","options":["next"]},
                "legal_actions":[{"action_id":"move-1",
                    "action":{"kind":"select_map_node","node_id":"next"}}]
            }),
        )
        .map_err(|error| PortError::new("observation", error.to_string(), false))
    }

    fn legal_actions(
        &mut self,
        state_id: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        EpisodeLegalActionSet::new(
            state_id,
            generation,
            vec![
                EpisodeLegalAction::new("move-1", ActionKind::SelectMapNode)
                    .map_err(|error| PortError::new("action_set", error.to_string(), false))?,
            ],
        )
        .map_err(|error| PortError::new("action_set", error.to_string(), false))
    }

    fn map_snapshot(
        &mut self,
        _state_id: &str,
        _generation: u64,
        _execution_id: ModelExecutionId,
    ) -> Result<Option<Value>, PortError> {
        Ok(Some(self.response.clone()))
    }

    fn dispatch_action(
        &mut self,
        _identity: &ActionIdentity,
        _action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        Err(PortError::new(
            "unused",
            "decision must not dispatch",
            false,
        ))
    }
}

#[derive(Default)]
struct OwnerInputCapture {
    input: Option<DecisionInput>,
}

impl DecisionSource for OwnerInputCapture {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.input = Some(input.clone());
        Err(PolicyError::ProviderUnavailable)
    }
}

pub(super) fn mapped_input(response: Value) -> Result<DecisionInput, Box<dyn std::error::Error>> {
    let config = EpisodeRunnerConfig::new(
        1,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "map receipt test",
        Vec::new(),
    )?
    .with_map_context_enabled(true);
    let mut port = OwnerMapPort { response };
    let mut source = OwnerInputCapture::default();
    let outcome = EpisodeRunner::new(config).run(&mut port, &mut source);
    if !matches!(outcome, Err(EpisodeRunnerError::Policy(_))) {
        return Err("runner did not reach the owner-validated policy input".into());
    }
    source
        .input
        .ok_or_else(|| "runner omitted its owner-validated decision input".into())
}
