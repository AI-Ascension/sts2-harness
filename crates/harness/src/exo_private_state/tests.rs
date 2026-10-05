// SPDX-License-Identifier: MIT

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{BridgeChildScope, GuardedRun};
use crate::ExoPrivateStatePolicy;

const CHILD_MODE: &str = "STS2_EXO_PRIVATE_STATE_CHILD_TEST";
const POLICY_ROOT: &str = "STS2_EXO_PRIVATE_STATE_POLICY_ROOT_TEST";

struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> io::Result<Self> {
        let path = std::env::temp_dir().join(format!(
            "sts2-private-state-{}-{}-{}",
            std::process::id(),
            label,
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn policy(&self) -> ExoPrivateStatePolicy {
        ExoPrivateStatePolicy {
            state_root: self.0.join("state").display().to_string(),
            cache_root: self.0.join("cache").display().to_string(),
            temp_root: self.0.join("temp").display().to_string(),
            quota_bytes: 4 * 1024 * 1024,
            max_retention_days: 1,
            permissions_octal: 0o700,
        }
    }
}

fn io_error(error: &'static str) -> io::Error {
    io::Error::other(error)
}

fn launch_isolated_test(name: &str, mode: &str) -> io::Result<()> {
    let output = Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg(name)
        .arg("--nocapture")
        .env(CHILD_MODE, mode)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "isolated helper {name} failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

#[test]
fn guarded_v2_subreaper_refuses_a_second_scope_in_one_bridge_process() {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("duplicate")) {
        return;
    }
    assert!(BridgeChildScope::enable().is_ok());
    assert_eq!(
        BridgeChildScope::enable().err(),
        Some("exo_private_subreaper_scope")
    );
}

#[test]
fn subreaper_helper_process_passes_the_no_descendant_and_escape_cases() -> io::Result<()> {
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_subreaper_child",
        "escape",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_clean_child",
        "clean",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::guarded_v2_subreaper_refuses_a_second_scope_in_one_bridge_process",
        "duplicate",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_preexisting_live_child_is_refused_without_reaping",
        "live-child",
    )?;
    launch_isolated_test(
        "exo_private_state::tests::children::isolated_preexisting_zombie_child_is_refused_without_reaping",
        "zombie-child",
    )
}

#[test]
fn overlapping_policy_roots_serialize_and_keep_the_first_attempt() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("overlapping-policy")?;
    for name in ["state-a", "state-b", "shared-cache", "shared-temp"] {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(scratch.0.join(name))?;
    }
    let mut holder = Command::new(std::env::current_exe()?)
        .arg("--exact")
        .arg("exo_private_state::tests::isolated_shared_policy_holder")
        .arg("--nocapture")
        .env(CHILD_MODE, "hold-shared-policy")
        .env(POLICY_ROOT, &scratch.0)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let mut holder_input = holder
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("policy holder stdin was not piped"))?;
    let ready_path = scratch.0.join("holder-ready");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !ready_path.exists() && std::time::Instant::now() < deadline {
        if let Some(status) = holder.try_wait()? {
            return Err(io::Error::other(format!("policy holder exited early: {status}")).into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let ready = ready_path.exists();
    let first_attempt_before = std::fs::read_dir(scratch.0.join("state-a"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".sts2-policy.lock")
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    let second = GuardedRun::create(
        &overlapping_policy(&scratch.0, "state-b"),
        &crate::sha256_hex(b"overlapping-policy-b"),
    );
    let second_refused_busy = second.err() == Some("exo_private_policy_busy");
    let second_attempts = std::fs::read_dir(scratch.0.join("state-b"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".sts2-policy.lock")
        .count();
    let first_attempt_after = std::fs::read_dir(scratch.0.join("state-a"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".sts2-policy.lock")
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    let first_attempt_survives =
        !first_attempt_before.is_empty() && first_attempt_before == first_attempt_after;
    holder_input.write_all(b"stop\n")?;
    drop(holder_input);
    let holder_status = holder.wait()?;
    let foreign_policy_refused = GuardedRun::create(
        &overlapping_policy(&scratch.0, "state-b"),
        &crate::sha256_hex(b"overlapping-policy-b"),
    )
    .err()
        == Some("exo_private_policy_owner");
    assert!(
        ready,
        "the first independent process reached its reservation"
    );
    assert!(
        first_attempt_survives,
        "the first attempt was not deleted by refusal"
    );
    assert!(
        second_refused_busy,
        "the overlapping policy was refused while locked"
    );
    assert_eq!(second_attempts, 0, "refusal created no second attempt");
    assert!(
        holder_status.success(),
        "the holder completed and cleaned its own run"
    );
    assert!(
        foreign_policy_refused,
        "a persistent shared-root policy identity cannot be overwritten"
    );
    fs::remove_dir_all(&scratch.0)?;
    Ok(())
}

fn overlapping_policy(base: &Path, state: &str) -> ExoPrivateStatePolicy {
    ExoPrivateStatePolicy {
        state_root: base.join(state).display().to_string(),
        cache_root: base.join("shared-cache").display().to_string(),
        temp_root: base.join("shared-temp").display().to_string(),
        quota_bytes: 4 * 1024 * 1024,
        max_retention_days: 1,
        permissions_octal: 0o700,
    }
}

#[test]
fn isolated_shared_policy_holder() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("hold-shared-policy")) {
        return Ok(());
    }
    let base = PathBuf::from(std::env::var_os(POLICY_ROOT).ok_or("policy root missing")?);
    let mut run = GuardedRun::create(
        &overlapping_policy(&base, "state-a"),
        &crate::sha256_hex(b"overlapping-policy-a"),
    )
    .map_err(io_error)?;
    std::fs::write(base.join("holder-ready"), b"ready")?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    if !input.starts_with("stop") {
        return Err(io::Error::other("policy holder stop signal missing").into());
    }
    run.finish().map_err(io_error)?;
    Ok(())
}

#[path = "tests/children.rs"]
mod children;
