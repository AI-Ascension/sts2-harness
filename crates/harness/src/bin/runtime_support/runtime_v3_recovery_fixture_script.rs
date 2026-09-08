// SPDX-License-Identifier: MIT

use super::super::reply_text;
use super::{Fixture, reply};
use serde_json::{Value, json};

pub(in super::super::super) fn response_script(
    fixture: &Fixture,
    lookup: &Value,
    reconcile: &Value,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut observed: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    observed["correlation_id"] = json!("1");
    observed["generation"] = json!(1);
    observed["observation"]["generation"] = json!(1);
    observed["observation"]["state"]["turn_index"] = json!(2);
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
        "cd '{}' || exit 1\ntrap 'status=$?; printf \"exit=%s\\n\" \"$status\" > child-status' EXIT\nif [ \"$STS2_RUNTIME_PROFILE\" = \"watchdog-recovery-v1\" ]; then\n{}{}{}{}else\n{}{}{}\nfi\n",
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
        reply_text(&observed_text)
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
