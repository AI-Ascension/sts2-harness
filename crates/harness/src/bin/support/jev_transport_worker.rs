// SPDX-License-Identifier: MIT

//! The seam the two transport workers of `exchange` are started through.
//!
//! Split from `sts2-jev-bridge` so that executable stays inside its preferred size, and kept apart
//! because the property this file exists to serve is about *who starts a thread*, not about what
//! the bridge decides. Production behaviour is the real `std::thread::Builder`; a test can decline
//! one named worker so the refusal can be driven through the real `exchange`, with a real child.

use std::cell::Cell;

/// Which of the two transport workers a spawn request is for.
///
/// The two arms of `exchange` fail for different reasons: the writer's failure happens before any
/// worker exists, and the reader's after one is already running. A refusal that cannot name which
/// spawn it is refusing could only assert the pair as a whole, which is weaker than the property
/// either arm holds on its own.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum TransportWorker {
    Writer,
    Reader,
}
pub(super) use TransportWorker::{Reader, Writer};
/// Starts one named transport worker through the real `std::thread::Builder`.
///
/// The caller applies the name on the real builder, so both workers are still named exactly as #747
/// named them in production. Only the *act* of starting the thread is behind this seam, which is
/// what lets a test make the host's `EAGAIN` deterministic without `RLIMIT_NPROC` -- that limit is
/// per-user across the whole machine, so its threshold moves with unrelated load.
pub(super) fn spawn_transport_worker<T: Send + 'static>(
    builder: std::thread::Builder,
    worker: TransportWorker,
    body: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<T>> {
    if declined_by_test(worker) {
        // A test that refuses a worker also insists on knowing which process it is refusing about,
        // so the transport is given a bounded moment to publish its pid before the refusal is
        // returned. This is the whole reason the test can be non-vacuous: without a pid, "the
        // child is gone" is indistinguishable from "the child never started", and a test that
        // cannot tell those apart passes whether or not the fix is present. The wait lives here,
        // on the branch only a test can reach, so production timing is untouched.
        wait_for_declined_transport();
        // The refusal a real host returns from `clone`/`clone3` at the process or thread limit.
        return Err(std::io::Error::from_raw_os_error(EAGAIN));
    }
    builder.spawn(body)
}
/// The `EAGAIN` a real `clone`/`clone3` returns when the thread limit is reached.
#[cfg(target_os = "linux")]
const EAGAIN: i32 = 11;
/// The `EAGAIN` a real `clone`/`clone3` returns when the thread limit is reached.
#[cfg(not(target_os = "linux"))]
const EAGAIN: i32 = 11;
thread_local! {
    /// The worker's spawn a test has asked to be refused, if any.
    ///
    /// `thread_local!` rather than a `static`, so the refusal is scoped to the one test that asked
    /// for it: `cargo test` runs each case on its own thread, so no other case can observe it and a
    /// refusal can never leak into an unrelated exchange in the same binary.
    static DECLINED_WORKER: Cell<Option<TransportWorker>> = const { Cell::new(None) };
}
/// Whether a test has asked for this worker's spawn to be refused.
fn declined_by_test(worker: TransportWorker) -> bool {
    DECLINED_WORKER.with(|declined| declined.get() == Some(worker))
}
/// Bounded wait for the transport `exchange` spawned to publish its pid, so a refusing test knows
/// which process it is refusing about.
///
/// This waits for *evidence* rather than sleeping a fixed interval, so a healthy runner pays only
/// the child's own startup. It is the difference between a test that can tell "the child was
/// killed" from "the child never ran", and one that cannot: an earlier version of this case read
/// the absence of a pid file as proof the child was gone, and passed with the fix deleted.
fn wait_for_declined_transport() {
    let pid_file = DECLINED_PID_FILE.with(|file| file.borrow().clone());
    let Some(pid_file) = pid_file else {
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !pid_file.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
thread_local! {
    /// Where a refusing test's transport is told to publish its own pid.
    ///
    /// Unconditional so the seam compiles unchanged with and without `cfg(test)`; only a test ever
    /// sets it, and an unset slot makes `wait_for_declined_transport` return at once, so a
    /// production refusal-free run never waits.
    static DECLINED_PID_FILE: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
#[must_use = "the refusal ends when the guard is dropped"]
/// Refuses the spawn of `worker` for as long as the returned guard is alive.
pub(super) struct RefusalGuard;
#[cfg(test)]
impl RefusalGuard {
    /// Arms the refusal of `worker`, and points it at the file the transport publishes its pid to.
    ///
    /// The pid file is what lets the test name the process it is about; without it a refusal
    /// arrives before the child has necessarily run, and the resulting liveness check would be
    /// asserting about a process nobody ever identified.
    pub(super) fn refuse(worker: TransportWorker, pid_file: &std::path::Path) -> Self {
        DECLINED_WORKER.with(|declined| declined.set(Some(worker)));
        DECLINED_PID_FILE.with(|file| *file.borrow_mut() = Some(pid_file.to_path_buf()));
        RefusalGuard
    }
}
#[cfg(test)]
impl Drop for RefusalGuard {
    fn drop(&mut self) {
        DECLINED_WORKER.with(|declined| declined.set(None));
        DECLINED_PID_FILE.with(|file| *file.borrow_mut() = None);
    }
}
