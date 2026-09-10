// SPDX-License-Identifier: MIT

use std::fs;

use serde_json::{Value, json};
use sts2_harness::{EpisodeRuntimePort, RecoveryError, RecoveryPort};

use super::*;

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

fn initialize_sequence() -> String {
    format!(
        "{}{}",
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(json!({
            "jsonrpc":"2.0",
            "id":2,
            "result":{"revision":"runtime-v3-gameplay-mcp","tools":gameplay_tools()}
        }))
    )
}

fn state_response(kind: &str, correlation_id: &str, generation: u64) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(correlation_id);
    value["state_id"] = json!("combat-1");
    value["generation"] = json!(generation);
    value["observation"]["state_id"] = json!("combat-1");
    value["observation"]["generation"] = json!(generation);
    value["observation"]["state"] = json!({"state":"combat","turn_index":generation + 1,"enemies":[]});
    value["legal_actions"] = json!([
        {"action_id":"combat.end-turn","action":{"kind":"end_turn"}}
    ]);
    Ok(value)
}

fn transient_rpc(id: u64, code: i64) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":"transient"}})
}

fn transient_tool(id: u64, code: i64, message: &str) -> Value {
    json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{"isError":true,"content":[{"type":"text",
            "text":format!("gateway error {code}: {message}")}]}
    })
}

fn read_requests(fixture: &Fixture) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let path = fixture.0.join("requests");
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(contents
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?)
}

fn setup_port(
    fixture: &Fixture,
    script: String,
) -> Result<RuntimeV3Port, Box<dyn std::error::Error>> {
    let mut runtime_config = config("127.0.0.1:15525".into());
    let content = format!(
        "cd '{}' || exit 1\n{}",
        fixture.0.display(),
        script
    );
    runtime_config.mcp_binary = fixture.script(&content)?;
    let mut port = RuntimeV3Port::new_with_telemetry(
        runtime_config,
        TelemetryHandle::disabled(),
    )?;
    port.allocated = true;
    port.mcp = Some(McpProcess::spawn(&port.config)?);
    Ok(port)
}

#[test]
fn initial_catalog_eof_timeout_and_unavailable_are_catalog_only_retries()
-> Result<(), Box<dyn std::error::Error>> {
    for (name, failure) in [
        ("eof", "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0"),
        ("timeout", "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\n/bin/sleep 10"),
        ("unavailable", "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' \"$FIXTURE_RESPONSE\""),
    ] {
        let fixture = Fixture::new()?;
        let script = if name == "unavailable" {
            format!(
                "FIXTURE_RESPONSE='{}'\n{}",
                transient_rpc(1, -32003).to_string().replace('\'', "'\\''"),
                failure
            )
        } else {
            failure.to_owned()
        };
        let mut port = setup_port(&fixture, script)?;
        let error = match port.legal_actions("combat-1", 0) {
            Ok(_) => return Err(format!("{name} catalog fault unexpectedly succeeded").into()),
            Err(error) => error,
        };
        assert_eq!(error.code(), "catalog_reobserve");
        assert!(error.is_retryable());
        assert!(port.operations.is_empty());
        assert!(read_requests(&fixture)?.iter().all(|request| {
            request["params"]["name"] != "sts2.dispatch_action"
        }));
        if let Some(mcp) = port.mcp.as_mut() {
            let _ = mcp.close();
        }
    }
    Ok(())
}

fn reconnect_script(
    fixture: &Fixture,
    failure: &str,
    second_phase: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let reobserve_id = 2;
    let (second_init, second_failure) = match second_phase {
        "eof" => (
            initialize_sequence(),
            "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0"
                .to_owned(),
        ),
        "timeout" => (
            initialize_sequence(),
            "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\n/bin/sleep 10"
                .to_owned(),
        ),
        "unavailable" => (
            initialize_sequence(),
            format!(
                "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'",
                transient_rpc(reobserve_id, -32003)
                    .to_string()
                    .replace('\'', "'\\''")
            ),
        ),
        "tool-unavailable" => (
            initialize_sequence(),
            format!(
                "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'",
                transient_tool(reobserve_id, -32003, "gateway is unavailable")
                    .to_string()
                    .replace('\'', "'\\''")
            ),
        ),
        "tool-timeout" => (
            initialize_sequence(),
            format!(
                "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'",
                transient_tool(reobserve_id, -32008, "gateway request timed out")
                    .to_string()
                    .replace('\'', "'\\''")
            ),
        ),
        "init-unavailable" => (
            format!(
                "{}{}",
                reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
                reply(transient_rpc(2, -32003))
            ),
            String::new(),
        ),
        "init-tool-unavailable" => (
            format!(
                "{}{}",
                reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
                reply(transient_tool(2, -32003, "gateway is unavailable"))
            ),
            String::new(),
        ),
        _ => return Err("unsupported second phase".into()),
    };
    Ok(format!(
        "cd '{}' || exit 1\nif [ -e started ]; then\n{}{}\nelse\n: > started\n{}\nfi",
        fixture.0.display(),
        second_init,
        second_failure,
        failure
    ))
}

#[test]
fn reobserve_eof_timeout_and_unavailable_fail_closed_without_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    for initial in ["eof", "timeout"] {
        for during_reobserve in [
            "eof",
            "timeout",
            "unavailable",
            "tool-unavailable",
            "tool-timeout",
        ] {
            let fixture = Fixture::new()?;
            let first_failure = match initial {
                "eof" => "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0",
                "timeout" => "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\n/bin/sleep 10",
                _ => unreachable!(),
            };
            let script = reconnect_script(&fixture, first_failure, during_reobserve)?;
            let mut port = setup_port(&fixture, script)?;
            let error = match port.legal_actions("combat-1", 0) {
                Ok(_) => return Err("initial catalog fault unexpectedly succeeded".into()),
                Err(error) => error,
            };
            assert_eq!(error.code(), "catalog_reobserve");
            let recovery = match RecoveryPort::reobserve(&mut port) {
                Ok(_) => return Err("reobserve fault unexpectedly succeeded".into()),
                Err(error) => error,
            };
            assert_eq!(
                recovery,
                RecoveryError::PortFailure,
                "reobserve fault for {initial}/{during_reobserve}"
            );
            assert_eq!(port.reconnect_attempts, 1);
            assert!(read_requests(&fixture)?.iter().all(|request| {
                request["params"]["name"] != "sts2.dispatch_action"
            }));
            if let Some(mcp) = port.mcp.as_mut() {
                let _ = mcp.close();
            }
        }
    }
    Ok(())
}

#[test]
fn reconnect_initialization_gateway_faults_are_transient_without_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    for second_phase in ["init-unavailable", "init-tool-unavailable"] {
        let fixture = Fixture::new()?;
        let first_failure =
            "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0";
        let script = reconnect_script(&fixture, first_failure, second_phase)?;
        let mut port = setup_port(&fixture, script)?;
        let error = match port.legal_actions("combat-1", 0) {
            Ok(_) => return Err("initial catalog fault unexpectedly succeeded".into()),
            Err(error) => error,
        };
        assert_eq!(error.code(), "catalog_reobserve");
        assert_eq!(
            RecoveryPort::reobserve(&mut port),
            Err(RecoveryError::PortFailure),
            "reconnect initialization fault for {second_phase}"
        );
        assert_eq!(port.reconnect_attempts, 1);
        assert!(read_requests(&fixture)?.iter().all(|request| {
            request["params"]["name"] != "sts2.dispatch_action"
                && request["params"]["name"] != "sts2.reobserve"
        }));
        if let Some(mcp) = port.mcp.as_mut() {
            let _ = mcp.close();
        }
    }
    Ok(())
}

#[test]
fn dispatch_transport_fault_is_terminal_and_never_retried_as_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let script = "IFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0";
    let mut port = setup_port(&fixture, script.to_owned())?;
    let state = state_response("state_response", "1", 0)?;
    let parsed = parse::observation(&state, "state_response", &port.config)?;
    let action = parsed.actions.actions()[0].clone();
    let observation = port.install(parsed)?;
    let identity = sts2_harness::ActionIdentity::new(
        "operation-1",
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    assert!(port.dispatch_action(&identity, &action).is_err());
    let requests = read_requests(&fixture)?;
    assert_eq!(
        requests
            .iter()
            .filter(|request| request["params"]["name"] == "sts2.dispatch_action")
            .count(),
        1
    );
    assert!(!requests.iter().any(|request| {
        request["params"]["name"] == "sts2.legal_actions"
            || request["params"]["name"] == "sts2.reobserve"
    }));
    Ok(())
}
