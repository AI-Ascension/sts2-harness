// SPDX-License-Identifier: MIT

//! Capability checks for the immutable peers the runtime peer lane declares.
//!
//! `contracts/runtime-peer-lane.json` names the exact gateway and MCP revisions this lane builds
//! against. The runtime spawns the configured MCP with the `watchdog-recovery-v1` profile before
//! it reads or reconciles a durable operation, validates the advertised catalog, and fails closed
//! when the peer refuses the profile. A declared peer that predates the profile therefore leaves
//! the lane's own recovery path unreachable while every other case in the lane still passes.
//!
//! These checks are operator-only: they need an explicitly built MCP peer and both peer sources.

#![cfg(unix)]

use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_harness::{RECOVERY_CONTRACT_VERSION, RECOVERY_SCHEMA_DIGEST};

const RECOVERY_PROFILE: &str = "watchdog-recovery-v1";
const RECOVERY_CATALOG_REVISION: &str = "watchdog-recovery-v1-mcp";
const RECOVERY_TOKEN: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const RECOVERY_TOOLS: [&str; 9] = [
    "watchdog.bootstrap",
    "watchdog.host_fence",
    "watchdog.lease_acquire",
    "watchdog.lease_renew",
    "watchdog.lease_revoke",
    "watchdog.operation_intent",
    "watchdog.operation_dispatch",
    "watchdog.operation_lookup",
    "watchdog.operation_reconcile",
];
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DIAGNOSTIC_BYTES: u64 = 2_048;

#[test]
#[ignore = "operator-only test; requires explicitly built runtime peers"]
fn configured_mcp_peer_serves_the_watchdog_recovery_sideband()
-> Result<(), Box<dyn std::error::Error>> {
    let mut peer = RecoveryPeer::spawn(&executable("STS2_MCP_BINARY")?)?;
    let initialize = peer.exchange(
        1,
        "initialize",
        &json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "sts2-harness-recovery", "version": "0.0.0"}
        }),
    )?;
    if initialize.get("result").is_none() {
        return Err(format!("MCP peer refused initialize: {initialize}").into());
    }
    let catalog = peer.exchange(2, "tools/list", &json!({}))?;
    let result = catalog
        .get("result")
        .ok_or_else(|| format!("MCP peer refused tools/list: {catalog}"))?;
    if result.get("revision").and_then(Value::as_str) != Some(RECOVERY_CATALOG_REVISION) {
        return Err(format!(
            "the {RECOVERY_PROFILE} catalog is not {RECOVERY_CATALOG_REVISION}: {result}"
        )
        .into());
    }
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("recovery catalog omitted tools: {result}"))?;
    let observed: Vec<&str> = tools
        .iter()
        .map(|tool| tool.get("name").and_then(Value::as_str).unwrap_or(""))
        .collect();
    if observed.as_slice() != RECOVERY_TOOLS.as_slice() {
        return Err(format!(
            "recovery catalog does not expose the exact sideband surface: {observed:?}"
        )
        .into());
    }
    Ok(())
}

#[test]
#[ignore = "operator-only test; requires both peer source checkouts"]
fn declared_peers_carry_one_recovery_frame_contract() -> Result<(), Box<dyn std::error::Error>> {
    for name in ["STS2_GATEWAY_PEER_ROOT", "STS2_MCP_PEER_ROOT"] {
        let root = peer_root(name)?;
        require_constant(&root, "RECOVERY_CONTRACT", RECOVERY_CONTRACT_VERSION, name)?;
        require_constant(
            &root,
            "RECOVERY_SCHEMA_DIGEST",
            RECOVERY_SCHEMA_DIGEST,
            name,
        )?;
    }
    Ok(())
}

fn executable(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required for this ignored operator test"))?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{name} is not a file").into())
    }
}

fn peer_root(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required for this ignored operator test"))?;
    if path.join("crates").is_dir() {
        Ok(path)
    } else {
        Err(format!("{name} is not a peer checkout").into())
    }
}

fn recovery_sources(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut found = Vec::new();
    collect(&root.join("crates"), &mut found)?;
    Ok(found)
}

fn collect(directory: &Path, found: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.path().is_dir() {
            if name != "target" && name != ".git" {
                collect(&entry.path(), found)?;
            }
            continue;
        }
        if name.contains("recovery") && name.ends_with(".rs") {
            found.push(entry.path());
        }
    }
    Ok(())
}

fn require_constant(
    root: &Path,
    constant: &str,
    expected: &str,
    label: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut declared = Vec::new();
    for source in recovery_sources(root)? {
        declared.extend(defined_strings(&source, constant)?);
    }
    if declared.is_empty() {
        return Err(format!("{label} does not declare {constant}").into());
    }
    if let Some(unexpected) = declared.iter().find(|value| value.as_str() != expected) {
        return Err(format!("{label} declares {constant} as {unexpected}, not {expected}").into());
    }
    Ok(())
}

fn defined_strings(
    source: &Path,
    constant: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let text = fs::read_to_string(source)?;
    let needle = format!("const {constant}: &str =");
    let mut values = Vec::new();
    let mut remaining = text.as_str();
    while let Some(index) = remaining.find(&needle) {
        let after = &remaining[index + needle.len()..];
        let path = source.display();
        let Some(quoted) = after.trim_start().strip_prefix('"') else {
            return Err(format!("{path} defines {constant} without a string literal").into());
        };
        let Some(end) = quoted.find('"') else {
            return Err(format!("{path} has an unterminated {constant}").into());
        };
        values.push(quoted[..end].to_string());
        remaining = after;
    }
    Ok(values)
}

struct RecoveryPeer {
    child: Child,
    input: Option<ChildStdin>,
    lines: Receiver<String>,
    reader: Option<JoinHandle<()>>,
    diagnostics: PathBuf,
}

impl RecoveryPeer {
    fn spawn(binary: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let pid = std::process::id();
        let diagnostics = std::env::temp_dir().join(format!("sts2-recovery-peer-{pid}.stderr"));
        let mut command = Command::new(binary);
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_GATEWAY_ADDR", "127.0.0.1:1")
            .env("STS2_GATEWAY_TOKEN", "recovery-sideband-probe")
            .env("STS2_RUNTIME_PROFILE", RECOVERY_PROFILE)
            .env("STS2_INSTANCE_ID", "22222222-2222-4222-8222-222222222222")
            .env("STS2_CALLER_ID", "11111111-1111-4111-8111-111111111111")
            .env("STS2_SESSION_ID", "33333333-3333-4333-8333-333333333333")
            .env(
                "STS2_MCP_SESSION_ID",
                "44444444-4444-4444-8444-444444444444",
            )
            .env("STS2_LEASE_ID", "55555555-5555-4555-8555-555555555555")
            .env("STS2_LEASE_EPOCH", "1")
            .env("STS2_RECOVERY_TOKEN", RECOVERY_TOKEN)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(File::create(&diagnostics)?));
        let mut child = command.spawn()?;
        let input = child.stdin.take();
        let stdout = child
            .stdout
            .take()
            .ok_or("recovery peer stdout was not captured")?;
        let (sender, lines) = mpsc::channel();
        let reader = thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if sender.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self {
            child,
            input,
            lines,
            reader: Some(reader),
            diagnostics,
        })
    }

    fn exchange(
        &mut self,
        id: u64,
        method: &str,
        params: &Value,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let input = self
            .input
            .as_mut()
            .ok_or("recovery peer stdin is already closed")?;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        input.write_all(&serde_json::to_vec(&request)?)?;
        input.write_all(b"\n")?;
        input.flush()?;
        let deadline = Instant::now() + EXCHANGE_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.silence(method).into());
            }
            let Ok(line) = self.lines.recv_timeout(remaining) else {
                return Err(self.silence(method).into());
            };
            let response: Value = serde_json::from_str(&line)
                .map_err(|_| format!("recovery peer sent a non-JSON line: {line}"))?;
            if response.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(response);
            }
        }
    }

    fn silence(&self, method: &str) -> String {
        let refusal = self.refusal();
        format!("recovery peer did not answer {method} within {EXCHANGE_TIMEOUT:?}: {refusal}")
    }

    fn refusal(&self) -> String {
        match fs::File::open(&self.diagnostics) {
            Ok(file) => {
                let mut reader = BufReader::new(file.take(MAX_DIAGNOSTIC_BYTES));
                let mut text = String::new();
                let _ = reader.read_to_string(&mut text);
                text.trim().to_string()
            }
            Err(_) => String::from("no peer diagnostics"),
        }
    }
}

impl Drop for RecoveryPeer {
    fn drop(&mut self) {
        self.input.take();
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while Instant::now() < deadline {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        let _ = fs::remove_file(&self.diagnostics);
    }
}
