// SPDX-License-Identifier: MIT

//! Platform-specific reaping for the one-shot lifecycle process effect.
//!
//! The effect spawns one child, exchanges bytes with it, and must leave nothing behind when the
//! exchange is cancelled or times out. How thoroughly that can be done is a property of the
//! operating system rather than of this repository, so the two implementations are kept side by
//! side here and the difference is stated rather than hidden behind a single name.
//!
//! On unix the effect puts the child in its own process group at spawn and this module signals the
//! whole group, so a descendant the bridge spawned is reaped with it.
//!
//! On Windows there is no process group to signal. This module terminates the child itself, which
//! leaves any descendant it spawned running. That is a real difference in guarantee, not an
//! implementation detail: a Windows operator whose transport spawns further processes owns their
//! cleanup. Closing it properly means attaching the child to a job object at spawn, which is a
//! larger change to the spawn path than making the library compile for the platform.

use std::time::Duration;
use tokio::process::Child;

/// Identity this platform needs in order to reap a spawned child.
#[cfg(unix)]
pub(super) type ReapHandle = rustix::process::Pid;

/// Identity this platform needs in order to reap a spawned child.
///
/// Windows terminates through the child handle the effect already holds, so nothing is carried.
#[cfg(windows)]
pub(super) type ReapHandle = ();

/// Captures whatever the platform needs to reap `child` later, or `None` when it cannot be read.
#[cfg(unix)]
pub(super) fn reap_handle(child: &Child) -> Option<ReapHandle> {
    child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw)
}

/// Captures whatever the platform needs to reap `child` later, or `None` when it cannot be read.
///
/// The identifier is read and discarded so an already-exited child is reported the same way on both
/// platforms rather than appearing reapable here and not there.
#[cfg(windows)]
pub(super) fn reap_handle(child: &Child) -> Option<ReapHandle> {
    child.id().map(|_| ())
}

/// Kills the child's process group and waits briefly for it to be reaped.
#[cfg(unix)]
pub(super) async fn terminate(child: &mut Child, handle: ReapHandle) {
    let _ = rustix::process::kill_process_group(handle, rustix::process::Signal::KILL);
    let _ = tokio::time::timeout(Duration::from_millis(250), child.wait()).await;
}

/// Terminates the child and waits briefly for it to be reaped.
///
/// Only the child is terminated; see the module documentation for what that does not cover.
#[cfg(windows)]
pub(super) async fn terminate(child: &mut Child, _handle: ReapHandle) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_millis(250), child.wait()).await;
}
