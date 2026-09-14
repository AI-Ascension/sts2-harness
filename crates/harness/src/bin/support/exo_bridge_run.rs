// SPDX-License-Identifier: MIT

use super::config::Loaded;
use serde::Deserialize;
use serde_json::json;
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use sts2_harness::{
    Decision, ExoBridgeRequestEnvelope, ExoWireOutcome, encode_bridge_response,
    parse_bridge_decision,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    version: String,
    request_id: String,
    host_turn_id: String,
    exo_turn_id: String,
    exo_session_id: String,
    decision: Option<String>,
    error_code: Option<String>,
    fetch_attempts: u64,
    forwarded_requests: u64,
    denied_requests: u64,
}

pub fn execute(
    loaded: &Loaded,
    envelope: ExoBridgeRequestEnvelope,
    synthetic: bool,
) -> Result<Vec<u8>, &'static str> {
    let credential = if synthetic {
        String::from("sts2-synthetic-model-key")
    } else {
        std::env::var("STS2_EXO_MODEL_KEY").map_err(|_| "exo_bridge_credentials_unavailable")?
    };
    if credential.is_empty() {
        return Err("exo_bridge_credentials_unavailable");
    }
    let private = PrivateRoot::create()?;
    let invocation = invocation(loaded, &envelope, &private, credential)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "exo_bridge_runtime")?;
    let result = runtime
        .block_on(exchange(loaded, &private, invocation))
        .and_then(|bytes| response(&envelope, &bytes));
    private.remove()?;
    result
}

fn invocation(
    loaded: &Loaded,
    envelope: &ExoBridgeRequestEnvelope,
    private: &PrivateRoot,
    credential: String,
) -> Result<Vec<u8>, &'static str> {
    // Control identities, revisions and credentials never enter the model-facing projection.
    let request = &envelope.request;
    let input = json!({
        "observation": request.observation,
        "legal_action_ids": request.legal_action_ids,
        "objective": request.objective,
        "hard_constraints": request.hard_constraints
    });
    serde_json::to_vec(&json!({
        "version": "sts2.exo-executor-input-v1",
        "request_id": envelope.request_id,
        "host_turn_id": envelope.turn_id,
        "model": loaded.config.model,
        "endpoint": loaded.config.endpoint,
        "module_path": loaded.config.extension,
        "source_root": loaded.config.source_root,
        "state_root": private.0.join("state"),
        "input": input,
        "timeout_millis": 115_000,
        "max_output_tokens": 4096,
        "credential": credential
    }))
    .map_err(|_| "exo_bridge_input")
}

fn executor_command(loaded: &Loaded, private: &PrivateRoot) -> Result<Command, &'static str> {
    let mut command = Command::new(&loaded.config.executor);
    command.as_std_mut().process_group(0);
    command
        .env_clear()
        .env(
            "PATH",
            loaded.config.node.parent().ok_or("exo_bridge_node")?,
        )
        .env("XDG_CONFIG_HOME", private.0.join("config"))
        .env("XDG_CACHE_HOME", private.0.join("cache"))
        .env("TMPDIR", private.0.join("temp"))
        .env("EXO_LITELLM_PRICES_PATH", private.0.join("no-prices.json"))
        .env("STS2_EXO_ALLOWED_ENDPOINT", &loaded.config.endpoint)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    Ok(command)
}

async fn exchange(
    loaded: &Loaded,
    private: &PrivateRoot,
    invocation: Vec<u8>,
) -> Result<Vec<u8>, &'static str> {
    let mut child = executor_command(loaded, private)?
        .spawn()
        .map_err(|_| "exo_bridge_executor_unavailable")?;
    let pid = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw)
        .ok_or("exo_bridge_executor_unavailable")?;
    let mut input = child.stdin.take().ok_or("exo_bridge_executor_pipe")?;
    let mut output = child.stdout.take().ok_or("exo_bridge_executor_pipe")?;
    let io = async {
        let write = async move {
            input.write_all(&invocation).await?;
            input.shutdown().await
        };
        let read = async {
            let mut bytes = Vec::new();
            (&mut output).take(16_385).read_to_end(&mut bytes).await?;
            Ok::<_, std::io::Error>(bytes)
        };
        let ((), bytes) = tokio::try_join!(write, read)?;
        Ok::<_, std::io::Error>(bytes)
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(118), io).await;
    let bytes = match result {
        Ok(Ok(bytes)) if bytes.len() <= 16_384 => bytes,
        _ => {
            // The child has not been reaped, so its process group identity cannot be reused.
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await;
            return Err("exo_bridge_executor_failed");
        }
    };
    let status = match tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await {
        Ok(Ok(status)) => status,
        _ => {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), child.wait()).await;
            return Err("exo_bridge_executor_timeout");
        }
    };
    if !status.success() {
        return Err("exo_bridge_executor_failed");
    }
    Ok(bytes)
}

fn response(envelope: &ExoBridgeRequestEnvelope, bytes: &[u8]) -> Result<Vec<u8>, &'static str> {
    let receipt: Receipt =
        serde_json::from_slice(bytes).map_err(|_| "exo_bridge_invalid_receipt")?;
    validate_receipt(envelope, &receipt)?;
    let terminal = receipt.decision.ok_or("exo_bridge_missing_decision")?;
    validate_decision(envelope, &terminal)?;
    let output = encode_bridge_response(
        &envelope.request_id,
        &envelope.turn_id,
        ExoWireOutcome::Decision,
        Some(terminal.as_bytes()),
        None,
    )
    .map_err(|_| "exo_bridge_invalid_decision")?;
    if output.len() > envelope.request.max_response_bytes as usize {
        return Err("exo_bridge_response_bound");
    }
    Ok(output)
}

fn validate_receipt(
    envelope: &ExoBridgeRequestEnvelope,
    receipt: &Receipt,
) -> Result<(), &'static str> {
    if receipt.version != "sts2.exo-executor-receipt-v1"
        || receipt.request_id != envelope.request_id
        || receipt.host_turn_id != envelope.turn_id
        || !valid_uuid(&receipt.exo_turn_id)
        || !valid_uuid(&receipt.exo_session_id)
    {
        return Err("exo_bridge_receipt_identity");
    }
    if receipt.forwarded_requests > 1
        || receipt.fetch_attempts > 16
        || receipt
            .forwarded_requests
            .checked_add(receipt.denied_requests)
            != Some(receipt.fetch_attempts)
    {
        return Err("exo_bridge_invalid_receipt");
    }
    eprintln!(
        "{}",
        json!({
            "schema": "sts2.exo-one-shot-evidence-v1",
            "exo_turn_id": receipt.exo_turn_id,
            "exo_session_id": receipt.exo_session_id,
            "fetch_attempts": receipt.fetch_attempts,
            "forwarded_requests": receipt.forwarded_requests,
            "denied_requests": receipt.denied_requests
        })
    );
    if receipt.error_code.is_some()
        || receipt.fetch_attempts != 1
        || receipt.forwarded_requests != 1
    {
        return Err("exo_bridge_executor_failed");
    }
    Ok(())
}

fn validate_decision(
    envelope: &ExoBridgeRequestEnvelope,
    terminal: &str,
) -> Result<(), &'static str> {
    let decision =
        parse_bridge_decision(terminal.as_bytes()).map_err(|_| "exo_bridge_invalid_decision")?;
    let legal = &envelope.request.legal_action_ids;
    match decision {
        Decision::Action { action_id, .. } if !legal.contains(&action_id) => {
            return Err("exo_bridge_illegal_action");
        }
        Decision::Plan { action_ids, .. } if action_ids.iter().any(|id| !legal.contains(id)) => {
            return Err("exo_bridge_illegal_action");
        }
        Decision::Recovery { .. } => return Err("exo_bridge_unsupported_recovery"),
        _ => {}
    }
    Ok(())
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36 && uuid::Uuid::parse_str(value).is_ok_and(|id| !id.is_nil())
}

struct PrivateRoot(PathBuf);

impl PrivateRoot {
    fn create() -> Result<Self, &'static str> {
        let parent = std::env::temp_dir();
        Self::create_under(&parent, uuid::Uuid::new_v4())
    }

    fn create_under(parent: &Path, identity: uuid::Uuid) -> Result<Self, &'static str> {
        if !parent.is_absolute()
            || !std::fs::symlink_metadata(parent)
                .map_err(|_| "exo_bridge_private_root")?
                .file_type()
                .is_dir()
            || parent
                .canonicalize()
                .map_err(|_| "exo_bridge_private_root")?
                != parent
        {
            return Err("exo_bridge_private_root");
        }
        let path = parent.join(format!("sts2-exo-{identity}"));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .map_err(|_| "exo_bridge_private_root")?;
        let root = Self(path);
        for name in ["state", "config", "cache", "temp"] {
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(root.0.join(name))
                .map_err(|_| "exo_bridge_private_root")?;
        }
        Ok(root)
    }

    fn remove(&self) -> Result<(), &'static str> {
        std::fs::remove_dir_all(&self.0).map_err(|_| "exo_bridge_private_cleanup")
    }
}

impl Drop for PrivateRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
#[path = "exo_bridge_run_tests.rs"]
mod tests;
