// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::io::Write;
use std::net::TcpListener;
use std::sync::Mutex;

use super::*;

// The retry policy is exercised through `retry_transient`'s attempt closure
// rather than by racing a real signal against a real socket. `SIGCHLD`
// delivery is asynchronous, so a test that waited for it would be
// timing-dependent; the contract under test — "a retryable interruption is
// retried, and the deadline still terminates the retry" — is what the closure
// supplies deterministically. The real-signal measurement lives in the issue
// and PR record, not in CI.

fn interrupted() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Interrupted system call")
}

#[test]
fn an_interrupted_attempt_is_retried_and_the_healthy_completion_is_returned() {
    let mut attempts = 0_usize;
    let result = retry_transient(
        Instant::now() + Duration::from_secs(5),
        "request deadline exceeded",
        |_| {
            attempts += 1;
            match attempts {
                1 | 2 => Err(interrupted()),
                _ => Ok(7_usize),
            }
        },
    );
    assert_eq!(result.map_err(|error| error.code), Ok(7));
    assert_eq!(attempts, 3, "the retry must resume rather than give up");
}

#[test]
fn an_interruption_never_becomes_the_fatal_io_error_that_reached_the_runner() {
    // The observable defect: `Interrupted` was mapped to a status-400
    // `io_error` naming the signal, which is what failed the hosted loopback
    // request. It must not be reachable through the retry path.
    // A short deadline on purpose: the closure interrupts unconditionally, so
    // this exercises the retry-until-deadline path rather than the retry-once
    // path, and it must not spend seconds spinning to do so.
    let result = retry_transient(
        Instant::now() + Duration::from_millis(50),
        "request deadline exceeded",
        |_| Err::<usize, _>(interrupted()),
    );
    let error = result.expect_err("an unrelenting interruption must not succeed");
    assert_eq!(error.code, "deadline_exceeded");
    assert!(
        !error.message.contains("Interrupted system call"),
        "the signal must not survive into the surfaced error: {}",
        error.message
    );
}

#[test]
fn the_deadline_terminates_a_retry_loop_rather_than_spinning() {
    let mut attempts = 0_usize;
    let started = Instant::now();
    let result = retry_transient(
        Instant::now() + Duration::from_millis(50),
        "request deadline exceeded",
        |_| {
            attempts += 1;
            Err::<usize, _>(interrupted())
        },
    );
    let error = result.expect_err("an expired deadline must surface");
    assert_eq!(error.code, "deadline_exceeded");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the retry loop must be bounded by the deadline, took {:?}",
        started.elapsed()
    );
    assert!(attempts > 0, "the loop must attempt at least once");
}

#[test]
fn an_already_expired_deadline_is_reported_without_attempting() {
    let mut attempts = 0_usize;
    let result = retry_transient(
        Instant::now() - Duration::from_millis(1),
        "request deadline exceeded",
        |_| {
            attempts += 1;
            Ok(1_usize)
        },
    );
    let error = result.expect_err("an expired deadline must surface");
    assert_eq!(error.code, "deadline_exceeded");
    assert_eq!(attempts, 0, "no syscall may be attempted past the deadline");
}

#[test]
fn the_timeout_kinds_still_map_to_the_deadline_error() {
    for kind in [io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock] {
        let mut attempts = 0_usize;
        let result = retry_transient(
            Instant::now() + Duration::from_secs(5),
            "request deadline exceeded",
            |_| {
                attempts += 1;
                Err::<usize, _>(io::Error::new(kind, "timed out"))
            },
        );
        let error = result.expect_err("a timeout must surface");
        assert_eq!(error.code, "deadline_exceeded", "kind {kind:?}");
        assert_eq!(attempts, 1, "a timeout is not retryable: {kind:?}");
    }
}

#[test]
fn a_fatal_error_is_surfaced_unchanged_and_not_retried() {
    let mut attempts = 0_usize;
    let result = retry_transient(
        Instant::now() + Duration::from_secs(5),
        "request deadline exceeded",
        |_| {
            attempts += 1;
            Err::<usize, _>(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "reset by peer",
            ))
        },
    );
    let error = result.expect_err("a fatal error must surface");
    assert_eq!(error.code, "io_error");
    assert_eq!(attempts, 1, "a fatal error must not be retried");
}

#[test]
fn a_silent_peer_still_reports_the_deadline_through_a_real_socket() {
    // Guards the real call path (socket + timeout + error mapping) rather than
    // only the closure: a connected peer that never writes must end as
    // `deadline_exceeded`, and must not turn into an `io_error`.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback listener");
    let address = listener.local_addr().expect("read the bound address");
    let accepted = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the client");
        std::thread::sleep(Duration::from_millis(500));
        drop(stream);
    });
    let mut stream = TcpStream::connect(address).expect("connect to the listener");
    let mut buffer = [0_u8; 32];
    let error = read_with_deadline(
        &mut stream,
        &mut buffer,
        Instant::now() + Duration::from_millis(50),
    )
    .expect_err("a silent peer must exhaust the deadline");
    assert_eq!(error.code, "deadline_exceeded");
    accepted.join().expect("the accept thread must finish");
}

// The write side needed none of these while it mapped every `io::Error`
// through `io_http_error`, so it is worth stating what the tests below hold to
// and what they do not. They do not exercise a partially consumed buffer:
// `write_all` keeps its progress internally and never hands a partial write
// back to this module, so the two kinds asserted here are always raised on a
// call that consumed nothing.

#[test]
fn a_write_timeout_maps_to_the_deadline_error_not_to_a_client_error() {
    // The observable defect: a write that hit its deadline reported
    // `io_error`, so a caller could not distinguish "this peer stopped
    // reading" from "this request was malformed". The read path already
    // answered `deadline_exceeded` for these same two kinds.
    //
    // This case does **not** by itself discriminate the two revisions, and it
    // is here to pin the contract rather than to catch the regression:
    // `retry_transient` already classified these kinds correctly before the
    // change, so a closure-driven test cannot see a defect that lived entirely
    // in the caller bypassing it. It passes on the pre-change tree too. The
    // case that does discriminate is
    // `a_write_that_fills_a_peer_send_buffer_reports_the_deadline_through_a_real_socket`
    // below, which drives the real socket.
    //
    // The assertion is on `code`, not on `status`: `HttpError::new` pins
    // `status: 400` for every code it constructs, `deadline_exceeded`
    // included, so both spellings carry the same status and only the code
    // separates them. Asserting a status here would have been an
    // unverified claim about a number this change does not alter.
    for kind in [io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock] {
        let mut attempts = 0_usize;
        let result = retry_transient(
            Instant::now() + Duration::from_secs(5),
            "response deadline exceeded",
            |_| {
                attempts += 1;
                Err::<(), _>(io::Error::new(kind, "timed out"))
            },
        );
        let error = result.expect_err("a timed-out write must surface");
        assert_eq!(error.code, "deadline_exceeded", "kind {kind:?}");
        assert_eq!(attempts, 1, "a write timeout is not retryable: {kind:?}");
    }
}

#[test]
fn an_expired_write_deadline_is_reported_without_attempting() {
    // Guards the specific claim this change makes about routing through
    // `retry_transient`: that routing preserves "no syscall past the deadline".
    // If a later refactor re-adds a local pre-check that attempts the socket
    // call anyway, this fails even though the error code is unchanged.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback listener");
    let address = listener.local_addr().expect("read the bound address");
    let accepted = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the client");
        drop(stream);
    });
    let mut stream = TcpStream::connect(address).expect("connect to the listener");
    let error = write_with_deadline(
        &mut stream,
        b"already past the deadline",
        Instant::now() - Duration::from_millis(1),
    )
    .expect_err("an expired write deadline must surface");
    assert_eq!(error.code, "deadline_exceeded");
    accepted.join().expect("the accept thread must finish");
}

// The reason the `WouldBlock` arm is terminal rather than retried. This is not
// a claim about this repository's code: it is a property of
// `std::io::Write::write_all`, and it is the one that would silently corrupt a
// response if a later change re-enabled the retry.
//
// `write_all` keeps its write cursor inside the call, so a partial write
// followed by a timeout reaches the caller as a bare `Err(WouldBlock)` with
// the progress discarded. Re-sending the same buffer after that would replay
// the already-written prefix. If the retry policy below ever grew a `WouldBlock`
// arm that re-entered the closure, this test would be the thing that catches
// the resulting duplicate-prefix bug.
#[test]
fn a_partial_write_that_then_times_out_reaches_the_caller_with_no_progress_kept() {
    struct Scripted {
        steps: Mutex<Vec<(io::ErrorKind, usize)>>,
        calls: usize,
    }
    impl Write for Scripted {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let step = self
                .steps
                .lock()
                .expect("the script mutex must not be poisoned")
                .get(self.calls)
                .copied()
                .unwrap_or((io::ErrorKind::Other, buffer.len()));
            self.calls += 1;
            match step.0 {
                io::ErrorKind::Other => Ok(step.1.min(buffer.len())),
                kind => Err(io::Error::new(kind, "scripted")),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let mut writer = Scripted {
        steps: Mutex::new(vec![
            (io::ErrorKind::Other, 4),
            (io::ErrorKind::WouldBlock, 0),
        ]),
        calls: 0,
    };
    let error = writer
        .write_all(&vec![b'z'; 10])
        .expect_err("a timeout after a partial write must surface");
    assert_eq!(
        error.kind(),
        io::ErrorKind::WouldBlock,
        "write_all must surface the timeout kind, not swallow it"
    );
    assert_eq!(
        writer.calls, 2,
        "write_all must resume once and then give up on the timeout"
    );
}

#[test]
fn a_write_that_fills_a_peer_send_buffer_reports_the_deadline_through_a_real_socket() {
    // The one property a synthetic closure cannot supply: that a real
    // blocking `TcpStream` write against a peer which never reads actually
    // produces the kind the arm above claims to handle, and that it surfaces
    // as `deadline_exceeded` rather than `io_error`. Without this test the arm
    // could be entirely uncovered while every closure-driven test passed.
    //
    // Two details are load-bearing and both were measured rather than assumed.
    // The body is far larger than any socket buffer, so the write cannot
    // complete. And the peer must hold the connection **open** while it never
    // reads: a peer that closes sends a FIN, and the writer then fails with
    // `ConnectionReset` about 70 ms in -- which is correctly still an
    // `io_error`, so an earlier revision of this test that dropped the peer
    // failed against the fixed code for a reason that had nothing to do with
    // the fix.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback listener");
    let address = listener.local_addr().expect("read the bound address");
    let accepted = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the client");
        // Never read. Holding the connection open keeps the client on its own
        // write timeout rather than on a peer-generated reset.
        std::thread::sleep(Duration::from_millis(1500));
        drop(stream);
    });
    let mut stream = TcpStream::connect(address).expect("connect to the listener");
    let body = vec![b'x'; 64 * 1024 * 1024];
    let error = write_with_deadline(
        &mut stream,
        &body,
        Instant::now() + Duration::from_millis(200),
    )
    .expect_err("a peer that never reads must exhaust the write deadline");
    assert_eq!(
        error.code, "deadline_exceeded",
        "a write that filled the send buffer must not surface as a client error"
    );
    accepted.join().expect("the accept thread must finish");
}
