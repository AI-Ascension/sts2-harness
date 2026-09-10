// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{EXO_MAX_MAP_REQUEST_BYTES, EXO_MAX_STANDARD_REQUEST_BYTES, ExoDecisionRequest};
use crate::episode::map::{MapDecisionContext, RUNTIME_MAP_PROFILE, RUNTIME_MAP_SCHEMA_DIGEST};
use crate::episode::{ActionKind, EpisodeLegalAction, EpisodeLegalActionSet};
use crate::exo::SanitizedObservation;
use crate::exo::{Decision, ExoConfig, ExoProvider, ExoSession};
use crate::identity::ModelExecutionId;
use crate::{ExoProcessConfig, ExoProcessTransport};

const REVISION: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

// Keep this fixture identical to the near-bound runtime-map stdout test. The test proves that the
// value returned by the executable is the value consumed by the private map parser and Exo wire.
fn near_bound_snapshot() -> Value {
    let mut nodes = vec![json!({
        "id":"start", "row":0, "column":0, "category":"start", "visited":true
    })];
    let mut node_ids = vec![String::from("start")];
    for index in 1..256 {
        let id = format!("z{index:03}{}", "a".repeat(123));
        node_ids.push(id.clone());
        nodes.push(json!({
            "id":id, "row":index, "column":0, "category":"monster", "visited":false
        }));
    }
    let mut edges = Vec::new();
    'outer: for from in 0..node_ids.len() {
        for to in (from + 1)..node_ids.len() {
            edges.push(json!({"from":node_ids[from],"to":node_ids[to]}));
            if edges.len() == 879 {
                break 'outer;
            }
        }
    }
    json!({
        "state_id":"state-1", "generation":1, "schema_version":"visible-map-v1",
        "projection_version":"runtime-map-v1", "game_build":"build", "mod_version":"mod",
        "map_instance_id":"map-1", "act_id":1, "scope_id":"scope-1", "availability":"available",
        "completeness":"complete", "freshness":"current", "reason":null,
        "nodes":nodes, "edges":edges, "position":{"kind":"current","node_id":"start"},
        "history":["start"], "terminal_node_ids":[node_ids[255]],
        "bindings":[{"graph_node_id":node_ids[1],"host_action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":node_ids[1]}}]
    })
}

fn envelope(snapshot: Value) -> Value {
    json!({
        "protocol_version": RUNTIME_MAP_PROFILE, "schema_digest": RUNTIME_MAP_SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/runtime-map-v1",
            "source":"schemas/runtime-map-v1.schema.json", "generator":"hand-authored"},
        "correlation_id":"3", "instance_id":"instance-1", "session_id":"gateway-session-1",
        "lease_id":"lease-1", "lease_epoch":1, "generation":1, "kind":"snapshot_response",
        "snapshot":snapshot, "timeout":null
    })
}

fn observation() -> SanitizedObservation {
    SanitizedObservation::new(json!({
        "state_id":"state-1", "generation":1, "visible_seed":null,
        "player":{"hp":50,"max_hp":50,"energy":3,"gold":99,
            "hand":[],"deck":[],"discard":[],"exhaust":[]},
        "state":{"state":"map","node_id":"start","options":["z001"]},
        "legal_actions":[{"action_id":"move-1",
            "action":{"kind":"select_map_node","node_id":"z001"}}]
    }))
    .expect("test observation must satisfy the fair-play contract")
}

#[test]
#[ignore = "requires STS2_REAL_MCP_BINARY and a real MCP executable"]
fn real_mcp_map_snapshot_reaches_map_context_and_exo_bridge()
-> Result<(), Box<dyn std::error::Error>> {
    let binary = std::env::var("STS2_REAL_MCP_BINARY")?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let expected = envelope(near_bound_snapshot());
    let snapshot_bytes = serde_json::to_vec(&expected["snapshot"])?.len();
    let gateway_body = serde_json::to_vec(&expected)?;
    let gateway_body_bytes = gateway_body.len();
    let server = std::thread::spawn(move || serve_gateway(listener, gateway_body));
    let mut child = Command::new(binary)
        .env_clear()
        .env("PATH", std::env::var("PATH")?)
        .env("STS2_GATEWAY_ADDR", address.to_string())
        .env("STS2_GATEWAY_TOKEN", "test-token")
        .env("STS2_RUNTIME_PROFILE", "runtime-map-v1")
        .env("STS2_INSTANCE_ID", "instance-1")
        .env("STS2_CALLER_ID", "harness")
        .env("STS2_SESSION_ID", "gateway-session-1")
        .env("STS2_MCP_SESSION_ID", "mcp-session-1")
        .env("STS2_LEASE_ID", "lease-1")
        .env("STS2_LEASE_EPOCH", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("MCP stdin was unavailable")?;
    let stdout = child.stdout.take().ok_or("MCP stdout was unavailable")?;
    let mut output = BufReader::new(stdout);
    let initialize = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-06-18","capabilities":{},
        "clientInfo":{"name":"sts2-harness-map-chain-test","version":"0.0.0"}}});
    let _ = call(&mut input, &mut output, &initialize)?;
    let _ = call(
        &mut input,
        &mut output,
        &json!({
        "jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
    )?;
    write_frame(
        &mut input,
        &json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
        "name":"sts2.map_snapshot","arguments":{"instance_id":"instance-1",
        "mcp_session_id":"mcp-session-1","lease_id":"lease-1","lease_epoch":1,
        "generation":1}}}),
    )?;
    let mut frame = String::new();
    output.read_line(&mut frame)?;
    if frame.is_empty() {
        return Err("MCP stdout ended before the map response".into());
    }
    let response: Value = serde_json::from_str(&frame)?;
    let returned: Value = serde_json::from_str(
        response["result"]["content"][0]["text"]
            .as_str()
            .ok_or("MCP map response omitted JSON text")?,
    )?;
    drop(input);
    drop(output);
    let server_result = server.join().map_err(|_| "map gateway fixture panicked")?;
    server_result.map_err(|error| error.to_string())?;
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("MCP executable exited with {status}").into());
    }
    assert_eq!(returned, expected, "MCP map envelope changed in transit");
    assert!(snapshot_bytes <= 256 * 1024);
    assert!(gateway_body_bytes <= 256 * 1024);
    assert!(gateway_body_bytes > 256 * 1024 - 4096);
    assert!(frame.len() > 256 * 1024 && frame.len() <= 512 * 1024);
    eprintln!(
        "real MCP to Exo map chain bytes: snapshot={snapshot_bytes} gateway_body={gateway_body_bytes} mcp_stdout_frame={}",
        frame.len()
    );

    let actions = EpisodeLegalActionSet::new(
        "state-1",
        1,
        vec![EpisodeLegalAction::new(
            "move-1",
            ActionKind::SelectMapNode,
        )?],
    )?;
    let context = MapDecisionContext::from_mcp_value(&returned, "state-1", 1, &actions)?;
    assert_eq!(context.to_wire()["snapshot"], returned["snapshot"]);
    let request = ExoDecisionRequest::new_with_map(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        REVISION,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        8 * 1024,
        context.clone(),
    )?;
    let encoded = request.encode(EXO_MAX_MAP_REQUEST_BYTES)?;
    let encoded_value: Value = serde_json::from_slice(&encoded)?;
    assert_eq!(
        encoded_value["map_context"]["snapshot"],
        returned["snapshot"]
    );
    assert!(encoded.len() > EXO_MAX_STANDARD_REQUEST_BYTES);
    let digest = context.snapshot_digest().to_owned();
    let bridge = ExoProcessConfig::new(
        "/bin/sh",
        vec![
            String::from("-c"),
            format!(
                r#"request=$(cat) || exit 1; bytes=$(printf '%s' "$request" | wc -c); test "$bytes" -gt 131072 || exit 2; case "$request" in *'"schema":"sts2.exo-decision-map-v1"'*'"snapshot_digest":"{digest}"'*) printf '%s' '{{"decision":"action","action_id":"move-1","rationale":"real map chain"}}' ;; *) exit 2 ;; esac"#
            ),
        ],
        None,
        Vec::new(),
    )?;
    let config = ExoConfig::new(REVISION, EXO_MAX_MAP_REQUEST_BYTES, 8 * 1024, 1_000)?;
    let provider = ExoProvider::new(ExoProcessTransport::new(bridge), config);
    let mut session = ExoSession::new(provider);
    let decision = session.decide_with_map(
        ModelExecutionId::new(1).ok_or("execution identity")?,
        "state-1",
        1,
        observation(),
        vec![String::from("move-1")],
        "choose a legal map node",
        Vec::new(),
        context,
    )?;
    assert_eq!(
        decision,
        Decision::Action {
            action_id: String::from("move-1"),
            rationale: String::from("real map chain"),
            confidence: None,
        }
    );
    Ok(())
}

fn write_frame(input: &mut impl Write, request: &Value) -> Result<(), Box<dyn std::error::Error>> {
    serde_json::to_writer(&mut *input, request)?;
    input.write_all(b"\n")?;
    input.flush()?;
    Ok(())
}

fn call(
    input: &mut impl Write,
    output: &mut impl BufRead,
    request: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    write_frame(input, request)?;
    let mut frame = String::new();
    output.read_line(&mut frame)?;
    if frame.is_empty() {
        return Err("MCP stdout ended before the response".into());
    }
    Ok(serde_json::from_str(&frame)?)
}

fn serve_gateway(
    listener: TcpListener,
    body: Vec<u8>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err("map gateway fixture accept timed out".into());
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.into()),
        }
    };
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("map gateway request ended before headers".into());
        }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > 8192 {
            return Err("map gateway headers exceeded the bound".into());
        }
    }
    let request = String::from_utf8(request)?;
    if !request.starts_with("GET /v1/instances/instance-1/map-snapshot HTTP/1.1\r\n") {
        return Err("map gateway route was unexpected".into());
    }
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    Ok(())
}
