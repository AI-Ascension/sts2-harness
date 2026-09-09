// SPDX-License-Identifier: MIT

use std::fs;

use sts2_harness::{
    ActionIdentity, ExecutionFingerprint, ExecutionLineage, ExecutionStore, OperationState,
    RecoveryPort,
};

use super::reconnect_support::Fixture;
use super::*;

#[test]
fn runtime_v3_durable_reconnect_requires_recovery_credentials_without_redispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let mut config = config("127.0.0.1:15525".into());
    config.mcp_binary = fixture.script(&format!(
        "cd '{}' || exit 1\nIFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0\n",
        fixture.0.display()
    ))?;
    let lineage = ExecutionLineage::new(
        config.run_id.clone(),
        config.episode_id.clone(),
        "attempt-synthetic",
        config.trajectory_id.clone(),
    )?;
    let fingerprint = ExecutionFingerprint::new(
        "synthetic-seed",
        "synthetic-build",
        "synthetic-state",
        "synthetic-config",
        "synthetic-provider",
    )?;
    let mut store = ExecutionStore::open_in_memory()?;
    store.start_episode(&lineage, &fingerprint)?;
    let durable = durable::DurableHandle::from_store_for_test(store, lineage, fingerprint)?;
    let mut port =
        RuntimeV3Port::new_with_store(config, TelemetryHandle::disabled(), durable.clone())?;
    port.allocated = true;
    port.recovery_authority = Some(super::reconnect_support::recovery_authority());
    port.mcp = Some(McpProcess::spawn(&port.config)?);
    let mut state: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    let operation_id = "33333333-3333-4333-8333-333333333333";
    state["state_id"] = json!("22222222-2222-4222-8222-222222222222");
    state["observation"]["state_id"] = state["state_id"].clone();
    state["legal_actions"] = json!([
        {"action_id":"combat.end-turn", "action":{"kind":"end_turn"}}
    ]);
    let parsed = parse::observation(&state, "state_response", &port.config)?;
    let action = parsed.actions.actions()[0].clone();
    let observation = port.install(parsed)?;
    let identity = ActionIdentity::new(
        operation_id,
        observation.state_id(),
        observation.generation(),
        action.action_id(),
    )?;
    assert!(port.dispatch_action(&identity, &action).is_err());
    assert_eq!(
        durable.operation_state(operation_id)?,
        OperationState::Unknown
    );
    assert!(port.mcp.as_ref().is_some_and(McpProcess::is_closed));
    assert!(port.reconcile(operation_id).is_err());
    assert_eq!(
        durable.operation_state(operation_id)?,
        OperationState::Unknown
    );
    assert_eq!(port.operations.len(), 1);
    let requests = fs::read_to_string(fixture.0.join("requests"))?;
    let requests: Vec<Value> = requests
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["params"]["name"], "sts2.dispatch_action");
    assert!(
        !requests
            .iter()
            .any(|value| value["params"]["name"] == "sts2.recover")
    );
    Ok(())
}
