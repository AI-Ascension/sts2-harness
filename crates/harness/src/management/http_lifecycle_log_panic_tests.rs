// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::panic, clippy::unwrap_used)]

// The panic path of the request-lifecycle log (#820), kept apart from
// `http_lifecycle_log_server_tests.rs` so neither file hides behind the other's size.
//
// A hang and a panic both end with a start line and no terminal line, which is the one
// distinction this log exists to make. Review of #825 found that a panicking handler
// produced exactly that, so the panic now carries its own terminal marker.

use super::*;
use crate::management::{AuthError, Authenticator};

/// An authenticator that panics instead of answering.
///
/// `dispatch` calls this on every request, so the panic is reached from inside the
/// handler -- after `log_start` has already been emitted and before any response is
/// computed. That is the exact window in which a missing terminal marker would be
/// indistinguishable from a hang.
struct PanickingAuthenticator;

impl Authenticator for PanickingAuthenticator {
    fn authenticate(&self, _bearer_token: Option<&str>) -> Result<AuthContext, AuthError> {
        panic!("the test authenticator must panic");
    }
}

/// A server whose handler panics, with its lifecycle lines captured.
///
/// The panic escapes the connection worker, so `shutdown` is what makes the terminal
/// marker observable: by the time it returns, the worker has unwound.
fn panicking_server() -> (SocketAddr, ServerHandle, Arc<CapturedLines>) {
    let authenticator = Arc::new(PanickingAuthenticator);
    let service = Arc::new(ManagementService::new(Arc::new(MemoryWorkflowStore::new())));
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("a parseable loopback address"),
        authenticator,
    )
    .expect("the loopback listen address must be accepted");
    config.limits.deadline = Duration::from_secs(5);
    let captured = Arc::new(CapturedLines::default());
    config.lifecycle_log = RequestLifecycleLog::with_sink(
        Arc::clone(&captured) as Arc<dyn super::lifecycle_log::LifecycleLogSink>
    );
    let handle = ManagementServer::start(config, service).expect("the server must start");
    (handle.address(), handle, captured)
}

#[test]
fn a_panicking_handler_is_reported_as_itself_and_not_as_a_stall() {
    // The third way a started request can fail to finish.
    //
    // A hang and a panic both leave a start line with no terminal line, which is the
    // one distinction this log exists to make: a hang is a live thread that never
    // returned, a panic is a thread that died. So the panic gets its own terminal
    // marker, emitted before the unwind is resumed.
    let (address, server, captured) = panicking_server();

    let mut peer = TcpStream::connect(address).expect("the peer must connect");
    peer.set_read_timeout(Some(Duration::from_secs(5)))
        .expect("the peer read timeout must be settable");
    // Not `/v1/health`: that route answers before authentication, so it would never
    // reach the panicking authenticator. This one authenticates on every request.
    peer.write_all(b"GET /v1/workflow-targets HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer token\r\n\r\n")
        .expect("the request must be written");
    let _ = read_available(&peer, 4096);
    drop(peer);

    // The panic is expected, so the join failing here is not itself a defect; what
    // matters is that the marker was written before the unwind.
    let _ = server.shutdown();

    let joined = captured.joined();
    assert!(
        joined.contains("request_start"),
        "the request reached the handler, so a start marker must exist, got {joined:?}"
    );
    assert!(
        joined.contains("request_panicked"),
        "a panicking handler must be reported as a panic, not left looking like a stall, got {joined:?}"
    );
    assert!(
        !joined.contains("request_end") && !joined.contains("request_abandoned"),
        "a panicking request neither ended nor was abandoned, got {joined:?}"
    );
    assert!(
        joined.contains("code=request_panicked"),
        "the panic marker must name the typed code, got {joined:?}"
    );
}
