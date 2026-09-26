// SPDX-License-Identifier: MIT

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
pub(crate) use served::{
    paths, run_served_cancel_after_accepted_barrier, run_served_context_receipt_recovery,
    run_served_context_source_adoption, run_served_peer_acceptance, run_served_policy_gate,
    run_served_policy_rebind_after_idle_adoption, run_served_restart_refuses_duplicate_effect,
};

#[path = "runtime_v4_executable_composition_process/assertions.rs"]
mod assertions;
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

/// The environment variable the workflow sets on each `served_*` step.
const SERVED_EVIDENCE_DIR: &str = "STS2_EXECUTABLE_COMPOSITION_EVIDENCE_DIR";

/// Bounded so a wedged child cannot fill the runner's disk. Matches the capture bound the
/// REST composition evidence writer already uses.
const MAX_GATEWAY_CAPTURE_BYTES: usize = 4 * 1024 * 1024;

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
/// This attaches the gateway's streams to the error **unconditionally** and, when
/// [`SERVED_EVIDENCE_DIR`] is set, additionally persists them so the dump step has bytes.
/// It stays a separate function from [`write_evidence`] because the served scenarios have
/// no `ScenarioResult` pair and no downstream ledger to summarise: they assert against the
/// synthetic mod ledger inline instead.
///
/// It is called from `map_err`, so a **passing** served scenario persists nothing. That is
/// deliberate — the evidence exists to explain a red — but it means an empty evidence
/// directory after a green step means "passed", not "the write never ran".
///
/// `label` is a human-readable scenario name for the message only. It is **never** used to
/// build a path: it embeds child-process output, which routinely contains `/`, newlines and
/// `..`, so using it as a filename would let a child's bytes shape the evidence tree and
/// could walk the write outside the evidence directory. The filename comes from `slug`.
pub(crate) fn gateway_failure_evidence(
    slug: &str,
    label: &str,
    gateway: &Output,
) -> Box<dyn std::error::Error> {
    let mut message = format!(
        "{label}: gateway_stdout={}; gateway_stderr={}",
        String::from_utf8_lossy(&gateway.stdout),
        String::from_utf8_lossy(&gateway.stderr),
    );
    // A persistence failure is *appended*, never substituted for the scenario's own
    // message. Returning early here would demote the CI-relevant failure to a storage
    // complaint and lose the gateway refusal that made this issue worth filing.
    if let Some(root) = std::env::var_os(SERVED_EVIDENCE_DIR) {
        let root = PathBuf::from(root);
        if let Err(error) = write_gateway_streams(&root, slug, gateway) {
            message.push_str(&format!(
                "\n--- gateway peer ({slug}) evidence was NOT persisted under {}: {error} ---",
                root.display()
            ));
        }
    }
    message.into()
}

/// Write one served scenario's gateway streams, so the lane's failure-only dump step has
/// bytes to print.
///
/// `slug` is caller-chosen and must already be restricted to `[A-Za-z0-9._-]`; it is the
/// only caller-supplied value in the path. Each served CI step also names its own evidence
/// subdirectory, and a slug that repeats within one step would overwrite — which is why
/// the graph lane folds `request_id` into its slug, since that lane runs twice per step.
fn write_gateway_streams(
    root: &Path,
    slug: &str,
    gateway: &Output,
) -> Result<(), Box<dyn std::error::Error>> {
    debug_assert!(
        !slug.is_empty()
            && slug
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')),
        "an evidence slug must be a bare filename component, got {slug:?}"
    );
    if gateway.stdout.len() > MAX_GATEWAY_CAPTURE_BYTES
        || gateway.stderr.len() > MAX_GATEWAY_CAPTURE_BYTES
    {
        return Err(format!(
            "gateway capture exceeds the bounded limit of {MAX_GATEWAY_CAPTURE_BYTES} bytes"
        )
        .into());
    }
    fs::create_dir_all(root)?;
    // Private, because this is `env_clear`-ed child output: the REST evidence writer
    // documents the same directory as holding unsanitised child bytes.
    for (name, bytes) in [("stdout", &gateway.stdout), ("stderr", &gateway.stderr)] {
        let path = root.join(format!("gateway-{slug}.{name}"));
        fs::write(&path, bytes)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
