// SPDX-License-Identifier: MIT
//! Real pinned Exo/bootstrap profile through the shipped `sts2-exo-bridge` relay.

#[allow(dead_code)]
mod support;

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use support::{Model, Result, digest, response};

const V1: &str = "sts2.exo-lookup-wire-v1";
const V2: &str = "sts2.exo-lookup-wire-v2-bootstrap";

#[test]
#[ignore = "requires the built shipped Harness relay, pinned Exo source/dependencies and Node"]
fn real_exo_bootstrap_profile_round_trips_through_shipped_relay() -> Result {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let binary = root.join("target/debug/sts2-exo-bridge");
    let executor = root.join("target/exo-executor/debug/sts2-exo-executor");
    let source = PathBuf::from(std::env::var("STS2_EXO_TEST_SOURCE")?).canonicalize()?;
    let node = PathBuf::from(std::env::var("STS2_EXO_TEST_NODE")?).canonicalize()?;
    let extension = root.join("experiments/exo-agent/extension/src/lookup.ts");
    let model = Model::start()?;
    let config = root.join("target/exo-bootstrap-oracle-config.json");
    std::fs::write(
        &config,
        serde_json::to_vec(&json!({
            "schema":"sts2.exo-lookup-config-v1","executor":executor,"executor_sha256":digest(&executor)?,
            "source_root":source,"extension":extension,"extension_sha256":digest(&extension)?,
            "node":node,"node_sha256":digest(&node)?,"model":"o3-pro","endpoint":model.endpoint
        }))?,
    )?;
    let described = Command::new(&binary)
        .arg("--lookup-bootstrap-describe")
        .arg(&config)
        .output()?;
    assert!(
        described.status.success(),
        "description: {}",
        String::from_utf8_lossy(&described.stderr)
    );

    let request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    let action = request["legal_action_ids"][0]
        .as_str()
        .ok_or("missing action")?
        .to_owned();
    model.set(200, bootstrap_tool_response())?;
    let temporary = root.join("target/exo-bootstrap-oracle-private");
    std::fs::create_dir_all(&temporary)?;
    let mut child = Command::new(&binary)
        .arg("--lookup-bootstrap-synthetic")
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
    let mut frame = json!({
        "wire_version":V2,"request_id":"bootstrap-private-request",
        "turn_id":"bootstrap-private-turn","sequence":0,
        "payload":{"kind":"start","request":request,"optional_byte_budget":7000}
    });
    write_frame(&mut input, &frame)?;

    let bootstrap = read_frame(&mut output)?;
    assert_eq!(bootstrap["wire_version"], V2);
    assert_eq!(bootstrap["sequence"], 1);
    assert_eq!(bootstrap["payload"]["kind"], "bootstrap");
    assert_eq!(
        bootstrap["payload"]["arguments"]["operation_id"],
        "bootstrap-live"
    );
    assert_eq!(
        bootstrap["payload"]["arguments"]["definition_ref"]["namespaced_id"],
        "ironclad:strike"
    );

    model.replace_response(200, query_tool_response())?;
    frame["sequence"] = json!(1);
    frame["payload"] = json!({"kind":"feedback","value":{
        "record_ordinal":0,"bootstrap":{"kind":"bootstrap_response","visible_entities":[]}
    }});
    write_frame(&mut input, &frame)?;

    let query = read_frame(&mut output)?;
    assert_eq!(query["wire_version"], V1);
    assert_eq!(query["sequence"], 2);
    assert_eq!(query["payload"]["kind"], "query");
    assert_eq!(query["payload"]["arguments"]["mode"], "live");

    model.replace_response(
        200,
        response(&serde_json::to_string(&json!({"action_id":action}))?, "message"),
    )?;
    frame["sequence"] = json!(2);
    frame["wire_version"] = json!(V1);
    frame["payload"] = json!({"kind":"feedback","value":{
        "record_ordinal":1,"data":{"authority":"untrusted_game_information_data","state":"live"}
    }});
    write_frame(&mut input, &frame)?;

    let decision = read_frame(&mut output)?;
    assert_eq!(decision["wire_version"], V1);
    assert_eq!(decision["sequence"], 3);
    assert_eq!(
        decision["payload"],
        json!({"kind":"decision","action_id":action})
    );
    drop(input);
    let result = child.wait_with_output()?;
    assert!(
        result.status.success(),
        "bootstrap relay: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(model.request_count(), 3);
    let requests = model.requests.lock().map_err(|_| "requests poisoned")?;
    let tools = requests[0]["tools"].to_string();
    assert!(tools.contains("sts2_lookup_bootstrap"));
    assert!(tools.contains("sts2_lookup_query"));
    drop(requests);

    model.set(200, response("{}", "message"))?;
    let mut legacy = Command::new(&binary)
        .arg("--lookup-synthetic")
        .arg(&config)
        .arg(digest(&config)?)
        .env_clear()
        .env("TMPDIR", &temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut legacy_input = legacy.stdin.take().ok_or("missing legacy stdin")?;
    let legacy_frame = json!({
        "wire_version":V2,"request_id":"legacy-private-request",
        "turn_id":"legacy-private-turn","sequence":0,
        "payload":{"kind":"start","request":request,"optional_byte_budget":7000}
    });
    write_frame(&mut legacy_input, &legacy_frame)?;
    drop(legacy_input);
    let legacy_result = legacy.wait_with_output()?;
    assert!(!legacy_result.status.success());
    assert!(legacy_result.stdout.is_empty());
    assert_eq!(model.request_count(), 0);
    std::fs::remove_file(config)?;
    Ok(())
}

fn bootstrap_tool_response() -> Value {
    tool_response(
        "sts2_lookup_bootstrap",
        json!({
            "operation_id":"bootstrap-live",
            "definition_ref":{
                "content_manifest_id":"synthetic-content","entity_kind":"card",
                "namespaced_id":"ironclad:strike","variant":null
            },
            "instance_ref":null
        }),
    )
}

fn query_tool_response() -> Value {
    tool_response(
        "sts2_lookup_query",
        json!({
            "operation_id":"live-query","mode":"live","query":{
                "query_kind":"detail","entity_kind":"card",
                "target":{"definition_ref":null},
                "filters":{"display_name":null,"namespaced_ids":[],"definition_refs":[],"instance_ids":[]},
                "projection":"standard","detail_level":"standard","fields":["cost"],
                "limits":{"page_items":1,"item_bytes":4096,"page_bytes":8192,"text_bytes":1024},
                "cursor":null
            }
        }),
    )
}

fn tool_response(name: &str, arguments: Value) -> Value {
    let mut value = response("", "tool");
    value["output"][0] = json!({
        "type":"function_call","id":format!("fc_{name}"),"call_id":format!("call_{name}"),
        "name":name,"arguments":arguments.to_string(),"status":"completed"
    });
    value
}

fn write_frame(input: &mut impl Write, frame: &Value) -> Result {
    input.write_all(&serde_json::to_vec(frame)?)?;
    input.write_all(b"\n")?;
    input.flush()?;
    Ok(())
}

fn read_frame(output: &mut impl BufRead) -> Result<Value> {
    let mut line = String::new();
    output.read_line(&mut line)?;
    if line.is_empty() {
        return Err("relay closed stdout".into());
    }
    Ok(serde_json::from_str(&line)?)
}
