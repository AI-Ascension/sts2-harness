// SPDX-License-Identifier: MIT

use super::super::reply_text_with_id;
use super::{Fixture, reply};
use serde_json::{Value, json};

pub(in super::super::super) fn response_script(
    fixture: &Fixture,
    lookup: &Value,
    reconcile: &Value,
) -> Result<String, Box<dyn std::error::Error>> {
    response_script_with_identity(
        fixture,
        lookup,
        reconcile,
        "instance-1",
        "session-1",
        "lease-1",
        1,
    )
}

pub(in super::super::super) fn response_script_with_identity(
    fixture: &Fixture,
    lookup: &Value,
    reconcile: &Value,
    instance_id: &str,
    session_id: &str,
    lease_id: &str,
    lease_epoch: u64,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut observed: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    observed["correlation_id"] = json!("1");
    observed["instance_id"] = json!(instance_id);
    observed["session_id"] = json!(session_id);
    observed["lease_id"] = json!(lease_id);
    observed["lease_epoch"] = json!(lease_epoch);
    observed["generation"] = json!(1);
    observed["observation"]["generation"] = json!(1);
    observed["observation"]["state"]["turn_index"] = json!(2);
    let operation = lookup["payload"]["operation"].clone();
    let operation_id = operation["operation_id"].as_str().unwrap_or("");
    let from_generation = operation["expected_boundary"]["generation"]
        .as_u64()
        .unwrap_or(0);
    let mut wait = observed.clone();
    wait["kind"] = json!("wait_response");
    wait["correlation_id"] = json!("1");
    wait["operation_id"] = json!(operation_id);
    wait["status"] = json!("settled");
    wait["transition"] = json!({
        "from_generation": from_generation,
        "to_generation": 1,
        "state_id": wait["state_id"],
        "effect_kind": "combat.end-turn_settled"
    });
    wait["wait_outcome"] = json!("successor");
    let wait_text = wait
        .to_string()
        .replace("\"legal_actions\":[]", "\"legal_actions\" : [ ]");
    let mut unresolved_wait = wait.clone();
    unresolved_wait["status"] = json!("unknown");
    unresolved_wait["observation"] = Value::Null;
    unresolved_wait["legal_actions"] = Value::Null;
    unresolved_wait["transition"] = Value::Null;
    unresolved_wait["error_code"] = json!("recovery_required");
    unresolved_wait["wait_outcome"] = json!("recovery_required");
    let unresolved_wait_text = unresolved_wait.to_string();
    let wait_response = format!(
        "if [ \"$STS2_GATEWAY_TOKEN\" = \"fixture-unresolved-witness\" ]; then\n{}else\n{}fi\n",
        reply_text_with_id(1, &unresolved_wait_text),
        reply_text_with_id(1, &wait_text)
    );
    observed["correlation_id"] = json!("2");
    let observed_text = observed
        .to_string()
        .replace("\"legal_actions\":[]", "\"legal_actions\" : [ ]");
    let gameplay_tools: Vec<_> = [
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
    let recovery_tools: Vec<_> = [
        "watchdog.bootstrap",
        "watchdog.host_fence",
        "watchdog.lease_acquire",
        "watchdog.lease_renew",
        "watchdog.lease_revoke",
        "watchdog.operation_intent",
        "watchdog.operation_dispatch",
        "watchdog.operation_lookup",
        "watchdog.operation_reconcile",
    ]
    .into_iter()
    .map(|name| json!({"name":name}))
    .collect();
    let script = format!(
        "cd '{}' || exit 1\ntrap 'status=$?; printf \"exit=%s\\n\" \"$status\" > child-status' EXIT\nif [ \"$STS2_RUNTIME_PROFILE\" = \"watchdog-recovery-v1\" ]; then\n{}{}{}{}else\n{}{}{}{}\nfi\n",
        fixture.0.display(),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"watchdog-recovery-v1-mcp","tools":recovery_tools}})
        ),
        recovery_reply(1, lookup),
        recovery_reply(2, reconcile),
        reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        reply(
            json!({"jsonrpc":"2.0","id":2,"result":{"revision":"runtime-v3-gameplay-mcp","tools":gameplay_tools}})
        ),
        wait_response,
        reply_text_with_id(2, &observed_text)
    );
    fixture.script(&script)
}

fn recovery_reply(id: u64, frame: &Value) -> String {
    let is_error = !matches!(
        frame["payload"]["result"]["status"].as_str(),
        Some("SETTLED" | "REJECTED" | "RECONCILED")
    );
    reply(json!({"jsonrpc":"2.0","id":id,"result":{
        "isError":is_error,"content":[{"text":frame.to_string()}]
    }}))
}
