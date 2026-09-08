// SPDX-License-Identifier: MIT

use super::*;
use sts2_harness::ExecutionLineage;
use sts2_harness::StoredWorkerHandoff;

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings as RuntimeSettings;
use super::super::super::runtime_v3_telemetry::{
    RuntimeV3Telemetry, TelemetryContext, TelemetryContextLineage,
};
use super::super::launch_options::RuntimeV3LaunchOptions;
use super::super::worker_store::try_lock_close;

impl WorkerRuntime {
    /// Runs the already-started handoff through the existing admitted runtime path.  This method
    /// is intentionally reachable to a future authenticated listener, but production `run`
    /// below does not call it until a real protected native transport is present.
    pub fn execute_started(
        &mut self,
        running: StoredWorkerHandoff,
        config: RuntimeConfig,
        settings: RuntimeSettings,
    ) -> Result<(), String> {
        let handoff_id = running.tuple.handoff_id.clone();
        let result = self.execute_started_inner(running, config, settings);
        if let Err(error) = result {
            // The execution side may already have projected uncertainty. This idempotent fence
            // also covers launch/configuration failures before it could enter that path. Preserve
            // both errors: a failed recovery write is itself evidence that the lane must remain
            // closed and cannot be silently downgraded to the original runtime failure.
            return Err(super::combine_failure(
                error,
                self.retain_unknown(&handoff_id),
            ));
        }
        Ok(())
    }

    fn execute_started_inner(
        &mut self,
        running: StoredWorkerHandoff,
        config: RuntimeConfig,
        settings: RuntimeSettings,
    ) -> Result<(), String> {
        let Some(active) = self.lane.tuple() else {
            return Err(String::from("worker execution lane has no active handoff"));
        };
        if running.tuple != *active || running.worker_boot_id != self.worker_boot_id {
            return Err(String::from(
                "started worker handoff does not match the active worker lane",
            ));
        }
        let lineage = ExecutionLineage::new(
            running.tuple.run_id.clone(),
            running.tuple.episode_id.clone(),
            running.tuple.attempt_id.clone(),
            running.tuple.trajectory_id.clone(),
        )
        .map_err(|error| format!("worker execution lineage is invalid: {error}"))?;
        // Run, episode and trajectory identity are frozen in the authenticated dispatch. Other
        // runtime configuration remains harness-owned process state.
        let mut config = config;
        config.run_id = lineage.run_id.clone();
        config.episode_id = lineage.episode_id.clone();
        config.trajectory_id = lineage.trajectory_id.clone();
        let launch_options = RuntimeV3LaunchOptions::from_environment()?;
        let telemetry_context = TelemetryContext::new(
            TelemetryContextLineage {
                run_id: &config.run_id,
                episode_id: &config.episode_id,
                trajectory_id: &config.trajectory_id,
                trace_id: &config.trace_id,
            },
            &config.instance_id,
            &config.session_id,
            &config.runtime_profile,
            &settings.exo.revision,
        )?;
        let telemetry = RuntimeV3Telemetry::new(telemetry_context);
        let telemetry_handle = telemetry.handle();
        let _ = telemetry_handle.run_started();
        let (durable, state) = super::super::durable::DurableHandle::from_admitted_shared_store(
            self.store.clone(),
            &running,
            &config,
            &settings,
            lineage,
            self.fingerprint.clone(),
        )?;
        let result = super::super::execution::run(
            config,
            settings,
            durable,
            state,
            launch_options,
            telemetry_handle,
            telemetry,
        );
        if result.is_ok() {
            self.release_completed(&running.tuple.handoff_id)?;
        }
        result
    }

    /// Releases the capacity-one lane only after the same durable handoff has a projected
    /// terminal receipt. Unknown, admitted, and running rows remain lookup-only.
    pub fn release_completed(&mut self, handoff_id: &str) -> Result<(), String> {
        let tuple = self
            .lane
            .tuple()
            .filter(|tuple| tuple.handoff_id == handoff_id)
            .cloned()
            .ok_or_else(|| String::from("worker completion does not match active lane"))?;
        let mut store = try_lock(&self.store)?;
        let handoff = match store
            .lookup_worker_handoff(&tuple)
            .map_err(|_| String::from("cannot reconcile worker completion"))?
        {
            sts2_harness::WorkerLookup::Known(handoff) => *handoff,
            sts2_harness::WorkerLookup::Unknown { .. } => {
                return Err(String::from("worker completion handoff is not retained"));
            }
        };
        if handoff.terminal.is_none() {
            return Err(String::from(
                "worker execution cannot release its lane before durable completion",
            ));
        }
        drop(store);
        self.lane.release(handoff_id)
    }

    pub fn close(&self) -> Result<(), String> {
        let mut store = try_lock_close(&self.store)?;
        store
            .close()
            .map_err(|_| String::from("cannot close worker execution store"))
    }

    pub fn worker_boot_id(&self) -> &str {
        &self.worker_boot_id
    }
}
