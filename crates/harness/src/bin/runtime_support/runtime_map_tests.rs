// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::*;

fn large_snapshot() -> Value {
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

fn map_response(config: &RuntimeConfig) -> Result<(String, usize), String> {
    let snapshot = large_snapshot();
    let snapshot_bytes = serde_json::to_vec(&snapshot)
        .map_err(|_| String::from("large map snapshot serialization failed"))?;
    let envelope = json!({
        "protocol_version":PROFILE,
        "schema_digest":SCHEMA_DIGEST,
        "provenance":{"artifact":"sts2-protocol/runtime-map-v1",
            "source":"schemas/runtime-map-v1.schema.json", "generator":"hand-authored"},
        "correlation_id":"3", "instance_id":config.instance_id,
        "session_id":config.session_id, "lease_id":config.lease_id, "lease_epoch":config.lease_epoch,
        "generation":1, "kind":"snapshot_response", "snapshot":snapshot, "timeout":null
    });
    let text = serde_json::to_string(&envelope)
        .map_err(|_| String::from("large map envelope serialization failed"))?;
    let frame = json!({
        "jsonrpc":"2.0", "id":3,
        "result":{"content":[{"type":"text","text":text}]}
    });
    let frame = serde_json::to_string(&frame)
        .map_err(|_| String::from("large MCP response serialization failed"))?;
    Ok((frame, snapshot_bytes.len()))
}

fn catalog_response() -> String {
    json!({
        "jsonrpc":"2.0", "id":2,
        "result":{"revision":CATALOG_REVISION,"tools":EXPECTED_TOOLS
            .iter().map(|name| json!({"name":name})).collect::<Vec<_>>()}
    })
    .to_string()
}

fn config_for_script(script: &Path, response: &str) -> RuntimeConfig {
    RuntimeConfig {
        seed_transport: None,
        gateway_address: String::from("127.0.0.1:15525"),
        gateway_token: response.to_owned(),
        mcp_binary: script.to_string_lossy().into_owned(),
        runtime_profile: String::from("runtime-v3-gameplay"),
        instance_id: String::from("instance-1"),
        caller_id: String::from("harness"),
        session_id: String::from("gateway-session-1"),
        mcp_session_id: String::from("mcp-session-1"),
        lease_id: String::from("lease-1"),
        lease_epoch: 1,
        run_id: String::from("run-1"),
        episode_id: String::from("episode-1"),
        trajectory_id: String::from("trajectory-1"),
        trace_id: String::from("trace-1"),
        artifact_id: String::from("artifact-1"),
        wait_for_combat_seconds: 0,
        settlement_timeout_seconds: 30,
        map_context_enabled: true,
        recovery_environment: Vec::new(),
    }
}

fn script_path() -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| String::from("clock moved backwards"))?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "sts2-runtime-map-wire-{}-{nanos}.sh",
        std::process::id()
    )))
}

#[test]
#[cfg(unix)]
fn map_profile_reads_a_complete_near_bound_response_from_mcp_stdout() -> Result<(), String> {
    let path = script_path()?;
    let response_path = path.with_extension("response");
    let bootstrap = json!({"jsonrpc":"2.0","id":1,"result":{}}).to_string();
    let catalog = catalog_response();
    let mut config = config_for_script(&path, "placeholder");
    let (response, snapshot_bytes) = map_response(&config)?;
    assert!(snapshot_bytes <= 256 * 1024);
    assert!(response.len() > 256 * 1024);
    assert!(response.len() <= MAX_RESPONSE_BYTES);
    config.gateway_address = response_path.to_string_lossy().into_owned();

    let script = format!(
        "#!/bin/sh\nread request || exit 1\nprintf '%s\\n' '{bootstrap}'\nread request || exit 1\nprintf '%s\\n' '{catalog}'\nread request || exit 1\ncat \"$STS2_GATEWAY_ADDR\" || exit 1\nprintf '\\n'\n"
    );
    fs::write(&path, script).map_err(|error| error.to_string())?;
    fs::write(&response_path, response).map_err(|error| error.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    let result = snapshot(&config, 1);
    let cleanup = fs::remove_file(&path);
    let response_cleanup = fs::remove_file(&response_path);
    let value = result?;
    cleanup.map_err(|error| error.to_string())?;
    response_cleanup.map_err(|error| error.to_string())?;
    assert_eq!(value["generation"], 1);
    assert_eq!(value["snapshot"]["state_id"], "state-1");
    assert_eq!(
        value["snapshot"]["edges"].as_array().map(Vec::len),
        Some(879)
    );
    Ok(())
}

#[test]
#[ignore = "requires STS2_REAL_MCP_BINARY and a real MCP executable"]
#[cfg(unix)]
fn map_profile_reads_the_real_mcp_binary_before_exo_serialization() -> Result<(), String> {
    let mcp_binary = std::env::var("STS2_REAL_MCP_BINARY")
        .map_err(|_| String::from("STS2_REAL_MCP_BINARY is required"))?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let address = listener.local_addr().map_err(|error| error.to_string())?;
    let mut config = config_for_script(Path::new(&mcp_binary), "placeholder");
    config.gateway_address = address.to_string();
    let (frame, snapshot_bytes) = map_response(&config)?;
    let frame: Value = serde_json::from_str(&frame).map_err(|error| error.to_string())?;
    let expected: Value = frame
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("near-bound map fixture omitted content"))
        .and_then(|text| serde_json::from_str(text).map_err(|error| error.to_string()))?;
    let body = frame
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|content| content.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("near-bound map fixture omitted content"))?
        .to_owned();
    assert!(snapshot_bytes <= 256 * 1024);
    assert!(body.len() <= 256 * 1024);
    assert!(body.len() > 256 * 1024 - 4096);
    eprintln!(
        "real MCP map fixture bytes: snapshot={} gateway_body={} mcp_frame={}",
        snapshot_bytes,
        body.len(),
        frame.to_string().len()
    );
    let server = std::thread::spawn(move || serve_real_map(listener, body));
    let value = snapshot(&config, 1);
    let server_result = server
        .join()
        .map_err(|_| String::from("real MCP HTTP fixture panicked"))?;
    server_result?;
    let value = value?;
    assert_eq!(
        value, expected,
        "MCP returned map envelope changed in transit"
    );
    assert_eq!(value["generation"], 1);
    assert_eq!(value["snapshot"]["state_id"], "state-1");
    assert_eq!(
        value["snapshot"]["edges"].as_array().map(Vec::len),
        Some(879)
    );
    Ok(())
}

#[cfg(unix)]
fn serve_real_map(listener: TcpListener, body: String) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(String::from("real MCP HTTP fixture accept timed out"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .map_err(|error| error.to_string())?;
    let mut request = Vec::new();
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let mut chunk = [0_u8; 4096];
        let count = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if count == 0 {
            return Err(String::from("real MCP HTTP request ended before headers"));
        }
        request.extend_from_slice(&chunk[..count]);
        if request.len() > 8192 {
            return Err(String::from("real MCP HTTP headers exceeded the bound"));
        }
    }
    let request = String::from_utf8(request).map_err(|error| error.to_string())?;
    if !request.starts_with("GET /v1/instances/instance-1/map-snapshot HTTP/1.1\r\n") {
        return Err(String::from("real MCP HTTP request route was unexpected"));
    }
    let bytes = body.as_bytes();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .map_err(|error| error.to_string())?;
    stream.write_all(bytes).map_err(|error| error.to_string())
}
