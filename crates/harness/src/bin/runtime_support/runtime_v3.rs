// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;

use serde_json::{Value, json};
use sts2_harness::{
    EpisodeLegalActionSet, EpisodeObservation, EpisodeRunner, ExoDecisionSource,
    ExoProcessTransport, ExoProvider, ExoSession, ShutdownError, ShutdownPort,
};

use super::config::RuntimeConfig;
use super::http::GatewayClient;
use super::mcp::{McpProcess, identity_headers};
use super::runtime_v3_parse as parse;
use super::runtime_v3_settings::RuntimeV3Settings;
use super::runtime_v3_telemetry::{
    CleanupStatus, GameOutcome, RuntimeV3Telemetry, TelemetryContext, TelemetryContextInput,
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
#[path = "runtime_v3_execution.rs"]
mod execution;
#[path = "runtime_v4_expert_port.rs"]
mod expert;
#[path = "runtime_v3_launch_options.rs"]
mod launch_options;
#[path = "runtime_v3_ledger.rs"]
mod ledger;
#[path = "runtime_v3_recording.rs"]
mod recording;
#[path = "runtime_v3_recovery.rs"]
mod recovery;
#[path = "runtime_v3_wait.rs"]
mod wait;
#[path = "runtime_v3_worker_runtime.rs"]
pub(super) mod worker_runtime;
#[path = "runtime_v3_worker_store.rs"]
mod worker_store;
#[path = "runtime_v3_workflow_binding.rs"]
mod workflow_binding;
use decision_admission::DecisionAdmission;
use ledger::OperationRecord;

#[path = "runtime_v3_combat_demo.rs"]
pub(crate) mod combat_demo;
#[path = "runtime_v3_episode_replay.rs"]
mod episode_replay;

#[cfg(test)]
#[path = "runtime_v3_lifecycle_test.rs"]
mod lifecycle_tests;

#[cfg(test)]
#[path = "runtime_v3_worker_store_tests.rs"]
mod worker_store_tests;

pub(super) fn run(config: RuntimeConfig) -> Result<(), String> {
    // Runtime-v4 expert remains the main-branch executable composition. Runtime-v3 gameplay is
    // admitted through the durable PR40 path below so completed and interrupted resumes are
    // decided before any gateway, MCP, or provider boundary is opened.
    if config.runtime_profile == "runtime-v4-expert" {
        return run_legacy(config);
    }
    let settings = RuntimeV3Settings::from_environment()?;
    let launch_options = launch_options::RuntimeV3LaunchOptions::from_environment()?;
    let workflow_binding = launch_options.workflow_binding()?;
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
    let (durable, state) = match durable::DurableHandle::open_with_binding(
        &config,
        &settings,
        workflow_binding,
        launch_options.resume,
    ) {
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

include!("runtime_v4_execution.rs");

pub(super) struct RuntimeV3Port {
    config: RuntimeConfig,
    gateway: GatewayClient,
    mcp: Option<McpProcess>,
    expert_mcp: Option<McpProcess>,
    allocated: bool,
    released: bool,
    next_rpc_id: u64,
    expert_next_rpc_id: u64,
    generation: u64,
    current_state: Option<String>,
    current_actions: Option<EpisodeLegalActionSet>,
    catalog: Option<Value>,
    catalog_raw: Option<Vec<u8>>,
    payloads: BTreeMap<String, Value>,
    operations: BTreeMap<String, OperationRecord>,
    reconnect_attempts: u8,
    telemetry: TelemetryHandle,
    durable: Option<durable::DurableHandle>,
    recovery_authority: Option<allocation_context::RecoveryAuthority>,
    recovery: Option<McpProcess>,
    recovery_context: Option<recovery::RecoveryContext>,
    recovery_rpc_id: u64,
    cancellation: sts2_harness::ExecutionCancellation,
}

include!("runtime_v3_port.rs");
