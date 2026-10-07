// SPDX-License-Identifier: MIT

use super::forward_map_snapshot;
use serde_json::Value;
use sts2_harness::{
    ActionIdentity, BarrierError, BarrierPort, EpisodeLegalAction, EpisodeLegalActionSet,
    EpisodeObservation, EpisodeRuntimePort, ModelExecutionId, PortError, RecoveryError,
    RecoveryPort, RuntimeLeaseBinding, ShutdownError, ShutdownPort, TransitionReceipt, WaitSample,
};

struct MapPort {
    calls: usize,
    state_id: String,
    generation: u64,
    execution_id: Option<ModelExecutionId>,
    result: Result<Option<Value>, PortError>,
}

impl BarrierPort for MapPort {
    fn wait_for_transition(
        &mut self,
        _operation_id: &str,
        _wait_for_millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        Err(BarrierError::PortFailure)
    }
}

impl RecoveryPort for MapPort {
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

impl ShutdownPort for MapPort {
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

impl EpisodeRuntimePort for MapPort {
    fn launch(&mut self) -> Result<(), PortError> {
        Err(PortError::new("unused", "not called", false))
    }

    fn current_lease_binding(&mut self) -> Result<RuntimeLeaseBinding, PortError> {
        Err(PortError::new("unused", "not called", false))
    }

    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        Err(PortError::new("unused", "not called", false))
    }

    fn legal_actions(
        &mut self,
        _state_id: &str,
        _generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        Err(PortError::new("unused", "not called", false))
    }

    fn map_snapshot(
        &mut self,
        state_id: &str,
        generation: u64,
        execution_id: ModelExecutionId,
    ) -> Result<Option<Value>, PortError> {
        self.calls += 1;
        self.state_id = state_id.to_owned();
        self.generation = generation;
        self.execution_id = Some(execution_id);
        self.result.clone()
    }

    fn dispatch_action(
        &mut self,
        _identity: &ActionIdentity,
        _action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        Err(PortError::new("unused", "not called", false))
    }
}

#[test]
fn worker_forwarding_calls_the_map_port_once_with_the_original_fence() -> Result<(), String> {
    let execution_id = ModelExecutionId::new(7)
        .ok_or_else(|| String::from("fixture execution identity must be nonzero"))?;
    let mut port = MapPort {
        calls: 0,
        state_id: String::new(),
        generation: 0,
        execution_id: None,
        result: Ok(Some(serde_json::json!({"generation": 12}))),
    };

    let result = forward_map_snapshot(&mut port, "map-state-12", 12, execution_id)
        .map_err(|error| error.to_string())?;

    assert_eq!(result, Some(serde_json::json!({"generation": 12})));
    assert_eq!(port.calls, 1);
    assert_eq!(port.state_id, "map-state-12");
    assert_eq!(port.generation, 12);
    assert_eq!(port.execution_id, Some(execution_id));
    Ok(())
}

#[test]
fn worker_forwarding_preserves_the_port_failure() {
    let mut port = MapPort {
        calls: 0,
        state_id: String::new(),
        generation: 0,
        execution_id: None,
        result: Err(PortError::new("map_snapshot_failed", "read failed", false)),
    };
    let execution_id = ModelExecutionId::new(8);
    let result = execution_id.map_or_else(
        || Err(PortError::new("fixture", "invalid identity", false)),
        |execution_id| forward_map_snapshot(&mut port, "map-state-13", 13, execution_id),
    );

    assert!(matches!(result, Err(error) if error.code() == "map_snapshot_failed"));
    assert_eq!(port.calls, 1);
}
