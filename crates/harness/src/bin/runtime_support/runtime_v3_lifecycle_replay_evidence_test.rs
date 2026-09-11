// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, EpisodeRunner, EpisodeRunnerConfig,
    EpisodeRunnerError, PolicyError, RecoveryController, StabilityBarrier,
};

use super::*;

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let path = std::env::temp_dir().join(format!(
            "sts2-v3-replay-evidence-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn script(&self, content: &str) -> Result<String, Box<dyn std::error::Error>> {
        let path = self.0.join("mcp");
        fs::write(&path, format!("#!/bin/sh\nset -eu\n{content}"))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path.to_str().ok_or("non-UTF-8 fixture path")?.to_owned())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct FirstActionSource;

impl DecisionSource for FirstActionSource {
    fn decide(&mut self, input: &DecisionInput) -> Result<Decision, PolicyError> {
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

fn state_value(
    kind: &str,
    correlation_id: &str,
    state_id: &str,
    generation: u64,
    stage: &str,
    legal_actions: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(correlation_id);
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    value["observation"]["state_id"] = json!(state_id);
    value["observation"]["generation"] = json!(generation);
    value["observation"]["state"] = match stage {
        "setup" => json!({"state":"setup","characters":["ironclad"]}),
        "map" => json!({"state":"map","node_id":"node-start","options":["node-next"]}),
        _ => return Err(format!("unsupported fixture stage {stage}").into()),
    };
    value["legal_actions"] = legal_actions;
    Ok(value)
}

fn start_action() -> Value {
    json!([{"action_id":"start-run","action":{"kind":"start_run","character_id":"ironclad"}}])
}

fn map_action() -> Value {
    json!([{"action_id":"map-next","action":{"kind":"select_map_node","node_id":"node-next"}}])
}

fn map_catalog() -> Value {
    json!([
        {"name":"sts2.observe"},
        {"name":"sts2.legal_actions"},
        {"name":"sts2.dispatch_action"},
        {"name":"sts2.wait_for_transition"},
        {"name":"sts2.reobserve"},
        {"name":"sts2.recover"},
        {"name":"sts2.map_snapshot"}
    ])
}

fn reply(value: Value) -> String {
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' '{}'\n",
        value.to_string().replace('\'', "'\\''")
    )
}

fn reply_artifact(id: u64, value: Value) -> String {
    reply(json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{"content":[{"type":"text","text":value.to_string()}]}
    }))
}

fn reply_artifact_with_request_operation(id: u64, value: Value) -> String {
    let response = json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{"content":[{"type":"text","text":value.to_string()}]}
    })
    .to_string()
    .replace("episode-action-1-1", "__OPERATION_ID__")
    .replace('\'', "'\\''");
    format!(
        "IFS= read -r line || exit 1\noperation_id=$(printf '%s\\n' \"$line\" | sed -n 's/.*\\\"operation_id\\\":\\\"\\([^\\\"]*\\)\\\".*/\\1/p')\nprintf '%s\\n' '{response}' | sed \"s/__OPERATION_ID__/$operation_id/g\"\n"
    )
}

fn gameplay_init() -> String {
    reply(json!({"jsonrpc":"2.0","id":1,"result":{}}))
        + &reply(json!({
            "jsonrpc":"2.0",
            "id":2,
            "result":{"revision":"runtime-v3-gameplay-mcp","tools":[
                {"name":"sts2.observe"},
                {"name":"sts2.legal_actions"},
                {"name":"sts2.dispatch_action"},
                {"name":"sts2.wait_for_transition"},
                {"name":"sts2.reobserve"},
                {"name":"sts2.recover"}
            ]}
        }))
}

fn map_response() -> Value {
    json!({
        "protocol_version":"runtime-map-v1",
        "schema_digest":"ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b",
        "provenance":{"artifact":"sts2-protocol/runtime-map-v1","source":"schemas/runtime-map-v1.schema.json","generator":"hand-authored"},
        "correlation_id":"3", "instance_id":"instance-1", "session_id":"session-1",
        "lease_id":"lease-1", "lease_epoch":1, "generation":2,
        "kind":"snapshot_response", "timeout":null,
        "snapshot": {
            "state_id":"wrong-map-state", "generation":2,
            "schema_version":"visible-map-v1", "projection_version":"runtime-map-v1",
            "game_build":"build", "mod_version":"mod", "map_instance_id":"map-1",
            "act_id":1, "scope_id":"scope-1", "availability":"available",
            "completeness":"complete", "freshness":"current", "reason":null,
            "nodes":[
                {"id":"node-start","row":0,"column":0,"category":"start","visited":true},
                {"id":"node-next","row":1,"column":0,"category":"monster","visited":false}
            ],
            "edges":[{"from":"node-start","to":"node-next"}],
            "position":{"kind":"current","node_id":"node-start"},
            "history":["node-start"], "terminal_node_ids":["node-next"],
            "bindings":[{"graph_node_id":"node-next","host_action_id":"map-next",
                "action":{"kind":"select_map_node","node_id":"node-next"}}]
        }
    })
}

fn fake_mcp_script() -> Result<String, Box<dyn std::error::Error>> {
    let setup = state_value("state_response", "1", "setup-1", 1, "setup", start_action())?;
    let mut catalog = state_value(
        "legal_actions_response",
        "2",
        "setup-1",
        1,
        "setup",
        start_action(),
    )?;
    catalog["observation"] = Value::Null;

    let mut accepted = state_value(
        "dispatch_action_response",
        "3",
        "setup-1",
        1,
        "setup",
        start_action(),
    )?;
    accepted["operation_id"] = json!("episode-action-1-1");
    accepted["status"] = json!("accepted");

    let mut waited = state_value("wait_response", "4", "map-1", 2, "map", map_action())?;
    waited["operation_id"] = json!("episode-action-1-1");
    waited["status"] = json!("settled");
    waited["transition"] = json!({
        "from_generation":1, "to_generation":2,
        "state_id":"map-1", "effect_kind":"start_run_settled"
    });
    waited["wait_outcome"] = json!("successor");

    let map_observation = state_value("state_response", "5", "map-1", 2, "map", map_action())?;
    let mut map_catalog_response = state_value(
        "legal_actions_response",
        "6",
        "map-1",
        2,
        "map",
        map_action(),
    )?;
    map_catalog_response["observation"] = Value::Null;

    let normal = format!(
        "{}{}{}{}{}{}{}{}",
        gameplay_init(),
        reply_artifact(1, setup),
        reply_artifact(2, catalog),
        reply_artifact_with_request_operation(3, accepted),
        reply_artifact_with_request_operation(4, waited),
        reply_artifact(5, map_observation),
        reply_artifact(6, map_catalog_response),
        ""
    );
    let map = format!(
        "{}{}{}",
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-map-v1-mcp","tools":map_catalog()}})
        ),
        reply_artifact(3, map_response())
    );
    Ok(format!(
        "if [ \"$STS2_RUNTIME_PROFILE\" = \"runtime-map-v1\" ]; then\n{map}else\n{normal}fi\n"
    ))
}

fn fake_gateway(listener: TcpListener) -> Result<(), String> {
    let mut allocation = super::accept(&listener)?;
    let headers = super::request(&mut allocation).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/sessions/allocate ") {
        return Err(String::from(
            "fake gateway received an unexpected allocation",
        ));
    }
    let body = json!({
        "status":"allocated", "instance_id":"instance-1", "caller_id":"harness",
        "session_id":"session-1", "lease_id":"lease-1", "lease_epoch":1
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);

    let mut release = super::accept(&listener)?;
    let headers = super::request(&mut release).map_err(|error| error.to_string())?;
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

#[test]
fn fake_mcp_map_error_flushes_a_replayable_settled_prefix() -> Result<(), Box<dyn std::error::Error>>
{
    let fixture = Fixture::new()?;
    let script = fixture.script(&fake_mcp_script()?)?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = super::config(listener.local_addr()?.to_string());
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
