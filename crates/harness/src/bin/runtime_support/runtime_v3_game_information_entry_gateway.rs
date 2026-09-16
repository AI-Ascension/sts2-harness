// SPDX-License-Identifier: MIT

use super::{AGENT, EPISODE, PROJECT, RUN, STATE_GENERATION, STATE_ID};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

pub(super) fn state_response(
    kind: &str,
    correlation: &str,
    stage: &str,
    generation: u64,
    legal_actions: Value,
) -> Value {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    ))
    .expect("runtime-v3 golden state response");
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(correlation);
    value["state_id"] = json!(STATE_ID);
    value["generation"] = json!(generation);
    value["observation"]["state_id"] = json!(STATE_ID);
    value["observation"]["generation"] = json!(generation);
    value["observation"]["state"] = if stage == "victory" {
        json!({"state":"victory"})
    } else {
        json!({"state":stage,"turn_index":generation + 1,"enemies":[]})
    };
    value["legal_actions"] = legal_actions;
    value
}

pub(super) fn dispatch_response() -> Value {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../../protocol-artifact/runtime-v3-gameplay/golden/dispatch-action-settled.json"
    ))
    .expect("settled action golden");
    value["state_id"] = json!(STATE_ID);
    value["generation"] = json!(STATE_GENERATION + 1);
    value["observation"]["state_id"] = json!(STATE_ID);
    value["observation"]["generation"] = json!(STATE_GENERATION + 1);
    value["observation"]["state"] = json!({"state":"victory"});
    value["transition"]["from_generation"] = json!(STATE_GENERATION);
    value["transition"]["to_generation"] = json!(STATE_GENERATION + 1);
    value["transition"]["state_id"] = json!(STATE_ID);
    value["transition"]["effect_kind"] = json!("end_turn.settled");
    value["generation"] = json!(STATE_GENERATION + 1);
    value
}

pub(super) fn serve_gateway(listener: TcpListener) -> Result<Vec<String>, String> {
    let deadline = Instant::now() + Duration::from_secs(25);
    let mut requests = Vec::new();
    let mut observation_count = 0_u64;
    loop {
        let (mut stream, _) = accept_until(&listener, deadline)?;
        let (headers, body) = read_request(&mut stream)
            .map_err(|error| format!("{error}; requests so far: {requests:?}"))?;
        let request_line = headers.lines().next().ok_or("missing request line")?;
        let path = request_line
            .split_whitespace()
            .nth(1)
            .ok_or("invalid request line")?;
        requests.push(format!(
            "{} {}",
            request_line.split_whitespace().next().unwrap_or_default(),
            path
        ));
        let response = if path == "/v1/sessions/allocate" {
            json!({
                "status":"allocated","instance_id":"instance-1","caller_id":"harness",
                "session_id":"session-1","lease_id":"lease-1","lease_epoch":1
            })
        } else if path == "/v1/instances/instance-1/game-information/lookup-binding" {
            let operation = body["operation"]
                .as_str()
                .ok_or("binding operation missing")?;
            let correlation = header(&headers, "x-sts2-correlation-id")
                .ok_or("binding correlation header missing")?;
            let filename = if operation == "discovery" {
                "discovery-response.json"
            } else {
                observation_count += 1;
                "observation-response.json"
            };
            let mut binding: Value = serde_json::from_str(match filename {
                "discovery-response.json" => include_str!(
                    "../../../../../protocol-artifact/game-information-lookup-binding-v1/golden/discovery-response.json"
                ),
                _ => include_str!(
                    "../../../../../protocol-artifact/game-information-lookup-binding-v1/golden/observation-response.json"
                ),
            })
            .map_err(|error| error.to_string())?;
            binding["correlation_id"] = json!(correlation);
            binding["binding"]["scope"] = json!({
                "project_id":PROJECT,"run_id":RUN,"episode_id":EPISODE,"agent_id":AGENT
            });
            binding["binding"]["instance_id"] = json!("instance-1");
            binding["binding"]["authority_epoch"] = json!(7);
            binding["binding"]["content_manifest_id"] = json!("content-1");
            binding["binding"]["game_profile"] = json!("sts2-native-v1");
            binding["binding"]["locale"] = json!("en-US");
            let id_input = json!({
                "agent_id":AGENT,"authority_epoch":7,"content_manifest_id":"content-1",
                "episode_id":EPISODE,"game_profile":"sts2-native-v1","locale":"en-US",
                "project_id":PROJECT,"run_id":RUN
            });
            let id_bytes = serde_json::to_vec(&id_input).map_err(|error| error.to_string())?;
            let binding_id = sts2_harness::sha256_hex(id_bytes);
            binding["binding"]["binding_id"] = json!(binding_id);
            if filename != "discovery-response.json" {
                binding["observation"]["binding_id"] = json!(binding_id);
                binding["observation"]["observation_id"] =
                    json!(format!("entry-observation-{observation_count}"));
                binding["observation"]["snapshot_id"] = json!("entry-snapshot-41");
                binding["observation"]["state_generation"] = json!(STATE_GENERATION);
            }
            binding
        } else if path == "/v1/instances/instance-1/release" {
            let response = json!({"status":"released"});
            write_http(&mut stream, &response)?;
            break;
        } else {
            return Err(format!("unexpected gateway route {path}"));
        };
        write_http(&mut stream, &response)?;
    }
    Ok(requests)
}

fn accept_until(
    listener: &TcpListener,
    deadline: Instant,
) -> Result<(TcpStream, std::net::SocketAddr), String> {
    loop {
        match listener.accept() {
            Ok(connection) => return Ok(connection),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(String::from("runtime did not release its gateway lease"));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Result<(String, Value), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|error| error.to_string())?;
        bytes.push(byte[0]);
        if bytes.len() > 16 * 1024 {
            return Err(String::from(
                "synthetic gateway request headers exceeded bound",
            ));
        }
    }
    let headers = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then_some(value.trim())
        })
        .ok_or("gateway request omitted content length")?
        .parse::<usize>()
        .map_err(|error| error.to_string())?;
    if length > 64 * 1024 {
        return Err(String::from("synthetic gateway body exceeded bound"));
    }
    let mut body = vec![0; length];
    stream
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).map_err(|error| error.to_string())?
    };
    Ok((headers, body))
}

fn write_http(stream: &mut TcpStream, value: &Value) -> Result<(), String> {
    let body = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .and_then(|()| stream.write_all(&body))
    .map_err(|error| error.to_string())
}

fn header<'a>(headers: &'a str, name: &str) -> Option<&'a str> {
    headers.lines().find_map(|line| {
        let (header, value) = line.split_once(':')?;
        header.eq_ignore_ascii_case(name).then_some(value.trim())
    })
}
