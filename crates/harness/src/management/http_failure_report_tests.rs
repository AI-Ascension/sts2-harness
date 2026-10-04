// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

// ## The hole this closes
//
// #817 stopped discarding a management response the owner could not transmit, and reported it
// with an `eprintln!`. Nothing asserted that line. Deleting the `eprintln!` and restoring
// `let _ = handle_connection(...)` passed all 524 lib tests -- verified twice by an independent
// review, because the line was write-only to a process-wide stream and no gate could see it.
//
// Observability that is hard to unit-test does not get skipped, it gets a port. These tests
// attach a recording `ManagementFailurePort` and assert that a connection which ends without a
// delivered response actually produces a report. Deleting the reporting path makes them fail.
//
// The attribution half is asserted in the same breath: a report that cannot be tied to the peer
// or the request that caused it is much weaker evidence when a run goes red, which is the property
// #816 was actually after.

use super::connection_failure_line;
use super::*;
use crate::management::contract::MAX_RESPONSE_BYTES;
use crate::management::{
    AuthContext, ManagementClient, ManagementFailurePort, ManagementFailureSink, ManagementServer,
    ManagementService, MemoryWorkflowStore, ServerConfig, ServerHandle, StaticAuthenticator,
};
use std::fs::{self, File};
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Barrier, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A port that keeps every line it is handed, so a test can assert on them.
#[derive(Clone, Default)]
struct RecordingFailurePort(Arc<Mutex<Vec<String>>>);

impl RecordingFailurePort {
    fn lines(&self) -> Vec<String> {
        self.0.lock().map(|lines| lines.clone()).unwrap_or_default()
    }
}

impl ManagementFailurePort for RecordingFailurePort {
    fn report(&self, line: &str) {
        if let Ok(mut lines) = self.0.lock() {
            lines.push(line.to_owned());
        }
    }
}

fn recording_sink() -> (ManagementFailureSink, RecordingFailurePort) {
    let port = RecordingFailurePort::default();
    (ManagementFailureSink::new(Arc::new(port.clone())), port)
}

/// Waits for the reporting worker to finish, so an assertion is not racing the accept loop.
fn wait_for_report(port: &RecordingFailurePort) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let lines = port.lines();
        if !lines.is_empty() {
            return lines;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    port.lines()
}

/// Starts a real management server whose undeliverable responses are recorded by `port`.
fn server_recording_to(
    sink: ManagementFailureSink,
    deadline: Duration,
    max_response_bytes: usize,
) -> (SocketAddr, ServerHandle) {
    let authenticator = Arc::new(
        StaticAuthenticator::single(
            "failure-report-token",
            AuthContext::new("failure-report", Vec::<String>::new())
                .expect("the test auth context must be valid"),
        )
        .expect("the test credential must be valid"),
    );
    let service = Arc::new(ManagementService::new(Arc::new(MemoryWorkflowStore::new())));
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("a parseable loopback address"),
        authenticator,
    )
    .expect("the loopback listen address must be accepted");
    config.limits.deadline = deadline;
    config.limits.max_response_bytes = max_response_bytes;
    let config = config.with_failure_sink(sink);
    let handle = ManagementServer::start(config, service).expect("the server must start");
    (handle.address(), handle)
}

/// The bound that makes a real response undeliverable without any timing.
///
/// `dispatch` composes the health answer and checks it against the *absolute* `MAX_RESPONSE_BYTES`,
/// so a small `limits.max_response_bytes` is not caught there. `write_response` then refuses the
/// same body against the *configured* bound and returns `response_too_large` before touching the
/// socket. That is a genuine undeliverable response reached over a real connection, and unlike a
/// spent deadline it cannot be flipped by a contended runner -- which is exactly the lesson #817's
/// own review recorded about a test that only passes when it is unlucky.
const UNDELIVERABLE_RESPONSE_BYTES: usize = 1;

#[test]
fn a_connection_that_cannot_deliver_its_response_is_reported_through_the_attached_port() {
    // The assertion #819 asks for, and the one that no test previously made.
    //
    // A real request is answered, and the response is then refused by the configured response
    // bound before any syscall -- the "answered, then nothing delivered" shape of #816. The peer
    // sees a closed socket; the report is the only surviving evidence that an answer existed.
    let (sink, port) = recording_sink();
    let (address, server) =
        server_recording_to(sink, Duration::from_secs(10), UNDELIVERABLE_RESPONSE_BYTES);

    // A complete, valid request. It parses, dispatches, and produces a response the owner then
    // cannot transmit.
    let client = ManagementClient::new(address, "failure-report-token")
        .expect("the client credential must be accepted");
    let outcome = client.request_json("GET", "/v1/health", None);
    assert!(
        outcome.is_err(),
        "the peer must not receive a response the owner could not send, got {outcome:?}"
    );

    let lines = wait_for_report(&port);
    server.shutdown().expect("the server must shut down");

    assert_eq!(
        lines.len(),
        1,
        "exactly one undeliverable connection must be reported, got {lines:?}"
    );
    let line = &lines[0];
    assert!(
        line.contains("connection ended without a delivered response"),
        "the report must say what happened, got {line:?}"
    );
}

#[test]
fn the_report_names_the_peer_and_the_route_that_failed() {
    // The attribution half. A report that cannot be tied to the request that caused it is much
    // weaker evidence when a run goes red, which is what #816 was after.
    //
    // Driven over a raw socket rather than `ManagementClient` so the peer's address is known
    // exactly: the report must name the ephemeral address the request actually came FROM, which is
    // what lets an operator tie the line to one connection instead of to the shared listener.
    let (sink, port) = recording_sink();
    let (address, server) =
        server_recording_to(sink, Duration::from_secs(10), UNDELIVERABLE_RESPONSE_BYTES);

    let mut client = TcpStream::connect(address).expect("the peer must connect");
    let client_address = client
        .local_addr()
        .expect("the peer's own address must be readable");
    client
        .write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("the request must be written");

    let lines = wait_for_report(&port);
    drop(client);
    server.shutdown().expect("the server must shut down");

    assert_eq!(lines.len(), 1, "expected one report, got {lines:?}");
    let line = &lines[0];
    assert!(
        line.contains(&format!("peer={client_address}")),
        "the report must name the peer address the request came from ({client_address}), got {line:?}"
    );
    assert!(
        line.contains("request=GET /v1/health"),
        "the report must name the route that could not be delivered, got {line:?}"
    );
    assert!(
        line.contains("cause=response_too_large"),
        "the report must carry the owner's own cause code, got {line:?}"
    );
}

#[test]
fn a_delivered_response_reports_nothing() {
    // The other direction. Attribution must not turn every connection into a report: a request the
    // owner answered normally is not an incident, and a port that reported it would be noise that
    // trains an operator to ignore the line.
    let (sink, port) = recording_sink();
    let (address, server) = server_recording_to(sink, Duration::from_secs(10), MAX_RESPONSE_BYTES);

    let client = ManagementClient::new(address, "failure-report-token")
        .expect("the client credential must be accepted");
    let answered = client.request_json("GET", "/v1/health", None);
    assert!(
        answered.is_ok(),
        "a healthy request must be answered, got {answered:?}"
    );

    // Give a report the chance to appear before asserting that none did.
    std::thread::sleep(Duration::from_millis(200));
    let lines = port.lines();
    server.shutdown().expect("the server must shut down");

    assert!(
        lines.is_empty(),
        "a delivered response must not be reported as a failure, got {lines:?}"
    );
}

#[test]
fn a_report_names_no_route_when_the_request_never_parsed() {
    // The honest inverse of attribution: when the owner never learned the route, the line says so
    // rather than guessing one. A guessed route is worse than none, because it would be believed.
    let error = HttpError::new(
        "incomplete_request",
        "request ended before headers completed",
    );
    let line = connection_failure_line(
        "127.0.0.1:5000".parse().expect("a parseable peer address"),
        None,
        &error,
    );
    assert!(
        line.contains("request=(not read)"),
        "an unread request must be reported as unread, got {line:?}"
    );
}

#[test]
fn the_report_carries_no_environment_secret_or_local_path() {
    // `CODING_STANDARDS.md` requires that a boundary never expose credentials, paths, panic text,
    // or arbitrary payloads. The peer is a socket address and the cause is an owner-owned message,
    // so neither can carry one -- but the REQUEST PATH is peer-supplied and must still be escaped
    // and bounded, or a hostile peer could inject newlines and forge operator lines.
    let hostile = "/v1/health\r\nsts2-management: forged line\n";
    let error = HttpError::new("deadline_exceeded", "response deadline exceeded");
    let line = connection_failure_line(
        "127.0.0.1:5000".parse().expect("a parseable peer address"),
        Some(hostile),
        &error,
    );
    assert!(
        !line.contains('\n'),
        "a peer-supplied route must not be able to inject a second line, got {line:?}"
    );
    assert!(
        line.contains("\\r\\n"),
        "a control character in the route must be escaped, not dropped, got {line:?}"
    );

    // And the field is bounded, so a peer cannot flood the operator log through one route.
    let long = format!("/v1/{}", "a".repeat(4096));
    let line = connection_failure_line(
        "127.0.0.1:5000".parse().expect("a parseable peer address"),
        Some(&long),
        &error,
    );
    assert!(
        line.chars().count() < 1024,
        "a report must stay one bounded line, got {} chars",
        line.chars().count()
    );
    assert!(
        line.contains("..."),
        "a clipped field must say that it was clipped, got {line:?}"
    );
}

const DEFAULT_SINK_CHILD_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD";
const DEFAULT_SINK_CHILD_TOKEN: &str = "capture-default-stderr-v1";
const DEFAULT_SINK_CHILD_MODE_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD_MODE";
const DEFAULT_SINK_CHILD_ROLE_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD_ROLE";
const DEFAULT_SINK_CHILD_SYNC_DIR_ENV: &str = "STS2_HARNESS_DEFAULT_SINK_CHILD_SYNC_DIR";
const DEFAULT_SINK_CHILD_LINE: &str = "sts2-management undeliverable response";
const DEFAULT_SINK_CHILD_POST_PANIC_LINE: &str = "sts2-management report after a caught test panic";
const DEFAULT_SINK_CHILD_PANIC_MARKER: &str = "synthetic default-sink child panic";
const DEFAULT_SINK_CHILD_TEST: &str =
    "management::http::failure_report::tests::default_sink_child_emits_report";
const DEFAULT_SINK_CHILD_CONCURRENT_TEST: &str =
    "management::http::failure_report::tests::default_sink_child_emits_concurrent_reports";
const DEFAULT_SINK_CHILD_PANIC_TEST: &str =
    "management::http::failure_report::tests::default_sink_child_reports_after_caught_panic";
const DEFAULT_SINK_CHILD_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_SINK_CHILD_BARRIER_TIMEOUT: Duration = Duration::from_secs(2);
const CONCURRENT_REPORTS_PER_CHILD: usize = 4;
const MAX_CHILD_OUTPUT_BYTES: u64 = 4096;

/// A unique child-owned capture directory under Cargo's target output.
struct ChildScratch(PathBuf);

impl ChildScratch {
    fn new() -> Self {
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

    fn stdout_path(&self, child: &str) -> PathBuf {
        self.0.join(format!("{child}-stdout.txt"))
    }

    fn stderr_path(&self, child: &str) -> PathBuf {
        self.0.join(format!("{child}-stderr.txt"))
    }
}

impl Drop for ChildScratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Kills and reaps the owned child if the bounded wait or an assertion unwinds early.
struct OwnedChild(Child);

impl OwnedChild {
    fn wait_until(&mut self, timeout: Duration) -> ExitStatus {
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

fn spawn_default_sink_child(
    scratch: &ChildScratch,
    child: &str,
    test: &str,
    mode: &str,
) -> OwnedChild {
    let stdout = File::create(scratch.stdout_path(child)).expect("the child stdout file must open");
    let stderr = File::create(scratch.stderr_path(child)).expect("the child stderr file must open");
    let child = Command::new(std::env::current_exe().expect("the test executable path"))
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(DEFAULT_SINK_CHILD_ENV, DEFAULT_SINK_CHILD_TOKEN)
        .env(DEFAULT_SINK_CHILD_MODE_ENV, mode)
        .env(DEFAULT_SINK_CHILD_ROLE_ENV, child)
        .env(DEFAULT_SINK_CHILD_SYNC_DIR_ENV, &scratch.0)
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

fn reports_for_child(role: &str) -> Vec<String> {
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

fn assert_child_test_passed(stdout: &str, helper: &str) {
    assert!(
        stdout.contains(&format!("{helper} ... ok"))
            && stdout.contains("test result: ok. 1 passed; 0 failed"),
        "the child must run exactly the isolated helper test, got {stdout:?}"
    );
}

fn read_child_output(path: &Path) -> String {
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
fn default_sink_child_emits_report() {
    if !is_default_sink_child("single") {
        return;
    }
    ManagementFailureSink::default().report(DEFAULT_SINK_CHILD_LINE);
}

#[test]
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
#[allow(clippy::panic)]
fn default_sink_child_reports_after_caught_panic() {
    if !is_default_sink_child("panic") {
        return;
    }
    let panic_result = std::panic::catch_unwind(|| panic!("{DEFAULT_SINK_CHILD_PANIC_MARKER}"));
    assert!(panic_result.is_err(), "the synthetic panic must be caught");
    ManagementFailureSink::default().report(DEFAULT_SINK_CHILD_POST_PANIC_LINE);
}

#[test]
fn the_default_sink_writes_its_report_to_the_production_path() {
    let scratch = ChildScratch::new();
    let mut child = spawn_default_sink_child(&scratch, "single", DEFAULT_SINK_CHILD_TEST, "single");
    let status = child.wait_until(DEFAULT_SINK_CHILD_TIMEOUT);
    assert!(
        status.success(),
        "the exact helper test must pass, got {status}"
    );

    let stdout = read_child_output(&scratch.stdout_path("single"));
    let stderr = read_child_output(&scratch.stderr_path("single"));
    assert_child_test_passed(&stdout, "default_sink_child_emits_report");
    assert_eq!(
        stderr,
        format!("{DEFAULT_SINK_CHILD_LINE}\n"),
        "the default sink must write exactly one line to the child's real stderr"
    );
}

#[test]
fn concurrent_default_sink_reports_stay_in_their_owned_child_stderr() {
    let scratch = ChildScratch::new();
    let mut left = spawn_default_sink_child(
        &scratch,
        "left",
        DEFAULT_SINK_CHILD_CONCURRENT_TEST,
        "concurrent",
    );
    let mut right = spawn_default_sink_child(
        &scratch,
        "right",
        DEFAULT_SINK_CHILD_CONCURRENT_TEST,
        "concurrent",
    );
    let left_status = left.wait_until(DEFAULT_SINK_CHILD_TIMEOUT);
    let right_status = right.wait_until(DEFAULT_SINK_CHILD_TIMEOUT);
    let left_stdout = read_child_output(&scratch.stdout_path("left"));
    let left_stderr = read_child_output(&scratch.stderr_path("left"));
    let right_stdout = read_child_output(&scratch.stdout_path("right"));
    let right_stderr = read_child_output(&scratch.stderr_path("right"));
    assert!(
        left_status.success(),
        "the left child must pass, got {left_status}; stdout={left_stdout:?}; stderr={left_stderr:?}"
    );
    assert!(
        right_status.success(),
        "the right child must pass, got {right_status}; stdout={right_stdout:?}; stderr={right_stderr:?}"
    );

    assert_child_test_passed(&left_stdout, "default_sink_child_emits_concurrent_reports");
    assert_child_test_passed(&right_stdout, "default_sink_child_emits_concurrent_reports");
    let assert_report_set = |role: &str, stderr: &str| {
        let mut actual = stderr.lines().map(str::to_owned).collect::<Vec<_>>();
        let mut expected = reports_for_child(role);
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(
            actual, expected,
            "the {role} child's stderr must contain only its own concurrent reports"
        );
    };
    assert_report_set("left", &left_stderr);
    assert_report_set("right", &right_stderr);
}

#[test]
fn default_sink_writes_to_actual_stderr_after_a_caught_panic() {
    let scratch = ChildScratch::new();
    let mut child =
        spawn_default_sink_child(&scratch, "panic", DEFAULT_SINK_CHILD_PANIC_TEST, "panic");
    let status = child.wait_until(DEFAULT_SINK_CHILD_TIMEOUT);
    assert!(
        status.success(),
        "the caught-panic child must pass, got {status}"
    );

    let stdout = read_child_output(&scratch.stdout_path("panic"));
    let stderr = read_child_output(&scratch.stderr_path("panic"));
    assert_child_test_passed(&stdout, "default_sink_child_reports_after_caught_panic");
    assert!(
        stderr.contains(DEFAULT_SINK_CHILD_PANIC_MARKER),
        "the child's caught panic must be visible in real stderr, got {stderr:?}"
    );
    assert_eq!(
        stderr.lines().last(),
        Some(DEFAULT_SINK_CHILD_POST_PANIC_LINE),
        "a later default-sink report must still reach real stderr after the caught panic"
    );
}
