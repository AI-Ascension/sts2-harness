// SPDX-License-Identifier: MIT

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings;
use super::super::super::runtime_v3_telemetry::{
    RuntimeV3Telemetry, TelemetryContext, TelemetryContextLineage,
};
use super::super::durable::DurableHandle;
use super::super::launch_options::RuntimeV3LaunchOptions;
use super::*;
use sts2_harness::{
    CompletionRecord, CompletionStatus, EpisodeRunnerConfig, ExecutionFingerprint,
    ExecutionLineage, ExecutionStore, ExoConfig, ExoProcessConfig, RecoveryController, ResumeState,
    StabilityBarrier,
};

fn config() -> RuntimeConfig {
    RuntimeConfig {
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: String::from("mcp"),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        mcp_session_id: String::from("mcp-session-1"),
        run_id: String::from("run-1"),
        episode_id: String::from("episode-1"),
        trajectory_id: String::from("trajectory-1"),
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        recovery_environment: Vec::new(),
    }
}

fn settings() -> Result<RuntimeV3Settings, String> {
    let runner = EpisodeRunnerConfig::new(
        1,
        StabilityBarrier::new(1, 1).map_err(|error| error.to_string())?,
        RecoveryController::new(1).map_err(|error| error.to_string())?,
        "synthetic objective",
        Vec::new(),
    )
    .map_err(|error| error.to_string())?;
    let revision = format!("1{}", "0".repeat(63));
    let exo = ExoConfig::new(revision, 1, 1, 1).map_err(|error| error.to_string())?;
    let process = ExoProcessConfig::new("bridge", Vec::new(), None, Vec::new())
        .map_err(|error| error.to_string())?;
    Ok(RuntimeV3Settings {
        runner,
        exo,
        process,
    })
}

#[test]
fn completed_resume_uses_the_admitted_store_without_opening_a_second_store() -> Result<(), String> {
    let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-1", "trajectory-1")
        .map_err(|error| error.to_string())?;
    let fingerprint = ExecutionFingerprint::new("seed", "build", "state", "config", "provider")
        .map_err(|error| error.to_string())?;
    let mut store = ExecutionStore::open_in_memory().map_err(|error| error.to_string())?;
    store
        .start_episode(&lineage, &fingerprint)
        .map_err(|error| error.to_string())?;
    let durable = DurableHandle::from_store_for_test(store, lineage.clone(), fingerprint)?;
    let completion = CompletionRecord::new(
        lineage,
        CompletionStatus::Completed,
        "terminal-victory-1",
        0,
        "result-digest",
    )
    .map_err(|error| error.to_string())?;
    let config = config();
    let revision = format!("1{}", "0".repeat(63));
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
        &revision,
    )?;
    let telemetry = RuntimeV3Telemetry::new(telemetry_context);
    let telemetry_handle = telemetry.handle();
    let result = run(
        config,
        settings()?,
        durable,
        ResumeState::Completed(completion),
        RuntimeV3LaunchOptions {
            resume: true,
            combat_demo: false,
            replay_path: Some("never-read.jsonl".into()),
            replay_prefix: Err(String::from("STS2_REPLAY_PREFIX must be true or false")),
        },
        telemetry_handle,
        telemetry,
    );
    assert!(result.is_ok());
    Ok(())
}
