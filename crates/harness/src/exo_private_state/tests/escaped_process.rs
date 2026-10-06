// SPDX-License-Identifier: MIT

use super::OwnedChild;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub(super) const ABRUPT_PID_PATH: &str = "STS2_EXO_PRIVATE_ABRUPT_PID_PATH";
pub(super) const ABRUPT_RELEASE_PATH: &str = "STS2_EXO_PRIVATE_ABRUPT_RELEASE_PATH";
pub(super) const ABRUPT_DONE_PATH: &str = "STS2_EXO_PRIVATE_ABRUPT_DONE_PATH";
pub(super) const ABRUPT_TOKEN: &str = "STS2_EXO_PRIVATE_ABRUPT_TOKEN";

const ESCAPED_CHILD_LIFETIME_SECONDS: u64 = 3;

#[derive(Clone, Debug)]
pub(super) struct EscapedIdentity {
    pub(super) pid: u32,
    pub(super) start_time_ticks: u64,
    pub(super) held_path: PathBuf,
    token: String,
}

pub(super) fn shell_executor(
    held_path: &Path,
    pid_path: &Path,
    token: &str,
    done_path: &Path,
) -> Command {
    let mut command = Command::new("/bin/sh");
    command
        .process_group(0)
        .arg("-c")
        .arg(format!(
            "umask 077; read trigger; setsid /bin/sh -c 'exec 3>>\"$1\"; printf x >&3; printf \"%s\\n%s\\n\" \"$$\" \"$1\" > \"$2\"; sleep {ESCAPED_CHILD_LIFETIME_SECONDS}; printf done > \"$4\"' child \"$1\" \"$2\" \"$3\" \"$4\" >/dev/null 2>&1 & exit 0"
        ))
        .arg("exo-private-test")
        .arg(held_path)
        .arg(pid_path)
        .arg(token)
        .arg(done_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

pub(super) fn wait_for_live_escape(
    pid_path: &Path,
    expected_held_path: Option<&Path>,
    token: &str,
    timeout: Duration,
) -> io::Result<EscapedIdentity> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some((pid, held_path)) = read_escape_record(pid_path)? {
            if expected_held_path.is_some_and(|expected| expected != held_path) {
                return Err(io::Error::other(
                    "escaped fixture held-path identity mismatch",
                ));
            }
            if let Some(start_time_ticks) = observe_live_escape(pid, &held_path, token)? {
                return Ok(EscapedIdentity {
                    pid,
                    start_time_ticks,
                    held_path,
                    token: token.to_owned(),
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "escaped fixture did not become observably live",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Read-only exact-birth liveness observation. This is not a wait/reap proof.
pub(super) fn wait_until_escape_is_not_live(
    identity: &EscapedIdentity,
    timeout: Duration,
) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        if !same_live_escape(identity)? {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "escaped fixture process identity is still live",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Confirm the exact observed birth identity, token, held descriptor, and path are still live.
/// This read-only check is not an ownership or reap proof; callers still require the subreaper's
/// final ECHILD result before treating the descendant tree as quiescent.
pub(super) fn require_live_escape(identity: &EscapedIdentity) -> io::Result<()> {
    if same_live_escape(identity)? {
        Ok(())
    } else {
        Err(io::Error::other(
            "escaped fixture identity was no longer live at the cleanup boundary",
        ))
    }
}

pub(super) fn wait_for_control_escape(
    child: &mut OwnedChild,
    pid_path: &Path,
    token: &str,
    deadline: Instant,
) -> io::Result<EscapedIdentity> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "abrupt target exited before fixture readiness ({status})"
            )));
        }
        if let Some((pid, held_path)) = read_escape_record(pid_path)?
            && let Some(start_time_ticks) = observe_live_escape(pid, &held_path, token)?
        {
            return Ok(EscapedIdentity {
                pid,
                start_time_ticks,
                held_path,
                token: token.to_owned(),
            });
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "abrupt target did not publish a live escaped child",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn exact_test_scratch_from_held_path(
    held_path: &Path,
    target_pid: u32,
) -> io::Result<PathBuf> {
    let attempt = held_path
        .parent()
        .ok_or_else(|| io::Error::other("held-state path has no attempt directory"))?;
    let policy_root = attempt
        .parent()
        .ok_or_else(|| io::Error::other("held-state path has no policy root"))?;
    let scratch = policy_root
        .parent()
        .ok_or_else(|| io::Error::other("held-state path has no test scratch root"))?;
    let scratch = scratch.canonicalize()?;
    let temporary_root = std::env::temp_dir().canonicalize()?;
    let name = scratch
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::other("test scratch name is not UTF-8"))?;
    let prefix = format!("sts2-private-state-{target_pid}-abrupt-target-");
    if scratch.parent() != Some(temporary_root.as_path()) || !name.starts_with(&prefix) {
        return Err(io::Error::other(
            "refusing non-owned abrupt-test scratch cleanup",
        ));
    }
    Ok(scratch)
}

pub(super) fn write_control_marker(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?
        .write_all(b"release\n")
}

fn read_escape_record(path: &Path) -> io::Result<Option<(u32, PathBuf)>> {
    let value = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut lines = value.lines();
    let Some(pid) = lines.next() else {
        return Ok(None);
    };
    let Some(held_path) = lines.next() else {
        return Ok(None);
    };
    let pid = pid
        .parse::<u32>()
        .map_err(|_| io::Error::other("escaped fixture PID record is malformed"))?;
    Ok(Some((pid, PathBuf::from(held_path))))
}

fn observe_live_escape(pid: u32, held_path: &Path, token: &str) -> io::Result<Option<u64>> {
    let Some((state, start_time_ticks)) = read_proc_identity(pid)? else {
        return Ok(None);
    };
    if matches!(state, 'Z' | 'X') {
        return Ok(None);
    }
    let command_line = match fs::read(format!("/proc/{pid}/cmdline")) {
        Ok(command_line) => command_line,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !command_line
        .split(|byte| *byte == 0)
        .any(|argument| argument == token.as_bytes())
    {
        return Ok(None);
    }
    let descriptor = match fs::read_link(format!("/proc/{pid}/fd/3")) {
        Ok(descriptor) => descriptor,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if descriptor != held_path || fs::metadata(held_path)?.len() == 0 {
        return Ok(None);
    }
    Ok(Some(start_time_ticks))
}

fn same_live_escape(identity: &EscapedIdentity) -> io::Result<bool> {
    Ok(
        observe_live_escape(identity.pid, &identity.held_path, &identity.token)?
            .is_some_and(|start_time_ticks| start_time_ticks == identity.start_time_ticks),
    )
}

fn read_proc_identity(pid: u32) -> io::Result<Option<(char, u64)>> {
    let value = match fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(value) => value,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let suffix = value
        .rfind(')')
        .and_then(|index| value.get(index + 1..))
        .ok_or_else(|| io::Error::other("escaped fixture proc stat is malformed"))?;
    let fields = suffix.split_whitespace().collect::<Vec<_>>();
    let state = fields
        .first()
        .and_then(|field| field.chars().next())
        .ok_or_else(|| io::Error::other("escaped fixture proc state is missing"))?;
    let start_time_ticks = fields
        .get(19)
        .and_then(|field| field.parse::<u64>().ok())
        .ok_or_else(|| io::Error::other("escaped fixture proc birth identity is missing"))?;
    Ok(Some((state, start_time_ticks)))
}
