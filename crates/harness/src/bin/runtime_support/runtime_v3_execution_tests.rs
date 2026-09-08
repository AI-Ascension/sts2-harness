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

#[test]
fn failed_store_close_preserves_original_and_quarantine_errors() {
    let error = combine_quarantine("episode failed".into(), Err("store unavailable".into()));
    let combined = combine_store_close(error, Err("close unavailable".into()));
    assert_eq!(
        combined,
        "episode failed; failed to persist interrupted-unknown quarantine: store unavailable; execution store close failed: close unavailable"
    );
}

#[test]
fn successful_store_close_keeps_the_original_failure() {
    assert_eq!(
        combine_store_close("episode failed".into(), Ok(())),
        "episode failed"
    );
}

#[test]
fn cleanup_results_preserve_every_failure_without_manufacturing_success() {
    assert_eq!(finish_cleanup(Ok(()), Ok(()), "close"), Ok(()));
    assert_eq!(
        finish_cleanup(Err("run".into()), Ok(()), "close"),
        Err("run".into())
    );
    assert_eq!(
        finish_cleanup(Ok(()), Err("io".into()), "close"),
        Err("close: io".into())
    );
    let result = finish_cleanup(
        Err("run".into()),
        Err("provider".into()),
        "provider close failed",
    );
    assert_eq!(
        finish_cleanup(result, Err("store".into()), "execution store close failed"),
        Err("run; provider close failed: provider; execution store close failed: store".into())
    );
}

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
            cancellation: sts2_harness::ExecutionCancellation::default(),
        },
        telemetry_handle,
        telemetry,
    );
    assert!(result.is_ok());
    Ok(())
}
