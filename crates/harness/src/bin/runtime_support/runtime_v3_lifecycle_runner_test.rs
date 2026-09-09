// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_harness::{
    Decision, DecisionInput, DecisionSource, EpisodeRunner, EpisodeRunnerConfig, EpisodeRunnerError,
    EpisodeStage, PolicyError, RecoveryController, RecoveryError, StabilityBarrier,
};

use super::Fixture;
use super::super::config;
use super::super::*;

struct FirstActionSource;

// The fixture may exercise two five-second recovery reads and up to two bounded process-close
// paths (one second graceful plus 250ms force-reap each) before the runner releases its lease.
// Keep one deadline for the whole gateway exchange so a valid recovery cannot outlive a
// phase-local wait, while a stalled fixture remains bounded.
const RUNNER_GATEWAY_DEADLINE: Duration = Duration::from_secs(20);

struct CancelGatewayOnDrop(Arc<AtomicBool>);

impl Drop for CancelGatewayOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

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
            rationale: String::from("synthetic test decision"),
            confidence: None,
        })
    }
}

fn runner_config() -> Result<EpisodeRunnerConfig, Box<dyn std::error::Error>> {
    Ok(EpisodeRunnerConfig::new(
        8,
        StabilityBarrier::new(1, 1)?,
        RecoveryController::new(2)?,
        "synthetic recovery test",
        Vec::new(),
    )?)
}

fn accept_runner_gateway(
    listener: &std::net::TcpListener,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<std::net::TcpStream, String> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(String::from("fake gateway cancelled"));
        }
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(String::from("fake gateway did not receive expected cleanup"));
                }
                std::thread::yield_now();
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn runner_gateway(
    listener: std::net::TcpListener,
    cancelled: Arc<AtomicBool>,
) -> Result<(), String> {
    let deadline = Instant::now() + RUNNER_GATEWAY_DEADLINE;
    let mut allocation = accept_runner_gateway(&listener, deadline, &cancelled)?;
    let headers = super::request(&mut allocation).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/sessions/allocate ") {
        return Err(String::from("runner did not allocate through the gateway"));
    }
    let body = json!({
        "status": "allocated",
        "instance_id": "instance-1",
        "caller_id": "harness",
        "session_id": "session-1",
        "lease_id": "lease-1",
        "lease_epoch": 1
    })
    .to_string();
    write!(
        allocation,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .map_err(|error| error.to_string())?;
    drop(allocation);

    let mut release = accept_runner_gateway(&listener, deadline, &cancelled)?;
    let headers = super::request(&mut release).map_err(|error| error.to_string())?;
    if !headers.starts_with("POST /v1/instances/instance-1/release ") {
        return Err(String::from("runner did not release through the gateway"));
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

fn v3_state(
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
    value["observation"]["state"] = if stage == "victory" {
        json!({"state": stage})
    } else {
        json!({"state": stage, "turn_index": generation + 1, "enemies": []})
    };
    value["legal_actions"] = legal_actions;
    Ok(value)
}

fn expert_state(
    state_id: &str,
    generation: u64,
    stage: &str,
    terminal: bool,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ))?;
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    if stage == "victory" {
        value["state"] = json!({"state": stage});
    } else {
        value["state"]["state"] = json!(stage);
    }
    if terminal {
        value["legal_actions"] = json!([]);
    }
    Ok(value)
}

fn reply_artifact(id: u64, value: Value) -> String {
    reply(json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{"content":[{"type":"text","text":value.to_string()}]}
    }))
}

fn gameplay_tools() -> Value {
    json!([
        {"name":"sts2.observe"},
        {"name":"sts2.legal_actions"},
        {"name":"sts2.dispatch_action"},
        {"name":"sts2.wait_for_transition"},
        {"name":"sts2.reobserve"},
        {"name":"sts2.recover"}
    ])
}

fn expert_tools() -> Value {
    json!([
        {"name":"sts2.expert_state"},
        {"name":"sts2.expert_action"},
        {"name":"sts2.expert_reconcile"}
    ])
}

fn init_sequence(expert: bool) -> String {
    let (revision, tools) = if expert {
        ("runtime-v4-expert-mcp", expert_tools())
    } else {
        ("runtime-v3-gameplay-mcp", gameplay_tools())
    };
    format!(
        "{}{}",
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(json!({
            "jsonrpc":"2.0",
            "id":2,
            "result":{"revision":revision,"tools":tools}
        }))
    )
}

fn keep_reading() -> &'static str {
    "while IFS= read -r line; do printf '%s\\n' \"$line\" >> requests; done\n"
}

fn pipe_eof() -> &'static str {
    "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nexit 0\n"
}

fn pipe_timeout() -> &'static str {
    "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\n/bin/sleep 10\n"
}

include!("runtime_v3_lifecycle_runner_fixture.rs");

fn run_runner_fixture(
    fixture: &Fixture,
    failure: &str,
    expert_profile: bool,
    expected_reconnects: u8,
    expected_recoveries: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = config(listener.local_addr()?.to_string());
    runtime_config.runtime_profile = if expert_profile {
        String::from("runtime-v4-expert")
    } else {
        String::from("runtime-v3-gameplay")
    };
    runtime_config.mcp_binary = runner_script(fixture, failure, expert_profile)?;
    let mut port = RuntimeV3Port::new_with_telemetry(
        runtime_config,
        TelemetryHandle::disabled(),
    )?;
    let cancelled = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway_cancelled = Arc::clone(&cancelled);
        let gateway = scope.spawn(move || runner_gateway(listener, gateway_cancelled));
        let _cancel_gateway_on_drop = CancelGatewayOnDrop(Arc::clone(&cancelled));
        let mut source = FirstActionSource;
        let report = EpisodeRunner::new(runner_config()?).run(&mut port, &mut source)?;
        assert_eq!(report.terminal_stage(), EpisodeStage::Victory);
        assert_eq!(report.transitions(), 0);
        if failure == "expert-reobserve-gateway" {
            // A closed expert child can race refresh_closed on the normal child. Both paths
            // remain bounded: the normal path needs two or, after one extra refresh, three
            // recovery reads before the terminal expert observation.
            assert!((2..=3).contains(&report.recoveries()));
        } else {
            assert_eq!(report.recoveries(), expected_recoveries);
        }
        assert_eq!(port.reconnect_attempts, expected_reconnects);
        assert!(port.released);
        cancelled.store(true, Ordering::Release);
        gateway.join().map_err(|_| "fake gateway panicked")??;
        Ok(())
    })?;
    let requests = fs::read_to_string(fixture.0.join("requests"))?;
    let requests: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert!(requests.iter().all(|request| {
        request["params"]["name"] != "sts2.dispatch_action"
    }));
    Ok(())
}

fn run_runner_fixture_terminal(
    fixture: &Fixture,
    failure: &str,
    expected_reconnects: u8,
) -> Result<(), Box<dyn std::error::Error>> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut runtime_config = config(listener.local_addr()?.to_string());
    runtime_config.runtime_profile = String::from("runtime-v4-expert");
    runtime_config.mcp_binary = runner_script(fixture, failure, true)?;
    let mut port = RuntimeV3Port::new_with_telemetry(
        runtime_config,
        TelemetryHandle::disabled(),
    )?;
    let cancelled = Arc::new(AtomicBool::new(false));
    std::thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        let gateway_cancelled = Arc::clone(&cancelled);
        let gateway = scope.spawn(move || runner_gateway(listener, gateway_cancelled));
        let _cancel_gateway_on_drop = CancelGatewayOnDrop(Arc::clone(&cancelled));
        let mut source = FirstActionSource;
        let result = EpisodeRunner::new(runner_config()?).run(&mut port, &mut source);
        assert_eq!(
            result,
            Err(EpisodeRunnerError::Recovery(RecoveryError::Terminal))
        );
        assert_eq!(port.reconnect_attempts, expected_reconnects);
        assert!(port.released);
        cancelled.store(true, Ordering::Release);
        gateway.join().map_err(|_| "fake gateway panicked")??;
        Ok(())
    })?;
    let requests: Vec<Value> = fs::read_to_string(fixture.0.join("requests"))?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert!(requests.iter().all(|request| {
        request["params"]["name"] != "sts2.dispatch_action"
    }));
    Ok(())
}

fn reply(value: Value) -> String {
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
        value.to_string().replace('\'', "'\\''")
    )
}

fn reply_for_method(method: &str, mut value: Value, max_requests: u8) -> String {
    value["correlation_id"] = json!("__REQUEST_ID__");
    let response = json!({
        "jsonrpc": "2.0",
        "id": 0,
        "result": {"content": [{"type": "text", "text": value.to_string()}]}
    })
    .to_string()
    .replacen("\"id\":0,", "\"id\":__REQUEST_ID__,", 1)
    .replace('\'', "'\\''");
    format!(
        "remaining={max_requests}\nhandled=0\nwhile IFS= read -r line; do\nprintf '%s\\n' \"$line\" >> requests\ncase \"$line\" in\n*'\"method\":\"tools/call\"'*'\"name\":\"{method}\"'*)\nif [ \"$remaining\" -eq 0 ]; then echo 'fixture received too many {method} requests' >&2; exit 1; fi\nid=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"id\":\\([0-9][0-9]*\\).*/\\1/p')\ncase \"$id\" in\n''|*[!0-9]*) echo 'fixture received a tools/call request without a numeric id' >&2; exit 1;;\nesac\nprintf '%s\\n' '{response}' | sed \"s/__REQUEST_ID__/$id/g\"\nremaining=$((remaining - 1))\nhandled=$((handled + 1))\n;;\n*) echo 'fixture received an unexpected MCP request' >&2; exit 1;;\nesac\ndone\nif [ \"$handled\" -eq 0 ]; then echo 'fixture did not receive {method}' >&2; exit 1; fi\n"
    )
}

#[test]
fn runner_catalog_pipe_eof_reconnects_without_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    run_runner_fixture(&fixture, "eof", false, 1, 1)
}

#[test]
fn runner_catalog_timeout_reconnects_without_dispatch() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    run_runner_fixture(&fixture, "timeout", false, 1, 1)
}

#[test]
fn runner_catalog_gateway_errors_reobserve_without_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    for failure in ["rpc", "tool"] {
        let fixture = Fixture::new()?;
        run_runner_fixture(&fixture, failure, false, 0, 1)?;
    }
    Ok(())
}

#[test]
fn expert_runner_reconnects_and_composes_without_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    run_runner_fixture(&fixture, "eof", true, 1, 1)
}


mod expert_catalog {
    include!("runtime_v3_lifecycle_expert_catalog_runner_test.rs");
}
