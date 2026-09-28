// SPDX-License-Identifier: MIT

//! Terminating a timed-out `gh`, per platform.
//!
//! Split out of the `gh api` seam because the reasoning here is about *process
//! ownership* rather than about the API read, and because the two platforms genuinely
//! differ: unix can address the whole group, Windows cannot. Keeping the difference
//! in one file with one explanation is what stops the unix path from being read as a
//! description of the Windows one.

use std::process::Child;

/// Kill a child's whole process group and reap it, so a timeout leaves nothing behind.
///
/// # Why the group and not just the child
///
/// `gh` is not a leaf. A timed-out `gh` may have spawned helpers, and killing only the direct
/// child leaves those running with the pipes still open -- so the drain threads in
/// `GhRunner::run` never see EOF either, and the gate's own boundedness guarantee would depend
/// on every descendant happening to exit on its own. `process_group(0)` in
/// `GhRunner::run` makes the child a group leader, which turns "kill everything it started"
/// into one addressable operation instead of a walk of a process tree that is racy to
/// enumerate.
///
/// This is the same remedy the harness already uses, for the same reason: see
/// `crates/harness/src/exo_lifecycle/process_reap.rs`, which kills the group with
/// `rustix::process::kill_process_group` before waiting.
///
/// # What this does not cover
///
/// On Windows there is no `process_group` equivalent in the standard library's `Command`, so
/// only the direct child is killed there. That is a real asymmetry rather than a claim of full
/// coverage, and it is stated so nobody reads the unix path as describing the Windows one.
/// The gate runs on `ubuntu-latest` in CI (`.github/workflows/review-of-record.yml`), so the
/// bounded path is the one that ships.
///
#[cfg(unix)]
pub(crate) fn reap(child: &mut Child) {
    if let Some(group) = child
        .id()
        .try_into()
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        // The group id equals the child's pid because the child leads it. An
        // already-reaped child can leave that id reused by an unrelated process, so this
        // signal is not guaranteed to be aimed at what we spawned. Killing the group is
        // still the safer error: the direct kill below runs regardless, and the wait still
        // reaps, so the worst case is a group that a reused id no longer matches.
        let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
    }
    // Both calls are best-effort by design. A child that already exited between the
    // deadline check and the kill reports failure here; the `wait` that follows is
    // still correct, because it is the call that reaps.
    let _ = child.kill();
    let _ = child.wait();
}

/// Kill a child and reap it, so a timeout does not leave a process behind.
///
/// See the unix `reap` for why this is not a `cfg`-shared body: Windows has no
/// `process_group(0)` in the standard library's `Command`, so only the direct child is
/// reachable from here.
#[cfg(not(unix))]
pub(crate) fn reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}
