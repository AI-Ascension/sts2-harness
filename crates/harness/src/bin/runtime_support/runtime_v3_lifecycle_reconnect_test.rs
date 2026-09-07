// SPDX-License-Identifier: MIT

use std::fs;

use sts2_harness::{ActionIdentity, RecoveryPort};

use super::reconnect_support::Fixture;
use super::*;

#[test]
fn runtime_v3_reconnect_requires_durable_identity_without_redispatch()
-> Result<(), Box<dyn std::error::Error>> {
    let fixture = Fixture::new()?;
    let mut config = config("127.0.0.1:15525".into());
    config.mcp_binary = fixture.script(&format!(
        "cd '{}' || exit 1\nIFS= read -r line || exit 0\nprintf '%s\\n' \"$line\" >> requests\nexit 0\n",
        fixture.0.display()
    ))?;
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
    assert!(port.reconcile("op-1").is_err());
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
