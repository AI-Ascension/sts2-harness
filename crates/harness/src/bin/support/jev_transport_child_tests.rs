// SPDX-License-Identifier: MIT

//! Liveness of the transport child when a worker thread cannot be created.
//!
//! Split from `sts2_jev_bridge_tests` so that suite stays under its preferred size, and
//! kept apart because the property under test is about the process table rather than about
//! what the bridge decides.

use super::*;
use crate::tests::shell_fixture;

/// A child that is not killed when a worker thread cannot start outlives the bridge.
///
/// `std::process::Child` drops without signalling the process, so a `?` on a failed thread spawn
/// returns an error *and* leaves the transport running. This asserts the kill, because that is the
/// part a reader of the error alone cannot see: the error is identical either way, and only the
/// process table distinguishes a reported failure from a leaked one.
///
/// The check is a `/proc` liveness poll rather than a `try_wait`, because the child is still ours
/// and has not been waited on, so `try_wait` could report "running" for a process that had in fact
/// already exited and been left as a zombie. `/proc` is the only view that tells the two apart.
#[cfg(unix)]
#[test]
fn killing_a_child_reports_the_error_and_leaves_no_running_transport() {
    let script = shell_fixture(
        "sts2-jev-transport-kill-on-spawn-failure.sh",
        "#!/bin/sh\nsleep 30\n",
    );
    let mut child = Command::new(&script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn the fixture transport");
    let pid = child.id();
    assert!(is_running(pid), "the fixture transport did not start");

    let reported = kill_child(&mut child, "cannot start the transport writer".to_owned());

    assert_eq!(reported, "cannot start the transport writer");
    let deadline = Instant::now() + Duration::from_secs(5);
    while is_running(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !is_running(pid),
        "the transport was still running {pid} five seconds after the failure was reported"
    );
    let _ = std::fs::remove_file(&script);
}

/// Whether `pid` names a process that is running rather than one that has exited.
#[cfg(unix)]
fn is_running(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        // No `/proc` entry at all is a process that is gone, which is the answer this wants.
        return false;
    };
    // `stat` is `pid (comm) state ...`, and comm may contain spaces or parentheses, so the state
    // is the field after the *last* `)`. `Z` is a zombie: exited, not reaped, and not running.
    let Some(after_comm) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
        return false;
    };
    !after_comm.trim_start().starts_with('Z')
}
