// SPDX-License-Identifier: MIT

//! Reaping the `gh` child and everything it spawned.
//!
//! #717 is the part of #702's "correct child reaping" criterion that #712 did not land. #702
//! merged the 60s deadline, the concurrent pipe drain, and the `ETXTBSY` retry, and it kills
//! the child it spawned when the deadline expires. It kills exactly one pid, though, and `gh` is
//! not a leaf process: it can shell out to helpers, and a descendant that outlives the direct
//! child keeps the stdout and stderr pipes open.
//!
//! That is why the shape here is a whole process group rather than a pid. How thoroughly this
//! can be done is a property of the operating system rather than of this seam, so the two
//! platforms are kept side by side here and the difference is stated rather than hidden behind a
//! single name:
//!
//! - On unix the child is put in its own process group at spawn (see
//!   [`into_own_process_group`]) and the group is signalled, so a descendant the `gh` spawned
//!   dies with it and the pipes reach EOF.
//! - On Windows there is no process group to signal: the standard library's `Command` has no
//!   `process_group` equivalent. Only the direct child is killed there, and any descendant it
//!   spawned keeps running. That is a real difference in guarantee, not an implementation
//!   detail. `review-of-record` runs on `ubuntu-latest`
//!   (`.github/workflows/review-of-record.yml`), so the bounded path is the one that ships, and
//!   the unix guarantee is what that lane relies on.

use std::process::Child;

/// Put `command`'s child in its own process group, so a later group signal reaches its
/// descendants too.
///
/// `process_group(0)` asks the kernel to give the child a new group whose id equals its own pid,
/// which is what makes the child the leader of a group nothing else joined. A group kill sent
/// afterwards therefore reaches the `gh` process and anything it spawned, and nothing unrelated
/// to this call.
///
/// No-op off unix, where there is no such call; see the module documentation for what that
/// costs.
#[cfg(unix)]
pub(super) fn into_own_process_group(command: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

/// No-op off unix, where `Command` has no process-group equivalent to ask for.
#[cfg(windows)]
pub(super) fn into_own_process_group(_command: &mut std::process::Command) {}

/// The pid the group signal is addressed to, or `None` when the platform needs none.
#[cfg(unix)]
pub(super) type GroupHandle = rustix::process::Pid;

/// Nothing is carried on Windows, where the child handle the seam already holds is the handle
/// the termination goes through.
#[cfg(windows)]
pub(super) type GroupHandle = ();

/// Capture whatever the platform needs to reap `child`, or `None` when it cannot be read.
#[cfg(unix)]
pub(super) fn group_handle(child: &Child) -> Option<GroupHandle> {
    // `Child::id` is a `u32` that stays readable after the child has been waited on, so this is
    // `Some` whenever the pid fits the kernel's signed pid type. It is `None` only where a pid
    // cannot be expressed as one, and `reap` then falls back to the direct kill, which is the
    // right direction: signal less, never fail to signal the child at all.
    i32::try_from(child.id())
        .ok()
        .and_then(rustix::process::Pid::from_raw)
}

/// Capture and discard the handle, so an already-exited child is reported the same way on both
/// platforms rather than appearing reapable here and not there.
#[cfg(windows)]
pub(super) fn group_handle(child: &Child) -> Option<GroupHandle> {
    child.id().map(|_| ())
}

/// Kill the child's process group, then kill and reap the child itself.
#[cfg(unix)]
pub(super) fn reap(child: &mut Child, handle: Option<GroupHandle>) {
    // #717: the group signal is the one that matters, because a descendant holding the pipes is
    // what makes the timeout's bound meaningless -- the gate returns promptly, but the work it
    // started keeps running with the pipes open. It is addressed to the child's own group only,
    // so it cannot reach anything outside this call.
    if let Some(handle) = handle {
        let _ = rustix::process::kill_process_group(handle, rustix::process::Signal::KILL);
    }
    // The direct kill stays as a fallback rather than being replaced. `child.kill()` addresses the
    // one pid the seam is certain about, so the child is signalled even if the group signal is
    // refused -- a child that already exited between the deadline check and here, or a group this
    // process is not permitted to signal.
    //
    // Both calls are best-effort by design, and the `wait` that follows is the call that reaps.
    // A child that exited between the deadline check and the kill reports failure here without
    // that making the reap wrong.
    let _ = child.kill();
    let _ = child.wait();
}

/// Kill and reap the child itself.
///
/// Only the child is killed; see the module documentation for what that does not cover.
#[cfg(windows)]
pub(super) fn reap(child: &mut Child, _handle: Option<GroupHandle>) {
    let _ = child.kill();
    let _ = child.wait();
}
