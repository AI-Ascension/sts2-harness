// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{EpisodeLegalActionSet, EpisodeObservation, ShutdownError, ShutdownPort};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers, release_correlation};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, RuntimeV3Telemetry, TelemetryContext, TelemetryContextLineage,
    TelemetryHandle, TelemetryStage,
};
use super::runtime_v3_wire as wire;

#[path = "runtime_allocation_context.rs"]
mod allocation_context;
#[path = "runtime_v3_completed_resume.rs"]
mod completed_resume;
#[path = "runtime_v3_decision_admission.rs"]
mod decision_admission;
#[path = "runtime_v3_decision_replay.rs"]
mod decision_replay;
#[path = "runtime_v3_durable.rs"]
mod durable;
#[path = "runtime_v3_episode.rs"]
mod episode;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_recording.rs"]
mod recording;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
#[path = "runtime_v3_wait.rs"]
mod wait;
#[path = "runtime_v3_worker_store.rs"]
mod worker_store;
use ledger::OperationRecord;

#[path = "runtime_v3_combat_demo.rs"]
pub(crate) mod combat_demo;
#[path = "runtime_v3_episode_replay.rs"]
mod episode_replay;
#[path = "runtime_v3_execution.rs"]
mod execution;
#[path = "runtime_v3_launch_options.rs"]
mod launch_options;
#[path = "runtime_v3_worker_runtime.rs"]
pub(super) mod worker_runtime;

use decision_admission::DecisionAdmission;

#[cfg(test)]
#[path = "runtime_v3_lifecycle_test.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "runtime_v3_worker_store_tests.rs"]
mod worker_store_tests;

pub(super) fn run(config: RuntimeConfig) -> Result<(), String> {
    let settings = RuntimeV3Settings::from_environment()?;
    let launch_options = launch_options::RuntimeV3LaunchOptions::from_environment()?;
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
    let (durable, state) =
        match durable::DurableHandle::open(&config, &settings, launch_options.resume) {
            Ok(result) => result,
            Err(error) => {
                let _ = telemetry_handle.failure(
                    "runtime_init",
                    super::runtime_v3_telemetry::FailureCode::Configuration,
                    false,
                    None,
                );
                let _ = telemetry_handle.run_finished(
                    GameOutcome::Unavailable,
                    TelemetryStage::Unknown,
                    CleanupStatus::Failed,
                );
                finish_telemetry(telemetry);
                return Err(error);
            }
        };
    execution::run(
        config,
        settings,
        durable,
        state,
        launch_options,
        telemetry_handle,
        telemetry,
    )
}

include!("runtime_v3_port.rs");
