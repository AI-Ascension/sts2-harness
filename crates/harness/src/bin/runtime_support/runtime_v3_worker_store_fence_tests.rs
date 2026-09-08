// SPDX-License-Identifier: MIT

use serde_json::json;
use sts2_harness::{
    ActionKind, DecisionInput, EpisodeLegalAction, EpisodeLegalActionSet, ModelExecutionId,
    OperationState, ProviderFailureClass, StoredWorkerHandoff, WorkerControlMode,
    WorkerControlRequest, WorkerOwnerProof,
};

use super::super::worker_store::{SharedExecutionStore, try_lock_recovery};
use super::{DurableHandle, checkpoint_observation, lineage, prepared_runtime};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct OwnedFixture(std::path::PathBuf);
impl Drop for OwnedFixture {
    fn drop(&mut self) {
        super::remove_executable(&self.0);
    }
}

fn change_control(
    shared: &SharedExecutionStore,
    handoff: &StoredWorkerHandoff,
    mode: WorkerControlMode,
) -> TestResult {
    let control = WorkerControlRequest::new(
        &handoff.tuple.deployment_id,
        &handoff.tuple.worker_owner_id,
        &handoff.tuple.worker_profile_digest,
        &handoff.watchdog_boot_id,
        &handoff.worker_boot_id,
        mode,
        2,
    )?;
    try_lock_recovery(shared)?
        .set_worker_control_mode(&control, &WorkerOwnerProof::new("test-owner")?)?;
    Ok(())
}

fn decision_input() -> Result<DecisionInput, Box<dyn std::error::Error>> {
    Ok(DecisionInput::new(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        checkpoint_observation("state-1", 1),
        EpisodeLegalActionSet::new(
            "state-1",
            1,
            vec![EpisodeLegalAction::new(
                "combat.end-turn",
                ActionKind::EndTurn,
            )?],
        )?,
        "synthetic worker control fence",
        Vec::new(),
    ))
}

#[test]
fn attached_worker_rechecks_control_before_decisions_and_dispatch() -> TestResult {
    for mode in [
        WorkerControlMode::Paused,
        WorkerControlMode::Draining,
        WorkerControlMode::Stopped,
        WorkerControlMode::Running,
    ] {
        let (path, config, settings, approved, shared, handoff) = prepared_runtime();
        let _fixture = OwnedFixture(path);
        let (handle, _) = DurableHandle::from_admitted_shared_store(
            shared.clone(),
            &handoff,
            &config,
            &settings,
            lineage(),
            approved,
        )?;
        let input = decision_input()?;
        let token = match handle.decision_admission_with_reuse(&input)? {
            super::super::DecisionAdmission::Fresh(token) => token,
            super::super::DecisionAdmission::Reused(_) => {
                return Err("unexpected reused decision".into());
            }
        };
        let operation_id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        let action = EpisodeLegalAction::new("combat.end-turn", ActionKind::EndTurn)?;
        let payload = json!({"kind": "end_turn"});
        let catalog = json!({"state_id":"state-1", "generation":1, "legal_actions":[{"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}]});
        let digest =
            handle.operation_intent(operation_id, "state-1", 1, &action, &payload, &catalog)?;
        handle.operation_dispatched(operation_id, &digest)?;
        let cloned = handle.clone();
        change_control(&shared, &handoff, mode)?;
        assert!(
            matches!(cloned.decision_admission_with_reuse(&input), Err(error) if error.contains("worker execution fence"))
        );
        assert!(
            matches!(handle.operation_dispatched(operation_id, &digest), Err(error) if error.contains("worker execution fence"))
        );
        assert!(
            matches!(handle.operation_intent("bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb", "state-1", 1, &action, &payload, &catalog), Err(error) if error.contains("worker execution fence"))
        );
        assert_eq!(
            handle.operation_state(operation_id)?,
            OperationState::MayHaveBeenDispatched
        );
        // Stopping new work must not prevent conservative accounting for work
        // that crossed its durable uncertainty boundary before the control change.
        handle.unknown_decision(&token, ProviderFailureClass::Cancelled)?;
        handle.operation_result(operation_id, &digest, OperationState::Unknown, None)?;
        assert_eq!(
            handle.operation_state(operation_id)?,
            OperationState::Unknown
        );
    }
    Ok(())
}

#[test]
fn changed_control_is_rejected_at_runtime_attachment() -> TestResult {
    let (path, config, settings, approved, shared, handoff) = prepared_runtime();
    let _fixture = OwnedFixture(path);
    change_control(&shared, &handoff, WorkerControlMode::Stopped)?;
    assert!(
        matches!(DurableHandle::from_admitted_shared_store(shared, &handoff, &config, &settings, lineage(), approved), Err(error) if error.contains("worker execution fence"))
    );
    Ok(())
}
