// SPDX-License-Identifier: MIT

use super::escaped_process::{
    ABRUPT_DONE_PATH, ABRUPT_PID_PATH, ABRUPT_RELEASE_PATH, ABRUPT_TOKEN,
    exact_test_scratch_from_held_path, require_live_escape, wait_for_control_escape,
    wait_until_escape_is_not_live, write_control_marker,
};
use super::escaped_scope::ScopeDrainGuard;
use super::{BridgeChildScope, CHILD_MODE, OwnedChild, Scratch, io_error};
use std::fs;
use std::io;
use std::process::Command;
use std::time::{Duration, Instant};

const TARGET_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(2);
const ESCAPED_IDENTITY_WAIT: Duration = Duration::from_secs(1);
const HELPER_EXIT_AFTER_ABRUPT_SPAWN: i32 = 73;

#[test]
fn isolated_abrupt_subreaper_supervisor() -> io::Result<()> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("abrupt-supervisor")) {
        return Ok(());
    }
    let mut scope = ScopeDrainGuard::new(BridgeChildScope::enable().map_err(io_error)?);
    let control = Scratch::new("abrupt-supervisor-control")?;
    let pid_path = control.0.join("escaped-identity");
    let release_path = control.0.join("release-target");
    let done_path = control.0.join("escaped-finished");
    let token = format!("escaped-{}", uuid::Uuid::new_v4().simple());
    let deadline = Instant::now() + TARGET_HANDSHAKE_TIMEOUT;
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--exact")
        .arg("exo_private_state::tests::children::isolated_abrupt_subreaper_target")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(CHILD_MODE, "abrupt-target")
        .env(ABRUPT_PID_PATH, &pid_path)
        .env(ABRUPT_RELEASE_PATH, &release_path)
        .env(ABRUPT_DONE_PATH, &done_path)
        .env(ABRUPT_TOKEN, &token);
    let mut target = OwnedChild::spawn(command)?;
    let target_pid = target.child_mut().id();
    let identity = wait_for_control_escape(&mut target, &pid_path, &token, deadline)?;
    write_control_marker(&release_path)?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    let output = target.wait_until(remaining)?;
    if output.status.code() != Some(HELPER_EXIT_AFTER_ABRUPT_SPAWN) {
        return Err(io::Error::other(format!(
            "abrupt target returned unexpected status {}",
            output.status
        )));
    }

    require_live_escape(&identity)?;
    // The target exited without Drop. This separate one-shot helper is now the nearest live
    // subreaper, and its final ECHILD drain proves that it reaped the adopted fixture process tree.
    let no_descendants = scope.drain()?;
    if no_descendants {
        return Err(io::Error::other(
            "supervisor did not observe the abrupt descendant",
        ));
    }
    wait_until_escape_is_not_live(&identity, ESCAPED_IDENTITY_WAIT)?;
    if !identity.held_path.exists() {
        return Err(io::Error::other(
            "abrupt target did not preserve its ambiguous held-state path",
        ));
    }
    let target_scratch = exact_test_scratch_from_held_path(&identity.held_path, target_pid)?;
    // Test-harness cleanup only: the isolated supervisor reaped every child (ECHILD) and observed
    // the ambiguous production path first. Production cleanup must retain that marker.
    fs::remove_dir_all(target_scratch)?;
    fs::remove_dir_all(&control.0)?;
    Ok(())
}
