// SPDX-License-Identifier: MIT

use super::escaped_fixture::EscapedChildFixture;
use super::escaped_process::{
    ABRUPT_DONE_PATH, ABRUPT_PID_PATH, ABRUPT_RELEASE_PATH, ABRUPT_TOKEN, EscapedIdentity,
    require_live_escape, shell_executor, wait_for_live_escape, wait_until_escape_is_not_live,
};
use super::{BridgeChildScope, CHILD_MODE, Scratch, io_error};
use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::exo_private_state::GuardedRun;

#[test]
fn isolated_subreaper_child() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("escape")) {
        return Ok(());
    }
    let (mut fixture, held_path, identity, status) =
        start_escaped_fixture("escaped", None, None, None)?;
    assert!(status.success(), "the executor leader must exit normally");
    let identity_path = fixture.paths().state_root.join("escaped-identity");
    let identity_mode = fs::metadata(&identity_path)?.mode() & 0o7777;
    assert_eq!(
        identity_mode, 0o600,
        "the escaped helper identity record must be private before production cleanup"
    );
    assert!(held_path.exists(), "live escaped child state is retained");
    require_live_escape(&identity)?;
    let clean = fixture.finish_child()?;
    assert!(!clean, "a killed escaped descendant must withhold success");
    fixture.finish_run()?;
    assert!(!held_path.exists(), "cleanup follows descendant drainage");
    wait_until_escape_is_not_live(&identity, Duration::from_secs(1))?;
    Ok(())
}

#[test]
#[allow(clippy::panic)]
fn isolated_subreaper_unwind_cleanup() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("escape-unwind")) {
        return Ok(());
    }
    let (fixture, held_path, identity, status) =
        start_escaped_fixture("escape-unwind", None, None, None)?;
    assert!(status.success(), "the executor leader must exit normally");
    let scratch_path = fixture.scratch_path().to_owned();
    let attempt_path = held_path
        .parent()
        .ok_or_else(|| io::Error::other("held-state path has no attempt directory"))?
        .to_owned();
    require_live_escape(&identity)?;
    let unwind = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _fixture = fixture;
        panic!("injected unwind after escaped child readiness");
    }));
    assert!(
        unwind.is_err(),
        "the injected panic must unwind the fixture guard"
    );
    wait_until_escape_is_not_live(&identity, Duration::from_secs(1))?;
    assert!(
        !attempt_path.exists(),
        "unwind cleanup removes only the quiescent attempt"
    );
    assert!(
        !scratch_path.exists(),
        "unwind cleanup removes its private fixture scratch"
    );
    Ok(())
}

#[test]
fn isolated_abrupt_subreaper_target() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("abrupt-target")) {
        return Ok(());
    }
    let pid_path = required_test_path(ABRUPT_PID_PATH)?;
    let release_path = required_test_path(ABRUPT_RELEASE_PATH)?;
    let done_path = required_test_path(ABRUPT_DONE_PATH)?;
    let token = std::env::var(ABRUPT_TOKEN)?;
    let (fixture, _held_path, identity, status) = start_escaped_fixture(
        "abrupt-target",
        Some(pid_path),
        Some(done_path),
        Some(token.clone()),
    )?;
    if !status.success() {
        return Err(io::Error::other("executor leader did not exit normally").into());
    }
    require_live_escape(&identity)?;
    let _fixture = fixture;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !release_path.exists() {
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("abrupt supervisor did not release escaped token {token}"),
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    // Deliberately bypass Rust Drop. The supervisor helper is the only process allowed to adopt
    // and reap descendants after this isolated target exits.
    std::process::exit(73)
}

#[test]
fn isolated_clean_child() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("clean")) {
        return Ok(());
    }
    let scratch = Scratch::new("clean")?;
    let mut run = GuardedRun::create(&scratch.policy(), &crate::sha256_hex(b"clean-config"))
        .map_err(io_error)?;
    let paths = run.paths().clone();
    let scope = BridgeChildScope::enable().map_err(io_error)?;
    run.begin_spawn().map_err(io_error)?;
    let mut child = Command::new("/bin/sh")
        .process_group(0)
        .arg("-c")
        .arg("read trigger; exit 0")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let _state = run.record_child(child.id()).map_err(io_error)?;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("clean child stdin was not piped"))?;
    input.write_all(b"start\n")?;
    drop(input);
    let status = child.wait()?;
    assert!(status.success());
    assert!(run.finish_child(&scope).map_err(io_error)?);
    run.finish().map_err(io_error)?;
    drop(run);
    for attempt in [paths.state_root, paths.cache_root, paths.temp_root] {
        assert!(!attempt.exists(), "successful cleanup removes {attempt:?}");
    }
    fs::remove_dir_all(&scratch.0)?;
    Ok(())
}

#[test]
fn isolated_preexisting_live_child_is_refused_without_reaping() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("live-child")) {
        return Ok(());
    }
    let child = std::thread::spawn(|| {
        Command::new("/bin/sh")
            .arg("-c")
            .arg("read trigger; exit 0")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    })
    .join()
    .map_err(|_| io::Error::other("child-spawn thread panicked"))??;
    assert_eq!(
        BridgeChildScope::enable().err(),
        Some("exo_private_preexisting_child")
    );
    let mut child = child;
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("live child stdin was not piped"))?;
    input.write_all(b"start\n")?;
    drop(input);
    assert!(
        child.wait()?.success(),
        "the refused live child remains owned"
    );
    Ok(())
}

#[test]
fn isolated_preexisting_zombie_child_is_refused_without_reaping() -> Result<(), Box<dyn Error>> {
    if std::env::var_os(CHILD_MODE).as_deref() != Some(std::ffi::OsStr::new("zombie-child")) {
        return Ok(());
    }
    let mut child = std::thread::spawn(|| {
        Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
    })
    .join()
    .map_err(|_| io::Error::other("child-spawn thread panicked"))??;
    let pid = child.id();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    loop {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat"))?;
        let state = stat
            .rfind(')')
            .and_then(|index| stat.get(index + 1..))
            .and_then(|suffix| suffix.split_whitespace().next())
            .and_then(|field| field.chars().next());
        if state == Some('Z') {
            break;
        }
        if std::time::Instant::now() >= deadline {
            return Err(io::Error::other("preexisting child did not become a zombie").into());
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        BridgeChildScope::enable().err(),
        Some("exo_private_preexisting_child")
    );
    assert!(
        child.wait()?.success(),
        "the refused zombie remains waitable"
    );
    Ok(())
}

fn start_escaped_fixture(
    label: &str,
    external_pid_path: Option<PathBuf>,
    external_done_path: Option<PathBuf>,
    token: Option<String>,
) -> io::Result<(
    EscapedChildFixture,
    PathBuf,
    EscapedIdentity,
    std::process::ExitStatus,
)> {
    let mut fixture = EscapedChildFixture::create(label, &crate::sha256_hex(label.as_bytes()))?;
    let held_path = fixture.paths().state_root.join("held-open-state");
    let pid_path =
        external_pid_path.unwrap_or_else(|| fixture.paths().state_root.join("escaped-identity"));
    let done_path =
        external_done_path.unwrap_or_else(|| fixture.paths().state_root.join("escaped-finished"));
    let token = token.unwrap_or_else(|| format!("escaped-{}", uuid::Uuid::new_v4().simple()));
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&held_path)?;
    fixture.enable_scope()?;
    fixture.spawn(shell_executor(&held_path, &pid_path, &token, &done_path))?;
    fixture.record_child()?;
    fixture.send_trigger()?;
    let status = fixture.wait_direct_child(Duration::from_secs(1))?;
    if !status.success() {
        return Err(io::Error::other("executor leader did not exit normally"));
    }
    let identity =
        wait_for_live_escape(&pid_path, Some(&held_path), &token, Duration::from_secs(1))?;
    Ok((fixture, held_path, identity, status))
}

fn required_test_path(name: &str) -> io::Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other(format!("missing test control path {name}")))
}
