// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

pub(super) fn invoke_target(path: &Path, request: &Value) -> Result<Value, String> {
    let mut child = Command::new(path)
        .arg("phase3-adapter")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn target adapter: {error}"))?;
    let body = serde_json::to_vec(request).map_err(|_| "request encoding failed".to_owned())?;
    child
        .stdin
        .take()
        .ok_or_else(|| "target stdin unavailable".to_owned())?
        .write_all(&body)
        .map_err(|error| format!("write target request: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait target adapter: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "target adapter failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("target result JSON: {error}"))
}

pub(super) fn invoke_fake_peer(path: &Path, request: &Value) -> Result<Value, String> {
    let mut child = Command::new(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn fake peer: {error}"))?;
    let body =
        serde_json::to_vec(request).map_err(|_| "peer request encoding failed".to_owned())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "fake peer stdin unavailable".to_owned())?;
    stdin
        .write_all(&body)
        .and_then(|()| stdin.write_all(b"\n"))
        .map_err(|error| format!("write fake peer request: {error}"))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("wait fake peer: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "fake peer failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("fake peer result JSON: {error}"))
}

pub(super) fn string_field(value: &Value, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("adapter result missing {key}"))
}
