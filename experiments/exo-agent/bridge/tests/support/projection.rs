// SPDX-License-Identifier: MIT

//! Synthetic fixtures and the assertions applied to every request the pinned runtime sends.

use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

use super::{Model, Result, invoke, response};

/// Forbidden upstream tool names, aliases and case/namespace variants the model may request. The
/// registry advertises none of them; each must be denied at dispatch with the typed code.
pub const FORBIDDEN_TOOLS: [(&str, &str); 12] = [
    ("shell", "shell"),
    ("shell_upper", "SHELL"),
    ("shell_mixed", "Shell"),
    ("shell_functions_namespace", "functions.shell"),
    ("shell_builtin_namespace", "built_in:shell"),
    ("install_agent_tool", "install_agent_tool"),
    ("uninstall_agent_tool", "uninstall_agent_tool"),
    ("manage_tool", "manage_tool"),
    ("inspect_tools", "inspect_tools"),
    ("install_skill", "install_skill"),
    ("remember", "remember"),
    ("lookup_query", "sts2_lookup_query"),
];

pub fn request_envelope(root: &std::path::Path) -> Result<Value> {
    let mut request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    request["objective"] = json!("synthetic exact objective sentinel");
    request["hard_constraints"] = json!(["synthetic complete constraint sentinel"]);
    Ok(json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "host-request-private-sentinel",
        "turn_id": "host-turn-private-sentinel", "request": request
    }))
}

pub fn ordinary_map(envelope: &Value) -> Result<Value> {
    let snapshot = json!({
        "state_id": "map-state", "generation": 0, "schema_version": "visible-map-v1",
        "projection_version": "runtime-map-v1", "game_build": "synthetic", "mod_version": "synthetic",
        "map_instance_id": "map-1", "act_id": 1, "scope_id": "scope-1",
        "availability": "available", "completeness": "complete", "freshness": "current", "reason": null,
        "nodes": [
            {"id": "next", "row": 1, "column": 0, "category": "monster", "visited": false},
            {"id": "start", "row": 0, "column": 0, "category": "start", "visited": true}
        ],
        "edges": [{"from": "start", "to": "next"}],
        "position": {"kind": "current", "node_id": "start"}, "history": ["start"],
        "terminal_node_ids": ["next"],
        "bindings": [{"graph_node_id": "next", "host_action_id": "move-1",
            "action": {"kind": "select_map_node", "node_id": "next"}}]
    });
    // Map snapshot digests require collection order canonicalized by node ID, not path order.
    let mut hash = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    hash.stdin
        .take()
        .ok_or("hash stdin missing")?
        .write_all(&serde_json::to_vec(&snapshot)?)?;
    let output = hash.wait_with_output()?;
    assert!(output.status.success());
    let digest = &String::from_utf8(output.stdout)?[..64];
    let mut map = envelope.clone();
    let request = &mut map["request"];
    request["schema"] = json!("sts2.exo-decision-map-v1");
    request["state_id"] = json!("map-state");
    request["observation"]["state_id"] = json!("map-state");
    request["observation"]["state"] =
        json!({"state": "map", "node_id": "start", "options": ["next"]});
    request["observation"]["legal_actions"] = json!([{"action_id": "move-1",
        "action": {"kind": "select_map_node", "node_id": "next"}}]);
    request["legal_action_ids"] = json!(["move-1"]);
    request["map_context"] = json!({
        "profile": "runtime-map-v1",
        "schema_digest": "ceab0d2dfc471d1ec36d12edaf4654b8c7fdced06548bf47265e11c63f98115b",
        "snapshot_digest": digest, "snapshot": snapshot
    });
    Ok(map)
}

/// Exactly one bounded `sts2.exo-one-shot-evidence-v1` row on stderr; nothing else structured.
pub fn evidence_row(stderr: &[u8], name: &str) -> Result<Value> {
    let diagnostics = std::str::from_utf8(stderr)?;
    let mut evidence = diagnostics
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(serde_json::from_str::<Value>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    assert_eq!(evidence.len(), 1, "{name}: {diagnostics}");
    Ok(evidence.remove(0))
}

pub fn projection(body: &Value, envelope: &Value) -> Result {
    assert!(body.get("tools").is_none_or(|tools| tools == &json!([])));
    let serialized = body.to_string();
    assert!(
        !serialized.contains("old-history-private-sentinel"),
        "old-history sentinel must be absent from the complete model request body"
    );
    for field in ["request_id", "turn_id"] {
        assert!(!serialized.contains(envelope[field].as_str().ok_or("missing host id")?));
    }
    assert!(
        !serialized.contains(
            envelope["request"]["model_execution_id"]
                .as_str()
                .ok_or("missing execution id")?
        )
    );
    fn strings<'a>(value: &'a Value, values: &mut Vec<&'a str>) {
        match value {
            Value::String(text) => values.push(text),
            Value::Array(items) => items.iter().for_each(|item| strings(item, values)),
            Value::Object(items) => items.values().for_each(|item| strings(item, values)),
            _ => {}
        }
    }
    let mut values = Vec::new();
    strings(body, &mut values);
    let projections = values
        .into_iter()
        .filter_map(|text| serde_json::from_str::<Value>(text).ok())
        .filter(|value| value.get("observation").is_some())
        .collect::<Vec<_>>();
    assert_eq!(projections.len(), 1);
    for key in [
        "observation",
        "legal_action_ids",
        "objective",
        "hard_constraints",
    ] {
        assert_eq!(projections[0][key], envelope["request"][key], "{key}");
    }
    Ok(())
}

pub fn old_history_sentinel_rejected(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let mut old_history = envelope.clone();
    old_history["request"]["history"] = json!(["old-history-private-sentinel"]);
    model.set(200, response("{}", "message"))?;
    let output = invoke(
        binary,
        config,
        &serde_json::to_vec(&old_history)?,
        "--synthetic",
        true,
    )?;
    let observed_requests = model.requests.lock().map_err(|_| "poisoned")?.len();
    assert_eq!(
        observed_requests, 0,
        "old_history_sentinel: {observed_requests} model request(s) reached egress"
    );
    let observed_egress = model.request_count();
    assert_eq!(
        observed_egress, 0,
        "old_history_sentinel: {observed_egress} model connection(s) reached egress"
    );
    assert!(!output.status.success() && output.stdout.is_empty());
    assert_eq!(output.stderr, b"exo_bridge_invalid_request\n");
    cases.push(json!({"case": "old_history_sentinel", "passed": true,
        "model_requests": 0, "refusal_before_model_egress": true}));
    Ok(())
}

pub fn synthetic_v2_success(
    model: &Model,
    binary: &std::path::Path,
    config: &std::path::Path,
    envelope: &Value,
    cases: &mut Vec<Value>,
) -> Result {
    let decision = json!({"decision": "wait", "rationale": "synthetic"});
    model.set(200, response(&decision.to_string(), "message"))?;
    let output = invoke(
        binary,
        config,
        &serde_json::to_vec(envelope)?,
        "--synthetic-v2",
        true,
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let returned: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(returned["wire_version"], "sts2.exo-bridge-wire-v2");
    assert_eq!(returned["request_id"], envelope["request_id"]);
    assert_eq!(returned["turn_id"], envelope["turn_id"]);
    assert_eq!(returned["outcome"], "decision");
    assert_eq!(returned["decision"], decision);
    assert!(returned["error_code"].is_null());
    let native = returned["native"].as_object().ok_or("missing v2 receipt")?;
    assert_eq!(native.len(), 5);
    for field in [
        "agent_id",
        "conversation_id",
        "session_id",
        "turn_id",
        "event_cursor",
    ] {
        assert_eq!(native[field].as_str().map(str::len), Some(36), "{field}");
    }
    let observed = model.requests.lock().map_err(|_| "poisoned")?;
    assert_eq!((observed.len(), model.request_count()), (1, 1));
    projection(&observed[0], envelope)?;
    let row = evidence_row(&output.stderr, "synthetic_v2_success")?;
    assert_eq!(row["forwarded_requests"], 1);
    cases.push(json!({"case": "synthetic_v2_success", "passed": true,
        "model_requests": 1, "v2_receipt": "present", "evidence": row}));
    Ok(())
}
