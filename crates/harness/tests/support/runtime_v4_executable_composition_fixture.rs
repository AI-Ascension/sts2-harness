// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

pub(crate) const INSTANCE_ID: &str = "instance-1";
pub(crate) const CALLER_ID: &str = "harness";
pub(crate) const SESSION_ID: &str = "gateway-session-1";
pub(crate) const MCP_SESSION_ID: &str = "mcp-session-1";
pub(crate) const LEASE_ID: &str = "lease-1";
pub(crate) const LEASE_EPOCH: u64 = 1;
pub(crate) const ACTION_ID: &str = "potion:7:potion:fire:enemy:1";
pub(crate) const REVIEWED_EXO_REVISION: &str = "7801005e6a1ab77008a05dbba80e0a2a7a56e35d";

#[derive(Clone, Copy)]
pub(crate) enum FixtureMode {
    Success,
    ForeignExpertState,
}

#[derive(Clone, Debug)]
pub(crate) struct DownstreamRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct DownstreamResponse {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) struct DownstreamLedger {
    pub(crate) requests: Vec<DownstreamRequest>,
    pub(crate) responses: Vec<DownstreamResponse>,
    pub(crate) errors: Vec<String>,
}

pub(crate) struct ModServer {
    pub(crate) address: SocketAddr,
    stop: Arc<AtomicBool>,
    ledger: Arc<Mutex<DownstreamLedger>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ModServer {
    pub(crate) fn new(mode: FixtureMode) -> Result<Self, Box<dyn std::error::Error>> {
        Self::bind("127.0.0.1:0", mode)
    }

    /// Bind the synthetic downstream to an explicit address so an operator can
    /// run it as a long-lived service for a soak campaign.
    pub(crate) fn bind(
        address: &str,
        mode: FixtureMode,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let ledger = Arc::new(Mutex::new(DownstreamLedger {
            requests: Vec::new(),
            responses: Vec::new(),
            errors: Vec::new(),
        }));
        let worker_ledger = Arc::clone(&ledger);
        let worker = thread::spawn(move || {
            while !worker_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => match read_request(&mut stream) {
                        Ok(request) => {
                            let response = fixture_response(&request, mode);
                            if let Ok(mut ledger) = worker_ledger.lock() {
                                ledger.requests.push(request);
                            }
                            match response {
                                Ok((status, body)) => {
                                    if let Ok(mut ledger) = worker_ledger.lock() {
                                        ledger.responses.push(DownstreamResponse {
                                            status,
                                            body: body.clone(),
                                        });
                                    }
                                    if let Err(error) = write_response(&mut stream, status, &body)
                                        && let Ok(mut ledger) = worker_ledger.lock()
                                    {
                                        ledger.errors.push(error.to_string());
                                    }
                                }
                                Err(error) => {
                                    if let Ok(mut ledger) = worker_ledger.lock() {
                                        ledger.errors.push(error);
                                    }
                                    let _ = write_response(
                                        &mut stream,
                                        500,
                                        &json!({"error_code":"fixture_invalid_request"}),
                                    );
                                }
                            }
                        }
                        Err(error) => {
                            if let Ok(mut ledger) = worker_ledger.lock() {
                                ledger.errors.push(error.to_string());
                            }
                        }
                    },
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => {
                        if let Ok(mut ledger) = worker_ledger.lock() {
                            ledger.errors.push(error.to_string());
                        }
                        break;
                    }
                }
            }
        });
        Ok(Self {
            address,
            stop,
            ledger,
            worker: Some(worker),
        })
    }

    pub(crate) fn finish(mut self) -> DownstreamLedger {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.ledger.lock().map_or_else(
            |_| DownstreamLedger {
                requests: Vec::new(),
                responses: Vec::new(),
                errors: vec![String::from("downstream ledger lock failed")],
            },
            |ledger| DownstreamLedger {
                requests: ledger.requests.clone(),
                responses: ledger.responses.clone(),
                errors: ledger.errors.clone(),
            },
        )
    }
}

impl Drop for ModServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn fixture_response(
    request: &DownstreamRequest,
    mode: FixtureMode,
) -> Result<(u16, Value), String> {
    if request.headers.get("authorization").map(String::as_str) != Some("Bearer mod-token") {
        return Err(String::from("downstream mod authorization is missing"));
    }
    match request.path.as_str() {
        "/api/v3/runtime/state" => Ok((200, v3_response("state_response", &request.headers))),
        "/api/v3/runtime/legal-actions" => {
            Ok((200, v3_response("legal_actions_response", &request.headers)))
        }
        "/api/v4/runtime/expert-state" => Ok((
            200,
            expert_observation(
                match mode {
                    FixtureMode::Success => "live:7",
                    FixtureMode::ForeignExpertState => "foreign-state",
                },
                7,
                false,
            ),
        )),
        "/api/v4/runtime/expert-action" => unknown_action(&request.body),
        path if path.starts_with("/api/v4/runtime/expert-actions/") => {
            settled_action(path, &request.headers)
        }
        _ => Err(format!("unexpected downstream path: {}", request.path)),
    }
}

fn unknown_action(request: &Value) -> Result<(u16, Value), String> {
    if request["action"]["action_id"] != ACTION_ID {
        return Err(String::from(
            "expert action was not the host-generated potion action",
        ));
    }
    let mut response = golden_action()?;
    for (field, value) in [
        ("correlation_id", request["correlation_id"].clone()),
        ("instance_id", json!(INSTANCE_ID)),
        ("session_id", json!(SESSION_ID)),
        ("lease_id", json!(LEASE_ID)),
        ("lease_epoch", json!(LEASE_EPOCH)),
        ("generation", json!(7)),
        ("state_id", json!("live:7")),
        ("operation_id", request["operation_id"].clone()),
        ("kind", json!("action_response")),
        ("action", request["action"].clone()),
        ("status", json!("unknown")),
        ("observation", Value::Null),
        ("transition", Value::Null),
        ("error_code", json!("transport_timeout")),
    ] {
        response[field] = value;
    }
    Ok((503, response))
}

fn settled_action(path: &str, headers: &BTreeMap<String, String>) -> Result<(u16, Value), String> {
    let operation_id = path
        .strip_prefix("/api/v4/runtime/expert-actions/")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| String::from("reconcile operation identity is missing"))?;
    let mut response = golden_action()?;
    for (field, value) in [
        (
            "correlation_id",
            json!(
                headers
                    .get("x-sts2-correlation-id")
                    .cloned()
                    .unwrap_or_default()
            ),
        ),
        ("instance_id", json!(INSTANCE_ID)),
        ("session_id", json!(SESSION_ID)),
        ("lease_id", json!(LEASE_ID)),
        ("lease_epoch", json!(LEASE_EPOCH)),
        ("generation", json!(8)),
        ("state_id", json!("live:8")),
        ("operation_id", json!(operation_id)),
        ("kind", json!("action_response")),
        ("action", action_reference()),
        ("status", json!("settled")),
        ("observation", expert_observation("live:8", 8, true)),
        (
            "transition",
            json!({"kind":"potion_use_settled","before_generation":7,
                   "after_generation":8,"potion_id":"potion:fire","removed":true}),
        ),
        ("error_code", Value::Null),
    ] {
        response[field] = value;
    }
    Ok((200, response))
}

fn action_reference() -> Value {
    json!({"action_id":ACTION_ID,
           "action":{"kind":"use_potion","potion_id":"potion:fire","target_id":"enemy:1"}})
}

fn golden_action() -> Result<Value, String> {
    serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert-action/golden/action-settled.json"
    )))
    .map_err(|error| error.to_string())
}

fn v3_response(kind: &str, headers: &BTreeMap<String, String>) -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v3-gameplay/golden/state-response.json"
    )))
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    value["kind"] = json!(kind);
    value["correlation_id"] = json!(
        headers
            .get("x-sts2-correlation-id")
            .cloned()
            .unwrap_or_default()
    );
    value["instance_id"] = json!(INSTANCE_ID);
    value["session_id"] = json!(SESSION_ID);
    value["lease_id"] = json!(LEASE_ID);
    value["lease_epoch"] = json!(LEASE_EPOCH);
    value["generation"] = json!(7);
    value["state_id"] = json!("live:7");
    value["observation"]["state_id"] = json!("live:7");
    value["observation"]["generation"] = json!(7);
    value["observation"]["state"] = json!({"state":"combat","turn_index":8,"enemies":[]});
    value["legal_actions"] = json!([{"action_id":"end:7","action":{"kind":"end_turn"}}]);
    if kind == "legal_actions_response" {
        value["observation"] = Value::Null;
    }
    value
}

fn expert_observation(state_id: &str, generation: u64, terminal: bool) -> Value {
    let mut value: Value = serde_json::from_slice(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../protocol-artifact/runtime-v4-expert/golden/observation.json"
    )))
    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
    value["state_id"] = json!(state_id);
    value["generation"] = json!(generation);
    if terminal {
        value["state"] = json!({"state":"victory"});
        value["legal_actions"] = json!([]);
        value["player"]["potions"] = json!([]);
    } else if let Some(actions) = value["legal_actions"].as_array_mut() {
        actions.sort_by_key(|action| {
            if action["action_id"] == "end:7" {
                0_u8
            } else {
                1_u8
            }
        });
    }
    value
}

fn read_request(stream: &mut TcpStream) -> Result<DownstreamRequest, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if bytes.len() > 64 * 1024 {
            return Err("downstream request header bound exceeded".into());
        }
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("downstream request ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let mut lines = std::str::from_utf8(&bytes[..header_end])?.split("\r\n");
    let request_line = lines.next().ok_or("downstream request line missing")?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().ok_or("downstream method missing")?.to_owned();
    let path = parts.next().ok_or("downstream path missing")?.to_owned();
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err("downstream request line invalid".into());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("downstream header invalid")?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .ok_or("downstream content length missing")?
        .parse::<usize>()?;
    if length > 128 * 1024 {
        return Err("downstream request body bound exceeded".into());
    }
    let body_start = header_end + 4;
    if bytes.len().saturating_sub(body_start) > length {
        return Err("downstream request has trailing bytes".into());
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; (length - body.len()).min(2048)];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("downstream request ended before body".into());
        }
        body.extend_from_slice(&chunk[..count]);
    }
    Ok(DownstreamRequest {
        method,
        path,
        headers,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body)?
        },
    })
}

fn write_response(stream: &mut TcpStream, status: u16, body: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    let headers = format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(&body)
}
