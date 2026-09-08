// SPDX-License-Identifier: MIT

//! Preserve the admitted worker control identity at every new-work boundary.

use sts2_harness::{WorkerControlMode, WorkerHandoffState};

use super::super::worker_store::{StoreLease, try_lock};
use super::DurableHandle;

impl DurableHandle {
    /// The returned lease holds the control checks and the following durable
    /// admission in one critical section. Historical results use recovery
    /// leases instead, so a stop cannot erase in-flight accounting.
    pub(in super::super) fn new_work_lease(&self) -> Result<StoreLease<'_>, String> {
        let store = try_lock(&self.store)?;
        let Some(expected) = &self.worker_handoff else {
            return Ok(store);
        };
        let current = store
            .worker_handoff(&expected.tuple.handoff_id)
            .map_err(|_| String::from("cannot inspect worker execution fence"))?
            .ok_or_else(|| String::from("worker execution handoff is missing"))?;
        let control = store
            .worker_control()
            .map_err(|_| String::from("cannot inspect worker control fence"))?
            .ok_or_else(|| String::from("worker execution control is missing"))?;
        if current != **expected
            || current.state != WorkerHandoffState::Running
            || control.deployment_id != expected.tuple.deployment_id
            || control.worker_owner_id != expected.tuple.worker_owner_id
            || control.worker_profile_digest != expected.tuple.worker_profile_digest
            || control.worker_boot_id != expected.worker_boot_id
            || control.watchdog_boot_id.as_deref() != Some(expected.watchdog_boot_id.as_str())
            || control.mode_sequence != expected.mode_sequence
            || control.mode != WorkerControlMode::Running
            || !control.authenticated
            || !control.admitting
        {
            return Err(String::from(
                "worker execution fence no longer admits new work",
            ));
        }
        Ok(store)
    }
}
