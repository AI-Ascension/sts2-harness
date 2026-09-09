// SPDX-License-Identifier: MIT

use serde_json::{Value, json};

use super::*;

pub(super) fn runner_script(
    fixture: &Fixture,
    mode: &str,
    catalog_digest: &str,
    canonical_b64: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let actions = json!([{"action_id": ACTION_ID, "action": {"kind": "end_turn"}}]);
    let initial = gameplay_state(
        "state_response",
        "1",
        PENDING_STATE_ID,
        0,
        "combat",
        actions.clone(),
    )?;
    let mut catalog = initial.clone();
    catalog["kind"] = json!("legal_actions_response");
    catalog["correlation_id"] = json!("2");
    catalog["observation"] = Value::Null;

    let (mut lookup, mut reconcile) = settled_frames(
        "__OPERATION_ID__",
        PENDING_STATE_ID,
        0,
        "__PAYLOAD_DIGEST__",
        catalog_digest,
        canonical_b64,
    );
    if mode == "unknown" {
        set_recovery_state(&mut lookup, "UNKNOWN");
        set_recovery_state(&mut reconcile, "UNKNOWN");
    }
    let mut not_found = lookup.clone();
    set_recovery_state(&mut not_found, "NOT_FOUND");
    not_found["payload"]["operation"] = Value::Null;

    let normal_tools = json!([
        {"name":"sts2.observe"}, {"name":"sts2.legal_actions"},
        {"name":"sts2.dispatch_action"}, {"name":"sts2.wait_for_transition"},
        {"name":"sts2.reobserve"}, {"name":"sts2.recover"}
    ]);
    let recovery_tools = json!([
        {"name":"watchdog.bootstrap"}, {"name":"watchdog.host_fence"},
        {"name":"watchdog.lease_acquire"}, {"name":"watchdog.lease_renew"},
        {"name":"watchdog.lease_revoke"}, {"name":"watchdog.operation_intent"},
        {"name":"watchdog.operation_dispatch"}, {"name":"watchdog.operation_lookup"},
        {"name":"watchdog.operation_reconcile"}
    ]);
    let normal_init = init_sequence("runtime-v3-gameplay-mcp", normal_tools);
    let recovery_init = init_sequence("watchdog-recovery-v1-mcp", recovery_tools);
    let lookup_response = recovery_response(1, &lookup, mode != "settled")?;
    let reconcile_response = recovery_response(2, &reconcile, mode != "settled")?;
    let not_found_response = recovery_response(1, &not_found, true)?;
    let wait_response = victory_wait()?;

    let mut script = format!("cd '{}' || exit 1\n", fixture.0.display());
    script.push_str("if [ \"$STS2_RUNTIME_PROFILE\" = \"watchdog-recovery-v1\" ]; then\n");
    script.push_str(&recovery_init);
    script.push_str(
        "mode=\"$STS2_GATEWAY_TOKEN\"\nwhile IFS= read -r line; do\nprintf '%s\\n' \"$line\" >> requests\noperation_id=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"operation_id\":\"\\([^\"]*\\)\".*/\\1/p')\npayload_digest=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"payload_digest\":\"\\([^\"]*\\)\".*/\\1/p')\ncase \"$line\" in\n*'\"name\":\"watchdog.operation_lookup\"'*)\n",
    );
    script.push_str("if [ \"$mode\" = \"fixture-not-found\" ]; then\nprintf '%s\\n' '");
    script.push_str(&shell_quote(&not_found_response));
    script.push_str("'\nelse\nprintf '%s\\n' '");
    script.push_str(&shell_quote(&lookup_response));
    script.push_str(
        "' | sed -e \"s/__OPERATION_ID__/$operation_id/g\" -e \"s/__PAYLOAD_DIGEST__/$payload_digest/g\"\nfi\n;;\n*'\"name\":\"watchdog.operation_reconcile\"'*)\nprintf '%s\\n' '",
    );
    script.push_str(&shell_quote(&reconcile_response));
    script.push_str(
        "' | sed -e \"s/__OPERATION_ID__/$operation_id/g\" -e \"s/__PAYLOAD_DIGEST__/$payload_digest/g\"\nexit 0\n;;\nesac\ndone\nelse\n",
    );
    script.push_str("if [ -e dispatch-seen ]; then\n");
    script.push_str(&normal_init);
    script.push_str(
        "while IFS= read -r line; do printf '%s\\n' \"$line\" >> requests; operation_id=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"operation_id\":\"\\([^\"]*\\)\".*/\\1/p'); case \"$line\" in *'\"name\":\"sts2.wait_for_transition\"'*) printf '%s\\n' '",
    );
    script.push_str(&shell_quote(&wait_response));
    script.push_str(
        "' | sed -e \"s/__OPERATION_ID__/$operation_id/g\"; exit 0;; esac; done; exit 1\nelse\n",
    );
    script.push_str(&normal_init);
    script.push_str(&artifact_response(1, &initial));
    script.push_str(&artifact_response(2, &catalog));
    script.push_str(dispatch_reader());
    script.push_str("\nfi\nfi\n");
    fixture.script(&script)
}

fn init_sequence(revision: &str, tools: Value) -> String {
    format!(
        "{}{}",
        super::reply(json!({"jsonrpc":"2.0","id":1,"result":{}})),
        super::reply(json!({"jsonrpc":"2.0","id":2,"result":{"revision":revision,"tools":tools}})),
    )
}

fn artifact_response(id: u64, value: &Value) -> String {
    super::reply(json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{"content":[{"type":"text","text":value.to_string()}]}
    }))
}

fn dispatch_reader() -> &'static str {
    "while IFS= read -r line; do printf '%s\\n' \"$line\" >> requests; case \"$line\" in *'\"name\":\"sts2.dispatch_action\"'*) : > dispatch-seen; exit 0;; esac; done; exit 1"
}

fn recovery_response(
    id: u64,
    frame: &Value,
    is_error: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = json!({
        "jsonrpc":"2.0", "id":id,
        "result":{"isError":is_error,"content":[{"text":"__FRAME__"}]}
    });
    let encoded = serde_json::to_string(&frame.to_string())?;
    Ok(response.to_string().replace("\"__FRAME__\"", &encoded))
}

fn victory_wait() -> Result<String, Box<dyn std::error::Error>> {
    let mut wait: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    wait["kind"] = json!("wait_response");
    wait["correlation_id"] = json!("4");
    let authority = recovery_authority();
    wait["instance_id"] = json!(authority.instance_id);
    wait["lease_id"] = json!(authority.lease_id);
    wait["lease_epoch"] = json!(authority.lease_epoch);
    wait["state_id"] = json!(PENDING_STATE_ID);
    wait["generation"] = json!(1);
    wait["observation"]["state_id"] = json!(PENDING_STATE_ID);
    wait["observation"]["generation"] = json!(1);
    wait["observation"]["state"] = json!({"state":"victory"});
    wait["legal_actions"] = json!([]);
    wait["operation_id"] = json!("__OPERATION_ID__");
    wait["status"] = json!("settled");
    wait["transition"] = json!({
        "from_generation":0,
        "to_generation":1,
        "state_id":PENDING_STATE_ID,
        "effect_kind":"combat.end-turn_settled"
    });
    wait["wait_outcome"] = json!("successor");
    wait["error_code"] = Value::Null;
    let response = json!({
        "jsonrpc":"2.0", "id":4,
        "result":{"content":[{"text":"__FRAME__"}]}
    });
    let encoded = serde_json::to_string(&wait.to_string())?;
    Ok(response.to_string().replace("\"__FRAME__\"", &encoded))
}

fn gameplay_state(
    kind: &str,
    correlation: &str,
    state_id: &str,
    generation: u64,
    stage: &str,
    actions: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))?;
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(correlation);
    let authority = recovery_authority();
    value["instance_id"] = json!(authority.instance_id);
    value["lease_id"] = json!(authority.lease_id);
    value["lease_epoch"] = json!(authority.lease_epoch);
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    value["observation"]["state_id"] = json!(state_id);
    value["observation"]["generation"] = json!(generation);
    value["observation"]["state"] = json!({"state":stage,"turn_index":1,"enemies":[]});
    value["legal_actions"] = actions;
    Ok(value)
}

fn set_recovery_state(frame: &mut Value, state: &str) {
    frame["payload"]["result"]["status"] = json!(state);
    frame["payload"]["operation"]["state"] = json!(state);
}

fn shell_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}
