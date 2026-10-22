// SPDX-License-Identifier: MIT

//! Linux descendant ownership for the dedicated management supervisor process.
//!
//! This scope is established before the supervisor creates any child or thread. A library
//! caller with existing children is refused without consuming a wait status or signalling
//! anything. Reap proof requires the kernel's ECHILD result, never an empty /proc snapshot.

use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use rustix::io::Errno;
use rustix::process::{
    Pid, PidfdFlags, Signal, WaitId, WaitIdOptions, WaitOptions, child_subreaper, getpid,
    pidfd_open, pidfd_send_signal, set_child_subreaper, wait, waitid,
};

const MAX_PROC_ENTRIES: usize = 65_536;
const MAX_OWNED_CHILDREN: usize = 4_096;
const MAX_STAT_BYTES: u64 = 4_096;
static SCOPE_CREATED: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ScopeError {
    AlreadyOwned,
    ExistingChild,
    Identity,
    Inspection,
    Signal,
    Reap,
    Pending,
}

/// An exclusive child scope, usable only by a dedicated management supervisor invocation.
pub(super) struct ChildScope {
    owner: Pid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ChildIdentity {
    pid: Pid,
    parent: u32,
    birth: u64,
    uid: u32,
}

impl ChildScope {
    pub(super) fn enable() -> Result<Self, ScopeError> {
        if SCOPE_CREATED.swap(true, Ordering::AcqRel) {
            return Err(ScopeError::AlreadyOwned);
        }
        no_existing_child()?;
        let enabled = Pid::from_raw(1).ok_or(ScopeError::Identity)?;
        set_child_subreaper(Some(enabled)).map_err(|_| ScopeError::Inspection)?;
        if child_subreaper().map_err(|_| ScopeError::Inspection)? != Some(enabled) {
            return Err(ScopeError::Inspection);
        }
        no_existing_child()?;
        Ok(Self { owner: getpid() })
    }

    /// Stop the direct backend, reap it, and drain every adopted descendant.
    ///
    /// A pending result retains the child and this scope. It does not permit a replacement
    /// backend, release capacity or assert that a mutation was cancelled.
    pub(super) fn terminate(&self, child: &mut Child, deadline: Instant) -> Result<(), ScopeError> {
        self.verify_owner()?;
        if child.try_wait().map_err(|_| ScopeError::Reap)?.is_none() {
            // An unreaped Child retains its numeric PID. No all-child wait occurs until this
            // handle has observed and cached its own exit status.
            if child.kill().is_err() && child.try_wait().map_err(|_| ScopeError::Reap)?.is_none() {
                return Err(ScopeError::Signal);
            }
        }
        loop {
            if child.try_wait().map_err(|_| ScopeError::Reap)?.is_some() {
                return self.drain(deadline);
            }
            pause_until(deadline)?;
        }
    }

    /// Kill only verified adopted direct children, then require complete kernel reaping.
    pub(super) fn drain(&self, deadline: Instant) -> Result<(), ScopeError> {
        self.verify_owner()?;
        loop {
            for identity in self.children(deadline)? {
                check_deadline(deadline)?;
                self.signal(identity, deadline)?;
            }
            let mut reaped = 0;
            loop {
                check_deadline(deadline)?;
                match wait(WaitOptions::NOHANG) {
                    Err(Errno::CHILD) => return Ok(()),
                    Ok(Some(_)) => {
                        reaped += 1;
                        if reaped >= MAX_OWNED_CHILDREN || Instant::now() >= deadline {
                            return Err(ScopeError::Pending);
                        }
                    }
                    Ok(None) => break,
                    Err(Errno::INTR) => continue,
                    Err(_) => return Err(ScopeError::Reap),
                }
            }
            pause_until(deadline)?;
        }
    }

    fn verify_owner(&self) -> Result<(), ScopeError> {
        if self.owner != getpid()
            || child_subreaper().map_err(|_| ScopeError::Inspection)? != Pid::from_raw(1)
        {
            return Err(ScopeError::Identity);
        }
        Ok(())
    }

    fn children(&self, deadline: Instant) -> Result<Vec<ChildIdentity>, ScopeError> {
        let mut children = Vec::new();
        for (count, entry) in fs::read_dir("/proc")
            .map_err(|_| ScopeError::Inspection)?
            .enumerate()
        {
            check_deadline(deadline)?;
            if count >= MAX_PROC_ENTRIES {
                return Err(ScopeError::Inspection);
            }
            let entry = entry.map_err(|_| ScopeError::Inspection)?;
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<i32>().ok())
                .and_then(Pid::from_raw)
            else {
                continue;
            };
            let identity = match read_identity(pid, deadline) {
                Ok(identity) => identity,
                // Unrelated processes may exit while /proc is enumerated. No absence claim
                // derives from this scan; a living owned child prevents the ECHILD proof.
                Err(ScopeError::Pending) => return Err(ScopeError::Pending),
                Err(_) => continue,
            };
            if identity.parent == self.owner.as_raw_nonzero().get() as u32 {
                if children.len() >= MAX_OWNED_CHILDREN {
                    return Err(ScopeError::Inspection);
                }
                children.push(identity);
            }
        }
        Ok(children)
    }

    fn signal(&self, before: ChildIdentity, deadline: Instant) -> Result<(), ScopeError> {
        check_deadline(deadline)?;
        let descriptor = match pidfd_open(before.pid, PidfdFlags::empty()) {
            Ok(descriptor) => descriptor,
            Err(Errno::SRCH) => return Ok(()),
            Err(_) => return Err(ScopeError::Inspection),
        };
        let after = read_identity(before.pid, deadline)?;
        if before != after
            || after.parent != self.owner.as_raw_nonzero().get() as u32
            || after.uid != rustix::process::geteuid().as_raw()
        {
            return Err(ScopeError::Identity);
        }
        match pidfd_send_signal(&descriptor, Signal::KILL) {
            Ok(()) | Err(Errno::SRCH) => Ok(()),
            Err(_) => Err(ScopeError::Signal),
        }
    }
}

fn no_existing_child() -> Result<(), ScopeError> {
    match waitid(
        WaitId::All,
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    ) {
        Err(Errno::CHILD) => Ok(()),
        Ok(_) => Err(ScopeError::ExistingChild),
        Err(_) => Err(ScopeError::Inspection),
    }
}

fn read_identity(pid: Pid, deadline: Instant) -> Result<ChildIdentity, ScopeError> {
    let directory = format!("/proc/{}", pid.as_raw_nonzero());
    check_deadline(deadline)?;
    let first = stat(&directory)?;
    check_deadline(deadline)?;
    let metadata = fs::metadata(&directory).map_err(|_| ScopeError::Inspection)?;
    check_deadline(deadline)?;
    let second = stat(&directory)?;
    check_deadline(deadline)?;
    let second_metadata = fs::metadata(&directory).map_err(|_| ScopeError::Inspection)?;
    if first != second || metadata.uid() != second_metadata.uid() {
        return Err(ScopeError::Identity);
    }
    Ok(ChildIdentity {
        pid,
        parent: second.0,
        birth: second.1,
        uid: second_metadata.uid(),
    })
}

fn stat(directory: &str) -> Result<(u32, u64), ScopeError> {
    let mut text = String::new();
    fs::File::open(format!("{directory}/stat"))
        .map_err(|_| ScopeError::Inspection)?
        .take(MAX_STAT_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| ScopeError::Inspection)?;
    if text.len() as u64 > MAX_STAT_BYTES {
        return Err(ScopeError::Inspection);
    }
    let suffix = text
        .rfind(')')
        .and_then(|end| text.get(end + 1..))
        .ok_or(ScopeError::Identity)?;
    let fields = suffix.split_whitespace().collect::<Vec<_>>();
    let parent = fields.get(1).and_then(|v| v.parse().ok());
    let birth = fields.get(19).and_then(|v| v.parse().ok());
    parent.zip(birth).ok_or(ScopeError::Identity)
}

fn pause_until(deadline: Instant) -> Result<(), ScopeError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ScopeError::Pending);
    }
    std::thread::sleep(remaining.min(Duration::from_millis(1)));
    Ok(())
}

fn check_deadline(deadline: Instant) -> Result<(), ScopeError> {
    if Instant::now() >= deadline {
        Err(ScopeError::Pending)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "http_owned_process_scope_tests.rs"]
mod tests;
