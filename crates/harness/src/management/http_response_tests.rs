// SPDX-License-Identifier: MIT

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::net::TcpListener;

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
