// SPDX-License-Identifier: MIT
//! Original synthetic HTTP/model/tool data; runs the pinned Exo implementation without a provider.
#[allow(dead_code)]
mod support;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use support::{Model, Result, digest, response};

#[test]
#[ignore = "requires built owned bridge/executor, pinned read-only Exo source and Node"]
fn real_exo_duplex_lookup_round_trips() -> Result {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let source = PathBuf::from(std::env::var("STS2_EXO_TEST_SOURCE")?).canonicalize()?;
    let node = PathBuf::from(std::env::var("STS2_EXO_TEST_NODE")?).canonicalize()?;
    let binary = root.join("target/debug/sts2-exo-bridge");
    let executor = root.join("target/exo-executor/debug/sts2-exo-executor");
    let extension = root.join("experiments/exo-agent/extension/src/lookup.ts");
    let model = Model::start()?;
    let config = root.join("target/exo-lookup-oracle-config.json");
    std::fs::write(
        &config,
        serde_json::to_vec(&json!({
            "schema":"sts2.exo-lookup-config-v1","executor":executor,"executor_sha256":digest(&executor)?,
            "source_root":source,"extension":extension,"extension_sha256":digest(&extension)?,
            "node":node,"node_sha256":digest(&node)?,"model":"o3-pro","endpoint":model.endpoint
        }))?,
    )?;
    let described = Command::new(&binary)
        .arg("--lookup-describe")
        .arg(&config)
        .output()?;
    assert!(
        described.status.success(),
        "description: {}",
        String::from_utf8_lossy(&described.stderr)
    );
    let descriptor: Value = serde_json::from_slice(&described.stdout)?;
    assert_eq!(descriptor["max_tool_round_trips"], 32);
    assert_eq!(model.request_count(), 0);
    model.set(200, tool_response("static", false))?;
    let temporary = root.join("target/exo-lookup-oracle-private");
    std::fs::create_dir_all(&temporary)?;
    let mut child = Command::new(&binary)
        .arg("--lookup-synthetic")
        .arg(&config)
        .arg(digest(&config)?)
        .env_clear()
        .env("TMPDIR", &temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("missing stdin")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("missing stdout")?);
    let request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    let action = request["legal_action_ids"][0]
        .as_str()
        .ok_or("action")?
        .to_owned();
    let mut frame = json!({"wire_version":"sts2.exo-lookup-wire-v1","request_id":"private-request",
        "turn_id":"private-turn","sequence":0,"payload":{"kind":"start","request":request}});
    write_frame(&mut input, &frame)?;
    for sequence in 1..=2 {
        let mut line = String::new();
        output.read_line(&mut line)?;
        let query: Value = serde_json::from_str(&line)?;
        assert_eq!(query["request_id"], "private-request");
        assert_eq!(query["turn_id"], "private-turn");
        assert_eq!(query["sequence"], sequence);
        assert_eq!(query["payload"]["kind"], "query");
        assert_eq!(
            query["payload"]["arguments"]["mode"],
            if sequence == 1 { "static" } else { "live" }
        );
        model.replace_response(
            200,
            if sequence == 1 {
                tool_response("live", true)
            } else {
                response(
                    &serde_json::to_string(&json!({"action_id":action}))?,
                    "message",
                )
            },
        )?;
        frame["sequence"] = json!(sequence);
        frame["payload"] = json!({"kind":"feedback","value":{"record_ordinal":sequence-1,
            "data":{"authority":"untrusted_game_information_data","synthetic_complete_payload":"sentinel",
                "cost":1}}});
        write_frame(&mut input, &frame)?;
    }
    let mut line = String::new();
    output.read_line(&mut line)?;
    let decision: Value = serde_json::from_str(&line)?;
    assert_eq!(decision["sequence"], 3);
    assert_eq!(
        decision["payload"],
        json!({"kind":"decision","action_id":action})
    );
    drop(input);
    let result = child.wait_with_output()?;
    assert!(
        result.status.success(),
        "bridge: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(model.request_count(), 3);
    let requests = model.requests.lock().map_err(|_| "poisoned")?;
    let tools = requests[0]["tools"].as_array().ok_or("tools")?;
    assert_eq!(tools.len(), 2);
    let encoded = serde_json::to_string(&requests[2])?;
    assert!(encoded.contains("synthetic_complete_payload"));
    assert!(!encoded.contains("private-request") && !encoded.contains("private-turn"));
    drop(requests);
    reject_feedback(&binary, &config, &temporary, &model, &frame, true)?;
    reject_feedback(&binary, &config, &temporary, &model, &frame, false)?;
    std::fs::write(
        root.join("target/exo-lookup-oracle-report.json"),
        serde_json::to_vec_pretty(&json!({
            "evidence":"confirmed-real-pinned-Exo-synthetic-model-no-game","exo_revision":"b06869ab789dee3f80ca474b5fa89dbe47ccb859",
        "model_requests":3,"tool_round_trips":2,"complete_tool_payload":true,
        "rejected_feedback_cases":["foreign_turn","oversized_data"],
            "bridge_sha256":digest(&binary)?,"executor_sha256":digest(&executor)?,
            "extension_sha256":digest(&extension)?,"node_sha256":digest(&node)?
        }))?,
    )?;
    std::fs::remove_file(config)?;
    Ok(())
}

fn reject_feedback(
    binary: &std::path::Path,
    config: &std::path::Path,
    temporary: &std::path::Path,
    model: &Model,
    template: &Value,
    foreign: bool,
) -> Result {
    model.set(200, tool_response("static", false))?;
    let mut child = Command::new(binary)
        .arg("--lookup-synthetic")
        .arg(config)
        .arg(digest(config)?)
        .env_clear()
        .env("TMPDIR", temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("stdout")?);
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    let mut frame = template.clone();
    frame["sequence"] = json!(0);
    frame["payload"] = json!({"kind":"start","request":request});
    write_frame(&mut input, &frame)?;
    let mut line = String::new();
    output.read_line(&mut line)?;
    assert_eq!(
        serde_json::from_str::<Value>(&line)?["payload"]["kind"],
        "query"
    );
    frame["sequence"] = json!(1);
    frame["payload"] =
        json!({"kind":"feedback","value":if foreign {json!({})}else{json!("x".repeat(7001))}});
    if foreign {
        frame["turn_id"] = json!("foreign-turn");
    }
    write_frame(&mut input, &frame)?;
    drop(input);
    line.clear();
    output.read_line(&mut line)?;
    assert!(
        line.is_empty(),
        "invalid feedback must not yield a decision"
    );
    assert!(!child.wait_with_output()?.status.success());
    assert_eq!(model.request_count(), 1);
    Ok(())
}

fn write_frame(input: &mut impl Write, value: &Value) -> Result {
    input.write_all(&serde_json::to_vec(value)?)?;
    input.write_all(b"\n")?;
    input.flush()?;
    Ok(())
}

fn tool_response(id: &str, live: bool) -> Value {
    let arguments = json!({"operation_id":id,"mode":if live {"live"}else{"static"},"query":{
        "query_kind":if live {"detail"}else{"list"},"entity_kind":"card",
        "target":{"definition_ref":null},"filters":{"display_name":null,"namespaced_ids":[],
            "definition_refs":[],"instance_ids":[]},"projection":"standard","detail_level":"standard",
        "fields":["cost"],"limits":{"page_items":1,"item_bytes":4096,"page_bytes":8192,"text_bytes":1024},
        "cursor":null}});
    let mut value = response("", "tool");
    value["output"][0] = json!({"type":"function_call","id":format!("fc_{id}"),
        "call_id":format!("call_{id}"),"name":"sts2_lookup_query","arguments":arguments.to_string(),
        "status":"completed"});
    value
}
