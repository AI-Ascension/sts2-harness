// SPDX-License-Identifier: MIT

//! Writer-side back-pressure and budget helpers for the `#148` bound/budget oracle.
//!
//! Two modules already sit beside this one. `support/mod.rs` is byte-pinned by the 20260917
//! process-evidence record, and `support/fault.rs` carries the `#488` fault matrix's model and
//! process driver. Neither can express what this lane measures: a writer that keeps offering bytes
//! until the *peer* stops reading, and a loopback endpoint whose reply and timing the test
//! controls, so the executor's own turn deadline can abort a turn mid-flight.
//!
//! The distinction these cases must not blur: `process_oracle.rs`'s `oversized_input` writes
//! 131,073 bytes and then sends EOF, which exercises the bridge's read/parse bound. Back-pressure
//! is about the writer — how many bytes a peer accepts before it stops reading — so these helpers
//! never send EOF early and report the count at which the peer stopped. Nothing here reaches a
//! provider, credential, game, save or native host.
//!
//! The test-controlled loopback endpoint and its synthetic replies live in the sibling
//! `loopback` module and the process-tree/argv checks in the sibling `process` module; both are
//! re-exported here, so callers keep one import path.

use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub mod loopback;
pub mod process;
pub mod shipped_bounds;

pub use loopback::{Loopback, one_shot_wait, truncated_at_budget};
pub use process::{assert_no_private_argv, descendants};
pub use shipped_bounds::pinned_bounds;

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

/// How far a writer got before its peer stopped reading.
pub struct Pressure {
    pub written: usize,
    pub error: Option<std::io::ErrorKind>,
}

/// A process that was driven while its writer kept offering bytes.
pub struct Pressured {
    pub output: Output,
    pub written: usize,
    pub write_error: Option<std::io::ErrorKind>,
    pub processes: usize,
}

impl Pressured {
    /// The typed single-line failure a shipped entrypoint prints on stderr.
    pub fn stderr_line(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr)
            .trim()
            .to_owned()
    }

    /// The count of connections a peer stopped accepting at, asserted as a range: the exact figure
    /// is a property of the kernel pipe buffer as much as of the process.
    pub fn stopped_between(&self, lower: usize, upper: usize) -> bool {
        self.written >= lower && self.written <= upper
    }
}

/// Offers `bytes` in bounded chunks, reporting how much the peer accepted before it stopped.
///
/// `Write::write_all` propagates the peer's EPIPE as an error instead of measuring it, so a
/// back-pressure case cannot use it.
pub fn offer(input: &mut impl Write, bytes: &[u8]) -> Pressure {
    let mut written = 0;
    while written < bytes.len() {
        let chunk = &bytes[written..bytes.len().min(written + 64 * 1024)];
        match input.write(chunk) {
            Ok(0) => {
                return Pressure {
                    written,
                    error: Some(std::io::ErrorKind::WriteZero),
                };
            }
            Ok(count) => written += count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => {
                return Pressure {
                    written,
                    error: Some(error.kind()),
                };
            }
        }
    }
    Pressure {
        written,
        error: None,
    }
}

/// The reviewed golden decision request with the declared synthetic sentinels.
pub fn request_envelope(root: &Path) -> Result<Value> {
    let mut request: Value = serde_json::from_slice(&std::fs::read(
        root.join("protocol-artifact/exo-bridge-v1/golden/request.json"),
    )?)?;
    request["objective"] = json!("synthetic exact objective sentinel");
    request["hard_constraints"] = json!(["synthetic complete constraint sentinel"]);
    Ok(json!({
        "wire_version": "sts2.exo-bridge-wire-v1",
        "request_id": "host-request-private-sentinel",
        "turn_id": "host-turn-private-sentinel", "request": request
    }))
}

/// Builds the executor handoff exactly as the bridge does, at an explicit budget.
///
/// `support::invoke_executor` hard-codes the bridge's own 115 s/4096 budget and asserts success, so
/// it cannot express a budget this lane varies or a refusal this lane expects.
pub fn executor_handoff(
    config: &Path,
    envelope: &Value,
    root: &Path,
    timeout_millis: u32,
    max_output_tokens: u32,
) -> Result<Value> {
    let config: Value = serde_json::from_slice(&std::fs::read(config)?)?;
    let request = &envelope["request"];
    Ok(json!({
        "version": "sts2.exo-executor-input-v1",
        "request_id": envelope["request_id"], "host_turn_id": envelope["turn_id"],
        "model": config["model"], "endpoint": config["endpoint"],
        "module_path": config["extension"], "source_root": config["source_root"],
        "state_root": root.join("state"),
        "input": {
            "observation": request["observation"], "legal_action_ids": request["legal_action_ids"],
            "objective": request["objective"], "hard_constraints": request["hard_constraints"]
        },
        "timeout_millis": timeout_millis, "max_output_tokens": max_output_tokens,
        "credential": "sts2-synthetic-model-key"
    }))
}

/// Serializes `handoff` padded to exactly `target` bytes with one synthetic constraint.
///
/// The executor's read bound is the only boundary in this lane that the bridge cannot reach: the
/// projection the bridge builds carries at most 32 constraints of 512 bytes, so a bridge-issued
/// handoff is always far below 160 KiB and the bridge's own 131,072-byte parse bound fires first.
/// This padding therefore probes the *executor's* contract directly, where `input` only has to be
/// an object, and the case says so rather than implying the bridge could produce it.
///
/// `state_root` optionally rebases the handoff onto a per-case root. Each drive needs a root no
/// earlier drive has written, because Exo refuses a second conversation in a store it already owns
/// (`exo_executor_agent`). The rebase happens *before* the padding, so the exact byte count holds
/// whatever the root is; rebasing afterwards would change the length the case pins.
pub fn handoff_at_size(
    handoff: &Value,
    target: usize,
    state_root: Option<&Path>,
) -> Result<Vec<u8>> {
    let mut padded = handoff.clone();
    if let Some(state_root) = state_root {
        padded["state_root"] = json!(state_root.join("state"));
    }
    padded["input"]["hard_constraints"] = json!([""]);
    let unpadded = serde_json::to_vec(&padded)?.len();
    if unpadded > target {
        return Err(
            format!("handoff is already {unpadded} bytes, over the {target} target").into(),
        );
    }
    padded["input"]["hard_constraints"] = json!(["x".repeat(target - unpadded)]);
    let bytes = serde_json::to_vec(&padded)?;
    if bytes.len() != target {
        return Err(format!("padded handoff is {} bytes, not {target}", bytes.len()).into());
    }
    Ok(bytes)
}

/// Creates the private root a direct executor drive owns, with its four runtime directories.
pub fn executor_root(workspace: &Path, label: &str) -> Result<PathBuf> {
    let root = workspace
        .join("target/exo-test-tmp")
        .join(format!("bound-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    for child in ["state", "config", "cache", "temp"] {
        std::fs::create_dir(root.join(child))?;
    }
    Ok(root)
}

/// Builds the executor drive exactly as the bridge does: cleared environment, the Node directory on
/// `PATH`, private XDG/TMPDIR roots and the endpoint admission variable. Nothing is passed on argv;
/// the credential crosses only through private stdin.
pub fn executor_command(config: &Path, root: &Path) -> Result<Command> {
    let config: Value = serde_json::from_slice(&std::fs::read(config)?)?;
    let node = Path::new(config["node"].as_str().ok_or("node missing")?);
    let mut command = Command::new(config["executor"].as_str().ok_or("executor missing")?);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", node.parent().ok_or("node parent missing")?)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("TMPDIR", root.join("temp"))
        .env("EXO_LITELLM_PRICES_PATH", root.join("no-prices.json"))
        .env(
            "STS2_EXO_ALLOWED_ENDPOINT",
            config["endpoint"].as_str().ok_or("endpoint missing")?,
        );
    Ok(command)
}

/// Builds the shipped bridge invocation exactly as the operator entrypoint does.
pub fn bridge_command(binary: &Path, config: &Path, digest: &str) -> Result<Command> {
    let temporary = config
        .parent()
        .ok_or("config parent missing")?
        .join("exo-test-tmp");
    std::fs::create_dir_all(&temporary)?;
    let mut command = Command::new(binary);
    command
        .arg("--synthetic")
        .arg(config)
        .arg(digest)
        .env_clear()
        .env("TMPDIR", temporary);
    Ok(command)
}

/// Spawns `command`, keeps offering `data` on its stdin until the peer stops reading, and waits.
///
/// The writer runs on its own thread with a bounded wait, because a peer that stops reading *and*
/// keeps running would otherwise block the test in `write`. EOF follows only after the writer
/// finishes, so a case that offers more than the peer will read measures back-pressure rather than
/// an oversized request.
pub fn run_offering(command: &mut Command, data: &[u8]) -> Result<Pressured> {
    // Own all three pipes here rather than relying on each caller: the test harness closes its own
    // stdin, and a command that inherits it has no pipe to offer bytes into.
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    assert_no_private_argv(child.id())?;
    let mut input = child.stdin.take().ok_or("stdin missing")?;
    let payload = data.to_vec();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(offer(&mut input, &payload));
    });
    let deadline = Instant::now() + Duration::from_secs(200);
    let mut processes = 0;
    let mut pressure = None;
    while child.try_wait()?.is_none() {
        processes = processes.max(descendants(child.id())?);
        if pressure.is_none() {
            pressure = receiver.try_recv().ok();
        }
        if Instant::now() > deadline {
            child.kill()?;
            child.wait()?;
            return Err("bound oracle deadline".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let pressure = match pressure.or_else(|| receiver.try_recv().ok()) {
        Some(pressure) => pressure,
        None => receiver
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "offering writer did not finish")?,
    };
    Ok(Pressured {
        output: child.wait_with_output()?,
        written: pressure.written,
        write_error: pressure.error,
        processes,
    })
}

/// Drives the separately built executor directly with caller-supplied bytes.
///
/// This is the only boundary where the executor's typed `error_code` is observable: the bridge maps
/// every executor failure to `exo_bridge_executor_failed` and never forwards the code.
pub fn run_executor_bytes(
    workspace: &Path,
    config: &Path,
    label: &str,
    payload: &[u8],
) -> Result<Pressured> {
    let root = executor_root(workspace, label)?;
    let mut command = executor_command(config, &root)?;
    let run = run_offering(&mut command, payload)?;
    std::fs::remove_dir_all(&root)?;
    Ok(run)
}

/// Drives the executor with a handoff built at the given budget.
pub fn run_executor(
    workspace: &Path,
    config: &Path,
    envelope: &Value,
    label: &str,
    timeout_millis: u32,
    max_output_tokens: u32,
) -> Result<Pressured> {
    let root = executor_root(workspace, label)?;
    let handoff = executor_handoff(config, envelope, &root, timeout_millis, max_output_tokens)?;
    let mut command = executor_command(config, &root)?;
    let run = run_offering(&mut command, &serde_json::to_vec(&handoff)?)?;
    std::fs::remove_dir_all(&root)?;
    Ok(run)
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

/// The revision the driven checkout is actually at, refusing anything but the reviewed pin.
///
/// Both sides are read from the tree rather than compiled in: the expected pin is parsed from
/// `EXO_SOURCE_REVISION` — the declaration the CI workflow itself reads and every contract preflight
/// checks — and the driven revision from the checkout under test. The report therefore names the
/// bytes that produced it, and neither an edited pin nor a checkout at another revision can leave a
/// report that disagrees with the tree it came from.
pub fn driven_exo_revision(root: &Path, source: &Path) -> Result<String> {
    let contract = std::fs::read_to_string(root.join("crates/harness/src/exo/contract/mod.rs"))?;
    let pinned = contract
        .lines()
        .find_map(|line| {
            let value = line
                .trim()
                .strip_prefix("pub const EXO_SOURCE_REVISION: &str = \"")?
                .strip_suffix("\";")?
                .to_owned();
            (value.len() == 40).then_some(value)
        })
        .ok_or("EXO_SOURCE_REVISION is not declared in the contract module")?;
    let output = Command::new("git")
        .arg("-C")
        .arg(source)
        .args(["rev-parse", "HEAD"])
        .output()?;
    let revision = String::from_utf8(output.stdout)?.trim().to_owned();
    if output.status.success() && revision == pinned {
        Ok(revision)
    } else {
        Err(format!("driven Exo checkout is at {revision}, not the reviewed pin {pinned}").into())
    }
}

/// The clean exact-pinned Exo checkout the bridge requires; `STS2_EXO_TEST_SOURCE` overrides it.
pub fn source_root(workspace: &Path) -> Result<PathBuf> {
    Ok(std::env::var_os("STS2_EXO_TEST_SOURCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target/exo-source")))
}

/// The one-shot bridge configuration, identical in shape to the reviewed operator config.
pub fn config_json(workspace: &Path, model: &Loopback) -> Result<Value> {
    let executor = workspace.join("target/exo-executor/debug/sts2-exo-executor");
    let extension = workspace.join("experiments/exo-agent/extension/src/index.ts");
    let node = PathBuf::from(std::env::var("STS2_EXO_TEST_NODE")?).canonicalize()?;
    Ok(json!({
        "schema": "sts2.exo-one-shot-config-v1",
        "executor": executor, "executor_sha256": digest(&executor)?,
        "source_root": source_root(workspace)?, "extension": extension,
        "extension_sha256": digest(&extension)?,
        "node": node, "node_sha256": digest(&node)?,
        "model": "o3-pro", "endpoint": model.endpoint
    }))
}

/// Writes `config` under `target/<name>/config.json` and returns its path.
pub fn write_config(workspace: &Path, name: &str, config: &Value) -> Result<PathBuf> {
    let directory = workspace.join("target").join(name);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("config.json");
    std::fs::write(&path, serde_json::to_vec(config)?)?;
    Ok(path)
}

/// Refuses to record evidence whose bytes the recorded revision does not contain.
///
/// Every digest in a report is computed from the worktree, while `harness_revision` comes from
/// `git rev-parse HEAD`. If one of those paths is modified or untracked, the emitted record would
/// name a revision that does not contain the evidence it claims. Recording is therefore gated on
/// each recorded source being committed unchanged, so the revision the report names always
/// describes the bytes the report binds.
pub fn assert_sources_are_committed(root: &Path, paths: &[&str]) -> Result {
    for path in paths {
        let committed = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["cat-file", "-e"])
            .arg(format!("HEAD:{path}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !committed.success() {
            return Err(format!("recorded source {path} is absent at HEAD").into());
        }
        let unmodified = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["diff", "--quiet", "HEAD", "--", path])
            .status()?;
        if !unmodified.success() {
            return Err(format!(
                "recorded source {path} differs from HEAD; commit the recorded bytes before \
                 recording evidence"
            )
            .into());
        }
    }
    Ok(())
}
