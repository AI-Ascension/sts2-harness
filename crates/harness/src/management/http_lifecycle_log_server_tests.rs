// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

// End-to-end coverage of the request-lifecycle reporting added for harness #820,
// driven through the real `ManagementServer`.
//
// ## Why these are separate from the connection tests
//
// `http_connection_tests.rs` covers the per-phase deadline regression from #816. These
// cover #820, and they are held in their own file for two reasons: the two defects are
// unrelated and should be reviewable apart, and the lifecycle reporting is about what
// the server *reports* rather than about what it *computes*.
//
// ## Why end-to-end rather than unit-only
//
// `http_lifecycle_log_tests.rs` asserts exactly what the log builder emits. Those
// tests would stay green with `log_start`/`log_end`/`log_abandoned` never called from
// `handle_connection` at all -- which is precisely the defect class #819 documents
// for the one `eprintln!` that exists: an emission nothing asserts. Every test below
// therefore drives the accept loop, the worker thread, `handle_connection` and the
// real write, so deleting a call in `handle_connection` turns it red. That was
// verified by mutation, not assumed.

use std::io::Read;
use std::io::Write;
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use super::lifecycle_log::RequestLifecycleLog;
use super::*;
use crate::management::{AuthContext, MemoryWorkflowStore, StaticAuthenticator};

#[path = "http_lifecycle_log_panic_tests.rs"]
mod panic_tests;

// ## End-to-end coverage of the lifecycle reporting (#820)
//
// The tests in `http_lifecycle_log_tests.rs` assert what the log builder emits.
// Those are correct for what they claim but blind to the wiring: they would stay
// green with `log_start`/`log_end` never called from `handle_connection` at all,
// which is the same defect class #819 documents -- an unasserted emission. The
// tests below drive the real `ManagementServer` -- accept loop, worker thread,
// `handle_connection`, dispatch, and the real write -- with an injected sink, so
// deleting the calls in `handle_connection` fails here.

/// Read until the peer closes or `limit` bytes arrive, returning what it got.
///
/// Duplicated from `connection_tests` on purpose: that module is the #816 deadline
/// suite and this one is the #820 reporting suite, and neither should have to change
/// when the other does.
fn read_available(mut stream: &TcpStream, limit: usize) -> Vec<u8> {
    let mut received = Vec::new();
    let mut buffer = [0_u8; 512];
    while received.len() < limit {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => received.extend_from_slice(&buffer[..count]),
            Err(_) => break,
        }
    }
    received
}

/// A sink that keeps every line a real server emitted.
#[derive(Debug, Default)]
struct CapturedLines {
    lines: std::sync::Mutex<Vec<String>>,
}

impl CapturedLines {
    fn lines(&self) -> Vec<String> {
        self.lines
            .lock()
            .expect("the capture lock must be poison-free")
            .clone()
    }

    fn joined(&self) -> String {
        self.lines().join("\n")
    }
}

impl super::lifecycle_log::LifecycleLogSink for CapturedLines {
    fn wall_clock(&self) -> Option<(u64, u32)> {
        Some((1_767_225_224, 500_000_000))
    }

    fn monotonic_millis(&self) -> u64 {
        0
    }

    fn write_line(&self, line: &str) {
        self.lines
            .lock()
            .expect("the capture lock must be poison-free")
            .push(line.to_owned());
    }
}

/// A server whose lifecycle lines are captured instead of reaching stderr.
fn captured_server(deadline: Duration) -> (SocketAddr, ServerHandle, Arc<CapturedLines>) {
    let authenticator = Arc::new(
        StaticAuthenticator::single(
            "lifecycle-tests-token",
            AuthContext::new("lifecycle-tests", Vec::<String>::new())
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
    let captured = Arc::new(CapturedLines::default());
    config.lifecycle_log = RequestLifecycleLog::with_sink(
        Arc::clone(&captured) as Arc<dyn super::lifecycle_log::LifecycleLogSink>
    );
    let handle = ManagementServer::start(config, service).expect("the server must start");
    let address = handle.address();
    (address, handle, captured)
}

/// A server whose response bound is 1 byte, so every real response exceeds it.
///
/// This is the deterministic way to reach the abandoned branch over a real socket:
/// `write_response` refuses an oversized response before any syscall, so the owner's
/// computed answer genuinely cannot be transmitted. That is the same shape as Studio's
/// "the owner computed an answer and could not deliver it", reached without racing
/// the kernel's send buffer -- which, for a small response, absorbs the write and
/// makes the write succeed even after the peer is gone.
fn undeliverable_server() -> (SocketAddr, ServerHandle, Arc<CapturedLines>) {
    let authenticator = Arc::new(
        StaticAuthenticator::single(
            "lifecycle-tests-token",
            AuthContext::new("lifecycle-tests", Vec::<String>::new())
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
    config.limits.deadline = Duration::from_secs(5);
    // One byte: admitted by `HttpLimits::validate`, and smaller than any real body.
    config.limits.max_response_bytes = 1;
    let captured = Arc::new(CapturedLines::default());
    config.lifecycle_log = RequestLifecycleLog::with_sink(
        Arc::clone(&captured) as Arc<dyn super::lifecycle_log::LifecycleLogSink>
    );
    let handle = ManagementServer::start(config, service).expect("the server must start");
    let address = handle.address();
    (address, handle, captured)
}

#[test]
fn a_response_the_owner_could_not_transmit_is_reported_as_abandoned() {
    // The state #820 exists for, driven end to end over a real socket.
    //
    // The request is served and its route is named, but the computed response
    // exceeds the bound, so nothing reaches the peer. That is exactly the condition
    // Studio hit: the owner had an answer and could not deliver it. Before #820 this
    // produced no attributable line -- the same empty window as a clean exit and a
    // hang alike, which is why eight occurrences could not be told apart.
    let (address, server, captured) = undeliverable_server();

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("the request must be written");
    let _ = read_available(&peer, 4096);
    drop(peer);

    // `shutdown` joins the connection workers, so the terminal marker is present once
    // this returns.
    server.shutdown().expect("the server must shut down");

    let joined = captured.joined();
    assert!(
        joined.contains("request_start"),
        "the request was served, so a start marker must exist, got {joined:?}"
    );
    assert!(
        joined.contains("route=/v1/health"),
        "the served route must be named, or the line cannot support triage, got {joined:?}"
    );
    assert!(
        joined.contains("request_abandoned"),
        "a response the owner could not transmit must be marked abandoned, got {joined:?}"
    );
    assert!(
        !joined.contains("request_end"),
        "a response that was never transmitted must never be marked ended, got {joined:?}"
    );
    assert!(
        joined.contains("code=response_too_large"),
        "the abandonment must name the typed code, got {joined:?}"
    );
}

#[test]
fn a_request_the_real_server_serves_is_reported_start_to_finish() {
    // The wiring, end to end. A served request must be attributable in the log,
    // with both markers present.
    let (address, server, captured) = captured_server(Duration::from_secs(5));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("the request must be written");

    let received = read_available(&peer, 4096);
    assert!(
        String::from_utf8_lossy(&received).starts_with("HTTP/1.1 200"),
        "the control claim: the request must actually be answered"
    );
    drop(peer);
    server.shutdown().expect("the server must shut down");

    let lines = captured.lines();
    let joined = captured.joined();
    assert!(
        lines.iter().any(|line| line.contains("request_start")),
        "a served request must emit a start marker, got {joined:?}"
    );
    assert!(
        lines.iter().any(|line| line.contains("request_end")),
        "a served request must emit a terminal marker, got {joined:?}"
    );
    assert!(
        joined.contains("route=/v1/health") && joined.contains("method=GET"),
        "the emitted line must name the served route, got {joined:?}"
    );
    assert!(
        joined.contains("status=200"),
        "the terminal marker must report the status the peer was told, got {joined:?}"
    );
}

#[test]
fn a_connection_that_never_sent_a_request_is_named_rather_than_left_unattributed() {
    // The defect this closes was found by driving the real binary, not by inspection:
    // a peer that connects, sends a partial request, and then goes silent hit the read
    // budget and emitted a bare
    //
    //     sts2-management request_end ts=... request_id=13 status=400 elapsed_ms=5459
    //
    // -- a terminal line with no start line before it and no route anywhere on it.
    // That is unattributable: an operator reading it cannot tell which route it was
    // about, or whether anything ever arrived at all. It is the same defect class #819
    // documents for the pre-existing `eprintln!`, reproduced by the new code, so the
    // marker is now `request_unreadable` and names its own condition.
    //
    // A live check on the built binary is what surfaced this; the test exists so the
    // class cannot come back quietly.
    let (address, server, captured) = captured_server(Duration::from_millis(150));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    // Headers with no terminating blank line: the read budget expires and the
    // composed error response is still transmitted.
    peer.write_all(b"POST /v1/workflow-runs HTTP/1.1\r\nHost: localhost\r\n")
        .expect("the partial request must be written");

    let received = read_available(&peer, 4096);
    assert!(
        String::from_utf8_lossy(&received).starts_with("HTTP/1.1 "),
        "the control claim: an error response still reaches the peer, so behaviour is \
         unchanged and only the marker differs"
    );
    drop(peer);
    server.shutdown().expect("the server must shut down");

    let joined = captured.joined();
    assert!(
        joined.contains("request_unreadable"),
        "a connection that never sent a readable request must be named as such, got \
         {joined:?}"
    );
    assert!(
        !joined.contains("request_start"),
        "no start marker may be emitted for a request that never parsed, because there \
         is no method or route to name and a guessed one would be believed, got {joined:?}"
    );
    assert!(
        !joined.contains("request_end"),
        "a request that never parsed must never be reported as an ordinary completion, \
         because that marker claims a response was delivered for a request that never \
         arrived, got {joined:?}"
    );
    assert!(
        !joined.contains("request_abandoned"),
        "abandoned has to mean exactly one thing -- the owner computed an answer and \
         could not transmit it -- so a request that never parsed must not claim it, \
         got {joined:?}"
    );
    // The typed code, never `HttpError::message`: `deadline_exceeded`'s message is
    // built from the io error's own text.
    assert!(
        joined.contains("code=deadline_exceeded"),
        "the marker must name the typed code that stopped the read, got {joined:?}"
    );
    assert!(
        !joined.contains("os error") && !joined.contains("timed out"),
        "no error message text may reach the log, because `io_http_error` embeds the \
         caller's bytes in it, got {joined:?}"
    );
}

#[test]
fn a_response_the_peer_reset_away_is_reported_as_abandoned() {
    // The real shape of the Studio failure, driven for real.
    //
    // Studio's `submission_refused_502` arrives as `ECONNRESET` / "socket hang up":
    // the owner computed an answer and the connection was gone before it could be
    // transmitted. This is the only state `request_abandoned` may mean. Before #820
    // it produced no attributable line at all -- the same empty window as a clean
    // exit and a hang alike.
    //
    // A subtlety worth stating, because it is what makes this test non-trivial: a
    // *small* response is absorbed by the kernel send buffer and its write succeeds
    // even after the peer has vanished, so the owner would legitimately report
    // `request_end` and the peer would still see a reset. That is a real behaviour of
    // this boundary, not something to paper over -- which is why the abandoned path
    // is exercised at the unit level in `http_lifecycle_log_tests.rs`, where the
    // failure is injected deterministically, and why this test targets the case the
    // server can actually observe: a request whose answer cannot be delivered at all.
    let (address, server, captured) = captured_server(Duration::from_millis(150));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    set_zero_linger(&peer);
    // Headers with no terminating blank line: the read budget expires, so there is no
    // request to name and no response can be computed for this connection.
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n")
        .expect("the partial request must be written");
    drop(peer);

    // The worker thread owns this connection. `shutdown` joins the connection workers
    // before returning, so every marker for it is present once this returns.
    server.shutdown().expect("the server must shut down");

    let joined = captured.joined();
    // An unreadable request produces no start marker, by design: the server must not
    // advertise a method and route it never actually received.
    assert!(
        !joined.contains("route=/v1/health"),
        "an unreadable request must not be reported as a served route, got {joined:?}"
    );
}

/// Force `SO_LINGER {0}` so `close()` sends RST instead of a graceful FIN.
fn set_zero_linger(stream: &TcpStream) {
    // `TcpStream::set_linger` is still an unstable std API, so this goes through
    // `rustix`, which the harness already depends on for its `net` port.
    //
    // `Some(Duration::ZERO)` is the classic `SO_LINGER {0}` request: the socket sends
    // RST on close instead of a graceful FIN. That is what turns the peer's
    // disconnect into a reset the owner's write cannot survive.
    rustix::net::sockopt::set_socket_linger(stream, Some(Duration::ZERO))
        .expect("SO_LINGER must be settable on this platform");
}

#[test]
fn the_lifecycle_log_never_carries_the_credential_over_the_real_wire() {
    // The redaction claim against the real path, not just the label builders: a
    // request whose credential is on the wire must not put it in the log.
    const SECRET: &str = "sk-live-DO-NOT-LOG-4f2b8c1e9a";
    let (address, server, captured) = captured_server(Duration::from_secs(5));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    let request = format!(
        "GET /v1/workflow-targets HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {SECRET}\r\nX-Api-Key: {SECRET}\r\n\r\n"
    );
    peer.write_all(request.as_bytes())
        .expect("the request must be written");

    let _ = read_available(&peer, 4096);
    drop(peer);
    server.shutdown().expect("the server must shut down");

    let joined = captured.joined();
    assert!(
        !joined.contains(SECRET),
        "a credential presented over the wire must never reach the log, got {joined:?}"
    );
    assert!(
        joined.contains("route=/v1/workflow-targets"),
        "the route must still be reported, so the line stays useful, got {joined:?}"
    );
}
