// SPDX-License-Identifier: MIT
#![cfg(target_os = "linux")]

#[path = "worker_server_entry/gateway.rs"]
mod gateway_support;
#[path = "worker_local_linux_support.rs"]
mod support;

use gateway_support::acknowledge_release;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use sts2_harness::worker_bootstrap::BOOTSTRAP_MAGIC;
use sts2_harness::worker_local_linux::AUTH_MAGIC;
use support::{Fixture, TestResult, read_frame, write_auth, write_frame};
use tokio::net::UnixStream;

struct RuntimeChild {
    child: Child,
    reaped: bool,
}

impl RuntimeChild {
    async fn finish(&mut self, success: bool) -> TestResult {
        rustix::process::kill_process(
            rustix::process::Pid::from_child(&self.child),
            rustix::process::Signal::TERM,
        )?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.reaped = true;
                assert_eq!(
                    status.success(),
                    success,
                    "unexpected worker shutdown: {status}"
                );
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("worker did not shut down within five seconds".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn cleanup(&mut self) {
        if !self.reaped {
            // The unreaped direct child pins this test-owned process-group ID.
            let _ = rustix::process::kill_process_group(
                rustix::process::Pid::from_child(&self.child),
                rustix::process::Signal::KILL,
            );
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                if matches!(self.child.try_wait(), Ok(Some(_))) {
                    self.reaped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

impl Drop for RuntimeChild {
    fn drop(&mut self) {
        self.cleanup();
    }
}

struct RuntimeFixture {
    child: RuntimeChild,
    local: Fixture,
}

impl RuntimeFixture {
    fn start() -> TestResult<Self> {
        Self::start_with_gateway(None)
    }

    fn start_with_gateway(gateway: Option<std::net::SocketAddr>) -> TestResult<Self> {
        Self::start_config(gateway, None)
    }

    fn start_config(
        gateway: Option<std::net::SocketAddr>,
        digest_override: Option<&str>,
    ) -> TestResult<Self> {
        let local = Fixture::new(b"synthetic-worker-server-secret")?;
        let binary = local.root.join("worker");
        fs::copy(env!("CARGO_BIN_EXE_sts2-harness-runtime"), &binary)?;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o500))?;
        let mcp = local.root.join("mcp");
        fs::copy("/usr/bin/true", &mcp)?;
        fs::set_permissions(&mcp, fs::Permissions::from_mode(0o500))?;
        let frame = bootstrap()?;
        let mut command = Command::new(binary);
        command
            .env_clear()
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        command.envs([
            ("STS2_GATEWAY_TOKEN", "synthetic-unused-token"),
            ("STS2_WORKER_MODE", "true"),
            ("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay"),
            ("STS2_WORKER_DEPLOYMENT_ID", "watchdog-deployment-1"),
            ("STS2_SEED", "synthetic-seed"),
            ("STS2_WORKER_TIMEOUT_MS", "5000"),
            (
                "STS2_EXO_BRIDGE_BINARY",
                "/nonexistent/synthetic-worker-provider",
            ),
            (
                "STS2_EXO_REVISION",
                "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
            ),
            ("STS2_OBJECTIVE", "synthetic worker control test"),
            ("STS2_RUN_ID", "dddddddd-dddd-4ddd-8ddd-dddddddddddd"),
            ("STS2_EPISODE_ID", "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"),
            ("STS2_TRAJECTORY_ID", "ffffffff-ffff-4fff-8fff-ffffffffffff"),
        ]);
        for (name, digit) in [
            ("STS2_WORKER_PROFILE_DIGEST", '1'),
            ("STS2_BUILD_DIGEST", '2'),
            ("STS2_STATE_DIGEST", '4'),
            ("STS2_PROVIDER_DIGEST", '5'),
        ] {
            command.env(name, digit.to_string().repeat(64));
        }
        command
            .env("STS2_MCP_BINARY", &mcp)
            .env(
                "STS2_RUNTIME_CONFIG_DIGEST",
                match digest_override {
                    Some(digest) => digest.to_owned(),
                    None => runtime_digest(&mcp, gateway)?,
                },
            )
            .env("STS2_WORKER_ENDPOINT", &local.endpoint)
            .env("STS2_WORKER_CREDENTIAL_PATH", &local.credential)
            .env(
                "STS2_EXECUTION_STORE_PATH",
                local.root.join("execution.sqlite3"),
            );
        if let Some(gateway) = gateway {
            command.env("STS2_GATEWAY_ADDR", gateway.to_string());
        }
        let mut child = RuntimeChild {
            child: command.spawn()?,
            reaped: false,
        };
        child
            .child
            .stdin
            .take()
            .ok_or("missing bootstrap pipe")?
            .write_all(&frame)?;
        Ok(Self { child, local })
    }

    async fn wait_endpoint(&mut self) -> TestResult {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !self.local.endpoint.exists() {
            if let Some(status) = self.child.child.try_wait()? {
                self.child.reaped = true;
                let mut diagnostic = String::new();
                if let Some(stderr) = self.child.child.stderr.take() {
                    stderr.take(4096).read_to_string(&mut diagnostic)?;
                }
                return Err(format!("worker exited during startup: {status}; {diagnostic}").into());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("worker endpoint startup deadline expired".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        Ok(())
    }
}

impl Drop for RuntimeFixture {
    fn drop(&mut self) {
        self.child.cleanup();
        if !self.child.reaped {
            return;
        }
        // Remove only this freshly created fixture's known files after reaping.
        for name in [
            "worker",
            "mcp",
            "execution.sqlite3",
            "execution.sqlite3-wal",
            "execution.sqlite3-shm",
        ] {
            let _ = fs::remove_file(self.local.root.join(name));
        }
    }
}

fn runtime_digest(
    mcp: &std::path::Path,
    gateway: Option<std::net::SocketAddr>,
) -> TestResult<String> {
    let image = fs::read(mcp)?;
    // Independent fixture for the existing runtime configuration fingerprint;
    // the approved job lineage is configured before dispatch, not bypassed.
    let policy = json!({
        "runtime_profile": "runtime-v3-gameplay",
        "gateway_address": gateway.map_or_else(|| "127.0.0.1:15525".to_owned(), |value| value.to_string()),
        "mcp_binary": mcp,
        "mcp_executable": {"path": mcp, "sha256": format!("{:x}", Sha256::digest(&image)), "bytes": image.len()},
        "instance_id": "instance-1", "caller_id": "harness", "session_id": "session-1",
        "lease_id": "lease-1", "lease_epoch": 1, "mcp_session_id": "mcp-session-1",
        "run_id": "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
        "episode_id": "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee",
        "trajectory_id": "ffffffff-ffff-4fff-8fff-ffffffffffff",
        "trace_id": "trace-runtime-0001", "artifact_id": "artifact-runtime-0001",
        "settlement_timeout_seconds": 30,
        "exo_revision": "7801005e6a1ab77008a05dbba80e0a2a7a56e35d",
        "exo_max_request_bytes": 131072, "exo_max_response_bytes": 8192,
        "exo_timeout_millis": 120000, "exo_forward_visible_seed": true,
        "exo_bridge": {"executable": "/nonexistent/synthetic-worker-provider", "arguments": [], "working_directory": null, "inherited_environment": []},
        "runner": {"max_steps": 1024, "objective": "synthetic worker control test", "hard_constraints": []}
    });
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&policy)?)
    ))
}

fn bootstrap() -> TestResult<Vec<u8>> {
    let executable = std::env::current_exe()?;
    let stat = fs::read_to_string(format!("/proc/{}/stat", std::process::id()))?;
    let close = stat.rfind(')').ok_or("process stat terminator")?;
    let creation = stat
        .get(close + 2..)
        .and_then(|suffix| suffix.split_whitespace().nth(19))
        .ok_or("process start token")?;
    let bytes = serde_json::to_vec(&json!({
        "version": 1,
        "launch_nonce": "cccccccc-cccc-4ccc-8ccc-cccccccccccc",
        "watchdog_boot_id": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
        "component_id": "harness",
        "expected_peer": {
            "platform": "linux", "pid": std::process::id(), "creation_token": creation,
            "executable": executable,
            "executable_sha256": format!("{:x}", Sha256::digest(fs::read(&executable)?)),
            "uid": rustix::process::getuid().as_raw(), "gid": rustix::process::getgid().as_raw()
        }
    }))?;
    let mut frame = BOOTSTRAP_MAGIC.to_vec();
    frame.extend_from_slice(&u32::try_from(bytes.len())?.to_be_bytes());
    frame.extend_from_slice(&bytes);
    Ok(frame)
}

async fn exchange(fixture: &RuntimeFixture, request: &Value) -> TestResult<Value> {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut client = UnixStream::connect(&fixture.local.endpoint).await?;
        write_auth(&mut client, &fixture.local.secret, AUTH_MAGIC).await?;
        write_frame(&mut client, &serde_json::to_vec(request)?).await?;
        let reply: Value = serde_json::from_slice(&read_frame(&mut client).await?)?;
        assert_eq!(reply["request_id"], request["request_id"]);
        Ok(reply)
    })
    .await?
}

#[tokio::test]
async fn invalid_runtime_policy_never_binds_the_worker_endpoint() -> TestResult {
    let mut fixture = RuntimeFixture::start_config(None, Some(&"0".repeat(64)))?;
    let error = fixture
        .wait_endpoint()
        .await
        .err()
        .ok_or("invalid policy opened an endpoint")?;
    assert!(
        error
            .to_string()
            .contains("approved fingerprint does not match the runtime configuration")
    );
    assert!(!fixture.local.endpoint.exists());
    assert!(fixture.child.reaped);
    Ok(())
}

#[tokio::test]
async fn executable_worker_authenticates_control_and_persists_stop_before_shutdown() -> TestResult {
    let mut fixture = RuntimeFixture::start()?;
    fixture.wait_endpoint().await?;
    // A credential-only failed exchange cannot kill the command loop.
    {
        let mut client = UnixStream::connect(&fixture.local.endpoint).await?;
        write_auth(&mut client, b"wrong-credential", AUTH_MAGIC).await?;
    }
    let probe: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/probe-request.json"
    ))?;
    let initial = exchange(&fixture, &probe).await?;
    assert_eq!(initial["ready"], false);
    let mut control: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/control-request.json"
    ))?;
    control["worker_boot_id"] = initial["worker_boot_id"].clone();
    control["timeout_ms"] = json!(5000);
    exchange(&fixture, &control).await?;
    assert_eq!(exchange(&fixture, &probe).await?["ready"], true);
    control["mode"] = json!("stopped");
    control["mode_sequence"] = json!(2);
    control["request_id"] = json!("dddddddd-dddd-4ddd-8ddd-dddddddddddd");
    exchange(&fixture, &control).await?;
    assert_eq!(exchange(&fixture, &probe).await?["ready"], false);
    fixture.child.finish(true).await?;
    assert!(!fixture.local.endpoint.exists());
    let store = rusqlite::Connection::open_with_flags(
        fixture.local.root.join("execution.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mode: String = store.query_row("SELECT mode FROM worker_control", [], |row| row.get(0))?;
    assert_eq!(mode, "stopped");
    Ok(())
}

#[tokio::test]
async fn executable_control_remains_available_during_owned_gateway_execution() -> TestResult {
    stalled_gateway_cancellation(true).await
}

#[tokio::test]
async fn executable_sigterm_cancels_owned_gateway_execution() -> TestResult {
    stalled_gateway_cancellation(false).await
}

async fn stalled_gateway_cancellation(operator_stop: bool) -> TestResult {
    let gateway = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let mut fixture = RuntimeFixture::start_with_gateway(Some(gateway.local_addr()?))?;
    fixture.wait_endpoint().await?;
    let probe: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/probe-request.json"
    ))?;
    let initial = exchange(&fixture, &probe).await?;
    let mut control: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/control-request.json"
    ))?;
    control["worker_boot_id"] = initial["worker_boot_id"].clone();
    control["timeout_ms"] = json!(5000);
    exchange(&fixture, &control).await?;
    let mut dispatch: Value = serde_json::from_slice(include_bytes!(
        "../../../protocol-artifact/watchdog-worker-v1/fixtures/valid/dispatch.json"
    ))?;
    dispatch["worker_boot_id"] = initial["worker_boot_id"].clone();
    let admitted = exchange(&fixture, &dispatch).await?;
    assert_eq!(admitted["status"], "accepted");
    let (blocked_allocation, _) = tokio::time::timeout(Duration::from_secs(5), gateway.accept())
        .await
        .map_err(|_| "owned execution did not reach the synthetic gateway")??;
    let began = tokio::time::Instant::now();
    if operator_stop {
        control["mode"] = json!("stopped");
        control["mode_sequence"] = json!(2);
        control["request_id"] = json!("dddddddd-dddd-4ddd-8ddd-dddddddddddd");
        exchange(&fixture, &control).await?;
        assert_eq!(exchange(&fixture, &probe).await?["ready"], false);
        // No gateway response was sent. Control completed before the runtime's
        // five-second HTTP deadline, while the actual execution thread owned I/O.
        assert!(began.elapsed() < Duration::from_secs(5));
        acknowledge_release(&gateway).await?;
        let store = rusqlite::Connection::open_with_flags(
            fixture.local.root.join("execution.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        loop {
            let state: String =
                store.query_row("SELECT state FROM worker_handoffs", [], |row| row.get(0))?;
            if state == "unknown" {
                break;
            }
            if began.elapsed() >= Duration::from_secs(3) {
                return Err("operator stop did not durably retain uncertainty".into());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(began.elapsed() < Duration::from_secs(3));
        assert!(fixture.child.child.try_wait()?.is_none());
    }
    // Keep allocation open in both cases. Neither stop nor SIGTERM may depend
    // on a remote close or the five-second allocation timeout.
    let shutdown_started = tokio::time::Instant::now();
    if operator_stop {
        fixture.child.finish(false).await?;
    } else {
        tokio::try_join!(fixture.child.finish(false), acknowledge_release(&gateway))?;
    }
    assert!(shutdown_started.elapsed() < Duration::from_secs(3));
    drop(blocked_allocation);
    drop(gateway);
    let store = rusqlite::Connection::open_with_flags(
        fixture.local.root.join("execution.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let state: String =
        store.query_row("SELECT state FROM worker_handoffs", [], |row| row.get(0))?;
    let mode: String = store.query_row("SELECT mode FROM worker_control", [], |row| row.get(0))?;
    assert_eq!(state, "unknown");
    assert_eq!(mode, if operator_stop { "stopped" } else { "running" });
    Ok(())
}
