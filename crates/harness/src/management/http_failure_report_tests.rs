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
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Captures what the *production* port writes, by redirecting the writer seam.
///
/// The seam is process-wide, so this holds [`STDERR_WRITER_LOCK`] for the whole substitution and
/// restores real stderr before releasing it. Without the restore the substitution would leak into
/// every other test that reports a management failure, which is the kind of cross-test coupling that
/// makes a suite report green while asserting nothing.
fn capture_production_reports(test_body: impl FnOnce()) -> Vec<String> {
    let _guard = STDERR_WRITER_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = Arc::clone(&captured);
    redirect_production_reports(Arc::new(move |line: &str| {
        if let Ok(mut lines) = recorder.lock() {
            lines.push(line.to_owned());
        }
    }))
    .expect("the stderr writer lock must be poison-free");

    test_body();

    restore_production_reports();
    captured
        .lock()
        .map(|lines| lines.clone())
        .unwrap_or_default()
}

#[test]
fn the_default_sink_writes_its_report_to_the_production_path() {
    // The assertion #824 asked for, and one that cannot pass for every implementation.
    //
    // The probe this replaces asserted `ManagementFailureSink::default().reports_unread_failures()`,
    // and `StderrFailurePort::reports()` returned a hard-coded `true`. Making the production
    // `report` a no-op therefore left the entire 530-test suite green, verified twice by independent
    // review: the test proved only that a constant was still `true`.
    //
    // This drives the *default* sink through the *production* port with only the final write
    // redirected, so the composition under test is the one #816 ships rather than a test double.
    // Deleting `report`, or making it not write, turns this red.
    let lines = capture_production_reports(|| {
        ManagementFailureSink::default().report("sts2-management undeliverable response");
    });

    assert_eq!(
        lines,
        vec!["sts2-management undeliverable response".to_owned()],
        "the default sink must write exactly the line it is handed, through the production path, got {lines:?}"
    );
}
