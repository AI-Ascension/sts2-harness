// SPDX-License-Identifier: MIT

use std::fs;
use std::path::PathBuf;

use super::super::super::super::config::RuntimeConfig;
use super::super::super::super::runtime_v3_settings::RuntimeV3Settings;
use super::super::super::workflow_binding::WorkflowBinding;
use sts2_harness::{
    EpisodeRunnerConfig, ExoConfig, ExoProcessConfig, RecoveryController, StabilityBarrier,
};

use super::{fingerprint_component, mcp_executable};

fn fixture_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "sts2-harness-completed-resume-{name}-{}",
        std::process::id()
    ))
}

#[test]
fn resume_requires_every_fingerprint_component() -> Result<(), String> {
    let prefix = format!("STS2_COMPLETED_RESUME_TEST_{}", std::process::id());
    let seed = format!("{prefix}_SEED");
    let visible_seed = format!("{prefix}_VISIBLE_SEED");
    let build = format!("{prefix}_BUILD");
    let state = format!("{prefix}_STATE");
    assert!(fingerprint_component(&seed, Some(&visible_seed), "fallback", true).is_err());
    assert!(fingerprint_component(&build, None, "fallback", true).is_err());
    assert!(fingerprint_component(&state, None, "fallback", true).is_err());
    let new_episode = fingerprint_component(&build, None, "fallback", false)?;
    assert_eq!(new_episode, super::digest_text("fallback"));
    Ok(())
}

#[test]
fn mcp_executable_digest_requires_a_bounded_regular_file() -> Result<(), String> {
    let path = fixture_path("regular");
    fs::write(&path, b"synthetic mcp executable")
        .map_err(|error| format!("cannot write fixture: {error}"))?;
    let value = mcp_executable(
        path.to_str()
            .ok_or_else(|| String::from("fixture path is not UTF-8"))?,
    )?;
    assert_eq!(value["bytes"], 24);
    assert!(value["sha256"].as_str().is_some());
    fs::remove_file(path).map_err(|error| format!("cannot remove fixture: {error}"))?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn mcp_executable_digest_rejects_a_symbolic_link() -> Result<(), String> {
    use std::os::unix::fs::symlink;

    let target = fixture_path("target");
    let link = fixture_path("link");
    fs::write(&target, b"synthetic mcp executable")
        .map_err(|error| format!("cannot write fixture: {error}"))?;
    symlink(&target, &link).map_err(|error| format!("cannot create symlink: {error}"))?;
    let result = mcp_executable(
        link.to_str()
            .ok_or_else(|| String::from("link path is not UTF-8"))?,
    );
    assert!(result.is_err());
    fs::remove_file(link).map_err(|error| format!("cannot remove link: {error}"))?;
    fs::remove_file(target).map_err(|error| format!("cannot remove target: {error}"))?;
    Ok(())
}

#[test]
fn worker_fingerprint_keeps_the_legacy_config_digest() -> Result<(), String> {
    let path = fixture_path("worker-fingerprint");
    fs::write(&path, b"synthetic mcp executable")
        .map_err(|error| format!("cannot write fixture: {error}"))?;
    let path_text = path
        .to_str()
        .ok_or_else(|| String::from("fixture path is not UTF-8"))?;
    let config = RuntimeConfig {
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: String::from("synthetic-token"),
        mcp_binary: path_text.to_owned(),
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
    };
    let settings = RuntimeV3Settings {
        runner: EpisodeRunnerConfig::new(
            1,
            StabilityBarrier::new(1, 1).map_err(|error| error.to_string())?,
            RecoveryController::new(1).map_err(|error| error.to_string())?,
            "synthetic objective",
            Vec::new(),
        )
        .map_err(|error| error.to_string())?,
        exo: ExoConfig::new(String::from("a").repeat(64), 1, 1, 1)
            .map_err(|error| error.to_string())?,
        process: ExoProcessConfig::new(path_text, Vec::new(), None, Vec::new())
            .map_err(|error| error.to_string())?,
    };
    let legacy = super::fingerprint(&config, &settings, false)?;
    let binding = WorkflowBinding::for_launch(true, false, None)?;
    let gameplay = super::fingerprint_with_binding(&config, &settings, &binding, false)?;
    assert_eq!(legacy.seed, gameplay.seed);
    assert_eq!(legacy.build_digest, gameplay.build_digest);
    assert_eq!(legacy.state_digest, gameplay.state_digest);
    assert_eq!(legacy.provider_digest, gameplay.provider_digest);
    assert_ne!(legacy.config_digest, gameplay.config_digest);
    fs::remove_file(path).map_err(|error| error.to_string())?;
    Ok(())
}
