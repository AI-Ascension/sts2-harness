// SPDX-License-Identifier: MIT

use super::*;
use crate::{Decision, PortError};

struct MapFailurePort {
    calls: Vec<&'static str>,
}

impl BarrierPort for MapFailurePort {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}

impl RecoveryPort for MapFailurePort {
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

impl ShutdownPort for MapFailurePort {
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

impl EpisodeRuntimePort for MapFailurePort {
    fn launch(&mut self) -> Result<(), PortError> {
        self.calls.push("launch");
        Ok(())
    }

    fn prepare_game_information_binding(&mut self) -> Result<(), PortError> {
        self.calls.push("prepare");
        Ok(())
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        self.calls.push("observe");
        EpisodeObservation::new(
            "map-state-1",
            1,
            EpisodeStage::Map,
            true,
            false,
            true,
            serde_json::json!({
                "state_id":"map-state-1",
                "generation":1,
                "visible_seed":null,
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
        self.calls.push("legal");
        let action = EpisodeLegalAction::new("move-1", ActionKind::SelectMapNode)
            .map_err(|error| PortError::new("action_set", error.to_string(), false))?;
        EpisodeLegalActionSet::new(state_id, generation, vec![action])
            .map_err(|error| PortError::new("action_set", error.to_string(), false))
    }

    fn refresh_game_information_binding(
        &mut self,
        _state_id: &str,
        _generation: u64,
    ) -> Result<(), PortError> {
        self.calls.push("refresh");
        Ok(())
    }

    fn map_snapshot(
        &mut self,
        _state_id: &str,
        _generation: u64,
        _execution_id: crate::ModelExecutionId,
    ) -> Result<Option<serde_json::Value>, PortError> {
        self.calls.push("map");
        Err(PortError::new(
            "map_snapshot_failed",
            "fixture map reader failed closed",
            false,
        ))
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

struct RecordingSource {
    called: bool,
}

impl DecisionSource for RecordingSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.called = true;
        Err(PolicyError::ProviderUnavailable)
    }
}

#[test]
fn map_read_failure_stops_before_decision_and_keeps_runner_order()
-> Result<(), Box<dyn std::error::Error>> {
    let config = EpisodeRunnerConfig::new(
        1,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "map-context-test",
        Vec::new(),
    )?
    .with_map_context_enabled(true);
    let mut port = MapFailurePort { calls: Vec::new() };
    let mut source = RecordingSource { called: false };

    let result = EpisodeRunner::new(config).run(&mut port, &mut source);

    assert!(matches!(
        result,
        Err(EpisodeRunnerError::LegalActions(error))
            if error.code() == "map_snapshot_failed"
    ));
    assert!(
        !source.called,
        "the decision source must not run without the map read"
    );
    assert_eq!(
        port.calls,
        [
            "launch", "prepare", "observe", "legal", "refresh", "map", "release", "mcp", "gateway"
        ]
    );
    Ok(())
}
