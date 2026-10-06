// SPDX-License-Identifier: MIT

use super::{CHILD_MODE, OwnedChild, POLICY_ROOT, Scratch, io_error, require_one_passing_test};
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ExoPrivateStatePolicy;
use crate::exo_private_state::GuardedRun;

#[test]
fn overlapping_policy_roots_serialize_and_keep_the_first_attempt() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new("overlapping-policy")?;
    for name in ["state-a", "state-b", "shared-cache", "shared-temp"] {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(scratch.0.join(name))?;
    }
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg("--exact")
        .arg("exo_private_state::tests::overlapping_roots::isolated_shared_policy_holder")
        .arg("--nocapture")
        .env(CHILD_MODE, "hold-shared-policy")
        .env(POLICY_ROOT, &scratch.0)
        .stdin(Stdio::piped());
    let mut holder = OwnedChild::spawn(command)?;
    let mut holder_input = holder
        .child_mut()
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("policy holder stdin was not piped"))?;
    let ready_path = scratch.0.join("holder-ready");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !ready_path.exists() && std::time::Instant::now() < deadline {
        if let Some(status) = holder.try_wait()? {
            let output = holder.collect(status, deadline)?;
            let test_result = require_one_passing_test(&output);
            return Err(io::Error::other(format!(
                "policy holder exited before reservation: {test_result:?}"
            ))
            .into());
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let ready = ready_path.exists();
    let holder_roots = ["state-a", "shared-cache", "shared-temp"].map(|name| scratch.0.join(name));
    let first_attempt_before = snapshot_policy_roots(&holder_roots)?;
    let second = GuardedRun::create(
        &overlapping_policy(&scratch.0, "state-b"),
        &crate::sha256_hex(b"overlapping-policy-b"),
    );
    let second_refused_busy = second.err() == Some("exo_private_policy_busy");
    let second_attempts = std::fs::read_dir(scratch.0.join("state-b"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name() != ".sts2-policy.lock")
        .count();
    let first_attempt_after = snapshot_policy_roots(&holder_roots)?;
    let first_attempt_survives = first_attempt_before == first_attempt_after
        && first_attempt_after.iter().all(|root| {
            root.attempts.len() == 1
                && !root.attempts[0].marker.is_empty()
                && !root.attempts[0].payload.is_empty()
        });
    holder_input.write_all(b"stop\n")?;
    drop(holder_input);
    let holder_output = holder.wait_until(std::time::Duration::from_secs(3))?;
    let holder_test_result = require_one_passing_test(&holder_output);
    let policy_locks_after_holder = holder_roots
        .iter()
        .map(|root| fs::read(root.join(".sts2-policy.lock")))
        .collect::<io::Result<Vec<_>>>()?;
    let policy_locks_persist = first_attempt_before
        .iter()
        .map(|root| root.policy_lock.clone())
        .collect::<Vec<_>>()
        == policy_locks_after_holder;
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
        "the losing policy changed an attempt, marker, payload or lock in a shared root"
    );
    assert!(
        second_refused_busy,
        "the overlapping policy was refused while locked"
    );
    assert_eq!(second_attempts, 0, "refusal created no second attempt");
    assert!(
        holder_output.status.success() && holder_test_result.is_ok(),
        "the holder failed the one-test check: {holder_test_result:?}"
    );
    assert!(
        policy_locks_persist,
        "the first policy identity was not retained in all three roots"
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

#[derive(Debug, Eq, PartialEq)]
struct PolicyRootSnapshot {
    policy_lock: Vec<u8>,
    attempts: Vec<PolicyAttemptSnapshot>,
}

#[derive(Debug, Eq, PartialEq)]
struct PolicyAttemptSnapshot {
    name: OsString,
    children: Vec<OsString>,
    owner_lock: Vec<u8>,
    marker: Vec<u8>,
    payload: Vec<u8>,
}

fn snapshot_policy_roots(roots: &[PathBuf]) -> io::Result<Vec<PolicyRootSnapshot>> {
    roots
        .iter()
        .map(|root| {
            let policy_lock = fs::read(root.join(".sts2-policy.lock"))?;
            let mut attempts = Vec::new();
            for entry in fs::read_dir(root)? {
                let entry = entry?;
                let name = entry.file_name();
                if name == OsStr::new(".sts2-policy.lock") {
                    continue;
                }
                let attempt = entry.path();
                let mut children = fs::read_dir(&attempt)?
                    .map(|entry| entry.map(|entry| entry.file_name()))
                    .collect::<io::Result<Vec<_>>>()?;
                children.sort();
                attempts.push(PolicyAttemptSnapshot {
                    name,
                    children,
                    owner_lock: fs::read(attempt.join(crate::exo_private_state::OWNER_LOCK_NAME))?,
                    marker: fs::read(attempt.join(crate::exo_private_state::OWNER_MARKER_NAME))?,
                    payload: fs::read(attempt.join("overlap-test-payload"))?,
                });
            }
            attempts.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(PolicyRootSnapshot {
                policy_lock,
                attempts,
            })
        })
        .collect()
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
    let paths = run.paths().clone();
    for root in [&paths.state_root, &paths.cache_root, &paths.temp_root] {
        let mut payload = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("overlap-test-payload"))?;
        payload.write_all(b"first-policy-owner")?;
        payload.sync_all()?;
    }
    std::fs::write(base.join("holder-ready"), b"ready")?;
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    if !input.starts_with("stop") {
        return Err(io::Error::other("policy holder stop signal missing").into());
    }
    run.finish().map_err(io_error)?;
    Ok(())
}
