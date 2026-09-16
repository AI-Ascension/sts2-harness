// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use std::fs;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sts2_harness::{ExecutionFingerprint, ExecutionLineage};

const EXO_REVISION: &str = "b06869ab789dee3f80ca474b5fa89dbe47ccb859";
const RUN_ID: &str = "run-runtime-startup-hostile";
const EPISODE_ID: &str = "episode-runtime-startup-hostile";
const ATTEMPT_ID: &str = "attempt-runtime-startup-hostile";
const TRAJECTORY_ID: &str = "trajectory-runtime-startup-hostile";

#[path = "support/completed_resume_process_support.rs"]
mod process_support;

#[path = "support/runtime_startup_helpers.rs"]
mod helpers;

use process_support::run_child;

use helpers::{
    assert_failure_contains, assert_hostile_startup_is_bounded, digest_bytes, fingerprint,
    seed_hostile_operation, seed_matching_digest_malformed_operation,
    seed_mismatched_action_digest_operation, write_probe,
};

struct Fixture {
    root: PathBuf,
    store: PathBuf,
    mcp: PathBuf,
    bridge: PathBuf,
    package: PathBuf,
    package_digest: String,
    counter: PathBuf,
    gateway: TcpListener,
    gateway_address: String,
    lineage: ExecutionLineage,
    fingerprint: ExecutionFingerprint,
}

impl Fixture {
    fn new() -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("system clock is before epoch: {error}"))?
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "sts2-runtime-startup-hostile-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root)
            .map_err(|error| format!("cannot create fixture directory: {error}"))?;
        let store = root.join("execution.sqlite3");
        let mcp = root.join("mcp-probe.sh");
        let bridge = root.join("provider-probe.sh");
        let package = root.join("package-artifact");
        let counter = root.join("boundary-calls.log");
        let gateway = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|error| format!("cannot bind gateway probe: {error}"))?;
        gateway
            .set_nonblocking(true)
            .map_err(|error| format!("cannot configure gateway probe: {error}"))?;
        let gateway_address = gateway
            .local_addr()
            .map_err(|error| format!("cannot read gateway probe address: {error}"))?
            .to_string();
        write_probe(&mcp, "mcp", &counter)?;
        write_probe(&bridge, "provider", &counter)?;
        let package_bytes = b"runtime-startup-package-artifact";
        fs::write(&package, package_bytes)
            .map_err(|error| format!("cannot write the package artifact probe: {error}"))?;
        let package_digest = digest_bytes(package_bytes);
        let lineage = ExecutionLineage::new(RUN_ID, EPISODE_ID, ATTEMPT_ID, TRAJECTORY_ID)
            .map_err(|error| format!("fixture lineage is invalid: {error}"))?;
        let fingerprint = fingerprint(&mcp, &bridge, &gateway_address)?;
        Ok(Self {
            root,
            store,
            mcp,
            bridge,
            package,
            package_digest,
            counter,
            gateway,
            gateway_address,
            lineage,
            fingerprint,
        })
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sts2-harness-runtime"));
        command
            .env_clear()
            .arg("--resume")
            .env("PATH", "/usr/bin:/bin")
            .env("STS2_RUNTIME_PROFILE", "runtime-v3-gameplay")
            .env("STS2_GATEWAY_ADDR", &self.gateway_address)
            .env("STS2_GATEWAY_TOKEN", "test-token")
            .env("STS2_MCP_BINARY", &self.mcp)
            .env("STS2_INSTANCE_ID", "instance-runtime-startup-hostile")
            .env("STS2_CALLER_ID", "caller-runtime-startup-hostile")
            .env("STS2_SESSION_ID", "session-runtime-startup-hostile")
            .env("STS2_LEASE_ID", "lease-runtime-startup-hostile")
            .env("STS2_LEASE_EPOCH", "1")
            .env("STS2_MCP_SESSION_ID", "mcp-runtime-startup-hostile")
            .env("STS2_RUN_ID", RUN_ID)
            .env("STS2_EPISODE_ID", EPISODE_ID)
            .env("STS2_ATTEMPT_ID", ATTEMPT_ID)
            .env("STS2_TRAJECTORY_ID", TRAJECTORY_ID)
            .env("STS2_TRACE_ID", "trace-runtime-startup-hostile")
            .env("STS2_ARTIFACT_ID", "artifact-runtime-startup-hostile")
            .env("STS2_EXECUTION_STORE_PATH", &self.store)
            .env("STS2_SEED", "seed-runtime-startup-hostile")
            .env("STS2_BUILD_DIGEST", "build-runtime-startup-hostile")
            .env("STS2_STATE_DIGEST", "state-runtime-startup-hostile")
            .env("STS2_EXO_REVISION", EXO_REVISION)
            // These fixtures use raw-wire process probes, which cannot accept the versioned
            // envelope, so they carry the explicit un-admitted acknowledgement.
            .env("STS2_EXO_ADMISSION", "legacy")
            .env("STS2_EXO_BRIDGE_BINARY", &self.bridge)
            .env("STS2_EXO_FORWARD_VISIBLE_SEED", "true")
            .env("STS2_OBJECTIVE", "complete the test episode");
        command
    }

    fn command_with_reviewed_admission_identity(&self) -> Command {
        let mut command = self.command();
        command
            .env_remove("STS2_EXO_ADMISSION")
            .env("STS2_EXO_PACKAGE_PATH", &self.package)
            .env("STS2_EXO_PACKAGE_DIGEST", "a".repeat(64))
            .env("STS2_EXO_EXTENSION_DIGEST", "b".repeat(64))
            .env("STS2_EXO_BRIDGE_DIGEST", "c".repeat(64))
            .env("STS2_EXO_MODEL_BINDING", "gpt-5-pro")
            .env("STS2_EXO_PROVIDER", "openai")
            .env("STS2_EXO_ENDPOINT", "https://api.openai.com/v1")
            .env("STS2_EXO_PROMPT_DIGEST", "d".repeat(64))
            .env("STS2_EXO_TOOL_DIGEST", "e".repeat(64))
            .env("STS2_EXO_CONFIG_DIGEST", "f".repeat(64))
            .env("STS2_EXO_NATIVE_INSTANCE_ID", "native-startup-instance")
            .env("STS2_EXO_MODEL_EXECUTION_ID", "execution-startup")
            .env("STS2_EXO_REQUEST_ID", "request-runtime-startup-hostile")
            .env("STS2_EXO_TURN_ID", "turn-runtime-startup-hostile");
        command
    }

    /// The same reviewed envelope, but with the package pin set to the digest of the exact artifact
    /// the fixture placed at `STS2_EXO_PACKAGE_PATH`.
    fn command_with_matching_package_identity(&self) -> Command {
        let mut command = self.command_with_reviewed_admission_identity();
        command.env("STS2_EXO_PACKAGE_DIGEST", &self.package_digest);
        command
    }

    fn assert_no_gateway_connection(&self) -> Result<(), String> {
        let mut attempts = 0_u32;
        let deadline = Instant::now() + Duration::from_millis(100);
        loop {
            match self.gateway.accept() {
                Ok((_stream, _address)) => attempts = attempts.saturating_add(1),
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(format!("gateway probe failed: {error}")),
            }
        }
        if attempts == 0 {
            Ok(())
        } else {
            Err(format!("runtime made {attempts} gateway TCP connection(s)"))
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn malformed_persisted_operation_fails_before_gateway_mcp_or_provider_calls() -> Result<(), String>
{
    let fixture = Fixture::new()?;
    seed_hostile_operation(&fixture)?;
    assert_hostile_startup_is_bounded(&fixture)
}

#[test]
fn matching_digest_malformed_operation_fails_before_gateway_mcp_or_provider_calls()
-> Result<(), String> {
    let fixture = Fixture::new()?;
    seed_matching_digest_malformed_operation(&fixture)?;
    assert_hostile_startup_is_bounded(&fixture)
}

#[test]
fn mismatched_action_digest_fails_before_gateway_mcp_or_provider_calls() -> Result<(), String> {
    let fixture = Fixture::new()?;
    seed_mismatched_action_digest_operation(&fixture)?;
    assert_hostile_startup_is_bounded(&fixture)
}

#[test]
fn missing_episode_resume_fails_before_gateway_mcp_or_provider_calls() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let output = run_child(fixture.command())?;
    assert_failure_contains(&output, "resume requested but no durable episode exists")?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() {
        return Err(String::from(
            "missing-state resume invoked an MCP or provider boundary",
        ));
    }
    Ok(())
}

#[test]
fn refused_exo_preflight_fails_before_gateway_mcp_or_provider_calls() -> Result<(), String> {
    const REFUSAL_PREFIX: &str = "Exo admission refused before any model or game effect";
    let fixture = Fixture::new()?;
    let output = run_child(fixture.command_with_reviewed_admission_identity())?;
    assert_failure_contains(&output, REFUSAL_PREFIX)?;
    // Admission inspects the package artifact at the locator, so a pin that does not match those
    // bytes is refused as an identity mismatch.
    assert_failure_contains(
        &output,
        "advertised Exo package_digest differs from the operator-trusted pin",
    )?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() {
        return Err(String::from(
            "a refused Exo preflight invoked an MCP or provider boundary",
        ));
    }
    Ok(())
}

/// A swapped package artifact must be refused as `IdentityMismatch("package_digest")` at the
/// production seam, before the runtime reaches the gateway, MCP or provider boundary.
#[test]
fn swapped_package_bytes_fail_before_gateway_mcp_or_provider_calls() -> Result<(), String> {
    const REFUSAL_PREFIX: &str = "Exo admission refused before any model or game effect";
    let fixture = Fixture::new()?;
    // Pin the digest of the artifact as it stands, then swap the bytes behind the locator.
    let command = fixture.command_with_matching_package_identity();
    fs::write(&fixture.package, b"swapped package bytes")
        .map_err(|error| format!("cannot swap the package artifact: {error}"))?;
    let output = run_child(command)?;
    assert_failure_contains(&output, REFUSAL_PREFIX)?;
    assert_failure_contains(
        &output,
        "advertised Exo package_digest differs from the operator-trusted pin",
    )?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() {
        return Err(String::from(
            "a swapped package artifact reached an MCP or provider boundary",
        ));
    }
    Ok(())
}

/// A missing or empty package locator must fail closed rather than admit the operator's
/// declaration with no inspected bytes behind the package axis.
#[test]
fn an_absent_or_empty_package_locator_fails_before_gateway_mcp_or_provider_calls()
-> Result<(), String> {
    for (label, value) in [("absent", None), ("empty", Some(""))] {
        let fixture = Fixture::new()?;
        let mut command = fixture.command_with_reviewed_admission_identity();
        match value {
            Some(value) => {
                command.env("STS2_EXO_PACKAGE_PATH", value);
            }
            None => {
                command.env_remove("STS2_EXO_PACKAGE_PATH");
            }
        }
        let output = run_child(command)?;
        let expected = match value {
            Some(_) => "STS2_EXO_PACKAGE_PATH must not be empty",
            None => "STS2_EXO_PACKAGE_PATH is required",
        };
        assert_failure_contains(&output, expected).map_err(|error| format!("{label}: {error}"))?;
        fixture.assert_no_gateway_connection()?;
        if fixture.counter.exists() {
            return Err(format!(
                "a {label} package locator invoked an MCP or provider boundary"
            ));
        }
    }
    Ok(())
}

/// Matching package bytes alone cannot authorize an uninspected bridge configuration.
#[test]
fn a_matching_package_binding_requires_the_actual_launch_configuration() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let mut command = fixture.command_with_matching_package_identity();
    let bridge_bytes = fs::read(&fixture.bridge).map_err(|error| error.to_string())?;
    command.env(
        "STS2_EXO_BRIDGE_DIGEST",
        sts2_harness::sha256_hex(bridge_bytes),
    );
    let output = run_child(command)?;
    assert_failure_contains(
        &output,
        "Exo envelope requires --run, absolute configuration path and digest",
    )?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() {
        return Err(String::from(
            "an uninspected configuration invoked an MCP or provider boundary",
        ));
    }
    Ok(())
}

#[test]
fn relative_bridge_cannot_change_identity_through_working_directory() -> Result<(), String> {
    let fixture = Fixture::new()?;
    let workdir = fixture.root.join("different-launch-directory");
    fs::create_dir(&workdir).map_err(|error| error.to_string())?;
    write_probe(
        &workdir.join("provider-probe.sh"),
        "different-provider",
        &fixture.counter,
    )?;
    let inspected = fs::read(&fixture.bridge).map_err(|error| error.to_string())?;
    let mut command = fixture.command_with_matching_package_identity();
    command
        .current_dir(&fixture.root)
        .env("STS2_EXO_BRIDGE_BINARY", "./provider-probe.sh")
        .env("STS2_EXO_BRIDGE_WORKDIR", &workdir)
        .env("STS2_EXO_BRIDGE_DIGEST", digest_bytes(&inspected));
    let output = run_child(command)?;
    assert_failure_contains(
        &output,
        "Exo envelope requires an absolute bridge executable path",
    )?;
    fixture.assert_no_gateway_connection()?;
    if fixture.counter.exists() || fixture.store.exists() {
        return Err("relative bridge refusal reached a durable or process boundary".to_owned());
    }
    Ok(())
}
