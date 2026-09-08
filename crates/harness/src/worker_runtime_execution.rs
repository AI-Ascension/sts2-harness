// SPDX-License-Identifier: MIT

//! One-use owned execution handoff, independent of the command-loop borrow.

use std::sync::Arc;

use super::*;

/// A non-cloneable execution claim. It may move to an owned execution thread;
/// neither gameplay nor a store lease is held by the command processor.
/// Dropping it without accepted durable completion quarantines its handoff.
pub struct WorkerExecutionTask {
    owner: Arc<()>,
    store: SharedExecutionStore,
    running: StoredWorkerHandoff,
    fingerprint: ExecutionFingerprint,
    cancellation: crate::ExecutionCancellation,
    armed: bool,
}

/// Correlated completion returned to the original command processor. Dropping
/// an unconsumed success also retains uncertainty, never silently frees a lane.
pub struct WorkerExecutionCompletion {
    task: WorkerExecutionTask,
    result: Result<(), String>,
}

impl WorkerRuntime {
    /// Claims the current running row exactly once. The execution adapter must
    /// still apply its execution-time control fence before new work.
    pub fn take_execution(
        &mut self,
        running: StoredWorkerHandoff,
    ) -> Result<WorkerExecutionTask, String> {
        if self.lane.execution_taken
            || self.lane.tuple() != Some(&running.tuple)
            || running.worker_boot_id != self.worker_boot_id
            || running.state != WorkerHandoffState::Running
        {
            return Err(String::from(
                "worker execution does not own the active running lane",
            ));
        }
        let store = try_lock(&self.store)?;
        let current = store
            .worker_handoff(&running.tuple.handoff_id)
            .map_err(|_| String::from("cannot inspect worker execution claim"))?;
        if current.as_ref() != Some(&running) {
            return Err(String::from(
                "worker execution handoff changed before claim",
            ));
        }
        drop(store);
        self.lane.execution_taken = true;
        let cancellation = crate::ExecutionCancellation::default();
        self.lane.cancellation = Some(cancellation.clone());
        Ok(WorkerExecutionTask {
            owner: Arc::clone(&self.execution_owner),
            store: self.store.clone(),
            running,
            fingerprint: self.fingerprint.clone(),
            cancellation,
            armed: true,
        })
    }

    /// Read the committed control under the command's existing store lease.
    /// A lost response cannot undo cancellation, and rejected commands never
    /// reach this check. Missing or unreadable authority cancels fail-closed.
    pub(super) fn cancel_changed_execution(
        &self,
        store: &crate::ExecutionStore,
    ) -> Result<(), String> {
        let Some(signal) = &self.lane.cancellation else {
            return Ok(());
        };
        let valid = (|| {
            let Some(tuple) = self.lane.tuple() else {
                return Ok(false);
            };
            let Some(running) = store.worker_handoff(&tuple.handoff_id)? else {
                return Ok(false);
            };
            let Some(control) = store.worker_control()? else {
                return Ok(false);
            };
            Ok::<_, crate::ExecutionStoreError>(
                running.tuple == *tuple
                    && control.deployment_id == tuple.deployment_id
                    && control.worker_owner_id == tuple.worker_owner_id
                    && control.worker_profile_digest == tuple.worker_profile_digest
                    && control.worker_boot_id == running.worker_boot_id
                    && control.watchdog_boot_id.as_deref()
                        == Some(running.watchdog_boot_id.as_str())
                    && control.mode_sequence == running.mode_sequence
                    && control.mode == crate::WorkerControlMode::Running
                    && control.authenticated
                    && control.admitting,
            )
        })();
        if !matches!(valid, Ok(true)) {
            signal.cancel();
        }
        valid
            .map(|_| ())
            .map_err(|_| String::from("cannot inspect active execution control"))
    }

    /// Accepts only this processor's one-use completion, then checks the
    /// durable terminal receipt before releasing capacity. A callback's Ok
    /// result is not evidence of durable episode completion.
    pub fn complete_execution(
        &mut self,
        mut completion: WorkerExecutionCompletion,
    ) -> Result<(), String> {
        if !Arc::ptr_eq(&self.execution_owner, &completion.task.owner)
            || !self.lane.execution_taken
            || self.lane.tuple() != Some(&completion.task.running.tuple)
        {
            return Err(String::from(
                "worker execution completion belongs to another lane",
            ));
        }
        self.lane.execution_taken = false;
        let handoff_id = &completion.task.running.tuple.handoff_id;
        let result = completion
            .result
            .clone()
            .and_then(|()| self.release_durable_completion(handoff_id));
        match result {
            Ok(()) => {
                completion.task.armed = false;
                Ok(())
            }
            Err(error) => {
                let quarantine = self.retain_unknown(handoff_id);
                completion.task.armed = false;
                Err(combine_failure(error, quarantine))
            }
        }
    }
}

impl WorkerExecutionTask {
    /// This claim owns a fresh monotonic signal, never reused by a later job.
    pub fn cancellation(&self) -> &crate::ExecutionCancellation {
        &self.cancellation
    }

    pub fn running(&self) -> &StoredWorkerHandoff {
        &self.running
    }

    pub fn store(&self) -> &SharedExecutionStore {
        &self.store
    }

    pub fn fingerprint(&self) -> &ExecutionFingerprint {
        &self.fingerprint
    }

    /// Runs the admitted adapter without borrowing the command processor.
    /// An unwind drops the armed task; ordinary errors retain both execution
    /// and persistence failures. No thread is spawned or detached here.
    pub fn run(
        mut self,
        execute: impl FnOnce(&Self) -> Result<(), String>,
    ) -> WorkerExecutionCompletion {
        let result = execute(&self).map_err(|error| {
            let quarantine =
                completion::retain_unknown(&self.store, &self.running.tuple.handoff_id);
            self.armed = false;
            combine_failure(error, quarantine)
        });
        WorkerExecutionCompletion { task: self, result }
    }
}

impl Drop for WorkerExecutionTask {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if self.armed {
            let _ = completion::retain_unknown(&self.store, &self.running.tuple.handoff_id);
        }
    }
}

#[cfg(test)]
#[path = "worker_runtime_execution_tests.rs"]
mod tests;
