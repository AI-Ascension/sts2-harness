// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn fake_mcp_map_error_flushes_a_replayable_settled_prefix() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    let script = fixture.script(&fake_mcp_script()?)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = super::super::config(listener.local_addr()?.to_string());
    runtime_config.mcp_binary = script;
    runtime_config.map_context_enabled = true;
    let mut port = RuntimeV3Port::new_with_telemetry(runtime_config, TelemetryHandle::disabled())?;
    let runner_config = EpisodeRunnerConfig::new(
        8,
        StabilityBarrier::new(2, 1)?,
        RecoveryController::new(1)?,
        "synthetic replay evidence",
        Vec::new(),
    )?
    .with_map_context_enabled(true);
    let gateway = std::thread::spawn(move || fake_gateway(listener));
    let mut source = FirstActionSource;
    let (result, bytes) = recording::capture_replay_events(|| {
        let result = EpisodeRunner::new(runner_config).run(
            &mut port,
            &mut recording::DecisionRecorder::new(&mut source, TelemetryHandle::disabled()),
        );
        if let Err(error) = &result {
            recording::episode_failure(error, &TelemetryHandle::disabled());
        }
        result
    });
    assert!(matches!(
        result,
        Err(EpisodeRunnerError::LegalActions(error))
            if error.code() == "map_snapshot_invalid"
    ));
    gateway.join().map_err(|_| "fake gateway panicked")??;

    let rows = std::str::from_utf8(&bytes)?
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(
        rows.iter()
            .filter_map(|row| row["event"].as_str())
            .collect::<Vec<_>>(),
        [
            "model_decision",
            "action_receipt",
            "operation_wait_completed",
            "episode_failed"
        ]
    );
    assert_eq!(rows[1]["status"], "Accepted");
    let operation_id = rows[2]["operation_id"]
        .as_str()
        .ok_or("replay wait row omitted operation id")?;
    let operation_uuid = uuid::Uuid::parse_str(operation_id)?;
    assert_eq!(operation_uuid.get_version_num(), 4);
    assert_eq!(rows[3]["error_code"], "map_snapshot_invalid");
    assert!(rows.iter().all(|row| {
        row["event"] != "episode_complete"
            && row.to_string().find("synthetic replay evidence").is_none()
    }));
    Ok(())
}
