// SPDX-License-Identifier: MIT

// `panic_used` is denied crate-wide. The only panics here are the three
// lookups in `slice_function`, which name a function `http.rs` defines; each
// is unreachable in any tree that compiled, and its message is what makes a
// rename of `ManagementServer::start` legible when it does happen.
#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use super::*;
use crate::management::{AuthContext, MemoryWorkflowStore, StaticAuthenticator};
use std::io::Read;
use std::io::Write;
use std::sync::Mutex;

/// Collects what the server would have written to stderr.
#[derive(Clone, Debug, Default)]
struct RecordingSink {
    reports: Arc<Mutex<Vec<UndeliveredResponse>>>,
}

impl RecordingSink {
    fn take(&self) -> Vec<UndeliveredResponse> {
        std::mem::take(&mut *self.reports.lock().expect("the sink lock must be held"))
    }
}

impl DiagnosticsSink for RecordingSink {
    fn report(&self, report: &UndeliveredResponse) {
        self.reports
            .lock()
            .expect("the sink lock must be held")
            .push(report.clone());
    }
}

/// Starts a server whose undelivered-response reports land in `sink`.
///
/// `max_response_bytes` is optional so a test can set it below the size of a
/// real response. That makes `write_response` refuse the body with
/// `response_too_large` on every runner, which is the deterministic way to
/// reach the undelivered-response path.
fn recording_server(
    deadline: Duration,
    max_response_bytes: Option<usize>,
) -> (SocketAddr, ServerHandle, RecordingSink) {
    let authenticator = Arc::new(
        StaticAuthenticator::single(
            "diagnostics-tests-token",
            AuthContext::new("diagnostics-tests", Vec::<String>::new())
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
    if let Some(limit) = max_response_bytes {
        config.limits.max_response_bytes = limit;
    }
    let sink = RecordingSink::default();
    let handle = ManagementServer::start_with_diagnostics(
        config,
        service,
        Arc::new(sink.clone()),
    )
    .expect("the server must start");
    (handle.address(), handle, sink)
}

#[test]
fn an_undelivered_response_is_reported_and_not_merely_printed() {
    // ## The hole this closes
    //
    // The report the connection worker emits when it ends without delivering a
    // response is the only evidence that the owner computed an answer it could
    // not transmit -- the fact that made #816 diagnosable. It was an
    // `eprintln!`, and an `eprintln!` cannot be asserted: deleting it passes
    // every other test in the crate, which is exactly why the gap survived 524
    // tests (harness#819).
    //
    // Asserting the sink's own API instead was tried first and is worthless --
    // deleting the `worker_sink.report(...)` call left the whole crate green.
    // So this drives the real server loop and reads the real worker's report.
    //
    // `max_response_bytes` is the lever, and it is deterministic rather than
    // timing-based: with it set below the size of a real response, the owner
    // composes the response and then refuses to write it, so the worker exits
    // with `response_too_large` on every runner, in bounded time, with no peer
    // cooperation required at all.
    let (address, server, sink) = recording_server(Duration::from_secs(5), Some(1));

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    let peer_address_port = peer.local_addr().expect("the peer address must be readable").port();
    // A complete request, so the owner composes a real response -- and then
    // refuses to write it because it exceeds `max_response_bytes`.
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("the request must be written");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    let mut buffer = [0_u8; 1024];
    let count = peer.read(&mut buffer).expect("the peer read must not fail");
    assert_eq!(
        count, 0,
        "the composed response must be refused rather than truncated and sent"
    );

    drop(peer);
    server.shutdown().expect("the server must shut down");

    let reports = sink.take();
    assert_eq!(
        reports.len(),
        1,
        "the connection that ended without a delivered response must be \
         reported exactly once; the report is the only surviving evidence that \
         the owner computed a response it could not transmit (harness#819). \
         Got {reports:?}"
    );
    assert_eq!(
        reports[0].code, "response_too_large",
        "the report must carry the management error's stable code, so a reader \
         can match it to a request without parsing prose"
    );
    assert!(
        !reports[0].message.is_empty(),
        "the report must carry the error's message, got {:?}",
        reports[0].message
    );
    assert_eq!(
        reports[0].peer.ip().to_string(),
        "127.0.0.1",
        "the report must name the peer it concerns, or it cannot be attributed"
    );
    assert_eq!(
        reports[0].peer.port(),
        peer_address_port,
        "the report must name the exact connection, not merely the host"
    );
}

#[test]
fn a_delivered_response_is_not_reported() {
    // The other half of the contract: the sink reports *undelivered* responses.
    // A server that reported every connection would make the first test
    // meaningless, and would bury the real faults in noise.
    let (address, server, sink) = recording_server(Duration::from_secs(5), None);

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    peer.write_all(b"GET /v1/health HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .expect("the request must be written");

    let mut buffer = [0_u8; 1024];
    let count = peer.read(&mut buffer).expect("the peer read must succeed");
    let text = String::from_utf8_lossy(&buffer[..count]).to_string();
    assert!(
        text.starts_with("HTTP/1.1 "),
        "the served request must be answered, got {text:?}"
    );

    drop(peer);
    server.shutdown().expect("the server must shut down");

    assert!(
        sink.take().is_empty(),
        "a response that was delivered must not be reported as undelivered"
    );
}

#[test]
fn the_production_entry_point_still_reports_to_stderr() {
    // `ManagementServer::start` is what every real caller uses. The seam added
    // for tests is only safe if that path still delegates to `StderrDiagnostics`
    // -- otherwise the report would vanish from production while every other
    // test in this file passed, which is the same hole in a new place.
    //
    // Asserted by source rather than by capturing stderr: capturing would pass
    // with the delegation removed, and would also be racy against the other
    // tests in this binary, which print to the same stream.
    let source = include_str!("http.rs");
    let start_body = slice_function(source, "pub fn start(");
    assert!(
        start_body.contains("start_with_diagnostics"),
        "ManagementServer::start must delegate to start_with_diagnostics, or the \
         production report disappears (harness#819)"
    );
    assert!(
        start_body.contains("StderrDiagnostics"),
        "ManagementServer::start must supply StderrDiagnostics, so the \
         production report still reaches stderr"
    );
}

/// Return the source of the named function, bounded by brace matching.
fn slice_function(source: &str, signature: &str) -> String {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("`{signature}` must exist in http.rs"));
    let open = source[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("`{signature}` must open a body"));
    let mut depth = 0_usize;
    for index in open..source.len() {
        match source.as_bytes()[index] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return source[start..=index].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("`{signature}` must close its body")
}
