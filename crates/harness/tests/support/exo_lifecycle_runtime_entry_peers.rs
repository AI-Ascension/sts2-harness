// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use super::{CALLER_ID, INSTANCE_ID, LEASE_ID, SESSION_ID};

pub(super) fn mcp_script(action_log: &Path) -> Result<String, String> {
    let action_log = serde_json::to_string(&action_log.to_string_lossy().as_ref())
        .map_err(|error| error.to_string())?;
    Ok(format!(
        r#"#!/usr/bin/python3
import json, os, sys
ACTION_LOG = {action_log}
PEER_LOG = ACTION_LOG.rsplit(".", 1)[0] + ".peer.log"
TOOLS = ["sts2.observe", "sts2.legal_actions", "sts2.dispatch_action",
         "sts2.wait_for_transition", "sts2.reobserve", "sts2.recover"]
def envelope(kind, correlation, generation, stage, actions, state_id="combat-1"):
    observation = None
    if kind in ("state_response", "dispatch_action_response", "wait_response"):
        state = {{"state": stage}}
        if stage != "victory":
            state = {{"state": stage, "turn_index": generation + 1, "enemies": []}}
        observation = {{
            "state_id": state_id, "generation": generation, "visible_seed": "entry-seed",
            "player": {{"hp": 50, "max_hp": 50, "energy": 3, "gold": 10,
                       "hand": [], "deck": [], "discard": [], "exhaust": []}},
            "state": state
        }}
    return {{
        "protocol_version": "runtime-v3-gameplay",
        "schema_digest": "daa216902d3211b9537924105b27e7718dd93dec82969a3c550131a27147c06b",
        "provenance": {{"artifact":"sts2-protocol/runtime-v3-gameplay",
                       "source":"schemas/runtime-v3-gameplay.schema.json",
                       "generator":"hand-authored"}},
        "correlation_id": str(correlation),
        "instance_id": os.environ["STS2_INSTANCE_ID"],
        "session_id": os.environ["STS2_SESSION_ID"],
        "lease_id": os.environ["STS2_LEASE_ID"],
        "lease_epoch": int(os.environ["STS2_LEASE_EPOCH"]),
        "generation": generation, "kind": kind, "state_id": state_id,
        "operation_id": None, "observation": observation, "legal_actions": actions,
        "action": None, "status": None, "transition": None, "error_code": None,
        "wait_for_millis": None, "wait_outcome": None, "recovery": None
    }}
def send(request, result):
    print(json.dumps({{"jsonrpc":"2.0", "id":request["id"], "result":result}},
                     separators=(",", ":")), flush=True)
def send_tool(request, value):
    text = json.dumps(value, separators=(",", ":"))
    send(request, {{"content":[{{"type":"text", "text":text}}]}})
for line in sys.stdin:
    request = json.loads(line)
    method = request["method"]
    with open(PEER_LOG, "a", encoding="utf-8") as output:
        output.write(method + " " + str(request.get("params", {{}}).get("name", "")) + "\n")
    if method == "initialize":
        send(request, {{}})
    elif method == "tools/list":
        send(request, {{"revision":"runtime-v3-gameplay-mcp",
                       "tools":[{{"name":name}} for name in TOOLS]}})
    elif method == "tools/call":
        name = request["params"]["name"]
        args = request["params"]["arguments"]
        if name == "sts2.observe":
            send_tool(request, envelope("state_response", request["id"], 0, "combat",
                                        [{{"action_id":"combat.end-turn",
                                          "action":{{"kind":"end_turn"}}}}]))
        elif name == "sts2.legal_actions":
            send_tool(request, envelope("legal_actions_response", request["id"], 0, "combat",
                                        [{{"action_id":"combat.end-turn",
                                          "action":{{"kind":"end_turn"}}}}]))
        elif name == "sts2.dispatch_action":
            with open(ACTION_LOG, "w", encoding="utf-8") as output:
                json.dump(args["action"], output, separators=(",", ":"))
            receipt = envelope("dispatch_action_response", request["id"], 1, "victory", [])
            receipt["operation_id"] = args["operation_id"]
            receipt["status"] = "settled"
            receipt["transition"] = {{
                "from_generation":args["generation"], "to_generation":1,
                "state_id":args["state_id"], "effect_kind":"end_turn"
            }}
            send_tool(request, receipt)
        elif name == "sts2.wait_for_transition":
            sample = envelope("wait_response", request["id"], 1, "victory", [])
            sample["operation_id"] = args["operation_id"]
            sample["status"] = "settled"
            sample["wait_outcome"] = "successor"
            sample["transition"] = {{
                "from_generation":args["generation"] - 1, "to_generation":1,
                "state_id":"combat-1", "effect_kind":"end_turn"
            }}
            send_tool(request, sample)
        else:
            raise RuntimeError("unexpected MCP tool " + name)
"#
    ))
}

pub(super) fn serve_gateway(listener: TcpListener) -> Result<(), String> {
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    for (index, path) in [
        "/v1/sessions/allocate",
        "/v1/instances/", /* release path checked below */
    ]
    .into_iter()
    .enumerate()
    {
        let timeout = if index == 0 { 15 } else { 10 };
        let mut stream = accept(&listener, Duration::from_secs(timeout))
            .map_err(|error| format!("offline gateway request {} failed: {error}", index + 1))?;
        let (headers, _body) = read_http_request(&mut stream)?;
        if path.ends_with("allocate") {
            if !headers.starts_with("POST /v1/sessions/allocate ") {
                return Err(String::from("runtime used an unexpected allocation route"));
            }
            respond(
                &mut stream,
                &json!({
                    "status":"allocated","instance_id":INSTANCE_ID,"caller_id":CALLER_ID,
                    "session_id":SESSION_ID,"lease_id":LEASE_ID,"lease_epoch":1
                }),
            )?;
        } else {
            if !headers.starts_with("POST /v1/instances/entry-instance/release ") {
                return Err(String::from("runtime used an unexpected release route"));
            }
            respond(&mut stream, &json!({"status":"released"}))?;
        }
    }
    Ok(())
}

fn accept(listener: &TcpListener, timeout: Duration) -> Result<TcpStream, String> {
    let deadline = Instant::now() + timeout;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(format!("offline gateway accept failed: {error}")),
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> Result<(String, Vec<u8>), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let mut header = Vec::new();
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|error| format!("offline gateway header read failed: {error}"))?;
        header.push(byte[0]);
        if header.len() > 8 * 1024 {
            return Err(String::from("offline gateway header exceeded its bound"));
        }
    }
    let text = String::from_utf8(header).map_err(|error| error.to_string())?;
    let length = text
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim().parse::<usize>().ok()?)
        })
        .ok_or_else(|| String::from("runtime request omitted content length"))?;
    if length > 16 * 1024 {
        return Err(String::from("offline gateway body exceeded its bound"));
    }
    let mut body = vec![0_u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    Ok((text, body))
}

fn respond(stream: &mut TcpStream, value: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .and_then(|()| stream.write_all(&body))
    .map_err(|error| error.to_string())
}
