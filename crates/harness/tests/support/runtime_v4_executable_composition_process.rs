// SPDX-License-Identifier: MIT

// This module is a shared test-support library compiled into multiple integration-test
// binaries (`runtime_v4_executable_composition` and `served_gateway_stderr_evidence`). Each
// binary exercises a different subset of the scenarios and helpers below, so items that one
// binary does not call are still needed by the other. Treat "never used" here as expected
// rather than a defect, the same way the other shared support modules do.
#![allow(dead_code)]

use std::fs;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use sts2_harness::management::{
    CommandKind, CommandParameters, CommandRequest, CommandResponse, MANAGEMENT_SCHEMA_VERSION,
    ManagementClient, PROVIDER_SESSION_POLICY_COMMAND_SCHEMA_VERSION,
    ProviderSessionPolicyAdoptImportedRequest, ProviderSessionPolicyCommandResponse,
    ProviderSessionPolicyViewResponse, RunRequest, RunTargetConfiguration,
    TARGET_ADMISSION_SCHEMA_VERSION, TARGET_CATALOG_SCHEMA_VERSION, TargetAdmissionRequest,
    TargetCatalogResponse, TargetPreflightResponse, digest_value,
};
use sts2_harness::provider_session::{
    NativeCapabilities, ProviderSessionMetadataStore, ProviderSessionPolicy,
    ProviderSessionPolicyOwner, SessionScope,
};

use super::fixture::{
    ACTION_ID, CALLER_ID, DownstreamLedger, FixtureMode, INSTANCE_ID, LEASE_EPOCH, LEASE_ID,
    MCP_SESSION_ID, ModServer, REVIEWED_EXO_REVISION, SESSION_ID,
};

pub(crate) struct ScenarioResult {
    runtime: Output,
    gateway: Output,
    ledger: DownstreamLedger,
}

pub(crate) struct TempDir {
    pub(super) path: PathBuf,
}

impl TempDir {
    pub(crate) fn new() -> Result<Self, Box<dyn std::error::Error>> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "sts2-runtime-v4-executable-composition-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(Self { path })
    }

    pub(crate) fn bridge(&self) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = self.path.join("bounded-exo-bridge.sh");
        fs::write(
            &path,
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"decision\":\"action\",\"action_id\":\"potion:7:potion:fire:enemy:1\",\"rationale\":\"use the visible potion\"}'\n",
        )?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        Ok(path)
    }

    pub(crate) fn bridge_capturing(
        &self,
        capture_path: &Path,
    ) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let path = self.path.join("bounded-exo-bridge-capture.sh");
        let capture_path = capture_path
            .to_str()
            .ok_or("provider capture path is not UTF-8")?;
        let count_path = capture_path
            .strip_suffix(".json")
            .map(|path| format!("{path}.count"))
            .ok_or("provider capture path must use a .json extension")?;
        if capture_path.contains('\'') || count_path.contains('\'') {
            return Err("provider capture path contains a shell quote".into());
        }
        fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf x >> '{count_path}'\ncat > '{capture_path}'\nprintf '%s' '{{\"decision\":\"action\",\"action_id\":\"potion:7:potion:fire:enemy:1\",\"rationale\":\"use the visible potion\"}}'\n"
            ),
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

pub(super) fn free_address() -> Result<SocketAddr, Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?)
}

pub(super) fn gateway(
    binary: &Path,
    address: SocketAddr,
    mod_address: SocketAddr,
) -> Result<Child, Box<dyn std::error::Error>> {
    gateway_with_identity(
        binary,
        address,
        mod_address,
        INSTANCE_ID,
        LEASE_ID,
        LEASE_EPOCH,
    )
}

pub(super) fn gateway_with_identity(
    binary: &Path,
    address: SocketAddr,
    mod_address: SocketAddr,
    instance_id: &str,
    lease_id: &str,
    lease_epoch: u64,
) -> Result<Child, Box<dyn std::error::Error>> {
    let mut command = Command::new(binary);
    command
        .env_clear()
        .env("STS2_GATEWAY_ADDR", address.to_string())
        .env("STS2_MOD_ADDR", mod_address.to_string())
        .env("STS2_GATEWAY_TOKEN", "gateway-token")
        .env("STS2_MOD_TOKEN", "mod-token")
        .env("STS2_INSTANCE_ID", instance_id)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_LEASE_ID", lease_id)
        .env("STS2_LEASE_EPOCH", lease_epoch.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    use std::os::unix::process::CommandExt;
    command.process_group(0);
    Ok(command.spawn()?)
}

pub(super) fn ready(
    child: &mut Child,
    address: SocketAddr,
) -> Result<(), Box<dyn std::error::Error>> {
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

pub(super) fn stop(mut child: Child) -> Result<Output, Box<dyn std::error::Error>> {
    if child.try_wait()?.is_none() {
        let group = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .status()?;
        if !group.success() {
            child.kill()?;
        }
    }
    Ok(child.wait_with_output()?)
}

fn bounded(mut command: Command) -> Result<Output, Box<dyn std::error::Error>> {
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if child.try_wait()?.is_some() {
            return Ok(child.wait_with_output()?);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("runtime deadline exceeded".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn run_scenario(
    gateway_binary: &Path,
    mcp_binary: &Path,
    harness_binary: &Path,
    bridge: &Path,
    mode: FixtureMode,
) -> Result<ScenarioResult, Box<dyn std::error::Error>> {
    let mod_server = ModServer::new(mode)?;
    let address = free_address()?;
    let execution_store = bridge
        .parent()
        .map(|path| {
            path.join(match mode {
                FixtureMode::Success => "execution-success.sqlite3",
                FixtureMode::UnknownOperation => "execution-unknown.sqlite3",
                FixtureMode::AcceptedBarrierThenSettled => "execution-accepted-barrier.sqlite3",
                FixtureMode::ForeignExpertState => "execution-foreign.sqlite3",
                FixtureMode::MalformedExpertState => "execution-malformed.sqlite3",
            })
        })
        .ok_or("synthetic bridge has no parent directory")?;
    let mut gateway_process = gateway(gateway_binary, address, mod_server.address)?;
    let runtime = (|| {
        ready(&mut gateway_process, address)?;
        let mut command = Command::new(harness_binary);
        command
            .env_clear()
            .env("STS2_EXECUTION_STORE_PATH", execution_store)
            .env("STS2_GATEWAY_ADDR", address.to_string())
            .env("STS2_GATEWAY_TOKEN", "gateway-token")
            .env("STS2_MCP_BINARY", mcp_binary)
            .env("STS2_RUNTIME_PROFILE", "runtime-v4-expert")
            .env("STS2_INSTANCE_ID", INSTANCE_ID)
            .env("STS2_CALLER_ID", CALLER_ID)
            .env("STS2_SESSION_ID", SESSION_ID)
            .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
            .env("STS2_LEASE_ID", LEASE_ID)
            .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
            .env("STS2_RUN_ID", "run-executable-composition")
            .env("STS2_EPISODE_ID", "episode-executable-composition")
            .env("STS2_TRAJECTORY_ID", "trajectory-executable-composition")
            .env("STS2_TRACE_ID", "trace-executable-composition")
            .env("STS2_ARTIFACT_ID", "artifact-executable-composition")
            .env("STS2_EXO_REVISION", REVIEWED_EXO_REVISION)
            .env("STS2_PROVIDER_KIND", "synthetic")
            // The synthetic probe is a raw-wire bridge: acknowledge it explicitly instead of
            // claiming the reviewed one-turn envelope admission.
            .env("STS2_EXO_ADMISSION", "legacy")
            .env("STS2_EXO_BRIDGE_BINARY", bridge)
            .env("STS2_EXO_TIMEOUT_MILLIS", "2000")
            .env("STS2_EXO_MAX_REQUEST_BYTES", "131072")
            .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
            .env("STS2_OBJECTIVE", "exercise served policy gate")
            .env("STS2_MAX_STEPS", "4")
            .env("STS2_BARRIER_MAX_POLLS", "1")
            .env("STS2_BARRIER_WAIT_MILLIS", "1")
            .env("STS2_RECOVERY_MAX_ATTEMPTS", "2")
            .env("STS2_RUNTIME_WAIT_FOR_COMBAT_SECONDS", "0")
            .env("STS2_RUNTIME_SETTLEMENT_TIMEOUT_SECONDS", "1")
            .env(
                "STS2_OBJECTIVE",
                "reach the bounded synthetic terminal state",
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        bounded(command)
    })();
    let gateway_output = stop(gateway_process)?;
    Ok(ScenarioResult {
        runtime: runtime?,
        gateway: gateway_output,
        ledger: mod_server.finish(),
    })
}

#[path = "runtime_v4_executable_composition_process/served.rs"]
mod served;
// This support module is compiled into more than one integration-test binary, and each
// binary exercises a different subset of the served scenarios. The re-exports below are the
// shared entry points; a given binary will not use every one of them, so an unused re-export
// in one binary is expected rather than a defect.
#[allow(unused_imports)]
pub(crate) use served::{
    paths, run_served_cancel_after_accepted_barrier, run_served_context_receipt_recovery,
    run_served_context_source_adoption, run_served_peer_acceptance, run_served_policy_gate,
    run_served_policy_rebind_after_idle_adoption, run_served_restart_refuses_duplicate_effect,
};

#[path = "runtime_v4_executable_composition_process/assertions.rs"]
mod assertions;
#[allow(unused_imports)]
pub(crate) use assertions::{assert_foreign_state_rejected, assert_success};

include!("runtime_v4_executable_composition_malformed.rs");

pub(crate) fn write_evidence(
    success: &ScenarioResult,
    foreign: &ScenarioResult,
    operation: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(root) = std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR") else {
        return Ok(());
    };
    let root = PathBuf::from(root);
    fs::create_dir_all(&root)?;
    fs::write(root.join("runtime-success.stdout"), &success.runtime.stdout)?;
    fs::write(root.join("runtime-success.stderr"), &success.runtime.stderr)?;
    fs::write(root.join("gateway-success.stdout"), &success.gateway.stdout)?;
    fs::write(root.join("gateway-success.stderr"), &success.gateway.stderr)?;
    fs::write(root.join("runtime-foreign.stdout"), &foreign.runtime.stdout)?;
    fs::write(root.join("runtime-foreign.stderr"), &foreign.runtime.stderr)?;
    fs::write(root.join("gateway-foreign.stdout"), &foreign.gateway.stdout)?;
    fs::write(root.join("gateway-foreign.stderr"), &foreign.gateway.stderr)?;
    let summary = |result: &ScenarioResult| {
        Value::Array(
            result
                .ledger
                .requests
                .iter()
                .zip(&result.ledger.responses)
                .map(|(request, response)| {
                    json!({"method":request.method,"path":request.path,"operation_id":request.body["operation_id"],"state_id":request.body["state_id"],"generation":request.body["generation"],"status":request.body["status"],"response_status":response.status,"response_operation_id":response.body["operation_id"],"response_state_id":response.body["state_id"],"response_generation":response.body["generation"],"response_status_value":response.body["status"]})
                })
                .collect(),
        )
    };
    fs::write(
        root.join("downstream-success.json"),
        serde_json::to_vec_pretty(&summary(success))?,
    )?;
    fs::write(
        root.join("downstream-foreign.json"),
        serde_json::to_vec_pretty(&summary(foreign))?,
    )?;
    fs::write(
        root.join("result.json"),
        serde_json::to_vec_pretty(
            &json!({"status":"confirmed","scope":"source-derived executable composition","operation_id":operation,"success_exit":success.runtime.status.code(),"foreign_exit":foreign.runtime.status.code(),"provider":"synthetic bounded bridge","game":"synthetic downstream"}),
        )?,
    )?;
    Ok(())
}

/// Persist the gateway's own captured streams for one served scenario, then hand the
/// failure back with those streams attached.
///
/// The `served_*` compositions never call [`write_evidence`], so before this helper the
/// gateway's stderr — the one stream that names a refused request header — was captured
/// by [`stop`], handed to the caller, and then dropped on every served path
/// (sts2-harness#548). Two things follow from that. A reader of a served failure saw a
/// `stderr=` label carrying the *workflow service's* bytes and reasonably concluded the
/// gateway's own refusal had been reported; it never had been. And the lane's
/// *"Show owned-process diagnostics"* step had nothing to print for a served step, because
/// no served step ever named an evidence directory.
///
/// This writes the gateway's streams under `STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR` when
/// that variable is set, and unconditionally attaches them to the error text. It is
/// deliberately a no-op on the write path when the variable is unset so an ordinary run
/// keeps its current behaviour, and it is a separate function from [`write_evidence`]
/// because the served scenarios have no `ScenarioResult` pair and no downstream ledger to
/// summarise: they assert against the synthetic mod ledger inline instead.
pub(crate) fn gateway_failure_evidence(
    label: &str,
    gateway: &Output,
) -> Box<dyn std::error::Error> {
    if std::env::var_os("STS2_TEMP_NEUTER_548").is_some() {
        return format!("{label}").into();
    }
    if let Some(root) = std::env::var_os("STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR") {
        let root = PathBuf::from(root);
        if let Err(error) = write_gateway_streams(&root, label, gateway) {
            return format!(
                "{label}: gateway diagnostics could not be persisted under {}: {error}",
                root.display()
            )
            .into();
        }
    }
    format!(
        "{label}: gateway_stdout={}; gateway_stderr={}",
        String::from_utf8_lossy(&gateway.stdout),
        String::from_utf8_lossy(&gateway.stderr),
    )
    .into()
}

/// Write one served scenario's gateway streams, so the lane's failure-only dump step has
/// bytes to print. Each served step names its own subdirectory, so `label` also keeps two
/// scenarios that share a step — the peer-acceptance step runs four negative cases and the
/// graph lane twice — from overwriting each other.
///
/// `label` arrives as the caller's full failure context, so it carries slashes, newlines and
/// other bytes that are not legal in a file name. The stream pair is therefore written under
/// a sanitised form of `label` that keeps it distinct per scenario but safe as a single path
/// component; the unsanitised text still goes into the error message itself.
fn write_gateway_streams(
    root: &Path,
    label: &str,
    gateway: &Output,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(root)?;
    let stem = sanitize_label(label);
    fs::write(root.join(format!("gateway-{stem}.stdout")), &gateway.stdout)?;
    fs::write(root.join(format!("gateway-{stem}.stderr")), &gateway.stderr)?;
    Ok(())
}

/// Reduce a failure context to a single safe path component.
///
/// Only `[A-Za-z0-9._-]` survive; runs of anything else become a single `_`, and the result is
/// capped so a very long context cannot produce an over-long path component. The label's
/// leading words are the scenario's own stable name (e.g. `wrong-instance`, `graph-changed`),
/// so the cap still leaves the cases that share a lane step distinguishable, which is the
/// collision this guards against.
fn sanitize_label(label: &str) -> String {
    let mut stem = String::with_capacity(label.len().min(120));
    let mut last_was_separator = false;
    for value in label.chars() {
        let keep = value.is_ascii_alphanumeric() || value == '.' || value == '-';
        if keep {
            stem.push(value);
            last_was_separator = false;
        } else if !last_was_separator {
            stem.push('_');
            last_was_separator = true;
        }
        if stem.len() >= 120 {
            break;
        }
    }
    let trimmed = stem.trim_matches('_').to_string();
    if trimmed.is_empty() {
        "gateway".to_string()
    } else {
        trimmed
    }
}
