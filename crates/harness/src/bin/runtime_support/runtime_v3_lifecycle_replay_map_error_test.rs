// SPDX-License-Identifier: MIT

use std::io::Write;
use std::net::TcpListener;

use sts2_harness::{ExecutionFingerprint, ExecutionLineage, ExecutionStore};

use super::super::super::{durable::DurableHandle, recording::DecisionRecorder};
use super::*;

struct MapErrorActionSource {
    calls: usize,
}

impl DecisionSource for MapErrorActionSource {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
        self.calls += 1;
        let action_id = input
            .legal_actions
            .actions()
            .first()
            .map(|action| action.action_id().to_owned())
            .ok_or(PolicyError::IllegalAction)?;
        Ok(Decision::Action {
            action_id,
            rationale: String::from("synthetic replay evidence decision"),
            confidence: None,
        })
    }
}

fn fake_gateway_for_durable_map(listener: TcpListener) -> Result<(), String> {
    let mut allocation = super::super::accept(&listener)?;
    let headers = super::super::request(&mut allocation).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/sessions/allocate ") {
        return Err(String::from(
            "fake gateway received an unexpected allocation",
        ));
    }
    let body = json!({
        "status":"allocated", "instance_id":"instance-1", "caller_id":"harness",
        "session_id":"gateway-session-1", "lease_id":"lease-1", "lease_epoch":1
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);

    let mut release = super::super::accept(&listener)?;
    let headers = super::super::request(&mut release).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/instances/instance-1/release ") {
        return Err(String::from("fake gateway received an unexpected release"));
    }
    let body = r#"{"status":"released"}"#;
    write!(
        release,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn durable_handle_for_replay(
    run_id: &str,
    episode_id: &str,
    trajectory_id: &str,
) -> Result<DurableHandle, Box<dyn std::error::Error>> {
    let lineage = ExecutionLineage::new(
        run_id,
        episode_id,
        "attempt-map-replay-error",
        trajectory_id,
    )?;
    let config_digest = sts2_harness::sha256_hex(b"synthetic-test-config");
    let fingerprint = ExecutionFingerprint::new(
        "synthetic-seed",
        "synthetic-build",
        "synthetic-state",
        config_digest,
        "synthetic-test-provider",
    )?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    Ok(DurableHandle::from_store_for_test(
        store,
        lineage,
        fingerprint,
    )?)
}

type DurableMapRuntimeFixture = (
    RuntimeV3Port,
    DurableHandle,
    std::thread::JoinHandle<Result<(), String>>,
);

fn durable_map_runtime_fixture(
    fixture: &Fixture,
) -> Result<DurableMapRuntimeFixture, Box<dyn std::error::Error>> {
    const OLD_SESSION: &str = r#"\"session_id\":\"session-1\""#;
    const NEW_SESSION: &str = r#"\"session_id\":\"gateway-session-1\""#;
    let mut runtime_config = super::super::config("127.0.0.1:0".into());
    runtime_config.session_id = String::from("gateway-session-1");
    let mut script = fake_mcp_script()?;
    if script.matches(OLD_SESSION).count() != 7 {
        return Err("map MCP fixture session identity changed".into());
    }
    script = script.replace(OLD_SESSION, NEW_SESSION);
    if script.matches(NEW_SESSION).count() != 7 || script.contains(OLD_SESSION) {
        return Err("map MCP fixture session identity binding changed".into());
    }
    runtime_config.mcp_binary = fixture.script(&script)?;
    let durable = durable_handle_for_replay(
        &runtime_config.run_id,
        &runtime_config.episode_id,
        &runtime_config.trajectory_id,
    )?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    runtime_config.gateway_address = listener.local_addr()?.to_string();
    runtime_config.map_context_enabled = true;
    let port = RuntimeV3Port::new_with_store(
        runtime_config,
        TelemetryHandle::disabled(),
        durable.clone(),
    )?;
    let gateway = std::thread::spawn(move || fake_gateway_for_durable_map(listener));
    Ok((port, durable, gateway))
}

fn assert_replay_prefix(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let rows = std::str::from_utf8(bytes)?
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

#[test]
fn fake_mcp_map_error_flushes_a_replayable_settled_prefix() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    let (mut port, durable, gateway) = durable_map_runtime_fixture(&fixture)?;
    let runner_config = EpisodeRunnerConfig::new(
        8,
        StabilityBarrier::new(2, 1)?,
        RecoveryController::new(1)?,
        "synthetic replay evidence",
        Vec::new(),
    )?
    .with_map_context_enabled(true);
    let mut source = MapErrorActionSource { calls: 0 };
    let mut recorder =
        DecisionRecorder::with_durable(&mut source, TelemetryHandle::disabled(), durable);
    let (result, bytes) = recording::capture_replay_events(|| {
        let result = EpisodeRunner::new(runner_config).run(&mut port, &mut recorder);
        if let Err(error) = &result {
            recording::episode_failure(error, &TelemetryHandle::disabled());
        }
        result
    });
    drop(recorder);
    assert!(matches!(
        result,
        Err(EpisodeRunnerError::LegalActions(error))
            if error.code() == "map_snapshot_invalid"
    ));
    assert_eq!(source.calls, 1);
    gateway.join().map_err(|_| "fake gateway panicked")??;
    assert_replay_prefix(&bytes)?;
    Ok(())
}
