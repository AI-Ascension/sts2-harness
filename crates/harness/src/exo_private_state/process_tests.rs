// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used)]

use super::*;
use std::process::{Child, Command, ExitStatus, Stdio};

const LIVE_CHILD_PROBE_ENV: &str = "STS2_PRIVATE_CHILD_PROBE";
const LIVE_CHILD_PROBE_VALUE: &str = "owned-live-child";
const LIVE_CHILD_PROBE_TEST: &str = "exo_private_state::process::tests::current_subreaper_check_does_not_reap_or_signal_unowned_processes";

struct OwnedProbeChild(Option<Child>);

impl OwnedProbeChild {
    fn spawn() -> Self {
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn the test-owned live child");
        Self(Some(child))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.0
            .as_mut()
            .expect("test-owned child handle remains available")
            .try_wait()
    }

    fn kill_and_reap(&mut self) -> std::io::Result<ExitStatus> {
        let child = self
            .0
            .as_mut()
            .expect("test-owned child is cleaned up only once");
        if child.try_wait()?.is_none() {
            match child.kill() {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {}
                Err(error) => return Err(error),
            }
        }
        let status = child.wait()?;
        let _ = self.0.take();
        Ok(status)
    }
}

impl Drop for OwnedProbeChild {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if child.try_wait().map_or(true, |status| status.is_none()) {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[test]
fn current_subreaper_check_does_not_reap_or_signal_unowned_processes() {
    if std::env::var(LIVE_CHILD_PROBE_ENV).as_deref() == Ok(LIVE_CHILD_PROBE_VALUE) {
        verify_refusal_preserves_owned_live_child();
        return;
    }

    let output = Command::new(std::env::current_exe().expect("current test executable"))
        .args(["--exact", LIVE_CHILD_PROBE_TEST, "--nocapture"])
        .env(LIVE_CHILD_PROBE_ENV, LIVE_CHILD_PROBE_VALUE)
        .output()
        .expect("launch the isolated exact-filter child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "isolated child test failed: stdout={stdout}; stderr={stderr}"
    );
    assert!(
        stdout.contains("running 1 test"),
        "unexpected test output: {stdout}"
    );
    assert!(
        stdout.contains(&format!("test {LIVE_CHILD_PROBE_TEST} ... ok")),
        "the exact child test did not report success: {stdout}"
    );
}

fn verify_refusal_preserves_owned_live_child() {
    let mut child = OwnedProbeChild::spawn();
    assert_eq!(
        verify_no_preexisting_children(),
        Err("exo_private_preexisting_child"),
        "the production admission check refuses the known live child"
    );
    assert!(
        child
            .try_wait()
            .expect("probe the child without reaping it")
            .is_none(),
        "the refusal must leave the owner's child alive and unreaped"
    );
    let status = child
        .kill_and_reap()
        .expect("the owner kills and reaps its own child");
    assert!(
        !status.success(),
        "the explicit cleanup terminates the child"
    );
}

#[test]
fn process_stat_includes_parent_and_start_identity() {
    let current = getpid().as_raw_nonzero().get() as u32;
    let identity = stat(current).expect("the current process has proc stat");
    assert!(identity.parent_pid > 0);
    assert!(identity.start_time_ticks > 0);
    assert!(identity.session > 0);
    assert!(identity.process_group > 0);
}
