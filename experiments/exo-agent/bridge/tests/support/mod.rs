// SPDX-License-Identifier: MIT

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub mod projection;

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

pub fn digest(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum").arg(path).output()?;
    if !output.status.success() {
        return Err("digest failed".into());
    }
    Ok(String::from_utf8(output.stdout)?[..64].to_owned())
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
        self.replace_response(status, output)?;
        Ok(())
    }

    pub fn replace_response(&self, status: u16, output: Value) -> Result {
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
        "tool" => output = tool_call("shell")["output"].clone(),
        "multiple" => output = json!([output[0].clone(), output[0].clone()]),
        _ => {}
    }
    json!({
        "id": "resp_synthetic", "object": "response", "created_at": 0,
        "status": "completed", "model": "o3-pro", "output": output,
        "usage": {"input_tokens": 11, "output_tokens": 5, "total_tokens": 16}
    })
}

/// Digests of the oracle support sources, recorded so the evidence binds every assertion module.
pub fn support_digests() -> Result<Value> {
    let support = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/support");
    Ok(json!({
        "tests/support/mod.rs": digest(&support.join("mod.rs"))?,
        "tests/support/projection.rs": digest(&support.join("projection.rs"))?
    }))
}

/// One synthetic Responses output whose only item calls `name`; the synthetic model returns it
/// verbatim, so the case exercises the actual upstream tool-call path with that exact name.
pub fn tool_call(name: &str) -> Value {
    json!({
        "id": "resp_synthetic", "object": "response", "created_at": 0,
        "status": "completed", "model": "o3-pro", "output": [{
            "type": "function_call", "id": "fc_synthetic", "call_id": "call_synthetic",
            "name": name, "arguments": "{}", "status": "completed"
        }],
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
    command.env_clear().env("TMPDIR", temporary);
    let output = run_bounded(command, data, eof)?;
    if String::from_utf8_lossy(&output.0.stderr).contains("sts2.exo-one-shot-evidence-v1") {
        assert!(output.1 >= 3, "real Exo process tree missing");
    }
    Ok(output.0)
}

/// Drives the separately built executor exactly as the bridge does (private handoff on stdin,
/// cleared environment, fresh private roots) and returns its receipt. This is the only boundary
/// where the executor's typed `error_code` is observable: the bridge maps every executor failure
/// to `exo_bridge_executor_failed` and never forwards the code.
pub fn invoke_executor(config: &Path, envelope: &Value, ordinal: usize) -> Result<Value> {
    let config: Value = serde_json::from_slice(&std::fs::read(config)?)?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../target/exo-test-tmp")
        .join(format!("executor-probe-{}-{ordinal}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    for child in ["state", "config", "cache", "temp"] {
        std::fs::create_dir(root.join(child))?;
    }
    let request = &envelope["request"];
    let handoff = json!({
        "version": "sts2.exo-executor-input-v1",
        "request_id": envelope["request_id"], "host_turn_id": envelope["turn_id"],
        "model": config["model"], "endpoint": config["endpoint"],
        "module_path": config["extension"], "source_root": config["source_root"],
        "state_root": root.join("state"),
        "input": {
            "observation": request["observation"], "legal_action_ids": request["legal_action_ids"],
            "objective": request["objective"], "hard_constraints": request["hard_constraints"]
        },
        "timeout_millis": 115_000, "max_output_tokens": 4096,
        "credential": "sts2-synthetic-model-key"
    });
    let node = Path::new(config["node"].as_str().ok_or("node missing")?);
    let mut command = Command::new(config["executor"].as_str().ok_or("executor missing")?);
    command
        .env_clear()
        .env("PATH", node.parent().ok_or("node parent missing")?)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("temp"))
        .env("EXO_LITELLM_PRICES_PATH", root.join("no-prices.json"))
        .env(
            "STS2_EXO_ALLOWED_ENDPOINT",
            config["endpoint"].as_str().ok_or("endpoint missing")?,
        );
    let (output, processes) = run_bounded(command, &serde_json::to_vec(&handoff)?, true)?;
    std::fs::remove_dir_all(&root)?;
    assert!(processes >= 2, "real Exo process tree missing");
    assert!(
        output.status.success(),
        "executor: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn run_bounded(mut command: Command, data: &[u8], eof: bool) -> Result<(Output, usize)> {
    let mut child = command
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
    Ok((child.wait_with_output()?, maximum_processes))
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
