// SPDX-License-Identifier: MIT

use std::error::Error;
use std::io::{ErrorKind, Read};
use std::net::SocketAddr;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};
use sts2_harness::management::ManagementClient;

use super::{
    CALLER_ID, EXO_REVISION, INSTANCE_ID, LEASE_EPOCH, LEASE_ID, MCP_SESSION_ID, PreparedExo,
    PrivateRoot, SESSION_ID, TestKeys, bounded_file_digest, context_owner_config,
};

pub(crate) type Result<T = (), E = Box<dyn Error>> = std::result::Result<T, E>;

const CHILD_DEADLINE: Duration = Duration::from_secs(5);
const SETTLEMENT_DEADLINE: Duration = Duration::from_secs(3);
const TERM_GRACE: Duration = Duration::from_millis(500);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) struct OwnedService {
    child: Option<Child>,
    process_group: Pid,
    settlement_deadline: Option<Instant>,
}

impl OwnedService {
    pub(crate) fn spawn(command: &mut Command) -> Result<Self> {
        command.stdin(Stdio::null()).process_group(0);
        let child = command.spawn()?;
        let process_group = owned_process_group(child.id())?;
        Ok(Self {
            child: Some(child),
            process_group,
            settlement_deadline: None,
        })
    }

    pub(crate) fn child_mut(&mut self) -> Result<&mut Child> {
        self.child
            .as_mut()
            .ok_or_else(|| "owned child is already settled".into())
    }

    pub(crate) fn settle(&mut self) -> Result<()> {
        let deadline = match self.settlement_deadline {
            Some(deadline) => deadline,
            None => {
                let deadline = Instant::now()
                    .checked_add(SETTLEMENT_DEADLINE)
                    .ok_or("owned process settlement deadline is invalid")?;
                self.settlement_deadline = Some(deadline);
                deadline
            }
        };
        let child = self
            .child
            .as_mut()
            .ok_or("owned child settlement was already recorded")?;
        settle_group(child, self.process_group, deadline)?;
        drop(self.child.take());
        Ok(())
    }
}

impl Drop for OwnedService {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.settle();
        }
    }
}

pub(crate) fn service_command(bindings: ServiceBindings<'_>) -> Result<Command> {
    let ServiceBindings {
        root,
        exo,
        keys,
        policy,
        mcp,
        gateway,
        workflow,
        run_id,
    } = bindings;
    let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
    command
        .arg("serve-workflow")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("STS2_WORKFLOW_LISTEN", workflow.to_string())
        .env("STS2_WORKFLOW_STORE", root.path.join("workflow.sqlite3"))
        .env("STS2_WORKFLOW_AUTH_PROFILE", "synthetic")
        .env("STS2_WORKFLOW_TOKEN_SYNTHETIC", "synthetic-test-token")
        .env("STS2_WORKFLOW_PROVIDER_POLICY_CONFIG", policy)
        .env("STS2_SERVED_PROVIDER_POLICY_KEY", keys.policy_hex())
        .env(
            "STS2_WORKFLOW_CONTEXT_OWNER_CONFIG",
            context_owner_config(root)?,
        )
        .env("STS2_SERVED_CONTEXT_OWNER_KEY", keys.context_hex())
        .env(
            "STS2_EXECUTION_STORE_PATH",
            root.path.join("execution.sqlite3"),
        )
        .env("STS2_GATEWAY_ADDR", gateway.to_string())
        .env("STS2_GATEWAY_TOKEN", "gateway-token")
        .env("STS2_MCP_BINARY", mcp)
        .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
        .env("STS2_INSTANCE_ID", INSTANCE_ID)
        .env("STS2_CALLER_ID", CALLER_ID)
        .env("STS2_SESSION_ID", SESSION_ID)
        .env("STS2_MCP_SESSION_ID", MCP_SESSION_ID)
        .env("STS2_LEASE_ID", LEASE_ID)
        .env("STS2_LEASE_EPOCH", LEASE_EPOCH.to_string())
        .env("STS2_EPISODE_ID", "fixture.episode")
        .env("STS2_RUN_ID", run_id)
        .env("STS2_TRAJECTORY_ID", "fixture-trajectory")
        .env("STS2_TRACE_ID", "fixture-trace")
        .env("STS2_ARTIFACT_ID", "fixture-artifact")
        .env("STS2_PROVIDER_KIND", "exo")
        .env("STS2_EXO_ADMISSION", "synthetic-envelope")
        .env("STS2_EXO_REVISION", EXO_REVISION)
        .env("STS2_EXO_PACKAGE_PATH", &exo.executor)
        .env(
            "STS2_EXO_PACKAGE_DIGEST",
            &exo.capabilities.native_binary_sha256,
        )
        .env("STS2_EXO_EXTENSION_DIGEST", &exo.extension_digest)
        .env(
            "STS2_EXO_BRIDGE_DIGEST",
            bounded_file_digest(&exo.bridge, super::MAX_EXO_BRIDGE_BYTES, true)?,
        )
        .env("STS2_EXO_MODEL_BINDING", "o3-pro")
        .env("STS2_EXO_PROVIDER", "openai")
        .env("STS2_EXO_ENDPOINT", &exo.endpoint)
        .env("STS2_EXO_PROMPT_DIGEST", &exo.extension_digest)
        .env(
            "STS2_EXO_TOOL_DIGEST",
            sts2_harness::ExoToolCatalog::reviewed().catalog_digest(),
        )
        .env("STS2_EXO_CONFIG_DIGEST", &exo.config_digest)
        .env("STS2_EXO_NATIVE_INSTANCE_ID", INSTANCE_ID)
        .env("STS2_EXO_MODEL_EXECUTION_ID", "fixture-model-execution")
        .env("STS2_EXO_REQUEST_ID", "fixture-provider-request")
        .env("STS2_EXO_TURN_ID", "fixture-provider-turn")
        .env("STS2_EXO_PRIVATE_STATE_ROOT", &exo.private_state_root)
        .env("STS2_EXO_BRIDGE_BINARY", &exo.bridge)
        .env(
            "STS2_EXO_BRIDGE_ARGS_JSON",
            serde_json::to_string(&[
                "--synthetic",
                exo.config_path.to_str().ok_or("config path is not UTF-8")?,
                &exo.config_digest,
            ])?,
        )
        .env("STS2_EXO_INHERITED_ENV_JSON", "[]")
        .env("STS2_EXO_TIMEOUT_MILLIS", "15000")
        .env("STS2_EXO_MAX_REQUEST_BYTES", "131072")
        .env("STS2_EXO_MAX_RESPONSE_BYTES", "8192")
        .env("STS2_ENABLE_GAME_INFORMATION_LOOKUP_BINDING", "false")
        .env("STS2_ENABLE_MAP_CONTEXT", "false")
        .env("STS2_OBJECTIVE", "one synthetic decision; no game action")
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    Ok(command)
}

pub(crate) struct ServiceBindings<'a> {
    pub(crate) root: &'a PrivateRoot,
    pub(crate) exo: &'a PreparedExo,
    pub(crate) keys: &'a TestKeys,
    pub(crate) policy: &'a str,
    pub(crate) mcp: &'a std::path::Path,
    pub(crate) gateway: SocketAddr,
    pub(crate) workflow: SocketAddr,
    pub(crate) run_id: &'a str,
}

pub(crate) fn wait_for_service(
    service: &mut OwnedService,
    address: SocketAddr,
) -> Result<ManagementClient> {
    let client = ManagementClient::new(address, "synthetic-test-token")?;
    let deadline = Instant::now()
        .checked_add(Duration::from_secs(8))
        .ok_or("served readiness deadline is invalid")?;
    loop {
        if service.child_mut()?.try_wait()?.is_some() {
            return Err("served workflow exited before HTTP readiness".into());
        }
        if client
            .request_json("GET", "/v1/health", None)
            .is_ok_and(|response| response.status == 200)
        {
            return Ok(client);
        }
        if Instant::now() >= deadline {
            return Err("served workflow readiness deadline elapsed".into());
        }
        sleep_bounded(deadline);
    }
}

pub(crate) fn bounded_child_output(
    mut command: Command,
    maximum: usize,
) -> Result<(ExitStatus, Vec<u8>)> {
    let deadline = Instant::now()
        .checked_add(CHILD_DEADLINE)
        .ok_or("child execution deadline is invalid")?;
    command.stdout(Stdio::piped()).stderr(Stdio::null());
    let mut process = OwnedService::spawn(&mut command)?;
    let Some(mut stdout) = process.child_mut()?.stdout.take() else {
        process.settle()?;
        return Err("child_stdout_missing".into());
    };
    let flags = match rustix::fs::fcntl_getfl(&stdout) {
        Ok(flags) => flags,
        Err(_) => {
            drop(stdout);
            process.settle()?;
            return Err("child_stdout_flags_unavailable".into());
        }
    };
    if rustix::fs::fcntl_setfl(&stdout, flags | rustix::fs::OFlags::NONBLOCK).is_err() {
        drop(stdout);
        process.settle()?;
        return Err("child_stdout_nonblocking_refused".into());
    }

    let mut output = Vec::with_capacity(maximum.min(64 * 1024));
    let mut direct_status = None;
    let mut eof = false;
    let mut failure = None;
    while direct_status.is_none() || !eof {
        if !eof {
            let room = maximum
                .saturating_sub(output.len())
                .saturating_add(1)
                .min(4096);
            let mut buffer = [0_u8; 4096];
            match stdout.read(&mut buffer[..room.max(1)]) {
                Ok(0) => eof = true,
                Ok(read) if output.len().saturating_add(read) <= maximum => {
                    output.extend_from_slice(&buffer[..read]);
                }
                Ok(_) => {
                    failure = Some("child_stdout_output_limit");
                    break;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {}
                Err(_) => {
                    failure = Some("child_stdout_read_failed");
                    break;
                }
            }
        }
        if direct_status.is_none() {
            match process.child_mut()?.try_wait() {
                Ok(status) => direct_status = status,
                Err(_) => {
                    failure = Some("child_wait_observation_failed");
                    break;
                }
            }
        }
        if Instant::now() >= deadline {
            failure = Some("child_execution_deadline_elapsed");
            break;
        }
        if direct_status.is_none() || !eof {
            sleep_bounded(deadline);
        }
    }
    drop(stdout);

    let descendants_remained = match group_exists(process.process_group) {
        Ok(alive) => alive,
        Err(_) => {
            process.settle()?;
            return Err("child_group_observation_failed".into());
        }
    };
    process.settle()?;
    if let Some(code) = failure {
        return Err(format!("{code}; owned group settled=true").into());
    }
    if descendants_remained {
        return Err("child_exited_with_owned_descendants; owned group was settled".into());
    }
    Ok((direct_status.ok_or("child_exit_status_missing")?, output))
}

fn settle_group(child: &mut Child, process_group: Pid, deadline: Instant) -> Result<()> {
    if Instant::now() >= deadline {
        return Err("owned_process_group_settlement_deadline_elapsed".into());
    }
    let mut cleanup_failure = None;
    match group_exists(process_group) {
        Ok(true) => retain_failure(
            &mut cleanup_failure,
            signal_group(process_group, Signal::TERM),
        ),
        Ok(false) => {}
        Err(_) => {
            cleanup_failure = Some("owned_process_group_observation_refused");
            retain_failure(
                &mut cleanup_failure,
                signal_group(process_group, Signal::TERM),
            );
        }
    }
    let grace_deadline = Instant::now()
        .checked_add(TERM_GRACE)
        .ok_or("owned process grace deadline is invalid")?
        .min(deadline);
    while Instant::now() < grace_deadline {
        if child.try_wait().is_err() {
            retain_code(&mut cleanup_failure, "owned_child_reap_observation_refused");
        }
        match group_exists(process_group) {
            Ok(false) => break,
            Ok(true) => {}
            Err(_) => retain_code(
                &mut cleanup_failure,
                "owned_process_group_observation_refused",
            ),
        }
        sleep_bounded(grace_deadline);
    }
    match group_exists(process_group) {
        Ok(true) | Err(_) => {
            if group_exists(process_group).is_err() {
                retain_code(
                    &mut cleanup_failure,
                    "owned_process_group_observation_refused",
                );
            }
            retain_failure(
                &mut cleanup_failure,
                signal_group(process_group, Signal::KILL),
            );
        }
        Ok(false) => {}
    }

    let mut status = None;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(observed) => status = observed,
                Err(_) => retain_code(&mut cleanup_failure, "owned_child_reap_observation_refused"),
            }
        }
        match group_exists(process_group) {
            Ok(false) if status.is_some() => {
                return cleanup_failure.map_or(Ok(()), |code| Err(code.into()));
            }
            Ok(false) | Ok(true) => {}
            Err(_) => retain_code(
                &mut cleanup_failure,
                "owned_process_group_observation_refused",
            ),
        }
        if Instant::now() >= deadline {
            return Err(cleanup_failure
                .unwrap_or("owned_process_group_settlement_deadline_elapsed")
                .into());
        }
        sleep_bounded(deadline);
    }
}

fn sleep_bounded(deadline: Instant) {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        thread::sleep(remaining.min(POLL_INTERVAL));
    }
}

fn owned_process_group(process_group: u32) -> Result<Pid> {
    let raw = i32::try_from(process_group).map_err(|_| "owned_process_group_id_out_of_range")?;
    let current =
        i32::try_from(std::process::id()).map_err(|_| "current_process_id_out_of_range")?;
    if raw <= 1 || raw == current {
        return Err("owned_process_group_id_out_of_range".into());
    }
    Pid::from_raw(raw).ok_or_else(|| "owned_process_group_id_out_of_range".into())
}

fn group_exists(process_group: Pid) -> Result<bool> {
    match test_kill_process_group(process_group) {
        Ok(()) => Ok(true),
        Err(Errno::SRCH) => Ok(false),
        Err(_) => Err("owned_process_group_observation_refused".into()),
    }
}

fn signal_group(process_group: Pid, signal: Signal) -> std::result::Result<(), &'static str> {
    match kill_process_group(process_group, signal) {
        Ok(()) | Err(Errno::SRCH) => Ok(()),
        Err(_) => Err("owned_process_group_signal_refused"),
    }
}

fn retain_code(slot: &mut Option<&'static str>, code: &'static str) {
    if slot.is_none() {
        *slot = Some(code);
    }
}

fn retain_failure(slot: &mut Option<&'static str>, result: std::result::Result<(), &'static str>) {
    if let Err(code) = result {
        retain_code(slot, code);
    }
}
