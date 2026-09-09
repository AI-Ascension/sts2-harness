// SPDX-License-Identifier: MIT

use super::WorkerRuntime;
use sts2_harness::worker_runtime::{WorkerExecutionCompletion, WorkerExecutionTask};
use sts2_harness::{ExecutionLineage, StoredWorkerHandoff};

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings as RuntimeSettings;
use super::super::super::runtime_v3_telemetry::{
    RuntimeV3Telemetry, TelemetryContext, TelemetryContextInput,
};
use super::super::launch_options::RuntimeV3LaunchOptions;

/// Owned immutable execution inputs. A server can move this value to its one
/// execution thread without retaining a mutable borrow of the command core.
/// Runtime-local Rc handles are constructed only inside run, on that thread.
pub(super) struct PreparedWorkerExecution {
    task: WorkerExecutionTask,
    config: RuntimeConfig,
    settings: RuntimeSettings,
    launch_options: RuntimeV3LaunchOptions,
}

impl WorkerRuntime {
    pub(super) fn prepare_started(
        &mut self,
        running: StoredWorkerHandoff,
        config: RuntimeConfig,
        settings: RuntimeSettings,
        mut launch_options: RuntimeV3LaunchOptions,
    ) -> Result<PreparedWorkerExecution, String> {
        let task = self.core.take_execution(running)?;
        launch_options.cancellation = task.cancellation().clone();
        Ok(PreparedWorkerExecution {
            task,
            config,
            settings,
            launch_options,
        })
    }
}

impl PreparedWorkerExecution {
    pub(super) fn cancellation(&self) -> &sts2_harness::ExecutionCancellation {
        self.task.cancellation()
    }

    pub(super) fn run(self) -> WorkerExecutionCompletion {
        let Self {
            task,
            config,
            settings,
            launch_options,
        } = self;
        task.run(|task| execute_task(task, config, settings, launch_options))
    }
}

fn execute_task(
    task: &WorkerExecutionTask,
    config: RuntimeConfig,
    settings: RuntimeSettings,
    launch_options: RuntimeV3LaunchOptions,
) -> Result<(), String> {
    let running = task.running();
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
    let telemetry_context = TelemetryContext::new(TelemetryContextInput {
        run_id: &config.run_id,
        episode_id: &config.episode_id,
        trajectory_id: &config.trajectory_id,
        trace_id: &config.trace_id,
        instance_id: &config.instance_id,
        session_id: &config.session_id,
        runtime_profile: &config.runtime_profile,
        provider_revision: &settings.exo.revision,
    })?;
    let telemetry = RuntimeV3Telemetry::new(telemetry_context);
    let telemetry_handle = telemetry.handle();
    let _ = telemetry_handle.run_started();
    let (durable, state) = super::super::durable::DurableHandle::from_admitted_shared_store(
        task.store().clone(),
        running,
        &config,
        &settings,
        lineage,
        task.fingerprint().clone(),
    )?;
    super::super::execution::run(
        config,
        settings,
        durable,
        state,
        launch_options,
        telemetry_handle,
        telemetry,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_execution_and_completion_can_move_to_owned_threads() {
        fn requires_send<T: Send>() {}
        requires_send::<PreparedWorkerExecution>();
        requires_send::<WorkerExecutionCompletion>();
    }
}
