// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn verified_prefix_reuses_one_real_runtime_port_for_live_continuation()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let script = fixture.script(&fake_prefix_continuation_mcp()?)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = super::super::config(listener.local_addr()?.to_string());
    runtime_config.mcp_binary = script;
    let mut port = RuntimeV3Port::new_with_telemetry(runtime_config, TelemetryHandle::disabled())?;
    let runner_config = EpisodeRunnerConfig::new(
        8,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(1)?,
        "synthetic prefix continuation",
        Vec::new(),
    )?;
    let gateway = std::thread::spawn(move || fake_gateway(listener));
    let mut live_source = FirstActionSource;
    let mut boundary_published = false;
    let mut publish_boundary = || {
        boundary_published = true;
        Ok(())
    };

    let result = episode_replay::run_prefix_and_continue(
        &mut port,
        &runner_config,
        &settled_prefix_bytes()?,
        &mut live_source,
        &mut publish_boundary,
    );
    let result = result?;

    gateway.join().map_err(|_| "fake gateway panicked")??;
    assert!(boundary_published);
    assert!(matches!(
        result,
        episode_replay::ReplayOutcome::Terminal {
            stage: EpisodeStage::Defeat,
            ..
        }
    ));
    Ok(())
}
