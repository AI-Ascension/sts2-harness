// SPDX-License-Identifier: MIT

//! Liveness of the transport child when a worker thread cannot be created.
//!
//! Split from `sts2_jev_bridge_tests` so that suite stays under its preferred size, and
//! kept apart because the property under test is about the process table rather than about
//! what the bridge decides.

use super::*;
use crate::tests::shell_fixture;
use crate::transport_worker::{RefusalGuard, TransportWorker};

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

/// A refused writer spawn must kill the transport, not merely report the refusal.
///
/// The refusal is driven through the real `exchange`, because a correct `kill_child` says nothing
/// about `exchange` calling it. The child is a real process spawned by the real `Command`, and the
/// refusal is the host's own `EAGAIN`; only the *timing* of the refusal is supplied by the test, so
/// nothing here depends on the machine's thread limit. Reverting this arm to a plain `map_err`
/// leaves the helper defined and untouched and lets the real child live out its `sleep`, which is
/// what the assertion below notices.
#[cfg(unix)]
#[test]
fn a_refused_writer_spawn_kills_the_transport_through_exchange() {
    let fixture = TransportFixture::new("refused-writer", "sleep 30\n");
    let reported = with_refused(Writer, &fixture);
    assert_refused(&reported, "cannot start the transport writer");
    fixture.assert_not_running("the transport outlived a refused writer spawn");
}

/// A refused reader spawn must kill the transport too, and this is the arm that needs it.
///
/// By the time the reader is attempted the writer already exists and the transport is already being
/// fed, so a bare `?` here leaves a live child *and* a detached writer parked on a pipe the bridge
/// has stopped servicing. The child is the real one the bridge spawned; only the refusal's timing
/// comes from the test.
#[cfg(unix)]
#[test]
fn a_refused_reader_spawn_kills_the_transport_through_exchange() {
    // The fixture drains stdin, so the writer thread is not what ends this exchange: the refusal is.
    let fixture = TransportFixture::new("refused-reader", "cat > /dev/null\nsleep 30\n");
    let reported = with_refused(Reader, &fixture);
    assert_refused(&reported, "cannot start the transport reader");
    fixture.assert_not_running("the transport outlived a refused reader spawn");
}

/// Runs the real `exchange` against `fixture` while `worker`'s spawn is refused.
///
/// The refusal is armed only for this call and only on this thread, so what runs is the code that
/// ships: the real `Command`, the real pipes, the real `kill_child`.
#[cfg(unix)]
fn with_refused(
    worker: TransportWorker,
    fixture: &TransportFixture,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let _refusal = RefusalGuard::refuse(worker, &fixture.pid_file);
    exchange(
        &fixture.script.display().to_string(),
        b"{}",
        Duration::from_secs(5),
    )
}

/// A transport that publishes the pid of the process `exchange` actually spawned.
///
/// Two simpler designs both turned out to be vacuous here, and ruling them out is the reason this
/// one exists. Watching a *probe* process proves nothing: `exchange` spawns its own child, the
/// probe is a different process the bridge never owned, and it exits on its own -- so the check
/// passed with the fix deleted. Letting the child write its pid and reading the file afterwards
/// fails the other way: the refusal can arrive before the child has run, so the file is never
/// written and "the child is gone" is indistinguishable from "the child never existed". Publishing
/// the pid *while the refusal waits for it* removes the race in both directions: the refusal cannot
/// return until the transport has identified itself, so the pid is always the real child's.
#[cfg(unix)]
struct TransportFixture {
    script: std::path::PathBuf,
    pid_file: std::path::PathBuf,
}

#[cfg(unix)]
impl TransportFixture {
    /// Writes a transport that announces its own pid and then stays alive to be observed.
    fn new(label: &str, body: &str) -> Self {
        let pid_file = std::env::temp_dir().join(format!("sts2-jev-{label}-pid"));
        let _ = std::fs::remove_file(&pid_file);
        let script = shell_fixture(
            &format!("sts2-jev-transport-{label}.sh"),
            &format!(
                "#!/bin/sh\nprintf '%s' \"$$\" > '{}'\n{body}",
                pid_file.display()
            ),
        );
        TransportFixture { script, pid_file }
    }

    /// Asserts the transport `exchange` spawned is gone.
    ///
    /// The pid is required to be present first: this asserts a named process stopped running, not
    /// that a lookup stopped finding something, so a case in which the transport never started
    /// fails loudly instead of passing.
    fn assert_not_running(&self, complaint: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Some(pid) = self.published_pid() {
                break pid;
            }
            assert!(
                Instant::now() < deadline,
                "the transport published no pid, so {complaint} cannot be concluded from this case"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        assert_not_running(pid, complaint);
    }

    /// The pid the transport published, once it has.
    fn published_pid(&self) -> Option<u32> {
        std::fs::read_to_string(&self.pid_file)
            .ok()
            .and_then(|written| written.trim().parse::<u32>().ok())
    }
}

#[cfg(unix)]
impl Drop for TransportFixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.script);
        let _ = std::fs::remove_file(&self.pid_file);
    }
}

/// Asserts `pid` is no longer running, having first given the kill time to land.
#[cfg(unix)]
fn assert_not_running(pid: u32, complaint: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while is_running(pid) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!is_running(pid), "{complaint} (pid {pid})");
}

/// Asserts the refusal was reported under the name `exchange` gives that arm, in the host's own
/// `EAGAIN` wording, so a case cannot pass by failing for some unrelated reason.
#[cfg(unix)]
fn assert_refused(reported: &Result<Vec<u8>, Box<dyn std::error::Error>>, expected_arm: &str) {
    let message = reported
        .as_ref()
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "the exchange reported success".to_owned());
    assert!(
        message.contains(expected_arm),
        "expected the {expected_arm} refusal, got: {message}"
    );
    assert!(
        message.contains("Resource temporarily unavailable"),
        "expected the host's own EAGAIN wording, got: {message}"
    );
}
