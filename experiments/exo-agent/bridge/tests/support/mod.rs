// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn digest(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum").arg(path).output()?;
    if !output.status.success() {
        return Err("digest failed".into());
    }
    Ok(String::from_utf8(output.stdout)?[..64].to_owned())
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

pub struct Model {
    pub requests: Arc<Mutex<Vec<Value>>>,
    connections: Arc<std::sync::atomic::AtomicUsize>,
    response: Arc<Mutex<(u16, Value)>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub endpoint: String,
}

impl Model {
    pub fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let received = connections.clone();
        let response = Arc::new(Mutex::new((200, Value::Null)));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (observed, configured, finished) = (requests.clone(), response.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !finished.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok((mut stream, _)) = listener.accept() {
                    received.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let result = serve(&mut stream, &observed, &configured);
                    assert!(result.is_ok(), "synthetic server failed");
                } else {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
        });
        Ok(Self {
            requests,
            connections,
            response,
            stop,
            worker: Some(worker),
            endpoint,
        })
    }

    pub fn set(&self, status: u16, output: Value) -> Result {
        self.connections
            .store(0, std::sync::atomic::Ordering::SeqCst);
        self.requests
            .lock()
            .map_err(|_| "requests poisoned")?
            .clear();
        let output = if status == 200 {
            output
        } else {
            json!({"error": {"message": "synthetic unavailable", "type": "rate_limit_error"}})
        };
        *self.response.lock().map_err(|_| "response poisoned")? = (status, output);
        Ok(())
    }

    pub fn request_count(&self) -> usize {
        self.connections.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve(
    stream: &mut std::net::TcpStream,
    requests: &Mutex<Vec<Value>>,
    response: &Mutex<(u16, Value)>,
) -> Result {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        header.push(byte[0]);
        if header.len() > 16384 {
            return Err("header bound".into());
        }
    }
    let header = String::from_utf8(header)?;
    assert!(header.starts_with("POST /responses HTTP/1.1\r\n"));
    let length: usize = header
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse())
                .transpose()
                .ok()
                .flatten()
        })
        .ok_or("missing length")?;
    if length > 160 * 1024 {
        return Err("request bound".into());
    }
    let mut input = vec![0; length];
    stream.read_exact(&mut input)?;
    requests
        .lock()
        .map_err(|_| "requests poisoned")?
        .push(serde_json::from_slice(&input)?);
    let (status, payload) = response.lock().map_err(|_| "response poisoned")?.clone();
    let bytes = serde_json::to_vec(&payload)?;
    write!(
        stream,
        "HTTP/1.1 {status} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )?;
    stream.write_all(&bytes)?;
    Ok(())
}

pub fn response(text: &str, kind: &str) -> Value {
    let mut output = json!([{
        "id": "msg_synthetic", "type": "message", "status": "completed", "role": "assistant",
        "content": [{"type": "output_text", "text": text, "annotations": []}]
    }]);
    match kind {
        "refusal" => output[0]["content"] = json!([{"type": "refusal", "refusal": "synthetic"}]),
        "tool" => {
            output = json!([{
                "type": "function_call", "id": "fc_synthetic", "call_id": "call_synthetic",
                "name": "shell", "arguments": "{}", "status": "completed"
            }])
        }
        "multiple" => output = json!([output[0].clone(), output[0].clone()]),
        _ => {}
    }
    json!({
        "id": "resp_synthetic", "object": "response", "created_at": 0,
        "status": "completed", "model": "o3-pro", "output": output,
        "usage": {"input_tokens": 11, "output_tokens": 5, "total_tokens": 16}
    })
}

pub fn invoke(binary: &Path, config: &Path, data: &[u8], mode: &str, eof: bool) -> Result<Output> {
    let mut command = Command::new(binary);
    let temporary = config
        .parent()
        .ok_or("config parent missing")?
        .join("exo-test-tmp");
    std::fs::create_dir_all(&temporary)?;
    command.arg(mode).arg(config);
    if mode != "--describe" {
        command.arg(digest(config)?);
    }
    let mut child = command
        .env_clear()
        .env("TMPDIR", temporary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut input = child.stdin.take().ok_or("stdin missing")?;
    input.write_all(data)?;
    let retained = if eof {
        drop(input);
        None
    } else {
        Some(input)
    };
    let deadline = Instant::now() + Duration::from_secs(125);
    let mut maximum_processes = 0;
    while child.try_wait()?.is_none() {
        maximum_processes = maximum_processes.max(private_argv_absent(child.id())?);
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            return Err("process oracle deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(retained);
    let output = child.wait_with_output()?;
    if String::from_utf8_lossy(&output.stderr).contains("sts2.exo-one-shot-evidence-v1") {
        assert!(maximum_processes >= 3, "real Exo process tree missing");
    }
    Ok(output)
}

fn private_argv_absent(pid: u32) -> Result<usize> {
    let mut pending = vec![pid];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !visited.insert(current) {
            continue;
        }
        assert!(visited.len() <= 32);
        let process = std::path::PathBuf::from(format!("/proc/{current}"));
        if let Ok(command) = std::fs::read(process.join("cmdline")) {
            for forbidden in [
                "host-request-private-sentinel",
                "host-turn-private-sentinel",
                "execution-11",
                "combat.end-turn",
                "synthetic exact objective sentinel",
                "synthetic complete constraint sentinel",
                "sts2-synthetic-model-key",
            ] {
                assert!(
                    !command
                        .windows(forbidden.len())
                        .any(|part| part == forbidden.as_bytes())
                );
            }
        }
        if let Ok(tasks) = std::fs::read_dir(process.join("task")) {
            for task in tasks.flatten() {
                if let Ok(children) = std::fs::read_to_string(task.path().join("children")) {
                    pending.extend(
                        children
                            .split_whitespace()
                            .filter_map(|child| child.parse::<u32>().ok()),
                    );
                }
            }
        }
    }
    Ok(visited.len())
}
