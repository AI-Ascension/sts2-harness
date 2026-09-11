// SPDX-License-Identifier: MIT

use super::*;

const LIVE_ACCEPTED_OPERATION_ID: &str = "22222222-2222-4222-8222-222222222222";

fn accepted_receipt(
    correlation_id: &str,
    operation_id: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value = live_state()?;
    value["kind"] = json!("dispatch_action_response");
    value["correlation_id"] = json!(correlation_id);
    value["operation_id"] = json!(operation_id);
    value["status"] = json!("accepted");
    value["transition"] = Value::Null;
    value["error_code"] = Value::Null;
    value["wait_for_millis"] = Value::Null;
    value["wait_outcome"] = Value::Null;
    value["recovery"] = Value::Null;
    Ok(value)
}

#[test]
fn live_accepted_reconcile_unknown_preserves_uncertainty_and_reaches_transition_barrier()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let lineage = ExecutionLineage::new("run-2", "episode-2", "attempt-2", "trajectory-2")?;
    let fingerprint =
        ExecutionFingerprint::new("seed-2", "build-2", "state-2", "config-2", "provider-2")?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
    let mut runtime_config = config("127.0.0.1:15525".into());
    runtime_config.mcp_binary = fixture.script(&format!(
        "cd '{}' || exit 1\n{}{}{}",
        fixture.0.display(),
        rpc_reply(
            1,
            &accepted_receipt("1", LIVE_ACCEPTED_OPERATION_ID)?,
            false,
        ),
        rpc_reply(
            2,
            &unknown_receipt("recover_response", "2", LIVE_ACCEPTED_OPERATION_ID)?,
            true,
        ),
        rpc_reply(3, &settled_wait(LIVE_ACCEPTED_OPERATION_ID)?, false),
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
        LIVE_ACCEPTED_OPERATION_ID,
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    let dispatched = port.dispatch_action(&identity, &action)?;
    assert_eq!(dispatched.status(), sts2_harness::DispatchStatus::Accepted);
    assert_eq!(
        durable.operation_state(LIVE_ACCEPTED_OPERATION_ID)?,
        sts2_harness::OperationState::Accepted
    );

    let reconciled = RecoveryPort::reconcile(&mut port, LIVE_ACCEPTED_OPERATION_ID)?;
    assert_eq!(reconciled.status(), sts2_harness::DispatchStatus::Unknown);
    assert_eq!(
        durable.operation_state(LIVE_ACCEPTED_OPERATION_ID)?,
        sts2_harness::OperationState::Unknown
    );

    let sample = StabilityBarrier::new(1, 1)?.await_transition_sample(
        &mut port,
        LIVE_ACCEPTED_OPERATION_ID,
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
    assert_eq!(
        names,
        [
            "sts2.dispatch_action",
            "sts2.recover",
            "sts2.wait_for_transition"
        ]
    );
    assert!(port.mcp.as_mut().ok_or("missing MCP")?.close().is_ok());
    Ok(())
}
