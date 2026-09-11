// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use sts2_harness::{
    ExecutionFingerprint, ExecutionLineage, ExecutionStore, RecoveryPort, StabilityBarrier,
    WaitOutcome,
};

use super::*;

use super::super::super::durable::DurableHandle;

const LIVE_UNKNOWN_OPERATION_ID: &str = "11111111-1111-4111-8111-111111111111";

fn recovery_script(fixture: &Fixture) -> Result<(), Box<dyn std::error::Error>> {
    let mut settled: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))?;
    settled["kind"] = json!("recover_response");
    settled["correlation_id"] = json!("2");
    let tools: Vec<_> = [
        "sts2.observe",
        "sts2.legal_actions",
        "sts2.dispatch_action",
        "sts2.wait_for_transition",
        "sts2.reobserve",
        "sts2.recover",
    ]
    .into_iter()
    .map(|name| json!({"name":name}))
    .collect();
    let script = format!(
        "cd '{}' || exit 1\n{}{}{}",
        fixture.0.display(),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":tools}})
        ),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"content":[{"text":settled.to_string()}]}})
        )
    );
    fixture.script(&script)?;
    Ok(())
}

fn catalog_recovery_script(fixture: &Fixture) -> Result<String, Box<dyn std::error::Error>> {
    let mut reobserve: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    reobserve["kind"] = json!("reobserve_response");
    reobserve["correlation_id"] = json!("2");
    let mut catalog = reobserve.clone();
    catalog["kind"] = json!("legal_actions_response");
    catalog["correlation_id"] = json!("3");
    catalog["observation"] = Value::Null;
    catalog["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    let transient = json!({
        "jsonrpc":"2.0",
        "id":1,
        "result":{
            "isError":true,
            "content":[{"type":"text","text":"gateway error -32008: gateway request timed out"}]
        }
    });
    let script = format!(
        "cd '{}' || exit 1\n{}{}{}",
        fixture.0.display(),
        reply(transient),
        reply(json!({
            "jsonrpc":"2.0",
            "id":2,
            "result":{"content":[{"text":reobserve.to_string()}]}
        })),
        reply(json!({
            "jsonrpc":"2.0",
            "id":3,
            "result":{"content":[{"text":catalog.to_string()}]}
        }))
    );
    fixture.script(&script)
}

fn live_state() -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    value["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    Ok(value)
}

fn unknown_receipt(
    kind: &str,
    correlation_id: &str,
    operation_id: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value = live_state()?;
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(correlation_id);
    value["state_id"] = Value::Null;
    value["operation_id"] = json!(operation_id);
    value["observation"] = Value::Null;
    value["legal_actions"] = Value::Null;
    value["action"] = Value::Null;
    value["status"] = json!("unknown");
    value["transition"] = Value::Null;
    value["error_code"] = json!("settlement_unproven");
    value["wait_for_millis"] = Value::Null;
    value["wait_outcome"] = Value::Null;
    value["recovery"] = Value::Null;
    Ok(value)
}

fn settled_wait(
    operation_id: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value = live_state()?;
    value["kind"] = json!("wait_response");
    value["correlation_id"] = json!("3");
    value["state_id"] = json!("combat-2");
    value["generation"] = json!(1);
    value["operation_id"] = json!(operation_id);
    value["observation"]["state_id"] = json!("combat-2");
    value["observation"]["generation"] = json!(1);
    value["observation"]["state"]["turn_index"] = json!(2);
    value["action"] = Value::Null;
    value["status"] = json!("settled");
    value["transition"] = json!({
        "from_generation": 0,
        "to_generation": 1,
        "state_id": "combat-2",
        "effect_kind": "combat.end-turn_settled"
    });
    value["error_code"] = Value::Null;
    value["wait_for_millis"] = Value::Null;
    value["wait_outcome"] = json!("successor");
    value["recovery"] = Value::Null;
    Ok(value)
}

fn rpc_reply(id: u64, value: &Value, is_error: bool) -> String {
    let response = json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "isError": is_error,
            "content": [{"type": "text", "text": value.to_string()}]
        }
    });
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> requests\nprintf '%s\\n' '{}'\n",
        response.to_string().replace('\'', "'\\''")
    )
}

#[test]
fn live_unknown_reconcile_preserves_uncertainty_and_reaches_transition_barrier()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let lineage = ExecutionLineage::new("run-1", "episode-1", "attempt-1", "trajectory-1")?;
    let fingerprint =
        ExecutionFingerprint::new("seed", "build", "state", "config", "provider")?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
    let mut runtime_config = config("127.0.0.1:15525".into());
    runtime_config.mcp_binary = fixture.script(&format!(
        "cd '{}' || exit 1\n{}{}{}",
        fixture.0.display(),
        rpc_reply(
            1,
            &unknown_receipt("dispatch_action_response", "1", LIVE_UNKNOWN_OPERATION_ID)?,
            true,
        ),
        rpc_reply(
            2,
            &unknown_receipt("recover_response", "2", LIVE_UNKNOWN_OPERATION_ID)?,
            true,
        ),
        rpc_reply(3, &settled_wait(LIVE_UNKNOWN_OPERATION_ID)?, false),
    ))?;
    let mut port = RuntimeV3Port::new_with_store(
        runtime_config,
        TelemetryHandle::disabled(),
        durable.clone(),
    )?;
    port.allocated = true;
    port.mcp = Some(McpProcess::spawn(&port.config)?);

    let parsed = parse::observation(&live_state()?, "state_response", &port.config)?;
    let action = parsed.actions.actions()[0].clone();
    let observation = port.install(parsed)?;
    let identity = ActionIdentity::new(
        LIVE_UNKNOWN_OPERATION_ID,
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    let dispatched = port.dispatch_action(&identity, &action)?;
    assert_eq!(dispatched.status(), sts2_harness::DispatchStatus::Unknown);
    assert_eq!(
        durable.operation_state(LIVE_UNKNOWN_OPERATION_ID)?,
        sts2_harness::OperationState::Unknown
    );

    let reconciled = RecoveryPort::reconcile(&mut port, LIVE_UNKNOWN_OPERATION_ID)?;
    assert_eq!(reconciled.status(), sts2_harness::DispatchStatus::Unknown);
    assert_eq!(
        durable.operation_state(LIVE_UNKNOWN_OPERATION_ID)?,
        sts2_harness::OperationState::Unknown
    );

    let sample = StabilityBarrier::new(1, 1)?.await_transition_sample(
        &mut port,
        LIVE_UNKNOWN_OPERATION_ID,
        &observation,
    )?;
    assert_eq!(sample.outcome(), WaitOutcome::Successor);
    assert_eq!(sample.observation().map(|value| value.generation()), Some(1));

    let requests = std::fs::read_to_string(fixture.0.join("requests"))?;
    let calls: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let names: Vec<&str> = calls
        .iter()
        .filter_map(|value| value["params"]["name"].as_str())
        .collect();
    assert_eq!(names, ["sts2.dispatch_action", "sts2.recover", "sts2.wait_for_transition"]);
    assert!(port.mcp.as_mut().ok_or("missing MCP")?.close().is_ok());
    Ok(())
}

#[test]
fn transient_catalog_transport_enters_reobserve_without_dispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let mut config = config("127.0.0.1:15525".into());
    config.mcp_binary = catalog_recovery_script(&fixture)?;
    let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
    port.allocated = true;
    port.mcp = Some(McpProcess::spawn(&port.config)?);

    let error = match port.legal_actions("combat-1", 0) {
        Ok(_) => return Err("transient catalog read did not request reobserve".into()),
        Err(error) => error,
    };
    assert_eq!(error.code(), "catalog_reobserve");
    assert!(error.is_retryable());

    let observation = RecoveryPort::reobserve(&mut port)?;
    assert_eq!(observation.state_id(), "combat-1");
    assert_eq!(observation.generation(), 0);
    let actions = port.legal_actions("combat-1", 0)?;
    assert_eq!(actions.actions().len(), 1);
    assert_eq!(actions.actions()[0].action_id(), "combat.end-turn");
    assert!(port.operations.is_empty());

    let requests = fs::read_to_string(fixture.0.join("requests"))?;
    let requests: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["params"]["name"], "sts2.legal_actions");
    assert_eq!(requests[1]["params"]["name"], "sts2.reobserve");
    assert_eq!(requests[2]["params"]["name"], "sts2.legal_actions");
    assert!(
        !requests
            .iter()
            .any(|request| request["params"]["name"] == "sts2.dispatch_action")
    );
    port.mcp.as_mut().ok_or("missing MCP")?.close()?;
    Ok(())
}

#[test]
fn malformed_catalog_identity_or_content_fails_closed_without_reobserve()
-> Result<(), Box<dyn std::error::Error>> {
    for malformed_identity in [true, false] {
        let fixture = Fixture::new()?;
        let mut catalog: Value = serde_json::from_str(include_str!(
            "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
        ))?;
        catalog["kind"] = json!("legal_actions_response");
        catalog["correlation_id"] = if malformed_identity {
            json!("wrong-correlation")
        } else {
            json!("1")
        };
        catalog["observation"] = Value::Null;
        catalog["legal_actions"] = json!([
            {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
        ]);
        if !malformed_identity {
            catalog["unexpected"] = json!(true);
        }
        let script = format!(
            "cd '{}' || exit 1\n{}",
            fixture.0.display(),
            reply(json!({
                "jsonrpc":"2.0",
                "id":1,
                "result":{"content":[{"text":catalog.to_string()}]}
            }))
        );
        let binary = fixture.script(&script)?;
        let mut config = config("127.0.0.1:15525".into());
        config.mcp_binary = binary;
        let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
        port.allocated = true;
        port.mcp = Some(McpProcess::spawn(&port.config)?);

        let error = match port.legal_actions("combat-1", 0) {
            Ok(_) => return Err("malformed catalog was accepted".into()),
            Err(error) => error,
        };
        assert!(!error.is_retryable());
        assert!(matches!(
            error.code(),
            "legal_actions_failed" | "legal_actions_invalid"
        ));
        let requests = fs::read_to_string(fixture.0.join("requests"))?;
        let requests: Vec<Value> = requests
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<_, _>>()?;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["params"]["name"], "sts2.legal_actions");
        assert!(
            !requests
                .iter()
                .any(|request| request["params"]["name"] == "sts2.reobserve")
        );
        assert!(port.operations.is_empty());
        if let Some(mcp) = port.mcp.as_mut() {
            mcp.close()?;
        }
    }
    Ok(())
}

#[test]
fn runtime_v3_reconnect_reconciles_same_operation_without_redispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let mut config = config("127.0.0.1:15525".into());
    config.mcp_binary = fixture.script("IFS= read -r line\nexit 0\n")?;
    let mut port = RuntimeV3Port::new_with_telemetry(config, TelemetryHandle::disabled())?;
    port.allocated = true;
    port.mcp = Some(McpProcess::spawn(&port.config)?);
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    state["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    let parsed = parse::observation(&state, "state_response", &port.config)?;
    let action = parsed.actions.actions()[0].clone();
    let observation = port.install(parsed)?;
    let identity = ActionIdentity::new(
        "op-1",
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    assert!(port.dispatch_action(&identity, &action).is_err());
    assert!(port.mcp.as_ref().is_some_and(McpProcess::is_closed));
    recovery_script(&fixture)?;
    let receipt = port.reconcile("op-1")?;
    assert_eq!(receipt.operation_id(), "op-1");
    assert_eq!(receipt.status(), sts2_harness::DispatchStatus::Settled);
    assert_eq!(port.operations.len(), 1);
    assert_eq!(port.reconnect_attempts, 1);
    let requests = fs::read_to_string(fixture.0.join("requests"))?;
    let requests: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2]["params"]["name"], "sts2.recover");
    assert_eq!(requests[2]["params"]["arguments"]["operation_id"], "op-1");
    assert!(
        !requests
            .iter()
            .any(|value| value["params"]["name"] == "sts2.dispatch_action")
    );
    port.mcp.as_mut().ok_or("missing MCP")?.close()?;
    port.reconnect_attempts = 2;
    assert!(port.reconcile("op-1").is_err());
    Ok(())
}

#[cfg(test)]
mod accepted_recovery {
    include!("runtime_v3_lifecycle_accepted_recovery_test.rs");
}
