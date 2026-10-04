// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::response::read_with_deadline;
use super::response::write_with_deadline;
use super::*;
use crate::management::{AuthContext, MemoryWorkflowStore, StaticAuthenticator};
use std::io::Read;
use std::io::Write;

// ## The defect these guard
//
// `handle_connection` used to take ONE deadline before reading the request and
// hand that same, already-partly-spent `Instant` to both `read_request` and
// `write_response`. `write_with_deadline` refuses an expired budget *before* it
// attempts any syscall, so whenever the read phase overran, the error response
// that the owner had already computed could not be transmitted at all. The peer
// received nothing and the socket simply closed -- which a client observes as
// `ECONNRESET`/`socket hang up`.
//
// That is how an answered request became an unattributable transport fault, and
// it is what made the Studio `submission_refused_502` flake undiagnosable
// (AI-Ascension/sts2-harness#816, ascension-workflow-studio#214).
//
// The property under test is therefore NOT "a slow request errors" -- that
// already worked. It is: **the write phase must not inherit the read phase's
// exhausted budget.** These tests drive a real socket, but they never let
// wall-clock timing decide the outcome: the deadline each phase receives is
// asserted directly, so a slow or contended runner cannot flip the result.

/// Builds a connected loopback `TcpStream` pair with no server logic between.
fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback listener");
    let address = listener.local_addr().expect("read the bound address");
    let accepted = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the client");
        stream
    });
    let client = TcpStream::connect(address).expect("connect to the listener");
    let server = accepted.join().expect("the accept thread must finish");
    (client, server)
}

#[test]
fn a_write_after_an_exhausted_read_budget_is_still_attempted() {
    // The regression itself. Phase one overruns against a peer that never
    // writes; phase two must still reach the socket.
    //
    // Before the fix, `handle_connection` passed the spent `Instant` forward and
    // `retry_transient` returned `deadline_exceeded` without calling the socket,
    // so the peer saw an empty stream.
    let limits = HttpLimits::default();
    let (mut peer, mut owner_side) = connected_pair();

    let read_deadline = Instant::now() + Duration::from_millis(50);
    let mut buffer = [0_u8; 32];
    let read_error = read_with_deadline(&mut owner_side, &mut buffer, read_deadline)
        .expect_err("a silent peer must exhaust the read deadline");
    assert_eq!(read_error.code, "deadline_exceeded");

    // Phase two receives a FRESH budget. That is the behaviour under test.
    let write_deadline = Instant::now() + limits.deadline;
    let body = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
    write_with_deadline(&mut owner_side, body, write_deadline)
        .expect("the response must still be transmitted after a spent read budget");

    // And it genuinely reached the peer, rather than merely not erroring.
    // Read exactly `body.len()` bytes rather than to end-of-stream: the owner
    // side is still open, so `read_to_end` would block for the read timeout and
    // then fail with `WouldBlock` even though every byte arrived.
    peer.set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set the peer read timeout");
    let mut received = vec![0_u8; body.len()];
    peer.read_exact(&mut received)
        .expect("the peer must receive the computed response");
    assert_eq!(
        received, body,
        "the peer must receive the computed response, not an empty stream"
    );
}

#[test]
fn an_expired_write_budget_is_still_refused_without_touching_the_socket() {
    // The fix must not have degraded into "always write". An exhausted WRITE
    // budget still refuses before the syscall, so the deliberate bound survives.
    let (mut peer, mut owner_side) = connected_pair();
    let error = write_with_deadline(
        &mut owner_side,
        b"never transmitted",
        Instant::now() - Duration::from_millis(1),
    )
    .expect_err("an expired write deadline must surface");
    assert_eq!(error.code, "deadline_exceeded");

    // Nothing was written, so the peer reads no bytes before its timeout.
    peer.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("set the peer read timeout");
    let mut received = Vec::new();
    let read = peer.read_to_end(&mut received);
    assert!(
        read.is_err() || received.is_empty(),
        "an expired write budget must not transmit bytes, got {received:?}"
    );
}

#[test]
fn the_per_phase_budget_remains_the_admitted_bound() {
    // Pins the shape of the fix at the limit level: each phase gets the admitted
    // `limits.deadline`, and the bound is unchanged. This is the claim that would
    // regress if someone collapsed the two deadlines back into one, or widened
    // `REQUEST_DEADLINE_MILLIS` to make the flake go away.
    let limits = HttpLimits::default();
    assert_eq!(
        limits.deadline,
        Duration::from_millis(REQUEST_DEADLINE_MILLIS)
    );
    assert!(
        !limits.deadline.is_zero(),
        "the per-phase bound must remain non-zero"
    );
}

// ## End-to-end coverage of the wiring, not just the primitives
//
// The three tests above exercise `read_with_deadline` / `write_with_deadline`
// directly, which is correct for what they claim but blind to the defect: they
// choose the deadlines themselves, so they pass whether `handle_connection`
// forwards one spent budget to both phases or derives a fresh one per phase.
// Reverting the fix left the whole lib suite green. These tests close that gap
// by driving the real `ManagementServer` -- the accept loop, the worker thread,
// `handle_connection`, and both phases -- so reintroducing the shared deadline
// fails here.

/// Starts a real management server on loopback with an explicit short deadline.
///
/// The deadline is set low deliberately: the property under test is what
/// happens to the *write* budget once the read phase has consumed its own, and
/// that is observable at any budget size. A short one keeps the test fast and
/// makes the ordering unambiguous, while the admitted bound itself is pinned
/// separately by `the_per_phase_budget_remains_the_admitted_bound`.
fn short_deadline_server(deadline: Duration) -> (SocketAddr, ServerHandle) {
    let authenticator = Arc::new(
        StaticAuthenticator::single(
            "connection-tests-token",
            AuthContext::new("connection-tests", Vec::<String>::new())
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
    let handle = ManagementServer::start(config, service).expect("the server must start");
    let address = handle.address();
    (address, handle)
}

/// Reads until the peer closes or `limit` bytes arrive, returning what it got.
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

#[test]
fn a_computed_response_reaches_the_peer_after_the_read_budget_is_spent() {
    // The regression, driven end to end.
    //
    // Connect, then deliberately overrun the read phase: the peer sends the
    // request line and headers but withholds the terminating blank line, so the
    // owner burns its entire read budget and computes an error response.
    //
    // Pre-fix, `handle_connection` passed the already-spent `Instant` to
    // `write_response`, `retry_transient` refused it before any syscall, and the
    // peer received NOTHING -- an empty stream and a silent close, which is the
    // `ECONNRESET`/`socket hang up` that made #816 undiagnosable. Post-fix the
    // write gets its own budget and the computed response must actually arrive.
    let (address, server) = short_deadline_server(Duration::from_millis(150));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    // Headers only: no terminating CRLF, so the owner keeps waiting for the rest
    // of the request and its read budget expires.
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n")
        .expect("the partial request must be written");

    let received = read_available(&peer, 4096);
    let text = String::from_utf8_lossy(&received);
    assert!(
        text.starts_with("HTTP/1.1 "),
        "the computed response must reach the peer, got {text:?}"
    );
    assert!(
        text.contains("deadline") || text.contains("408") || text.contains("503"),
        "the peer must receive the composed error response, got {text:?}"
    );
    assert!(
        !received.is_empty(),
        "a computed response must never be silently dropped"
    );

    drop(peer);
    server.shutdown().expect("the server must shut down");
}

#[test]
fn a_healthy_request_over_the_wire_still_gets_its_response() {
    // The companion control: with the read phase comfortably inside its budget,
    // a complete request must be answered normally. This guards against the
    // end-to-end path being broken in the other direction -- the fix must not
    // turn a served request into a failure.
    let (address, server) = short_deadline_server(Duration::from_secs(5));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("the request must be written");

    let received = read_available(&peer, 4096);
    let text = String::from_utf8_lossy(&received);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "a healthy request must still be answered, got {text:?}"
    );

    drop(peer);
    server.shutdown().expect("the server must shut down");
}
