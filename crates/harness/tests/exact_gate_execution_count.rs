// SPDX-License-Identifier: MIT

//! Pin the execution count `tools/exact-gate.sh` reads out of a lane's log
//! (sts2-harness#540).
//!
//! The gate exists so a `--ignored --exact <name>` filter that matches nothing
//! cannot report green. It does that by counting, separately from the command's
//! exit status, how many named tests actually executed. The counter was reading
//! `test result: ok. 1 passed;` -- a *summary* line -- and that is the wrong
//! place to look: a run that executed one test and that test FAILED prints
//! `test result: FAILED. 0 passed; 1 failed;`, whose `0 passed` is
//! indistinguishable from the `0 passed; N filtered out` of an empty run. The
//! gate therefore reported a test that ran and failed as `0 executed` and then
//! announced that it "would have passed as an empty run".
//!
//! These cases drive the committed script over the four libtest shapes the
//! counter has to separate, with a stub command that emits each shape, so the
//! test pins the shipped counter rather than a copy of it. Nothing here runs a
//! real `cargo test`; the shapes are the measured output of the pinned
//! toolchain's libtest, and `docs/WORKFLOWS.md` carries the command.
//!
//! This proves the gate's *counting and messaging*, which no hosted lane can
//! show: every real lane either passes (so the counter is never consulted
//! against a failure) or fails on a real defect (so the message is read in a
//! situation nobody would stage). It does not run a lane, and it is not a
//! substitute for the hosted run that proves the wiring.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![cfg(target_os = "linux")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// The gate as it is addressed from a harness checkout.
fn gate() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/exact-gate.sh")
}

/// One measured libtest result line, and what the gate must conclude from it.
struct Case {
    /// What the lane's log looks like.
    log: &'static str,
    /// Exit status of the guarded command.
    status: i32,
    /// Tests the log shows as executed and finished.
    executed: i32,
    /// `--exact` filters the command names, which the gate counts separately.
    named: i32,
    /// The account the gate must give. An empty-run refusal claims a renamed or
    /// removed test; the new failure branch must not say that about a test the
    /// log shows as having run.
    must_say: &'static str,
    /// Something the message must not claim, so the inverted account is pinned
    /// by its absence as well as by the count.
    must_not_say: &'static str,
}

/// `cargo test -- --ignored --exact <name>` over a test that passes.
const PASSING: &str = "\
     Running unittests src/lib.rs (target/debug/deps/probe-d7347656e9176156)

running 1 test
test beta_ignored_runs ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.01s
";

/// `cargo test -- --exact <name>` over a test that fails. The `0 passed` in the
/// summary is the shape the old counter could not read.
const FAILING: &str = "\
     Running unittests src/lib.rs (target/debug/deps/probe-d7347656e9176156)

running 1 test
test gamma_fails ... FAILED

failures:

    gamma_fails

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 3 filtered out; finished in 0.00s
";

/// A filter that matches nothing: the case the gate exists to catch.
const EMPTY: &str = "\
     Running unittests src/lib.rs (target/debug/deps/probe-d7347656e9176156)

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.00s
";

/// `--ignored --exact <name>` where the named test is not `#[ignore]`d, so
/// libtest lists it and then skips it. The `ignored` line is an execution
/// decision libtest made *after* selecting the test, and the gate must not read
/// it as evidence the test body ran.
const FILTERED_AFTER_SELECTION: &str = "\
     Running unittests src/lib.rs (target/debug/deps/probe-d7347656e9176156)

running 1 test
test beta_ignored_runs ... ignored

test result: ok. 0 passed; 0 failed; 1 ignored; 0 measured; 3 filtered out; finished in 0.00s
";

/// Two named tests in one invocation, one of which failed. A single result
/// line covers both, which is why the count has to come from the per-test lines.
const TWO_NAMED_ONE_FAILED: &str = "\
     Running unittests src/lib.rs (target/debug/deps/probe-d7347656e9176156)

running 2 tests
test beta_ignored_runs ... ok
test alpha_ok ... FAILED

failures:

    alpha_ok

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 2 filtered out; finished in 0.01s
";

/// A scratch directory for one gate invocation's log and stub.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        static NONCE: AtomicU64 = AtomicU64::new(0);
        let nonce = NONCE.fetch_add(1, Ordering::Relaxed);
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |value| value.as_secs());
        let root = std::env::temp_dir().join(format!(
            "sts2-exact-gate-{label}-{seconds}-{nonce}-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root)
            .unwrap_or_else(|error| panic!("create {}: {error}", root.display()));
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Run the committed gate over a stub that prints `log` and exits `status`.
///
/// The stub stands in for `cargo test` so the case is deterministic and costs
/// no compilation. The gate is the real file at the real path, and the log
/// path is passed as an argument, so nothing here re-implements the counter.
fn run_gate(case: &Case) -> Output {
    let scratch = Scratch::new("shape");
    let log_path = scratch.path("lane.log");
    let stub = scratch.path("stub.sh");
    // The gate counts the `--exact` tokens in the command it is asked to run, so
    // the stub is named with the same filters the log's case declares. The stub
    // ignores them and prints `log` instead, which keeps the case
    // deterministic; what is under test is the gate's reading of the log.
    let filters: Vec<&str> = std::iter::repeat_n("--exact", case.named as usize).collect();
    fs::write(
        &stub,
        format!(
            "printf '%s' {} \nexit {}\n",
            shell_single_quote(case.log),
            case.status
        ),
    )
    .unwrap_or_else(|error| panic!("write {}: {error}", stub.display()));

    // The gate is a bash script and the lanes execute it directly, so it is
    // executed directly here too: handing it to `/bin/sh` fails on
    // `set -o pipefail`, which is a property of this harness rather than of the
    // gate.
    let mut command = Command::new(gate());
    command.arg(&log_path).arg("/bin/sh").arg(&stub);
    for filter in filters {
        command.arg(filter);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("run {}: {error}", gate().display()));

    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    // The gate reports the count it used on stdout; fold it into the log so a
    // later reader sees the same stream a lane reader sees.
    combined.push_str(&fs::read_to_string(&log_path).unwrap_or_default());
    Output {
        status: output.status,
        stdout: combined.into_bytes(),
        stderr: Vec::new(),
    }
}

/// Quote `text` for a single-quoted `/bin/sh` word.
fn shell_single_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The `exact-gate: exit=… named=… executed=…` line the gate prints.
fn counters(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find(|line| line.starts_with("exact-gate: exit="))
        .unwrap_or_else(|| {
            panic!(
                "the gate printed no counter line:\n{}",
                counters_debug(output)
            )
        })
        .to_owned()
}

fn counters_debug(output: &Output) -> String {
    format!(
        "--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// The executed count the gate reported, read back from its own counter line.
fn reported_executed(output: &Output) -> i32 {
    let line = counters(output);
    line.split_whitespace()
        .find_map(|field| field.strip_prefix("executed="))
        .unwrap_or_else(|| panic!("the counter line has no executed= field: {line}"))
        .parse()
        .unwrap_or_else(|error| panic!("parse the executed count from {line}: {error}"))
}

fn reported_named(output: &Output) -> i32 {
    let line = counters(output);
    line.split_whitespace()
        .find_map(|field| field.strip_prefix("named="))
        .unwrap_or_else(|| panic!("the counter line has no named= field: {line}"))
        .parse()
        .unwrap_or_else(|error| panic!("parse the named count from {line}: {error}"))
}

/// The gate accepts only a run that both executed the named tests and passed.
fn gate_accepted(output: &Output) -> bool {
    output.status.success()
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// A test that ran and passed is executed, and the gate accepts it.
#[test]
fn a_passing_named_test_is_counted_as_executed_and_accepted() {
    let case = Case {
        log: PASSING,
        status: 0,
        executed: 1,
        named: 1,
        must_say: "",
        must_not_say: "REFUSED",
    };
    let output = run_gate(&case);
    assert_eq!(reported_named(&output), case.named);
    assert_eq!(reported_executed(&output), case.executed);
    assert!(gate_accepted(&output), "{}", counters_debug(&output));
    assert!(!text(&output).contains(case.must_not_say));
}

/// The defect #540 reported: a test that ran and FAILED was counted as zero
/// executions, and the refusal then told the reader the test had been renamed
/// or removed -- the one account that is false of a run that demonstrably
/// happened.
#[test]
fn a_failing_named_test_is_counted_as_executed_and_not_reported_as_missing() {
    let case = Case {
        log: FAILING,
        status: 101,
        executed: 1,
        named: 1,
        must_say: "this is a test failure, not a missing test",
        must_not_say: "a renamed or removed test would have passed as an empty run",
    };
    let output = run_gate(&case);
    assert_eq!(reported_executed(&output), case.executed);
    assert!(
        !gate_accepted(&output),
        "a failing test must not be accepted:\n{}",
        counters_debug(&output)
    );
    assert!(
        text(&output).contains(case.must_say),
        "the refusal must account for a failing test as such:\n{}",
        counters_debug(&output)
    );
    assert!(
        !text(&output).contains(case.must_not_say),
        "a run that executed must not be described as an empty run:\n{}",
        counters_debug(&output)
    );
}

/// The case the gate was written for still fails closed: a filter that matched
/// nothing keeps the empty-run refusal, so the fix does not trade a diagnostic
/// defect for a hole in the protection.
#[test]
fn an_empty_run_is_still_refused_as_a_renamed_or_removed_test() {
    let case = Case {
        log: EMPTY,
        status: 0,
        executed: 0,
        named: 1,
        must_say: "a renamed or removed test would have passed as an empty run",
        must_not_say: "not a missing test",
    };
    let output = run_gate(&case);
    assert_eq!(reported_executed(&output), 0);
    assert!(
        !gate_accepted(&output),
        "an empty run must still be refused:\n{}",
        counters_debug(&output)
    );
    assert!(
        text(&output).contains(case.must_say),
        "{}",
        counters_debug(&output)
    );
    assert!(!text(&output).contains(case.must_not_say));
}

/// `ignored` is not an execution. libtest lists a selected test and then skips
/// it when it is not `#[ignore]`d, and the old count would have to keep reading
/// that line as zero executions -- so this pins that the count stays zero and
/// the gate keeps refusing, rather than reading a selection as a run.
#[test]
fn a_test_listed_but_skipped_by_libtest_is_not_counted_as_executed() {
    let case = Case {
        log: FILTERED_AFTER_SELECTION,
        status: 0,
        executed: 0,
        named: 1,
        must_say: "a renamed or removed test would have passed as an empty run",
        must_not_say: "not a missing test",
    };
    let output = run_gate(&case);
    assert_eq!(reported_executed(&output), 0);
    assert!(!gate_accepted(&output), "{}", counters_debug(&output));
    assert!(text(&output).contains(case.must_say));
    assert!(!text(&output).contains(case.must_not_say));
}

/// One invocation naming two tests is counted from the per-test lines, not from
/// a single summary line, so both executions are seen even though the summary
/// reports `1 passed`.
#[test]
fn two_named_tests_in_one_invocation_are_counted_from_the_per_test_lines() {
    let case = Case {
        log: TWO_NAMED_ONE_FAILED,
        status: 101,
        executed: 2,
        named: 2,
        must_say: "this is a test failure, not a missing test",
        must_not_say: "a renamed or removed test would have passed as an empty run",
    };
    let output = run_gate(&case);
    assert_eq!(reported_executed(&output), case.executed);
    assert!(!gate_accepted(&output), "{}", counters_debug(&output));
    assert!(text(&output).contains(case.must_say));
    assert!(!text(&output).contains(case.must_not_say));
}
