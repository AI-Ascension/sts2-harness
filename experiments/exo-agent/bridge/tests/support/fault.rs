// SPDX-License-Identifier: MIT

//! Fault-injection helpers for the `fault_oracle` lane (`sts2-harness#148` T2/T3).
//!
//! Deliberately separate from `support/mod.rs`: `process_oracle`'s recorded evidence pins the
//! support sources it ships, so this lane brings its own loopback endpoint and process driver
//! instead of editing shared oracle support. Nothing here reaches a provider, credential, game,
//! save or native host; every run is a synthetic loopback model with the declared synthetic host.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// One synthetic Responses payload that returns a single `wait` decision.
fn one_shot_wait() -> Value {
    let text = json!({"decision": "wait", "rationale": "synthetic"}).to_string();
    json!({
        "id": "resp_synthetic", "object": "response", "created_at": 0,
        "status": "completed", "model": "o3-pro",
        "output": [{
            "id": "msg_synthetic", "type": "message", "status": "completed",
            "role": "assistant",
            "content": [{"type": "output_text", "text": text, "annotations": []}]
        }],
        "usage": {"input_tokens": 11, "output_tokens": 5, "total_tokens": 16}
    })
}

/// A loopback Responses endpoint that can either answer or drop the connection before replying.
///
/// `drop_replies` is the *lost reply* fault: the request is consumed in full, then the socket is
/// closed with no response, so the pinned runtime sees a truncated exchange rather than a typed
/// HTTP error.
pub struct FaultModel {
    connections: Arc<AtomicUsize>,
    drop_reply: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub endpoint: String,
}

impl FaultModel {
    pub fn start() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let endpoint = format!("http://{}", listener.local_addr()?);
        let connections = Arc::new(AtomicUsize::new(0));
        let drop_reply = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (counted, dropping, finished) = (connections.clone(), drop_reply.clone(), stop.clone());
        let worker = std::thread::spawn(move || {
            while !finished.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        counted.fetch_add(1, Ordering::SeqCst);
                        let _ = consume_request(&mut stream);
                        if !dropping.load(Ordering::SeqCst) {
                            let body = serde_json::to_vec(&one_shot_wait()).unwrap_or_default();
                            let _ = write!(
                                stream,
                                "HTTP/1.1 200 Synthetic\r\nContent-Type: application/json\r\n\
                                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            );
                            let _ = stream.write_all(&body);
                        }
                        // Dropping `stream` with no write is the intended lost reply.
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(10)),
                }
            }
        });
        Ok(Self {
            connections,
            drop_reply,
            stop,
            worker: Some(worker),
            endpoint,
        })
    }

    pub fn request_count(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    pub fn reset(&self) {
        self.connections.store(0, Ordering::SeqCst);
    }

    pub fn drop_replies(&self) {
        self.drop_reply.store(true, Ordering::SeqCst);
    }

    pub fn answer(&self) {
        self.drop_reply.store(false, Ordering::SeqCst);
    }
}

impl Drop for FaultModel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Reads any request the caller sent, best effort, so a dropped connection counts one egress.
fn consume_request(stream: &mut std::net::TcpStream) -> Result {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        header.push(byte[0]);
        if header.len() > 16384 {
            return Ok(());
        }
    }
    let header = String::from_utf8_lossy(&header).to_string();
    let length = header.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())
            .flatten()
    });
    if let Some(length) = length {
        if length > 160 * 1024 {
            return Ok(());
        }
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body)?;
    }
    Ok(())
}

pub fn digest(path: &Path) -> Result<String> {
    let output = Command::new("sha256sum").arg(path).output()?;
    if !output.status.success() {
        return Err("digest failed".into());
    }
    Ok(String::from_utf8(output.stdout)?[..64].to_owned())
}

pub fn workspace_root() -> Result<PathBuf> {
    Ok(PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?)
}

pub fn source_root() -> Result<PathBuf> {
    let root = workspace_root()?;
    Ok(std::env::var_os("STS2_EXO_TEST_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| root.join("target/exo-source")))
}

/// The one-shot bridge configuration, identical in shape to the reviewed operator config.
pub fn config_json(model: &FaultModel) -> Result<Value> {
    let root = workspace_root()?;
    let executor = root.join("target/exo-executor/debug/sts2-exo-executor");
    let extension = root.join("experiments/exo-agent/extension/src/index.ts");
    let node = PathBuf::from(std::env::var("STS2_EXO_TEST_NODE")?).canonicalize()?;
    Ok(json!({
        "schema": "sts2.exo-one-shot-config-v1",
        "executor": executor, "executor_sha256": digest(&executor)?,
        "source_root": source_root()?, "extension": extension,
        "extension_sha256": digest(&extension)?,
        "node": node, "node_sha256": digest(&node)?,
        "model": "o3-pro", "endpoint": model.endpoint
    }))
}

/// Writes `config` under `target/<name>/config.json`.
///
/// Distinct names yield distinct config parents, and [`invoke`] derives the private `TMPDIR` from
/// that parent, so two named configs cannot share a temporary or state root.
pub fn write_config(name: &str, config: &Value) -> Result<PathBuf> {
    let directory = workspace_root()?.join("target").join(name);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("config.json");
    std::fs::write(&path, serde_json::to_vec(config)?)?;
    Ok(path)
}

/// Runs the shipped bridge exactly as the operator entrypoint does: cleared environment, a private
/// `TMPDIR` derived from the config directory, and the config digest on argv unless `--describe`.
///
/// `digest_override` exists only so the argv-identity fault can present a digest that is not the
/// file's own; every real run passes `None`.
pub fn invoke(
    binary: &Path,
    mode: &str,
    config: &Path,
    digest_override: Option<&str>,
    data: &[u8],
    eof: bool,
) -> Result<Output> {
    let temporary = config
        .parent()
        .ok_or("config parent missing")?
        .join("exo-test-tmp");
    std::fs::create_dir_all(&temporary)?;
    let mut command = Command::new(binary);
    command.arg(mode).arg(config);
    if mode != "--describe" {
        command.arg(match digest_override {
            Some(text) => text.to_owned(),
            None => digest(config)?,
        });
    }
    command.env_clear().env("TMPDIR", temporary);
    run_bounded(&mut command, data, eof).map(|(output, _)| output)
}

pub fn run_bounded(command: &mut Command, data: &[u8], eof: bool) -> Result<(Output, usize)> {
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
    let mut maximum = 0;
    while child.try_wait()?.is_none() {
        maximum = maximum.max(descendants(child.id())?);
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            return Err("fault oracle deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(retained);
    Ok((child.wait_with_output()?, maximum))
}

/// Number of live processes in the tree rooted at `pid`, bounded to refuse a runaway walk.
pub fn descendants(pid: u32) -> Result<usize> {
    let mut pending = vec![pid];
    let mut visited = std::collections::BTreeSet::new();
    while let Some(current) = pending.pop() {
        if !visited.insert(current) {
            continue;
        }
        assert!(visited.len() <= 32, "process tree bound");
        let process = PathBuf::from(format!("/proc/{current}"));
        if let Ok(tasks) = std::fs::read_dir(process.join("task")) {
            for task in tasks.flatten() {
                if let Ok(children) = std::fs::read_to_string(task.path().join("children")) {
                    pending.extend(
                        children
                            .split_whitespace()
                            .filter_map(|id: &str| id.parse::<u32>().ok()),
                    );
                }
            }
        }
    }
    Ok(visited.len())
}
