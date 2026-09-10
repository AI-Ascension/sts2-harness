// SPDX-License-Identifier: MIT

use super::*;
use crate::worker_handoff::{WorkerCapability, WorkerReply};
use crate::worker_runtime::control_tests::command;
use crate::worker_runtime::tests::{admitted_reservation, handoff_state, runtime};
use crate::{Checkpoint, CompletionRecord, CompletionStatus, ExecutionLineage};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn started(runtime: &mut WorkerRuntime) -> Result<StoredWorkerHandoff, Box<dyn std::error::Error>> {
    let reservation = admitted_reservation(runtime)?;
    match runtime.finish_reservation(Some(reservation), ResponseWriteStatus::Written)? {
        WorkerStartOutcome::Started(running) => Ok(*running),
        _ => Err("expected running reservation".into()),
    }
}

#[test]
fn claim_is_one_use_and_abandonment_retains_unknown() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let task = core.take_execution(running.clone())?;
    let cancellation = task.cancellation().clone();
    assert!(!cancellation.is_cancelled());
    assert!(core.take_execution(running).is_err());
    assert!(core.close().is_err());
    drop(task);
    assert!(cancellation.is_cancelled());
    assert_eq!(handoff_state(&core)?, WorkerHandoffState::Unknown);
    assert!(!core.store().admission_open());
    assert!(core.active_tuple().is_some());
    Ok(())
}

#[test]
fn fabricated_or_changed_running_rows_cannot_claim_a_lane() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let mut wrong = running.clone();
    wrong.mode_sequence += 1;
    assert!(core.take_execution(wrong).is_err());
    let task = core.take_execution(running)?;
    drop(task);
    Ok(())
}

#[test]
fn durable_control_changes_cancel_before_response_and_never_reset() -> TestResult {
    for mode in ["paused", "draining", "stopped", "running"] {
        let mut core = runtime()?;
        let running = started(&mut core)?;
        let task = core.take_execution(running)?;
        let signal = task.cancellation().clone();
        // An identical Running replay does not invalidate this execution.
        core.handle_authenticated(&command(WorkerCapability::SetControlMode, None)?)?;
        assert!(!signal.is_cancelled());
        let control = command(WorkerCapability::SetControlMode, Some(mode))?;
        let unauthorized = AuthenticatedWorkerRequest::from_transport(
            control.request().clone(),
            WorkerCapability::Probe,
            crate::WorkerOwnerProof::new("test-owner")?,
        );
        assert!(core.handle_authenticated(&unauthorized).is_err());
        assert!(!signal.is_cancelled());
        let (reply, _) = core.handle_authenticated(&control)?.into_parts();
        assert!(matches!(reply, WorkerReply::Control { accepted: true }));
        assert!(signal.is_cancelled());
        let mut fields = control.request().fields().clone();
        fields.insert("mode".into(), serde_json::json!("running"));
        fields.insert("mode_sequence".into(), serde_json::json!(3));
        let resume = AuthenticatedWorkerRequest::from_transport(
            crate::worker_handoff::WorkerRequest::decode(&serde_json::to_vec(&fields)?)?,
            WorkerCapability::SetControlMode,
            crate::WorkerOwnerProof::new("test-owner")?,
        );
        core.handle_authenticated(&resume)?;
        assert!(signal.is_cancelled());
        drop(task);
    }
    Ok(())
}

#[test]
fn reported_success_without_durable_completion_is_not_completion() -> TestResult {
    for shutdown in [false, true] {
        let mut core = runtime()?;
        let running = started(&mut core)?;
        let handoff_id = running.tuple.handoff_id.clone();
        let completion = core.take_execution(running)?.run(|_| Ok(()));
        if shutdown {
            core.retain_unknown(&handoff_id)?;
        }
        assert!(core.complete_execution(completion).is_err());
        assert_eq!(handoff_state(&core)?, WorkerHandoffState::Unknown);
        assert!(core.active_tuple().is_some());
    }
    Ok(())
}

#[test]
fn dropping_unconsumed_success_retains_unknown() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let completion = core.take_execution(running)?.run(|_| Ok(()));
    drop(completion);
    assert_eq!(handoff_state(&core)?, WorkerHandoffState::Unknown);
    assert!(!core.store().admission_open());
    Ok(())
}

#[test]
fn durable_completion_releases_only_after_its_owned_task_returns() -> TestResult {
    for shutdown in [false, true] {
        let mut core = runtime()?;
        let running = started(&mut core)?;
        let handoff_id = running.tuple.handoff_id.clone();
        let completion = core.take_execution(running)?.run(|task| {
            let tuple = &task.running().tuple;
            let lineage = ExecutionLineage::new(
                &tuple.run_id,
                &tuple.episode_id,
                &tuple.attempt_id,
                &tuple.trajectory_id,
            )
            .map_err(|error| error.to_string())?;
            let mut store = try_lock(task.store())?;
            store
                .save_checkpoint(
                    &Checkpoint::new(
                        lineage.clone(),
                        0,
                        "state-1",
                        1,
                        task.fingerprint().clone(),
                        b"{}".to_vec(),
                        "catalog-1",
                    )
                    .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            store
                .record_completion(
                    &CompletionRecord::new(
                        lineage,
                        CompletionStatus::Completed,
                        "terminal-1",
                        0,
                        "b".repeat(64),
                    )
                    .map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
            Ok(())
        });
        assert!(core.release_completed(&handoff_id).is_err());
        if shutdown {
            // Shutdown can win after the durable receipt but before the command
            // loop consumes the owned result. It must not manufacture a failure.
            core.retain_unknown(&handoff_id)?;
        }
        core.complete_execution(completion)?;
        assert!(core.active_tuple().is_none());
        assert_eq!(core.store().admission_open(), !shutdown);
        if shutdown {
            assert!(admitted_reservation(&mut core).is_err());
        }
        core.close()?;
    }
    Ok(())
}

#[test]
fn completion_from_another_processor_cannot_release_same_named_lane() -> TestResult {
    let mut first = runtime()?;
    let first_running = started(&mut first)?;
    let mut second = runtime()?;
    let second_running = started(&mut second)?;
    let second_task = second.take_execution(second_running)?;
    let first_signal = first.take_execution(first_running.clone())?;
    let cancellation = first_signal.cancellation().clone();
    cancellation.cancel();
    assert!(!second_task.cancellation().is_cancelled());
    let completion = first_signal.run(|_| Ok(()));
    assert!(second.complete_execution(completion).is_err());
    assert_eq!(handoff_state(&first)?, WorkerHandoffState::Unknown);
    assert_eq!(handoff_state(&second)?, WorkerHandoffState::Running);
    assert!(second.active_tuple().is_some());
    drop(second_task);
    Ok(())
}

#[test]
fn drop_with_busy_store_latches_gate_before_failed_unknown_write() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let task = core.take_execution(running)?;
    let shared = core.store().clone();
    let held = try_lock_recovery(&shared)?;
    drop(task);
    assert!(!core.store().admission_open());
    drop(held);
    assert_eq!(handoff_state(&core)?, WorkerHandoffState::Running);
    let (reply, _) = core
        .handle_authenticated(&command(WorkerCapability::Probe, None)?)?
        .into_parts();
    assert!(matches!(reply, WorkerReply::Probe(probe) if !probe.ready));
    Ok(())
}

#[test]
fn execution_error_retains_primary_and_busy_store_failure() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let task = core.take_execution(running)?;
    let shared = core.store().clone();
    let held = try_lock_recovery(&shared)?;
    let completion = task.run(|_| Err(String::from("synthetic provider timeout")));
    let error = completion
        .result
        .as_ref()
        .err()
        .ok_or("missing execution error")?;
    assert!(error.contains("synthetic provider timeout"));
    assert!(error.contains("failed to retain unknown worker handoff"));
    assert!(!core.store().admission_open());
    drop(held);
    assert!(core.complete_execution(completion).is_err());
    assert_eq!(handoff_state(&core)?, WorkerHandoffState::Unknown);
    core.close()?;
    Ok(())
}

#[test]
fn unwind_quarantines_without_detaching_execution() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let task = core.take_execution(running)?;
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        task.run(|_| std::panic::resume_unwind(Box::new("synthetic execution unwind")))
    }));
    assert!(outcome.is_err());
    assert_eq!(handoff_state(&core)?, WorkerHandoffState::Unknown);
    Ok(())
}

#[test]
fn control_and_probe_remain_available_while_owned_execution_waits() -> TestResult {
    let mut core = runtime()?;
    let running = started(&mut core)?;
    let task = core.take_execution(running)?;
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::scope(|scope| -> TestResult {
        let worker = scope.spawn(move || {
            task.run(|_| {
                entered_tx.send(()).map_err(|_| "entry receiver closed")?;
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(2))
                    .map_err(|_| "synthetic execution release deadline")?;
                Err(String::from("synthetic execution interrupted"))
            })
        });
        entered_rx.recv_timeout(std::time::Duration::from_secs(2))?;
        let (reply, _) = core
            .handle_authenticated(&command(WorkerCapability::SetControlMode, Some("stopped"))?)?
            .into_parts();
        assert!(matches!(reply, WorkerReply::Control { accepted: true }));
        let (reply, _) = core
            .handle_authenticated(&command(WorkerCapability::Probe, None)?)?
            .into_parts();
        assert!(matches!(reply, WorkerReply::Probe(probe) if !probe.ready));
        release_tx.send(())?;
        let completion = worker.join().map_err(|_| "synthetic worker panicked")?;
        assert!(core.complete_execution(completion).is_err());
        Ok(())
    })?;
    assert_eq!(handoff_state(&core)?, WorkerHandoffState::Unknown);
    Ok(())
}
