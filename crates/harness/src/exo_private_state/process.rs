// SPDX-License-Identifier: MIT

use std::fs;
use std::os::fd::OwnedFd;
use std::os::unix::fs::MetadataExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{
    Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, WaitOptions, child_subreaper, getpid,
    pidfd_open, pidfd_send_signal, set_child_subreaper, wait, waitid,
};

use super::ProcessIdentity;
#[path = "process_children.rs"]
mod process_children;

use process_children::children_across_tasks;

const DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const DRAIN_POLL: Duration = Duration::from_millis(10);
const SUBREAPER_PID: i32 = 1;
static BRIDGE_SCOPE_CREATED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct ProcStat {
    parent_pid: u32,
    process_group: u32,
    session: u32,
    start_time_ticks: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChildIdentity {
    pid: u32,
    parent_pid: u32,
    uid: u32,
    start_time_ticks: u64,
}

impl ProcessIdentity {
    pub(super) fn read(pid: u32) -> Result<Self, &'static str> {
        let bridge_pid = getpid().as_raw_nonzero().get() as u32;
        let identity = child_identity(pid, bridge_pid)?;
        let process = stat(pid)?;
        if process.process_group != pid || process.session == 0 {
            return Err("exo_private_process_identity");
        }
        Ok(Self {
            boot_id: boot_id()?,
            pid,
            parent_pid: identity.parent_pid,
            process_group: process.process_group,
            session: process.session,
            start_time_ticks: identity.start_time_ticks,
            uid: identity.uid,
        })
    }
}

/// Process-global Linux child ownership for the dedicated guarded-v2 bridge invocation only.
/// Construct this in the one-shot bridge composition, never in filesystem-only library callers or
/// in the shared unit-test process. The lifetime is intentionally the bridge process lifetime.
pub struct BridgeChildScope {
    bridge_pid: u32,
}

impl BridgeChildScope {
    pub fn enable() -> Result<Self, &'static str> {
        if BRIDGE_SCOPE_CREATED.swap(true, Ordering::AcqRel) {
            return Err("exo_private_subreaper_scope");
        }
        enable_subreaper_without_children()?;
        Ok(Self {
            bridge_pid: getpid().as_raw_nonzero().get() as u32,
        })
    }

    pub fn drain(&self) -> Result<bool, &'static str> {
        if self.bridge_pid != getpid().as_raw_nonzero().get() as u32 {
            return Err("exo_private_subreaper_scope");
        }
        drain_adopted_descendants()
    }

    pub fn confirm_spawn_error(&self) -> Result<(), &'static str> {
        if self.bridge_pid != getpid().as_raw_nonzero().get() as u32 {
            return Err("exo_private_subreaper_scope");
        }
        confirm_no_child_after_spawn_error()
    }
}

/// Establish the one-shot bridge process as a verified Linux child subreaper before spawn.
///
/// The bridge has no unrelated process children at this boundary. All task IDs are checked because
/// Tokio or another runtime may create the child from a worker thread. Read-only `waitid` with
/// `WNOWAIT` and a complete task-child scan refuse preexisting children without consuming them.
fn enable_subreaper_without_children() -> Result<(), &'static str> {
    verify_no_preexisting_children()?;
    let requested = Pid::from_raw(SUBREAPER_PID).ok_or("exo_private_subreaper")?;
    set_child_subreaper(Some(requested)).map_err(|_| "exo_private_subreaper")?;
    if child_subreaper().map_err(|_| "exo_private_subreaper")? != Some(requested) {
        return Err("exo_private_subreaper");
    }
    verify_no_preexisting_children()
}

/// Drain any descendants left by the executor after its direct child has been reaped.
///
/// A descendant is never signalled by a recyclable numeric PID alone. Its direct parent, service
/// UID, start time, and pidfd are rechecked before signalling. We kill each adopted child and loop:
/// its own living descendants then become adopted children of this one-shot bridge. A final
/// all-child `wait(NO_HANG)` returning ECHILD is required; an empty sampled `/proc` list or
/// `Ok(None)` from wait is not proof of quiescence.
fn drain_adopted_descendants() -> Result<bool, &'static str> {
    if child_subreaper().map_err(|_| "exo_private_subreaper")?
        != Some(Pid::from_raw(SUBREAPER_PID).ok_or("exo_private_subreaper")?)
    {
        return Err("exo_private_subreaper");
    }
    let bridge_pid = getpid().as_raw_nonzero().get() as u32;
    let deadline = Instant::now()
        .checked_add(DRAIN_TIMEOUT)
        .ok_or("exo_private_process_bound")?;
    let mut found_descendant = false;
    loop {
        let children = children_across_tasks()?;
        if !children.is_empty() {
            found_descendant = true;
            for pid in children {
                signal_verified_child(pid, bridge_pid)?;
            }
        }
        match wait(WaitOptions::NOHANG) {
            Err(Errno::CHILD) => {
                if children_across_tasks()?.is_empty() {
                    return Ok(!found_descendant);
                }
            }
            Ok(Some((_pid, _status))) => found_descendant = true,
            Ok(None) => {}
            Err(_) => return Err("exo_private_process_wait"),
        }
        if Instant::now() >= deadline {
            return Err("exo_private_process_drain_timeout");
        }
        std::thread::sleep(DRAIN_POLL);
    }
}

/// Confirm that a failed spawn left no child from this attempt. The bridge is a one-shot process,
/// so ECHILD is the required kernel proof rather than a sampled empty process list.
fn confirm_no_child_after_spawn_error() -> Result<(), &'static str> {
    verify_no_preexisting_children().map_err(|_| "exo_private_process_ambiguous")
}

fn verify_no_preexisting_children() -> Result<(), &'static str> {
    if !children_across_tasks()?.is_empty() {
        return Err("exo_private_preexisting_child");
    }
    match waitid(
        WaitId::All,
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    ) {
        Err(Errno::CHILD) => Ok(()),
        Ok(_) => Err("exo_private_preexisting_child"),
        Err(_) => Err("exo_private_subreaper"),
    }
}

fn signal_verified_child(pid: u32, bridge_pid: u32) -> Result<(), &'static str> {
    if pid == 0 || pid == bridge_pid {
        return Err("exo_private_process_identity");
    }
    let before = child_identity(pid, bridge_pid)?;
    let pid = i32::try_from(pid)
        .ok()
        .and_then(Pid::from_raw)
        .ok_or("exo_private_process_identity")?;
    let pidfd: OwnedFd = match pidfd_open(pid, PidfdFlags::empty()) {
        Ok(pidfd) => pidfd,
        Err(Errno::SRCH) => return Ok(()),
        Err(_) => return Err("exo_private_process_pidfd"),
    };
    let after = child_identity(before.pid, bridge_pid)?;
    if before != after {
        return Err("exo_private_process_identity_raced");
    }
    pidfd_send_signal(&pidfd, Signal::KILL).map_err(|error| {
        if error == Errno::SRCH {
            "exo_private_process_identity_raced"
        } else {
            "exo_private_process_signal"
        }
    })
}

fn child_identity(pid: u32, expected_parent: u32) -> Result<ChildIdentity, &'static str> {
    let first = stat(pid)?;
    let metadata =
        fs::metadata(format!("/proc/{pid}")).map_err(|_| "exo_private_process_identity")?;
    let second = stat(pid)?;
    let second_metadata =
        fs::metadata(format!("/proc/{pid}")).map_err(|_| "exo_private_process_identity")?;
    if first.parent_pid != expected_parent
        || second.parent_pid != expected_parent
        || first.start_time_ticks != second.start_time_ticks
        || metadata.uid() != second_metadata.uid()
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err("exo_private_process_identity");
    }
    Ok(ChildIdentity {
        pid,
        parent_pid: expected_parent,
        uid: second_metadata.uid(),
        start_time_ticks: second.start_time_ticks,
    })
}

fn stat(pid: u32) -> Result<ProcStat, &'static str> {
    let value = fs::read_to_string(format!("/proc/{pid}/stat"))
        .map_err(|_| "exo_private_process_identity")?;
    let end = value
        .rfind(')')
        .and_then(|index| value.get(index + 1..))
        .ok_or("exo_private_process_identity")?;
    let fields = end.split_whitespace().collect::<Vec<_>>();
    // The suffix begins at field 3: state, ppid, pgrp, session, ... starttime is field 22.
    let state = fields
        .first()
        .and_then(|value| value.chars().next())
        .ok_or("exo_private_process_identity")?;
    let parent_pid = parse_field::<u32>(&fields, 1)?;
    let process_group = parse_field::<u32>(&fields, 2)?;
    let session = parse_field::<u32>(&fields, 3)?;
    let start_time_ticks = parse_field::<u64>(&fields, 19)?;
    if !matches!(state, 'R' | 'S' | 'D' | 'T' | 't' | 'Z' | 'X' | 'I' | 'P') {
        return Err("exo_private_process_identity");
    }
    Ok(ProcStat {
        parent_pid,
        process_group,
        session,
        start_time_ticks,
    })
}

fn parse_field<T: std::str::FromStr>(fields: &[&str], index: usize) -> Result<T, &'static str> {
    fields
        .get(index)
        .and_then(|value| value.parse().ok())
        .ok_or("exo_private_process_identity")
}

pub(super) fn boot_id() -> Result<String, &'static str> {
    let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| "exo_private_process_identity")?;
    let value = value.trim();
    if value.len() != 36
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        return Err("exo_private_process_identity");
    }
    Ok(value.to_ascii_lowercase())
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
