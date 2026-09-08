// SPDX-License-Identifier: MIT

//! Helper process ownership and fail-closed cleanup state.

#![cfg(target_os = "linux")]

use std::os::fd::OwnedFd;
use std::process::Child;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use tokio::io::unix::AsyncFd;

pub(super) const SESSION_AVAILABLE: u8 = 0;
pub(super) const SESSION_ACTIVE: u8 = 1;
pub(super) const SESSION_POISONED: u8 = 2;
const CLEANUP_CERTAIN: u8 = 0;
const CLEANUP_UNCERTAIN: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChildStatus {
    Alive,
    Exited,
    Unknown,
}

/// A process-wide fail-closed registry for a helper whose kill/reap result was
/// uncertain. A new controller may launch a child only after every retained
/// authority has been observed reaped.
static RETAINED_SESSIONS: OnceLock<Mutex<Vec<Arc<VerifierSession>>>> = OnceLock::new();
static RETAINED_BLOCK: AtomicU8 = AtomicU8::new(0);
static RETAINED_REGISTRY_INCOMPLETE: AtomicU8 = AtomicU8::new(0);

pub(super) struct VerifierSession {
    pub(super) control: Mutex<Option<OwnedFd>>,
    pub(super) async_control: Mutex<Option<Arc<AsyncFd<OwnedFd>>>>,
    pub(super) child: Mutex<Child>,
    pub(super) state: AtomicU8,
    pub(super) cleanup: AtomicU8,
}

fn retained_sessions() -> &'static Mutex<Vec<Arc<VerifierSession>>> {
    RETAINED_SESSIONS.get_or_init(|| Mutex::new(Vec::new()))
}

pub(super) fn may_launch_helper() -> bool {
    if RETAINED_BLOCK.load(Ordering::Acquire) != 0 {
        if RETAINED_REGISTRY_INCOMPLETE.load(Ordering::Acquire) != 0 {
            return false;
        }
        let Ok(mut sessions) = retained_sessions().try_lock() else {
            return false;
        };
        sessions.retain(|session| session.child_status() != ChildStatus::Exited);
        let blocked = sessions
            .iter()
            .any(|session| session.child_status() != ChildStatus::Exited);
        if !blocked {
            RETAINED_BLOCK.store(0, Ordering::Release);
        }
        return !blocked;
    }
    let Ok(mut sessions) = retained_sessions().try_lock() else {
        return false;
    };
    sessions.retain(|session| session.child_status() != ChildStatus::Exited);
    !sessions
        .iter()
        .any(|session| session.child_status() != ChildStatus::Exited)
}

pub(super) fn retain_uncertain_session(session: &Arc<VerifierSession>) {
    RETAINED_BLOCK.store(1, Ordering::Release);
    if session
        .cleanup
        .compare_exchange(
            CLEANUP_CERTAIN,
            CLEANUP_UNCERTAIN,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .is_err()
    {
        return;
    }
    if let Ok(mut sessions) = retained_sessions().try_lock() {
        sessions.push(Arc::clone(session));
    } else {
        // The controller still owns the child, but the process-wide registry
        // could not take an Arc. Keep the global block set permanently: a
        // later controller cannot prove that this child has been reaped.
        RETAINED_REGISTRY_INCOMPLETE.store(1, Ordering::Release);
    }
}

impl VerifierSession {
    pub(super) fn child_status(&self) -> ChildStatus {
        let Ok(mut child) = self.child.try_lock() else {
            return ChildStatus::Unknown;
        };
        match child.try_wait() {
            Ok(Some(_)) => ChildStatus::Exited,
            Ok(None) => ChildStatus::Alive,
            Err(_) => ChildStatus::Unknown,
        }
    }

    pub(super) fn poison(self: &Arc<Self>) {
        self.state.store(SESSION_POISONED, Ordering::Release);
        let Ok(mut child) = self.child.try_lock() else {
            retain_uncertain_session(self);
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                if child.kill().is_err() || matches!(child.try_wait(), Ok(None) | Err(_)) {
                    retain_uncertain_session(self);
                }
            }
            Err(_) => {
                let _ = child.kill();
                retain_uncertain_session(self);
            }
        }
    }
}
