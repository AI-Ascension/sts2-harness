// SPDX-License-Identifier: MIT

//! Behavioural coverage for `tools/exact-gate.sh` itself (#540).
//!
//! `exact_gate_lane_coverage.rs` proves every name-filtering lane step is *wired*
//! to the gate. It does not prove the gate itself counts correctly, and a text
//! sweep cannot: the gate's verdict comes from how it reads a real libtest log.
//! This file runs the gate as a process and asserts its verdicts.
//!
//! The defect this pins: the gate originally counted `test result: ok. 1 passed;`
//! summary lines, which counts *passes*, not *executions*. A test that ran and
//! FAILED was reported `matched=0` with the banner "would have passed as an empty
//! run" -- false and inverted, and delivered precisely when a lane is already red
//! and the reader is hunting the wrong cause. The same pattern could never be
//! satisfied by one invocation naming two `--exact` filters, because that prints
//! `test result: ok. 2 passed;`.
//!
//! Each case below runs the real script against a fake `cargo test` that prints a
//! real libtest log shape, so the assertion is about the shipped script, not a
//! restatement of its logic.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// The gate, addressed the way a test in this crate sees it: the harness checkout
/// holding the crate also holds the gate it guards.
fn gate_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tools/exact-gate.sh")
}

/// Scratch directory for the fake libtest command and the captured log.
fn scratch(label: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!("exact-gate-{label}-{}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("scratch directory");
    directory
}

/// Write a fake `cargo test` that prints `log` and exits the way libtest would:
/// non-zero when the log carries a `FAILED` result line, zero otherwise.
fn fake_test(directory: &Path, log: &str) -> PathBuf {
    let path = directory.join("cargo-test.sh");
    let body = "printf '%s' \"$LOG\"\ncase \"$LOG\" in *FAILED*) exit 101 ;; *) exit 0 ;; esac\n";
    std::fs::write(
        &path,
        format!(
            "#!/usr/bin/env bash\nLOG='{}'\n{body}",
            log.replace('\'', "'\\''")
        ),
    )
    .expect("fake test script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("mark the fake test executable");
    }
    path
}

/// The libtest log shapes the gate has to tell apart, and what each must decide.
struct Case {
    label: &'static str,
    log: &'static str,
    filters: &'static [&'static str],
    /// Whether the gate must accept the run.
    accept: bool,
    /// The execution count the banner must report.
    executed: usize,
}

const ONE_PASSED: &str = "running 1 test\n\ntest real_name ... ok\n\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.01s\n";
const ONE_FAILED: &str = "running 1 test\n\ntest real_name ... FAILED\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 7 filtered out; finished in 0.03s\n";
const NONE_RUN: &str = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.00s\n";
const TWO_PASSED: &str = "running 2 tests\n\ntest a ... ok\ntest b ... ok\n\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 8 filtered out; finished in 0.02s\n";

/// The gate's decision is the product of its execution count and the command's
/// exit status, so each log shape is asserted on both.
#[test]
fn the_gate_counts_executions_and_reports_them_truthfully() {
    let cases = [
        Case {
            label: "one-named-test-ran-and-passed",
            log: ONE_PASSED,
            filters: &["--exact", "real_name"],
            accept: true,
            executed: 1,
        },
        Case {
            label: "the-named-test-ran-and-failed",
            log: ONE_FAILED,
            filters: &["--exact", "real_name"],
            accept: false,
            executed: 1,
        },
        Case {
            label: "the-named-test-no-longer-exists",
            log: NONE_RUN,
            filters: &["--exact", "real_name"],
            accept: false,
            executed: 0,
        },
        Case {
            label: "one-invocation-naming-two-filters",
            log: TWO_PASSED,
            filters: &["--exact", "a", "--exact", "b"],
            accept: true,
            executed: 2,
        },
    ];
    for case in cases {
        let directory = scratch(case.label);
        let command = fake_test(&directory, case.log);
        let output = Command::new("bash")
            .arg(gate_path())
            .arg("-")
            .arg(&command)
            .args(case.filters)
            // The gate makes its own scratch log via mktemp; a TMPDIR inherited
            // from the host can name a directory that does not exist.
            .env("TMPDIR", &directory)
            .output()
            .expect("run the gate");
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let counter = stdout
            .lines()
            .find(|line| line.starts_with("exact-gate: exit="))
            .unwrap_or("<no counter line>");
        assert_eq!(
            output.status.code(),
            Some(if case.accept { 0 } else { 1 }),
            "the gate must {} {}: {counter}{stderr}",
            if case.accept { "accept" } else { "refuse" },
            case.label
        );
        assert!(
            counter.contains(&format!("matched={} ", case.executed)),
            "the counter must report {} executions for {}: {counter}",
            case.executed,
            case.label
        );
    }
}

/// A failing test is a command failure, not an empty run. The banner must say so:
/// the pre-#540 gate printed "0 executed ... would have passed as an empty run"
/// for a test that had run and failed, sending the reader after a rename that
/// never happened.
#[test]
fn a_failing_test_is_reported_as_a_command_failure_not_an_empty_run() {
    let directory = scratch("failing-test-reason");
    let command = fake_test(&directory, ONE_FAILED);
    let output = Command::new("bash")
        .arg(gate_path())
        .arg("-")
        .arg(&command)
        .args(["--exact", "real_name"])
        .env("TMPDIR", &directory)
        .output()
        .expect("run the gate");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr.contains("this is a command failure, not an empty run"),
        "a red lane must be told the cause: {stderr}"
    );
    assert!(
        !stderr.contains("would have passed as an empty run"),
        "a test that ran and failed is not an empty run: {stderr}"
    );
    assert!(
        stderr.contains("exited 101 after executing 1 of 1"),
        "the banner must carry the real exit status and execution count: {stderr}"
    );
}

/// The empty-run protection itself must survive the switch to counting
/// executions: a filter that matches nothing still prints no per-test line, so
/// the gate still refuses it, and still says why.
#[test]
fn the_empty_run_protection_survives_the_execution_count() {
    let directory = scratch("empty-run-refused");
    let command = fake_test(&directory, NONE_RUN);
    let output = Command::new("bash")
        .arg(gate_path())
        .arg("-")
        .arg(&command)
        .args(["--exact", "real_name"])
        .env("TMPDIR", &directory)
        .output()
        .expect("run the gate");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr.contains("1 --exact filter(s) named but 0 executed"),
        "a renamed or removed test must still be refused by name: {stderr}"
    );
}
