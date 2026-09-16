// SPDX-License-Identifier: MIT

use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

fn read_request(stream: &mut TcpStream) -> Result<(String, Value), Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.len() > 8 * 1024 {
            return Err("runtime request headers exceeded the fixture bound".into());
        }
    }
    let header = String::from_utf8(header)?;
    let length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim().parse::<usize>().ok()?)
        })
        .ok_or("runtime request omitted content length")?;
    if length > 16 * 1024 {
        return Err("runtime request body exceeded the fixture bound".into());
    }
    let mut body = vec![0_u8; length];
    stream.read_exact(&mut body)?;
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)?
    };
    Ok((header, value))
}

fn reply(stream: &mut TcpStream, value: &Value) -> Result<(), Box<dyn std::error::Error>> {
    let body = serde_json::to_vec(value)?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    Ok(())
}

fn serve_gateway(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    paths: Arc<Mutex<Vec<String>>>,
) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let (headers, body) =
                    read_request(&mut stream).map_err(|error| error.to_string())?;
                let path = headers
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .ok_or_else(|| String::from("runtime request omitted its path"))?
                    .to_owned();
                paths
                    .lock()
                    .map_err(|_| String::from("Gateway path mutex poisoned"))?
                    .push(path.clone());
                if path == "/v1/recovery/continuation/owner/adopt" {
                    let lower_headers = headers.to_ascii_lowercase();
                    if !lower_headers.contains("authorization: bearer recovery-token\r\n")
                        || !lower_headers
                            .contains("x-sts2-recovery-capability: continuation_owner_adopt\r\n")
                    {
                        return Err(String::from(
                            "owner adoption omitted its configured authority headers",
                        ));
                    }
                    let correlation = body["correlation_id"]
                        .as_str()
                        .ok_or_else(|| String::from("adoption request omitted correlation"))?;
                    let selected_owner = &body["payload"]["expected_owner"];
                    let operation_id = body["payload"]["operation_id"]
                        .as_str()
                        .ok_or_else(|| String::from("adoption request omitted claim operation"))?;
                    reply(
                        &mut stream,
                        &json!({
                            "contract":"sts2-continuation-owner-adopt-v1",
                            "schema_digest":OWNER_ADOPT_SCHEMA,
                            "message_id":"00000000-0000-4000-8000-000000000020",
                            "correlation_id":correlation,
                            "actor":{"principal_id":CALLER_ID,"role":"gateway"},
                            "auth":{
                                "principal_id":CALLER_ID,
                                "capability":"continuation_owner_adopt",
                                "proof":null
                            },
                            "kind":"owner_adopt_response",
                            "payload":{
                                "result":"ADOPTED",
                                "claim":{
                                    "operation_id":operation_id,
                                    "request_digest":"a".repeat(64),
                                    "owner":selected_owner,
                                    "claimed_at_millis":selected_owner["lease_expires_at_millis"]
                                        .as_u64().unwrap_or_default().saturating_sub(60_000)
                                },
                                "owner":selected_owner,
                                "recovery_authority":recovery_authority(selected_owner)
                            }
                        }),
                    )
                    .map_err(|error| error.to_string())?;
                } else if path == format!("/v1/instances/{INSTANCE_ID}/release") {
                    reply(&mut stream, &json!({"status":"released"}))
                        .map_err(|error| error.to_string())?;
                } else {
                    return Err(format!("runtime attempted forbidden Gateway route {path}"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn mcp_script(log_path: &Path, mismatch: bool) -> Result<String, Box<dyn std::error::Error>> {
    let log_path = serde_json::to_string(&log_path.to_string_lossy().as_ref())?;
    let state_id = if mismatch {
        "state:foreign"
    } else {
        "state:selected"
    };
    let template = json!({
        "protocol_version":"runtime-v3-gameplay",
        "schema_digest":"8e99cea36b7ede97532348fd8efe302ca79260895265a7bf14ddf7e006d8ff63",
        "provenance":{
            "artifact":"sts2-protocol/runtime-v3-gameplay",
            "source":"schemas/runtime-v3-gameplay.schema.json",
            "generator":"hand-authored"
        },
        "correlation_id":"1",
        "instance_id":INSTANCE_ID,
        "session_id":SESSION_ID,
        "lease_id":LEASE_ID,
        "lease_epoch":8,
        "generation":3,
        "kind":"state_response",
        "state_id":state_id,
        "operation_id":null,
        "observation":{
            "state_id":state_id,
            "generation":3,
            "visible_seed":"seed:selected",
            "player":{
                "hp":50,"max_hp":50,"energy":3,"gold":10,
                "hand":[],"deck":[],"discard":[],"exhaust":[]
            },
            "state":{"state":"combat","turn_index":4,"enemies":[]}
        },
        "legal_actions":[{"action_id":"combat.end-turn","action":{"kind":"end_turn"}}],
        "action":null,
        "status":null,
        "transition":null,
        "error_code":null,
        "wait_for_millis":null,
        "wait_outcome":null,
        "recovery":null
    });
    let payload = serde_json::to_string(&template)?;
    Ok(format!(
        r#"#!/usr/bin/python3
import json, sys
LOG = {log_path}
TEMPLATE = json.loads({payload:?})
TOOLS = ["sts2.observe", "sts2.legal_actions", "sts2.dispatch_action",
         "sts2.wait_for_transition", "sts2.reobserve", "sts2.recover"]
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    with open(LOG, "a", encoding="utf-8") as output:
        output.write(method + " " + str(request.get("params", {{}}).get("name", "")) + "\n")
    if method == "initialize":
        result = {{}}
    elif method == "tools/list":
        result = {{"revision":"runtime-v3-gameplay-mcp",
                  "tools":[{{"name":name}} for name in TOOLS]}}
    elif method == "tools/call" and request["params"]["name"] in (
            "sts2.observe", "sts2.reobserve", "sts2.legal_actions"):
        payload = dict(TEMPLATE)
        payload["correlation_id"] = str(request["id"])
        payload["lease_id"] = __import__("os").environ["STS2_LEASE_ID"]
        payload["lease_epoch"] = int(__import__("os").environ["STS2_LEASE_EPOCH"])
        if request["params"]["name"] == "sts2.reobserve":
            payload["kind"] = "reobserve_response"
        elif request["params"]["name"] == "sts2.legal_actions":
            payload["kind"] = "legal_actions_response"
            payload["observation"] = None
        result = {{"content":[{{"type":"text",
                  "text":json.dumps(payload, separators=(",", ":"), sort_keys=True)}}]}}
    else:
        raise RuntimeError("unexpected MCP method or gameplay tool")
    print(json.dumps({{"jsonrpc":"2.0", "id":request["id"], "result":result}},
                     separators=(",", ":")), flush=True)
"#
    ))
}

fn write_executable(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    fs::write(path, bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

pub(super) fn run_case(
    mismatch_observation: bool,
    mismatch_seed: bool,
) -> Result<(bool, Vec<String>, String, String), Box<dyn std::error::Error>> {
    let temp = TempDir::new()?;
    seed_branch_and_boundary(
        temp.path(),
        if mismatch_seed {
            "seed:branch-mismatch"
        } else {
            "seed:selected"
        },
    )?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let gateway_address = listener.local_addr()?.to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let paths = Arc::new(Mutex::new(Vec::new()));
    let gateway_stop = Arc::clone(&stop);
    let gateway_paths = Arc::clone(&paths);
    let gateway_thread =
        thread::spawn(move || serve_gateway(listener, gateway_stop, gateway_paths));

    let mcp_path = temp.path().join("mcp-peer.py");
    let mcp_log = temp.path().join("mcp.log");
    write_executable(
        &mcp_path,
        mcp_script(&mcp_log, mismatch_observation)?.as_bytes(),
    )?;
    let provider_path = temp.path().join("inert-provider.sh");
    let provider_hit = temp.path().join("provider-hit");
    let provider_input = temp.path().join("provider-input.json");
    write_executable(
        &provider_path,
        format!(
            "#!/bin/sh\ncat > '{}'\nprintf 'called\\n' > '{}'\nexit 7\n",
            provider_input.display(),
            provider_hit.display()
        )
        .as_bytes(),
    )?;

    let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
    command
        .arg("--resume")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("STS2_RESUME", "true")
        .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
        .env("STS2_GATEWAY_ADDR", gateway_address)
        .env("STS2_GATEWAY_TOKEN", "runtime-bearer-not-used-for-adopt")
        .env("STS2_MCP_BINARY", &mcp_path)
        .env("STS2_INSTANCE_ID", INSTANCE_ID)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_LEASE_ID", "configured-stale-lease")
        .env("STS2_LEASE_EPOCH", "1")
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_RUN_ID", "run:parent")
        .env("STS2_EPISODE_ID", "episode:parent")
        .env("STS2_ATTEMPT_ID", ATTEMPT_ID)
        .env("STS2_TRAJECTORY_ID", "trajectory:parent")
        .env("STS2_TRACE_ID", "trace:parent")
        .env("STS2_ARTIFACT_ID", "artifact:parent")
        .env("STS2_EXPERIMENT_ID", EXPERIMENT_ID)
        .env("STS2_BRANCH_ID", BRANCH_ID)
        .env(
            "STS2_BRANCH_STORE_PATH",
            temp.path().join("branches.sqlite3"),
        )
        .env(
            "STS2_EXACT_ARTIFACT_STORE_PATH",
            temp.path().join("artifacts"),
        )
        .env(
            "STS2_EXECUTION_STORE_PATH",
            temp.path().join("execution.sqlite3"),
        )
        .env("STS2_SEED", "seed:selected")
        .env("STS2_BUILD_DIGEST", "build:selected")
        .env("STS2_STATE_DIGEST", "state:selected")
        .env("STS2_APPROVED_WORKER_FINGERPRINT", "true")
        .env("STS2_WORKER_SEED", "seed:selected")
        .env("STS2_WORKER_RELEASE_DIGEST", "build:selected")
        .env("STS2_WORKER_STATE_DIGEST", "state:selected")
        .env("STS2_WORKER_CONFIG_DIGEST", "config:selected")
        .env("STS2_WORKER_PROVIDER_DIGEST", "provider:selected")
        .env("STS2_RECOVERY_TOKEN", "recovery-token")
        .env("STS2_RECOVERY_PRINCIPAL_ID", CALLER_ID)
        .env("STS2_EXO_REVISION", EXO_SOURCE_REVISION)
        .env("STS2_EXO_ADMISSION", "legacy")
        .env("STS2_EXO_BRIDGE_BINARY", &provider_path)
        .env("STS2_EXO_BRIDGE_ARGS_JSON", "[]")
        .env("STS2_EXO_INHERITED_ENV_JSON", "[]")
        .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
        .env("STS2_EXO_TIMEOUT_MILLIS", "2000")
        .env(
            "STS2_OBJECTIVE",
            "resume selected branch after a verified boundary",
        )
        .env("STS2_MAX_STEPS", "2");

    let output = super::process_support::run_child_with_timeout(command, Duration::from_secs(15))
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    stop.store(true, Ordering::Release);
    let gateway_result = gateway_thread
        .join()
        .map_err(|_| String::from("local Gateway peer panicked"))?;
    gateway_result.map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    let gateway_paths = paths
        .lock()
        .map_err(|_| "Gateway path mutex poisoned")?
        .clone();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Ok((
        provider_hit.exists(),
        gateway_paths,
        format!("{}{}", stdout, stderr),
        fs::read_to_string(mcp_log).unwrap_or_default(),
    ))
}
