// SPDX-License-Identifier: MIT

//! Helper process ownership and fail-closed cleanup state.

#![cfg(target_os = "linux")]

use std::os::fd::OwnedFd;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, TryLockError};

use tokio::io::unix::AsyncFd;

pub(super) const SESSION_AVAILABLE: u8 = 0;
pub(super) const SESSION_ACTIVE: u8 = 1;
pub(super) const SESSION_POISONED: u8 = 2;
pub(super) const SESSION_STARTING: u8 = 3;
const CLEANUP_CERTAIN: u8 = 0;
const CLEANUP_UNCERTAIN: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ChildStatus {
    Starting,
    Absent,
    Alive,
    Exited,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RegistryError {
    Busy,
    Occupied,
    Poisoned,
}

/// A single owner slot and a launch gate make the startup transition atomic
/// with respect to other constructors. The owner remains in the slot for the
/// complete controller lifetime, so dropping the controller cannot lose the
/// only process authority while cleanup is uncertain.
struct LaunchRegistry<T> {
    launching: AtomicBool,
    owner: Mutex<Option<Arc<T>>>,
}

impl<T> LaunchRegistry<T> {
    const fn new() -> Self {
        Self {
            launching: AtomicBool::new(false),
            owner: Mutex::new(None),
        }
    }

    fn reserve<'a, R>(
        &'a self,
        owner: &Arc<T>,
        reclaimable: R,
    ) -> Result<LaunchReservation<'a, T>, RegistryError>
    where
        R: Fn(&T) -> bool,
    {
        if self
            .launching
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(RegistryError::Busy);
        }
        let reservation = LaunchReservation { registry: self };
        (|| {
            let mut current = match self.owner.try_lock() {
                Ok(current) => current,
                Err(TryLockError::WouldBlock) => return Err(RegistryError::Busy),
                Err(TryLockError::Poisoned(_)) => return Err(RegistryError::Poisoned),
            };
            if let Some(existing) = current.as_ref() {
                if !reclaimable(existing) {
                    return Err(RegistryError::Occupied);
                }
                let previous = current.replace(Arc::clone(owner));
                drop(current);
                drop(previous);
                return Ok(reservation);
            }
            *current = Some(Arc::clone(owner));
            drop(current);
            Ok(reservation)
        })()
    }

    #[cfg(test)]
    fn current(&self) -> Result<Option<Arc<T>>, RegistryError> {
        let owner = match self.owner.try_lock() {
            Ok(owner) => owner,
            Err(TryLockError::WouldBlock) => return Err(RegistryError::Busy),
            Err(TryLockError::Poisoned(_)) => return Err(RegistryError::Poisoned),
        };
        Ok(owner.as_ref().map(Arc::clone))
    }

    fn remove_if(&self, owner: &Arc<T>) -> Result<bool, RegistryError> {
        let removed = {
            let mut current = match self.owner.try_lock() {
                Ok(current) => current,
                Err(TryLockError::WouldBlock) => return Err(RegistryError::Busy),
                Err(TryLockError::Poisoned(_)) => return Err(RegistryError::Poisoned),
            };
            if current
                .as_ref()
                .is_some_and(|existing| Arc::ptr_eq(existing, owner))
            {
                current.take()
            } else {
                None
            }
        };
        let was_removed = removed.is_some();
        drop(removed);
        Ok(was_removed)
    }
}

pub(super) struct LaunchReservation<'a, T> {
    registry: &'a LaunchRegistry<T>,
}

impl<T> Drop for LaunchReservation<'_, T> {
    fn drop(&mut self) {
        self.registry.launching.store(false, Ordering::Release);
    }
}

static RETAINED_SESSIONS: OnceLock<LaunchRegistry<VerifierSession>> = OnceLock::new();

fn retained_sessions() -> &'static LaunchRegistry<VerifierSession> {
    RETAINED_SESSIONS.get_or_init(LaunchRegistry::new)
}

pub(super) fn reserve_helper_slot(
    session: &Arc<VerifierSession>,
) -> Result<LaunchReservation<'static, VerifierSession>, RegistryError> {
    retained_sessions().reserve(session, |existing| {
        match existing.state.load(Ordering::Acquire) {
            // A startup owner has no safe reclaim point. In particular, do
            // not infer that `child == None` means that spawn did not happen.
            SESSION_STARTING => false,
            // Ready and active owners still belong to their controller. Their
            // child is never killed or removed by another constructor.
            SESSION_AVAILABLE | SESSION_ACTIVE => false,
            // Only an already-poisoned owner may receive a bounded cleanup
            // attempt. The owner remains in the slot unless exact reap/absent
            // evidence is returned.
            SESSION_POISONED => matches!(
                existing.try_cleanup(),
                ChildStatus::Exited | ChildStatus::Absent
            ),
            _ => false,
        }
    })
}

/// Remove an exact session only after a nonblocking wait proves that its
/// process is reaped (or that startup never created one). A busy or poisoned
/// registry deliberately leaves the Arc retained for a later constructor.
pub(super) fn release_reaped_session(session: &Arc<VerifierSession>) {
    if session.state.load(Ordering::Acquire) == SESSION_STARTING {
        return;
    }
    if !matches!(
        session.child_status(),
        ChildStatus::Exited | ChildStatus::Absent
    ) {
        return;
    }
    let _ = retained_sessions().remove_if(session);
}

pub(super) struct VerifierSession {
    pub(super) control: Mutex<Option<OwnedFd>>,
    pub(super) async_control: Mutex<Option<Arc<AsyncFd<OwnedFd>>>>,
    pub(super) child: Mutex<Option<Child>>,
    pub(super) state: AtomicU8,
    pub(super) cleanup: AtomicU8,
}

impl VerifierSession {
    pub(super) fn child_status(&self) -> ChildStatus {
        let state = self.state.load(Ordering::Acquire);
        let Ok(mut child_slot) = self.child.try_lock() else {
            return ChildStatus::Unknown;
        };
        let Some(child) = child_slot.as_mut() else {
            return if state == SESSION_STARTING {
                ChildStatus::Starting
            } else {
                ChildStatus::Absent
            };
        };
        match child.try_wait() {
            Ok(Some(_)) => ChildStatus::Exited,
            Ok(None) => ChildStatus::Alive,
            Err(_) => ChildStatus::Unknown,
        }
    }

    /// Make one bounded cleanup attempt for an already-poisoned owner. A
    /// wait error, lock contention, or a still-live child keeps the exact Arc
    /// in the registry; no status flag substitutes for that owner.
    fn try_cleanup(&self) -> ChildStatus {
        let Ok(mut child_slot) = self.child.try_lock() else {
            self.cleanup.store(CLEANUP_UNCERTAIN, Ordering::Release);
            return ChildStatus::Unknown;
        };
        let Some(child) = child_slot.as_mut() else {
            self.cleanup.store(CLEANUP_CERTAIN, Ordering::Release);
            return ChildStatus::Absent;
        };
        let status = match child.try_wait() {
            Ok(Some(_)) => ChildStatus::Exited,
            Ok(None) => {
                if child.kill().is_err() {
                    ChildStatus::Unknown
                } else {
                    match child.try_wait() {
                        Ok(Some(_)) => ChildStatus::Exited,
                        Ok(None) => ChildStatus::Alive,
                        Err(_) => ChildStatus::Unknown,
                    }
                }
            }
            Err(_) => {
                let _ = child.kill();
                ChildStatus::Unknown
            }
        };
        if matches!(status, ChildStatus::Exited | ChildStatus::Absent) {
            self.cleanup.store(CLEANUP_CERTAIN, Ordering::Release);
        } else {
            self.cleanup.store(CLEANUP_UNCERTAIN, Ordering::Release);
        }
        status
    }

    pub(super) fn poison(self: &Arc<Self>) {
        self.state.store(SESSION_POISONED, Ordering::Release);
        let uncertain = match self.child.try_lock() {
            Err(_) => true,
            Ok(mut child_slot) => match child_slot.as_mut() {
                None => false,
                Some(child) => match child.try_wait() {
                    Ok(Some(_)) => false,
                    Ok(None) => match child.kill() {
                        Err(_) => true,
                        Ok(()) => matches!(child.try_wait(), Ok(None) | Err(_)),
                    },
                    Err(_) => {
                        let _ = child.kill();
                        true
                    }
                },
            },
        };
        if uncertain {
            retain_uncertain_session(self);
        } else {
            self.cleanup.store(CLEANUP_CERTAIN, Ordering::Release);
            release_reaped_session(self);
        }
    }
}

fn retain_uncertain_session(session: &Arc<VerifierSession>) {
    // The session Arc was registered before spawn and remains in the single
    // registry slot until an exact reap. This flag only records why cleanup is
    // uncertain; it never stands in for the owner itself.
    session.cleanup.store(CLEANUP_UNCERTAIN, Ordering::Release);
}

#[cfg(test)]
#[path = "worker_linux_verifier_lifecycle_tests.rs"]
mod tests;
