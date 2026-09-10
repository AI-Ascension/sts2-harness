// SPDX-License-Identifier: MIT

use sts2_harness::{
    ActionIdentity, DispatchStatus, EpisodeRuntimePort, EpisodeShutdown, RecoveryPort,
};

use super::*;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Result<Self, std::io::Error> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sts2-v4-rest-action-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
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

    fn shell_path(&self) -> Result<String, Box<dyn std::error::Error>> {
        Ok(self
            .0
            .to_str()
            .ok_or("non-UTF-8 fixture directory")?
            .replace('\\', "\\\\")
            .replace('\'', "'\\''"))
    }

    fn read_requests(&self, name: &str) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(self.0.join(name))?;
        content
            .lines()
            .map(serde_json::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn shell_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

fn print_json(value: &Value) -> String {
    format!("printf '%s\\n' '{}'\n", shell_quote(&value.to_string()))
}

fn logged_reply(directory: &str, log: &str, value: &Value) -> String {
    format!(
        "IFS= read -r line || exit 1\nprintf '%s\\n' \"$line\" >> '{directory}/{log}'\n{}",
        print_json(value)
    )
}

fn profile_script(
    directory: &str,
    log: &str,
    revision: &str,
    tools: Value,
    responses: Vec<Value>,
) -> String {
    let initialize = json!({"jsonrpc":"2.0","id":1,"result":{}});
    let catalog = json!({
        "jsonrpc":"2.0",
        "id":2,
        "result":{"revision":revision,"tools":tools}
    });
    let mut script = format!("cd '{directory}'\n: > '{log}'\n");
    script.push_str(&logged_reply(directory, log, &initialize));
    script.push_str(&logged_reply(directory, log, &catalog));
    script.push_str("n=0\nwhile IFS= read -r line; do\nprintf '%s\\n' \"$line\" >> '");
    script.push_str(directory);
    script.push('/');
    script.push_str(log);
    script.push_str("'\nn=$((n+1))\ncase \"$n\" in\n");
    for (index, response) in responses.iter().enumerate() {
        script.push_str(&format!("{}){};;\n", index + 1, print_json(response)));
    }
    script.push_str("*) exit 1;;\nesac\ndone\n");
    script
}

fn normal_tools() -> Value {
    json!([
        {"name":"sts2.observe"},
        {"name":"sts2.legal_actions"},
        {"name":"sts2.dispatch_action"},
        {"name":"sts2.wait_for_transition"},
        {"name":"sts2.reobserve"},
        {"name":"sts2.recover"}
    ])
}

fn rest_tools() -> Value {
    json!([
        {"name":"sts2.expert_state"},
        {"name":"sts2.expert_rest_action"},
        {"name":"sts2.expert_rest_reconcile"}
    ])
}

fn rpc_value(id: u64, value: Value) -> Value {
    json!({
        "jsonrpc":"2.0",
        "id":id,
        "result":{"content":[{"type":"text","text":value.to_string()}]}
    })
}

fn v3_state(
    kind: &str,
    correlation_id: &str,
    state_id: &str,
    generation: u64,
    stage: &str,
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
        "rest" => json!({
            "state":"rest",
            "options":["smith","mend"]
        }),
        "selection" => json!({
            "state":"selection",
            "choices":["card:1","card:2"]
        }),
        _ => return Err(format!("unsupported fixture stage {stage}").into()),
    };
    value["legal_actions"] = json!([]);
    if kind == "legal_actions_response" {
        value["observation"] = Value::Null;
    }
    Ok(value)
}

fn expert_state(
    state_id: &str,
    generation: u64,
    stage: &str,
    legal_actions: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    ))?;
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    value["state"] = match stage {
        "rest" => json!({
            "state":"rest",
            "choices":[
                {"choice_id":"rest:smith","label":"Smith","kind":"rest","domain":null},
                {"choice_id":"rest:mend","label":"Mend","kind":"rest","domain":null}
            ]
        }),
        "selection" => json!({
            "state":"selection",
            "choices":[
                {"choice_id":"card:1","label":"Strike","kind":"selection","domain":null},
                {"choice_id":"card:2","label":"Bash","kind":"selection","domain":null}
            ]
        }),
        _ => return Err(format!("unsupported expert fixture stage {stage}").into()),
    };
    value["legal_actions"] = legal_actions;
    Ok(value)
}

fn rest_option(action_id: &str, option: &str) -> Value {
    json!({"action_id":action_id,"action":{"kind":"rest_option","rest_option_id":option}})
}

fn generic_selection_actions(generation: u64, player: bool) -> Value {
    if player {
        json!([{
            "action_id":format!("cancel_selection:{generation}"),
            "action":{"kind":"cancel_selection","selection_id":null}
        }])
    } else {
        json!([
            {"action_id":format!("select_card:{generation}:card:1"),"action":{"kind":"select_card","selection_id":null,"card_id":"card:1"}},
            {"action_id":format!("select_card:{generation}:card:2"),"action":{"kind":"select_card","selection_id":null,"card_id":"card:2"}},
            {"action_id":format!("confirm_selection:{generation}"),"action":{"kind":"confirm_selection","selection_id":null}},
            {"action_id":format!("cancel_selection:{generation}"),"action":{"kind":"cancel_selection","selection_id":null}}
        ])
    }
}

fn selector_action(
    action_id: &str,
    kind: &str,
    selection_id: &str,
    option: &str,
    choice: Option<&str>,
) -> Value {
    let action = match kind {
        "select_card" => json!({
            "kind":"select_card", "selection_id":selection_id,
            "rest_option_id":option, "card_id":choice.unwrap_or("")
        }),
        "select_player" => json!({
            "kind":"select_player", "selection_id":selection_id,
            "rest_option_id":option, "player_id":choice.unwrap_or("")
        }),
        "confirm_selection" | "cancel_selection" => json!({
            "kind":kind, "selection_id":selection_id, "rest_option_id":option
        }),
        _ => Value::Null,
    };
    json!({"action_id":action_id,"action":action})
}

fn set_response_identity(
    value: &mut Value,
    correlation_id: &str,
    state_id: &str,
    generation: u64,
    operation_id: &str,
    action_id: &str,
    action: Value,
) {
    value["correlation_id"] = json!(correlation_id);
    value["instance_id"] = json!("instance-1");
    value["session_id"] = json!("session-1");
    value["lease_id"] = json!("lease-1");
    value["lease_epoch"] = json!(1);
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    value["operation_id"] = json!(operation_id);
    value["action"] = json!({"action_id":action_id,"action":action});
    if value["effect_witness"].is_object() {
        value["effect_witness"]["operation_id"] = json!(operation_id);
    }
    if value["transition"]["effect_witness"].is_object() {
        value["transition"]["effect_witness"]["operation_id"] = json!(operation_id);
    }
}
