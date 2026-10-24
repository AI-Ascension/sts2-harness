// SPDX-License-Identifier: MIT

use super::*;
use crate::episode::legal_actions::{ActionKind, EpisodeLegalAction};
use crate::{
    BarrierError, BarrierPort, Decision, EpisodeRunnerConfig, PolicyError, PortError,
    RecoveryController, RecoveryPort, ShutdownError, ShutdownPort, StabilityBarrier,
    TransitionReceipt, WaitSample,
};
use std::cell::Cell;
use std::rc::Rc;

struct MapTestPort {
    calls: Vec<&'static str>,
    fail_map: bool,
    map_done: Rc<Cell<bool>>,
}

impl BarrierPort for MapTestPort {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}

impl RecoveryPort for MapTestPort {
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

impl ShutdownPort for MapTestPort {
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

impl EpisodeRuntimePort for MapTestPort {
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
            crate::episode::observation::EpisodeStage::Map,
            true,
            false,
            true,
            serde_json::json!({
                "state_id":"map-state-1", "generation":1, "visible_seed":null,
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
        self.map_done.set(true);
        if self.fail_map {
            return Err(PortError::new(
                "map_snapshot_failed",
                "fixture map reader failed closed",
                false,
            ));
        }
        Ok(Some(map_response()))
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

fn map_response() -> serde_json::Value {
    serde_json::json!({
        "protocol_version":"runtime-map-v1",
        "schema_digest":crate::episode::map::RUNTIME_MAP_SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/runtime-map-v1",
            "source":"schemas/runtime-map-v1.schema.json","generator":"hand-authored"},
        "correlation_id":"3","instance_id":"instance-1","session_id":"gateway-1",
        "lease_id":"lease-1","lease_epoch":1,"generation":1,
        "kind":"snapshot_response","timeout":null,
        "snapshot":{
            "state_id":"map-state-1","generation":1,
            "schema_version":"visible-map-v1","projection_version":"runtime-map-v1",
            "game_build":"build","mod_version":"mod","map_instance_id":"map-1",
            "act_id":1,"scope_id":"scope-1","availability":"available",
            "completeness":"complete","freshness":"current","reason":null,
            "nodes":[
                {"id":"start","row":0,"column":0,"category":"start","visited":true},
                {"id":"next","row":1,"column":0,"category":"monster","visited":false}
            ],
            "edges":[{"from":"start","to":"next"}],
            "position":{"kind":"current","node_id":"start"},"history":["start"],
            "terminal_node_ids":["next"],
            "bindings":[{"graph_node_id":"next","host_action_id":"move-1",
                "action":{"kind":"select_map_node","node_id":"next"}}]
        }
    })
}

struct RecordingSource {
    called: bool,
    map_done: Rc<Cell<bool>>,
    saw_map_before_decision: bool,
}

impl DecisionSource for RecordingSource {
    fn decide(&mut self, _input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.called = true;
        self.saw_map_before_decision = self.map_done.get();
        Err(PolicyError::ProviderUnavailable)
    }
}

fn runner_config() -> Result<EpisodeRunnerConfig, Box<dyn std::error::Error>> {
    Ok(EpisodeRunnerConfig::new(
        1,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "map-context-test",
        Vec::new(),
    )?
    .with_map_context_enabled(true))
}

#[test]
fn map_read_failure_stops_before_decision_and_cleanup_still_runs()
-> Result<(), Box<dyn std::error::Error>> {
    let map_done = Rc::new(Cell::new(false));
    let mut port = MapTestPort {
        calls: Vec::new(),
        fail_map: true,
        map_done: map_done.clone(),
    };
    let mut source = RecordingSource {
        called: false,
        map_done,
        saw_map_before_decision: false,
    };

    let result = EpisodeRunner::new(runner_config()?).run(&mut port, &mut source);

    assert!(matches!(
        result,
        Err(EpisodeRunnerError::LegalActions(error)) if error.code() == "map_snapshot_failed"
    ));
    assert!(!source.called);
    assert_eq!(
        port.calls,
        [
            "launch", "prepare", "observe", "legal", "refresh", "map", "release", "mcp", "gateway"
        ]
    );
    Ok(())
}

#[test]
fn exactly_one_validated_map_read_precedes_the_decision_source()
-> Result<(), Box<dyn std::error::Error>> {
    let map_done = Rc::new(Cell::new(false));
    let mut port = MapTestPort {
        calls: Vec::new(),
        fail_map: false,
        map_done: map_done.clone(),
    };
    let mut source = RecordingSource {
        called: false,
        map_done,
        saw_map_before_decision: false,
    };

    let result = EpisodeRunner::new(runner_config()?).run(&mut port, &mut source);

    assert!(matches!(result, Err(EpisodeRunnerError::Policy(_))));
    assert!(source.called);
    assert!(source.saw_map_before_decision);
    assert_eq!(port.calls.iter().filter(|call| **call == "map").count(), 1);
    Ok(())
}

#[test]
fn owner_digest_normalizes_map_order_and_binds_map_identity()
-> Result<(), Box<dyn std::error::Error>> {
    let response = map_response();
    let mut reordered = response.clone();
    reordered["snapshot"]["nodes"]
        .as_array_mut()
        .ok_or("nodes")?
        .reverse();
    reordered["snapshot"]["edges"]
        .as_array_mut()
        .ok_or("edges")?
        .reverse();
    reordered["snapshot"]["terminal_node_ids"]
        .as_array_mut()
        .ok_or("terminals")?
        .reverse();
    reordered["snapshot"]["bindings"]
        .as_array_mut()
        .ok_or("bindings")?
        .reverse();
    let actions = EpisodeLegalActionSet::new(
        "map-state-1",
        1,
        vec![EpisodeLegalAction::new(
            "move-1",
            ActionKind::SelectMapNode,
        )?],
    )?;
    let digest = |value: &serde_json::Value| {
        crate::DecisionInput::canonical_runtime_map_snapshot_digest(
            value,
            "map-state-1",
            1,
            &actions,
        )
    };
    assert_eq!(digest(&response), digest(&reordered));
    reordered["snapshot"]["map_instance_id"] = serde_json::json!("different-map");
    assert_ne!(digest(&response), digest(&reordered));
    Ok(())
}
