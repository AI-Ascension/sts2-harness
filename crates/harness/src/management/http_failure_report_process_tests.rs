// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use crate::management::ManagementFailureSink;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_SINK_CHILD_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD";
const DEFAULT_SINK_CHILD_TOKEN: &str = "capture-default-stderr-v1";
const DEFAULT_SINK_CHILD_MODE_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD_MODE";
const DEFAULT_SINK_CHILD_ROLE_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD_ROLE";
const DEFAULT_SINK_CHILD_SYNC_DIR_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD_SYNC_DIR";
pub(super) const DEFAULT_SINK_CHILD_LINE: &str = "sts2-management undeliverable response";
pub(super) const DEFAULT_SINK_CHILD_POST_PANIC_LINE: &str =
    "sts2-management report after a caught test panic";
pub(super) const DEFAULT_SINK_CHILD_PANIC_MARKER: &str = "synthetic default-sink child panic";
pub(super) const DEFAULT_SINK_CHILD_TEST: &str =
    "management::http::failure_report::tests::process_support::default_sink_child_emits_report";
pub(super) const DEFAULT_SINK_CHILD_CONCURRENT_TEST: &str = "management::http::failure_report::tests::process_support::default_sink_child_emits_concurrent_reports";
pub(super) const DEFAULT_SINK_CHILD_PANIC_TEST: &str = "management::http::failure_report::tests::process_support::default_sink_child_reports_after_caught_panic";
pub(super) const DEFAULT_SINK_CHILD_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SINK_CHILD_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);
const CONCURRENT_REPORTS_PER_CHILD: usize = 4;
const MAX_CHILD_OUTPUT_BYTES: u64 = 4096;

/// A unique child-owned capture directory under Cargo's target output.
pub(super) struct ChildScratch(PathBuf);

impl ChildScratch {
    pub(super) fn new() -> Self {
        let test_directory = std::env::current_exe()
            .expect("the test executable path must be available")
            .parent()
            .expect("the test executable must have a parent directory")
            .to_path_buf();
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the system clock must be after the Unix epoch")
            .as_nanos();
        let directory = test_directory.join(format!(
            "management-default-stderr-{}-{timestamp}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("the child capture directory must be unique");
        Self(directory)
    }

    pub(super) fn stdout_path(&self, child: &str) -> PathBuf {
        self.0.join(format!("{child}-stdout.txt"))
    }

    pub(super) fn stderr_path(&self, child: &str) -> PathBuf {
        self.0.join(format!("{child}-stderr.txt"))
    }
}

impl Drop for ChildScratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Kills and reaps the owned child if the bounded wait or an assertion unwinds early.
pub(super) struct OwnedChild(Child);

impl OwnedChild {
    pub(super) fn wait_until(&mut self, timeout: Duration) -> ExitStatus {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .0
                .try_wait()
                .expect("the child status must be readable")
            {
                return status;
            }
            assert!(
                Instant::now() < deadline,
                "the default-sink test child exceeded its {timeout:?} bound"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

pub(super) fn spawn_default_sink_child(
    scratch: &ChildScratch,
    child: &str,
    test: &str,
    mode: &str,
) -> OwnedChild {
    let stdout = File::create(scratch.stdout_path(child)).expect("the child stdout file must open");
    let stderr = File::create(scratch.stderr_path(child)).expect("the child stderr file must open");
    let child = Command::new(std::env::current_exe().expect("the test executable path"))
        .args([
            "--exact",
            test,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(DEFAULT_SINK_CHILD_ENV, DEFAULT_SINK_CHILD_TOKEN)
        .env(DEFAULT_SINK_CHILD_MODE_ENV, mode)
        .env(DEFAULT_SINK_CHILD_ROLE_ENV, child)
        .env(DEFAULT_SINK_CHILD_SYNC_DIR_ENV, &scratch.0)
        .env("RUST_BACKTRACE", "0")
        .env("RUST_LIB_BACKTRACE", "0")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("the isolated default-sink test child must start");
    OwnedChild(child)
}

fn is_default_sink_child(mode: &str) -> bool {
    std::env::var(DEFAULT_SINK_CHILD_ENV).as_deref() == Ok(DEFAULT_SINK_CHILD_TOKEN)
        && std::env::var(DEFAULT_SINK_CHILD_MODE_ENV).as_deref() == Ok(mode)
}

pub(super) fn reports_for_child(role: &str) -> Vec<String> {
    (0..CONCURRENT_REPORTS_PER_CHILD)
        .map(|index| format!("sts2-management concurrent child={role} report={index}"))
        .collect()
}

/// Releases two bounded child processes together before either emits its distinct report set.
fn wait_for_peer_child(role: &str) {
    let sync_dir = PathBuf::from(
        std::env::var_os(DEFAULT_SINK_CHILD_SYNC_DIR_ENV)
            .expect("the concurrent child sync directory must be set"),
    );
    let ready_path = sync_dir.join(format!("{role}.ready"));
    File::create(ready_path).expect("the child readiness marker must be created");
    assert!(
        matches!(role, "left" | "right"),
        "the concurrent child role must be left or right"
    );
    let peer_role = if role == "left" { "right" } else { "left" };
    let peer_path = sync_dir.join(format!("{peer_role}.ready"));
    let deadline = Instant::now() + DEFAULT_SINK_CHILD_BARRIER_TIMEOUT;
    while !peer_path.exists() {
        assert!(
            Instant::now() < deadline,
            "the concurrent child barrier did not release before its bound"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn assert_child_test_passed(stdout: &str, helper: &str) {
    assert!(
        stdout.contains(&format!("{helper} ... ok"))
            && stdout.contains("test result: ok. 1 passed; 0 failed"),
        "the child must run exactly the isolated helper test, got {stdout:?}"
    );
}

pub(super) fn read_child_output(path: &Path) -> String {
    let size = fs::metadata(path)
        .expect("the child output metadata must be readable")
        .len();
    assert!(
        size <= MAX_CHILD_OUTPUT_BYTES,
        "the bounded child output exceeded {MAX_CHILD_OUTPUT_BYTES} bytes: {size}"
    );
    fs::read_to_string(path).expect("the child output must be UTF-8 text")
}

#[test]
#[ignore = "invoked only by the owned subprocess regressions"]
fn default_sink_child_emits_report() {
    if !is_default_sink_child("single") {
        return;
    }
    ManagementFailureSink::default().report(DEFAULT_SINK_CHILD_LINE);
}

#[test]
#[ignore = "invoked only by the owned subprocess regressions"]
fn default_sink_child_emits_concurrent_reports() {
    if !is_default_sink_child("concurrent") {
        return;
    }
    let role =
        std::env::var(DEFAULT_SINK_CHILD_ROLE_ENV).expect("the concurrent child role must be set");
    wait_for_peer_child(&role);

    let reports = reports_for_child(&role);
    let barrier = Arc::new(Barrier::new(reports.len() + 1));
    let workers = reports
        .into_iter()
        .map(|line| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                ManagementFailureSink::default().report(&line);
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    for worker in workers {
        worker
            .join()
            .expect("every concurrent report worker must finish");
    }
}

#[test]
#[ignore = "invoked only by the owned subprocess regressions"]
#[allow(clippy::panic)]
fn default_sink_child_reports_after_caught_panic() {
    if !is_default_sink_child("panic") {
        return;
    }
    let panic_result = std::panic::catch_unwind(|| panic!("{DEFAULT_SINK_CHILD_PANIC_MARKER}"));
    assert!(panic_result.is_err(), "the synthetic panic must be caught");
    ManagementFailureSink::default().report(DEFAULT_SINK_CHILD_POST_PANIC_LINE);
}
