// SPDX-License-Identifier: MIT

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};
use sts2_harness::management::{
    CommandKind, CommandOutcome, CommandParameters, CommandRequest, CommandResponse, ErrorClass,
    ErrorResponse, MANAGEMENT_SCHEMA_VERSION, ManagementClient, ValidateRequest,
};
use sts2_harness::workflow::{WorkflowDefinition, validate_definition};

use super::{fixture::Result, http_exchange};

const HTTP_HEADER_LIMIT: usize = 8 * 1024;
const HTTP_BODY_LIMIT: usize = 16 * 1024;
const HTTP_IO_POLL: Duration = Duration::from_millis(100);

pub(crate) struct Request {
    pub(crate) path: String,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

pub(crate) const MANAGEMENT_PATH: &str = "/v1/workflow-definitions/validate";
pub(crate) const DEFINITION_ID: &str = "h391.served.one-turn";
pub(crate) const REQUEST_ID: &str = "h391-served-one-turn";

pub(crate) fn require(condition: bool, message: &'static str) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

pub(crate) fn served_definition() -> Result<Value> {
    let mut value: Value = serde_json::from_str(include_str!(
        "../../../../conformance/workflow-v1/valid-strict.json"
    ))?;
    value["workflow_id"] = json!(DEFINITION_ID);
    value["game_profile"] = json!("sts2-live-v1");
    value["policy_ref"] = json!("policy.served.v1");
    value["limits"] = json!({
        "max_steps":8,"max_subworkflow_depth":1,"max_provider_calls":1,
        "max_parallel_analyses":1,"max_output_tokens":4096
    });
    value["graphs"][0]["nodes"] = json!([
        {"id":"observe","kind":"observe","config":{"projection_ref":"fair-play.live.v1"}},
        {"id":"decide","kind":"decide","config":{"decision_profile_ref":"decision.live.v1","context_ref":"context.live.v1"}},
        {"id":"done","kind":"terminal","config":{"outcome":"completed"}},
        {"id":"blocked","kind":"terminal","config":{"outcome":"needs_operator"}}
    ]);
    value["graphs"][0]["edges"] = json!([
        {"from":"observe","to":"decide","on":"ok","priority":0},
        {"from":"decide","to":"done","on":"unavailable","priority":0},
        {"from":"observe","to":"blocked","on":"error","priority":0},
        {"from":"decide","to":"blocked","on":"error","priority":0},
        {"from":"decide","to":"done","on":"ok","priority":0}
    ]);
    let object = value
        .as_object_mut()
        .ok_or("workflow fixture is not an object")?;
    drop(object.remove("annotations"));
    drop(object.remove("fixture_constraints"));
    let definition: WorkflowDefinition = serde_json::from_value(value)?;
    validate_definition(&definition)?;
    Ok(serde_json::to_value(definition)?)
}

pub(crate) fn validate_refusal(status: u16, body: &[u8]) -> Result<()> {
    if body.len() > 8 * 1024 {
        return Err("management refusal exceeded the response bound".into());
    }
    let response: ErrorResponse = serde_json::from_slice(body)?;
    let class_matches_status = matches!(
        (status, response.error.class),
        (400, ErrorClass::InvalidInput)
            | (403, ErrorClass::Forbidden)
            | (
                409,
                ErrorClass::Capability | ErrorClass::Conflict | ErrorClass::Budget
            )
    );
    if !class_matches_status
        || response.schema_version != MANAGEMENT_SCHEMA_VERSION
        || response.error.code.is_empty()
        || response.error.code.len() > 64
        || !response.error.code.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
        || response.error.message.len() > 1024
    {
        return Err("management refusal had an invalid bounded classification".into());
    }
    Ok(())
}

pub(crate) fn read_request(
    stream: &mut TcpStream,
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<Request, &'static str> {
    read_request_with_body_limit(stream, deadline, cancel, HTTP_BODY_LIMIT)
}

pub(crate) fn read_request_with_body_limit(
    stream: &mut TcpStream,
    deadline: Instant,
    cancel: &AtomicBool,
    body_limit: usize,
) -> std::result::Result<Request, &'static str> {
    if body_limit > 131_072 {
        return Err("HTTP caller body limit exceeded the fixture maximum");
    }
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        read_exact_bounded(stream, &mut byte, deadline, cancel)?;
        header.push(byte[0]);
        if header.len() > HTTP_HEADER_LIMIT {
            return Err("HTTP header bound exceeded");
        }
    }
    let text = std::str::from_utf8(&header).map_err(|_| "HTTP headers were not UTF-8")?;
    let mut lines = text.split("\r\n");
    let line = lines.next().ok_or("HTTP request line missing")?;
    let mut parts = line.split_ascii_whitespace();
    if parts.next() != Some("POST")
        || parts.next().is_none()
        || parts.next() != Some("HTTP/1.1")
        || parts.next().is_some()
    {
        return Err("HTTP request line refused");
    }
    let path = line
        .split_ascii_whitespace()
        .nth(1)
        .ok_or("HTTP path missing")?
        .to_owned();
    let mut headers = Vec::new();
    let mut length = None;
    for row in lines.filter(|row| !row.is_empty()) {
        if headers.len() >= 64 {
            return Err("HTTP header count exceeded its bound");
        }
        let (name, value) = row.split_once(':').ok_or("HTTP header syntax refused")?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name == "transfer-encoding" || name == "expect" || name == "upgrade" {
            return Err("HTTP ambiguous request framing refused");
        }
        if name == "connection"
            && value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case("upgrade"))
        {
            return Err("HTTP protocol upgrade refused");
        }
        if headers.iter().any(|(existing, _)| existing == &name) {
            return Err("HTTP duplicate request header refused");
        }
        if name == "content-length" {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("HTTP content length refused");
            }
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "HTTP content length refused")?;
            if parsed.to_string() != value {
                return Err("HTTP content length was not canonical");
            }
            length = Some(parsed);
        }
        headers.push((name, value));
    }
    let length = length.ok_or("HTTP content length missing")?;
    if length > body_limit {
        return Err("HTTP request body bound exceeded");
    }
    let mut body = vec![0; length];
    read_exact_bounded(stream, &mut body, deadline, cancel)?;
    Ok(Request {
        path,
        headers,
        body,
    })
}

pub(crate) fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn read_exact_bounded(
    stream: &mut TcpStream,
    output: &mut [u8],
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<(), &'static str> {
    let mut offset = 0;
    while offset < output.len() {
        if cancel.load(Ordering::Acquire) {
            return Err("HTTP read cancelled");
        }
        if Instant::now() >= deadline {
            return Err("HTTP read deadline elapsed");
        }
        match stream.read(&mut output[offset..]) {
            Ok(0) => return Err("HTTP request ended early"),
            Ok(read) => offset += read,
            Err(error) if error.kind() == ErrorKind::WouldBlock => pause_io(deadline, cancel)?,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err("HTTP request read failed"),
        }
    }
    Ok(())
}

pub(crate) fn write_response(
    stream: &mut TcpStream,
    body: &[u8],
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<(), &'static str> {
    if body.len() > HTTP_HEADER_LIMIT {
        return Err("HTTP response bound exceeded");
    }
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    write_all_bounded(stream, header.as_bytes(), deadline, cancel)?;
    write_all_bounded(stream, body, deadline, cancel)
}

fn write_all_bounded(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
    cancel: &AtomicBool,
) -> std::result::Result<(), &'static str> {
    let mut offset = 0;
    while offset < bytes.len() {
        if cancel.load(Ordering::Acquire) {
            return Err("HTTP write cancelled");
        }
        if Instant::now() >= deadline {
            return Err("HTTP write deadline elapsed");
        }
        match stream.write(&bytes[offset..]) {
            Ok(0) => return Err("HTTP write made no progress"),
            Ok(written) => offset += written,
            Err(error) if error.kind() == ErrorKind::WouldBlock => pause_io(deadline, cancel)?,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err("HTTP response write failed"),
        }
    }
    Ok(())
}

fn pause_io(deadline: Instant, cancel: &AtomicBool) -> std::result::Result<(), &'static str> {
    if cancel.load(Ordering::Acquire) {
        return Err("HTTP operation cancelled");
    }
    if Instant::now() >= deadline {
        return Err("HTTP operation deadline elapsed");
    }
    thread::sleep(HTTP_IO_POLL.min(deadline.saturating_duration_since(Instant::now())));
    Ok(())
}

pub(crate) fn request_bytes<T: serde::Serialize>(request: &T) -> Result<Vec<u8>> {
    Ok(serde_json::to_vec(request)?)
}

pub(crate) fn decode<T: DeserializeOwned>(
    response: sts2_harness::management::ClientResponse,
) -> Result<T> {
    require(
        response.status == 200,
        "served management request was refused",
    )?;
    Ok(serde_json::from_slice(&response.body)?)
}

pub(crate) fn get_json(client: &ManagementClient, path: &str) -> Result<Value> {
    let response = client.request_json("GET", path, None)?;
    require(response.status == 200, "served management GET was refused")?;
    Ok(serde_json::from_slice(&response.body)?)
}

pub(crate) fn post<T: DeserializeOwned>(
    client: &ManagementClient,
    path: &str,
    request: &impl serde::Serialize,
) -> Result<T> {
    decode(client.request_json("POST", path, Some(&request_bytes(request)?))?)
}

pub(crate) fn refuse(response: sts2_harness::management::ClientResponse) -> Result<()> {
    validate_refusal(response.status, &response.body)
}

pub(crate) fn invalid_workflow_requests(
    client: &ManagementClient,
    address: std::net::SocketAddr,
    valid: &ValidateRequest,
) -> Result<()> {
    let base = String::from_utf8(request_bytes(valid)?)?;
    let needle = format!("\"workflow_id\":\"{DEFINITION_ID}\"");
    let duplicate = base.replacen(
        &needle,
        &format!("\"workflow_id\":\"{DEFINITION_ID}\",{needle}"),
        1,
    );
    require(
        duplicate != base,
        "invalid_workflow_requests.duplicate_key_fixture_unavailable",
    )?;
    let duplicate_refusal =
        (|| refuse(client.request_json("POST", MANAGEMENT_PATH, Some(duplicate.as_bytes()))?))();
    duplicate_refusal.map_err(|_| "invalid_workflow_requests.duplicate_key_request_or_refusal")?;

    let mut unknown = valid.clone();
    unknown.definition["unknown_nested_fixture_field"] = json!(true);
    let unknown_refusal = (|| {
        refuse(client.request_json("POST", MANAGEMENT_PATH, Some(&request_bytes(&unknown)?))?)
    })();
    unknown_refusal.map_err(|_| "invalid_workflow_requests.unknown_field_request_or_refusal")?;

    let mut deep = valid.clone();
    deep.capabilities = (0..40).fold(json!(null), |item, _| json!([item]));
    let deep_refusal =
        (|| refuse(client.request_json("POST", MANAGEMENT_PATH, Some(&request_bytes(&deep)?))?))();
    deep_refusal.map_err(|_| "invalid_workflow_requests.depth_request_or_refusal")?;

    let mut crowded = valid.clone();
    crowded.capabilities = json!(vec![Value::Null; 16_385]);
    let crowded_refusal = (|| {
        refuse(client.request_json("POST", MANAGEMENT_PATH, Some(&request_bytes(&crowded)?))?)
    })();
    crowded_refusal.map_err(|_| "invalid_workflow_requests.item_limit_request_or_refusal")?;

    let body = vec![b' '; 1_048_577];
    refuse_oversized_http(address, MANAGEMENT_PATH, "synthetic-test-token", &body)
        .map_err(|_| "invalid_workflow_requests.oversized_request_or_refusal".into())
}

pub(crate) fn failure_stage(result: &Result<()>, fallback: &'static str) -> &'static str {
    let message = result
        .as_ref()
        .err()
        .map_or_else(String::new, |error| error.to_string());
    const CASES: [&str; 6] = [
        "invalid_workflow_requests.duplicate_key_fixture_unavailable",
        "invalid_workflow_requests.duplicate_key_request_or_refusal",
        "invalid_workflow_requests.unknown_field_request_or_refusal",
        "invalid_workflow_requests.depth_request_or_refusal",
        "invalid_workflow_requests.item_limit_request_or_refusal",
        "invalid_workflow_requests.oversized_request_or_refusal",
    ];
    CASES
        .iter()
        .copied()
        .find(|case| message.as_str() == *case)
        .unwrap_or(fallback)
}

fn refuse_oversized_http(address: SocketAddr, path: &str, token: &str, body: &[u8]) -> Result<()> {
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let response =
        http_exchange::exchange(address, head.as_bytes(), body, 1_048_577, 8 * 1024, true)?;
    validate_refusal(response.status, &response.body)
}

pub(crate) fn submit_step(
    client: &ManagementClient,
    run_id: &str,
    revision: u64,
    id: &str,
) -> Result<CommandResponse> {
    post(
        client,
        &format!("/v1/workflow-runs/{run_id}/commands"),
        &CommandRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            command_id: id.to_owned(),
            run_id: run_id.to_owned(),
            expected_revision: revision,
            actor_scope: "profile:synthetic".to_owned(),
            kind: CommandKind::Step,
            parameters: CommandParameters::default(),
        },
    )
}

pub(crate) fn cancel_before_decision(
    client: &ManagementClient,
    run_id: &str,
    revision: u64,
) -> Result<()> {
    let response: CommandResponse = post(
        client,
        &format!("/v1/workflow-runs/{run_id}/commands"),
        &CommandRequest {
            schema_version: MANAGEMENT_SCHEMA_VERSION.to_owned(),
            command_id: "cleanup-cancel".to_owned(),
            run_id: run_id.to_owned(),
            expected_revision: revision,
            actor_scope: "profile:synthetic".to_owned(),
            kind: CommandKind::Cancel,
            parameters: CommandParameters::default(),
        },
    )?;
    require(
        response.outcome == CommandOutcome::Applied,
        "best-effort run cancellation was not confirmed",
    )?;
    Ok(())
}
