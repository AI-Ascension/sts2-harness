// SPDX-License-Identifier: MIT

use std::fs;
use std::io::Read;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::fixture::{
    CALLER_ID, DownstreamLedger, INSTANCE_ID, LEASE_EPOCH, LEASE_ID, MCP_SESSION_ID, ModServer,
    REST_SCHEMA_DIGEST, SESSION_ID,
};

const MAX_CAPTURE_BYTES: usize = 4 * 1024 * 1024;
const OUTPUT_TRUNCATION_MARKER: &[u8] = b"\n{\"event\":\"process_output_truncated\"}\n";
type CaptureHandle = thread::JoinHandle<std::io::Result<Vec<u8>>>;
type CapturePipes = (CaptureHandle, CaptureHandle);

pub(crate) struct ScenarioResult {
    pub(crate) runtime: Output,
    pub(crate) gateway: Output,
    pub(crate) ledger: DownstreamLedger,
}

pub(crate) struct TempDir {
    path: PathBuf,
}

impl TempDir {
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sts2-runtime-v4-rest-executable-composition-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    /// The bounded bridge selects only IDs present in the current host catalog. Each Exo
    /// exchange is a fresh process, so selection is derived from the request rather than hidden
    /// mutable state.
    pub(crate) fn bridge(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = self.path.join("bounded-rest-exo-bridge.sh");
        fs::write(
            &path,
            r##"#!/bin/sh
set -eu
request=$(cat)
emit() {
  printf '%s\n' "$1"
}
case "$request" in
  *select_card:10:smith:card:1*)
    emit '{"decision":"action","action_id":"select_card:10:smith:card:1","rationale":"choose the first Smith card"}' ;;
  *select_card:11:smith:card:2*)
    emit '{"decision":"action","action_id":"select_card:11:smith:card:2","rationale":"choose the second Smith card"}' ;;
  *confirm_selection:12:smith*)
    emit '{"decision":"action","action_id":"confirm_selection:12:smith","rationale":"confirm the Smith selection"}' ;;
  *select_player:14:mend:player:local*)
    emit '{"decision":"action","action_id":"select_player:14:mend:player:local","rationale":"choose the local player for Mend"}' ;;
  *rest-option:13:mend*)
    emit '{"decision":"action","action_id":"rest-option:13:mend","rationale":"choose Mend"}' ;;
  *rest-option:9:smith*)
    emit '{"decision":"action","action_id":"rest-option:9:smith","rationale":"choose Smith"}' ;;
  *)
    exit 3 ;;
esac
"##,
        )?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub(crate) fn executable(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{name} is required for this ignored operator test"))?;
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!("{name} is not a file").into())
    }
}

fn sha256_file(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    Ok(format!("{:x}", Sha256::digest(fs::read(path)?)))
}

fn free_address() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?)
}

fn gateway(
    binary: &Path,
    address: SocketAddr,
    mod_address: SocketAddr,
) -> Result<Child, Box<dyn std::error::Error>> {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("STS2_GATEWAY_ADDR", address.to_string())
        .env("STS2_MOD_ADDR", mod_address.to_string())
        .env("STS2_GATEWAY_TOKEN", "gateway-token")
        .env("STS2_MOD_TOKEN", "mod-token")
        .env("STS2_INSTANCE_ID", INSTANCE_ID)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_LEASE_ID", LEASE_ID)
        .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok(command.spawn()?)
}

fn ready(child: &mut Child, address: SocketAddr) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(format!("gateway exited: {status}").into());
        }
        if TcpStream::connect(address).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("gateway readiness deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

include!("runtime_v4_rest_executable_composition_process_io.rs");

/// Run the source-derived composition with explicitly supplied synthetic gateway, MCP, and
/// harness binaries. The cleared child environment prevents inherited credentials, but it does
/// not make arbitrary executable output public or suitable for evidence retention.
pub(crate) fn run_scenario(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    bridge: &Path,
) -> Result<ScenarioResult, Box<dyn std::error::Error>> {
    let mod_server = ModServer::new()?;
    let address = free_address()?;
    let mut gateway_process = gateway(gateway_binary, address, mod_server.address)?;
    let bridge_revision = sha256_file(bridge)?;
    let runtime = (|| {
        ready(&mut gateway_process, address)?;
        // Live recording is enabled only for this local synthetic bridge so the executable's
        // bounded action_receipt stream is captured with the child diagnostics. The shell
        // redirects the recorder's stdout to stderr; the runtime completion report and receipt
        // records then share one captured, inspectable channel.
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "exec \"$1\" 1>&2", "sts2-rest-runtime"])
            .arg(harness_binary);
        command
            .env_clear()
            .env("STS2_GATEWAY_ADDR", address.to_string())
            .env("STS2_GATEWAY_TOKEN", "gateway-token")
            .env("STS2_MCP_BINARY", mcp_binary)
            .env("STS2_RUNTIME_PROFILE", "runtime-v4-expert-rest-action")
            .env("STS2_INSTANCE_ID", INSTANCE_ID)
            .env("STS2_CALLER_ID", CALLER_ID)
            .env("STS2_SESSION_ID", SESSION_ID)
            .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
            .env("STS2_LEASE_ID", LEASE_ID)
            .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
            .env("STS2_RUN_ID", "run-rest-executable-composition")
            .env("STS2_EPISODE_ID", "episode-rest-executable-composition")
            .env(
                "STS2_TRAJECTORY_ID",
                "trajectory-rest-executable-composition",
            )
            .env("STS2_TRACE_ID", "trace-rest-executable-composition")
            .env("STS2_ARTIFACT_ID", "artifact-rest-executable-composition")
            .env("STS2_EXO_REVISION", bridge_revision)
            .env("STS2_PROVIDER_KIND", "openai-astra")
            .env("STS2_LIVE_EPISODE", "true")
            .env("STS2_EXO_BRIDGE_BINARY", bridge)
            .env("STS2_EXO_TIMEOUT_MILLIS", "2000")
            .env("STS2_EXO_MAX_REQUEST_BYTES", "131072")
            .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
            .env("STS2_MAX_STEPS", "8")
            .env("STS2_BARRIER_MAX_POLLS", "1")
            .env("STS2_BARRIER_WAIT_MILLIS", "1")
            .env("STS2_RECOVERY_MAX_ATTEMPTS", "2")
            .env("STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS", "0")
            .env("STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS", "1")
            .env(
                "STS2_OBJECTIVE",
                "reach the bounded synthetic REST terminal state",
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        bounded(command)
    })();
    let gateway_output = stop(gateway_process)?;
    let ledger = mod_server.finish();
    Ok(ScenarioResult {
        runtime: runtime?,
        gateway: gateway_output,
        ledger,
    })
}

fn runtime_records(output: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(output)
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

fn output_was_truncated(output: &[u8]) -> bool {
    runtime_records(output)
        .iter()
        .any(|record| record["event"] == "process_output_truncated")
}

fn completion_report(output: &[u8]) -> Result<Value, Box<dyn std::error::Error>> {
    runtime_records(output)
        .into_iter()
        .rev()
        .find(|record| {
            record["protocol"] == "runtime-v4-expert-rest-action" && record["status"] == "complete"
        })
        .ok_or_else(|| "runtime completion report is missing".into())
}

fn assert_persisted_receipts(result: &ScenarioResult) -> Result<(), Box<dyn std::error::Error>> {
    let records = runtime_records(&result.runtime.stderr);
    let receipt = |operation_id: &str, status: &str| {
        records.iter().find(|record| {
            record["event"] == "action_receipt"
                && record["operation_id"] == operation_id
                && record["status"] == status
        })
    };
    for (operation_id, action_id, generation, state_id) in [
        ("episode-action-9-1", "rest-option:9:smith", 10, "live:10"),
        ("episode-action-13-5", "rest-option:13:mend", 14, "live:14"),
    ] {
        let unknown = receipt(operation_id, "Unknown")
            .ok_or_else(|| format!("runtime receipt ledger omitted Unknown {operation_id}"))?;
        if unknown["action_id"] != action_id || !unknown["observation"].is_null() {
            return Err(format!(
                "runtime Unknown receipt lost original identity for {operation_id}: {unknown}"
            )
            .into());
        }
        let settled = receipt(operation_id, "Settled")
            .ok_or_else(|| format!("runtime receipt ledger omitted Settled {operation_id}"))?;
        if settled["action_id"] != action_id
            || settled["effect"] != "rest_option_selection_requested"
            || settled["observation"]["state_id"] != state_id
            || settled["observation"]["generation"] != generation
        {
            return Err(format!(
                "runtime Settled receipt lost original identity for {operation_id}: {settled}"
            )
            .into());
        }
    }
    let accepted_index = records
        .iter()
        .position(|record| {
            record["event"] == "action_receipt"
                && record["operation_id"] == "episode-action-10-2"
                && record["status"] == "Accepted"
        })
        .ok_or("runtime receipt ledger omitted Accepted episode-action-10-2")?;
    let settled_index = records
        .iter()
        .enumerate()
        .skip(accepted_index + 1)
        .find_map(|(index, record)| {
            (record["event"] == "action_receipt"
                && record["operation_id"] == "episode-action-10-2"
                && record["status"] == "Settled")
                .then_some(index)
        })
        .ok_or("runtime receipt ledger omitted Settled episode-action-10-2")?;
    let accepted = &records[accepted_index];
    let settled = &records[settled_index];
    if accepted["action_id"] != "select_card:10:smith:card:1"
        || !accepted["effect"].is_null()
        || !accepted["observation"].is_null()
        || settled["action_id"] != "select_card:10:smith:card:1"
        || settled["effect"] != "rest_option_selection_progressed"
        || settled["observation"]["state_id"] != "live:11"
        || settled["observation"]["generation"] != 11
    {
        return Err(format!(
            "runtime Accepted to Settled receipt lost progress identity: accepted={accepted} settled={settled}"
        )
        .into());
    }
    assert_additional_settled_receipts(&records)?;
    let final_settled = receipt("episode-action-14-6", "Settled")
        .ok_or("runtime receipt ledger omitted Settled episode-action-14-6")?;
    if final_settled["action_id"] != "select_player:14:mend:player:local"
        || final_settled["effect"] != "rest_option_selection_completed"
        || final_settled["observation"]["state_id"] != "live:15"
        || final_settled["observation"]["generation"] != 15
    {
        return Err(
            format!("runtime final Mend receipt lost settled identity: {final_settled}").into(),
        );
    }
    Ok(())
}

fn action_requests(ledger: &DownstreamLedger) -> Vec<&super::fixture::DownstreamRequest> {
    ledger
        .requests
        .iter()
        .filter(|request| request.path == "/api/v4/runtime/expert-rest-action")
        .collect()
}

include!("runtime_v4_rest_executable_composition_process_assertions.rs");
pub(crate) fn write_evidence(
    result: &ScenarioResult,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    // Evidence retention is deliberately limited to this synthetic fixture and the explicitly
    // supplied synthetic binaries. Every capture is bounded and each file is private; callers
    // must still keep the directory private because env_clear does not sanitize child output.
    write_private(&root, "runtime.stdout", &result.runtime.stdout)?;
    write_private(&root, "runtime.stderr", &result.runtime.stderr)?;
    write_private(&root, "gateway.stdout", &result.gateway.stdout)?;
    write_private(&root, "gateway.stderr", &result.gateway.stderr)?;
    let summary = Value::Array(
        result
            .ledger
            .requests
            .iter()
            .zip(&result.ledger.responses)
            .map(|(request, response)| {
                json!({
                    "method": request.method,
                    "path": request.path,
                    "operation_id": request.body["operation_id"],
                    "action_id": request.body["action"]["action_id"],
                    "state_id": request.body["state_id"],
                    "generation": request.body["generation"],
                    "response_status": response.status,
                    "response_operation_id": response.body["operation_id"],
                    "response_state_id": response.body["state_id"],
                    "response_generation": response.body["generation"],
                    "response_status_value": response.body["status"],
                    "response_effect_witness": response.body["effect_witness"]
                })
            })
            .collect(),
    );
    write_private(
        &root,
        "downstream.json",
        &serde_json::to_vec_pretty(&summary)?,
    )?;
    write_private(
        &root,
        "result.json",
        &serde_json::to_vec_pretty(&json!({
            "status": "confirmed",
            "scope": "source-derived executable REST composition",
            "operation_id": operation,
            "runtime_exit": result.runtime.status.code(),
            "gateway_exit": result.gateway.status.code(),
            "provider": "synthetic bounded bridge",
            "game": "synthetic REST downstream"
        }))?,
    )?;
    Ok(())
}

fn write_private(root: &Path, name: &str, bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if bytes.len() > MAX_CAPTURE_BYTES {
        return Err(format!("evidence file {name} exceeds the bounded output limit").into());
    }
    let path = root.join(name);
    fs::write(&path, bytes)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}
