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

#[test]
fn the_write_side_timeout_kinds_map_to_the_deadline_error() {
    // The write-side mirror of `the_timeout_kinds_still_map_to_the_deadline_error`
    // above. Before this mapping existed, `write_with_deadline` routed every
    // `io::Error` through `io_http_error`, so a send buffer that filled up
    // surfaced as `io_error` (status 400) even though the read side called the
    // very same two kinds a deadline.
    for kind in [io::ErrorKind::TimedOut, io::ErrorKind::WouldBlock] {
        let error = write_result(Err(io::Error::new(kind, "timed out")))
            .expect_err("a write timeout must surface");
        assert_eq!(error.code, "deadline_exceeded", "kind {kind:?}");
    }
}

#[test]
fn a_non_timeout_write_error_is_still_the_fatal_io_error() {
    // The other half of the contract: the arm above matches exactly the two
    // timeout kinds and nothing else, so a genuine write failure keeps the
    // `io_error` code and its underlying message.
    for kind in [
        io::ErrorKind::ConnectionReset,
        io::ErrorKind::BrokenPipe,
        io::ErrorKind::Interrupted,
    ] {
        let error = write_result(Err(io::Error::new(kind, "broken pipe")))
            .expect_err("a fatal write error must surface");
        assert_eq!(error.code, "io_error", "kind {kind:?}");
        assert!(error.message.contains("broken pipe"), "kind {kind:?}");
    }
}

#[test]
fn a_completed_write_is_still_a_success() {
    assert!(write_result(Ok(())).is_ok());
}

#[test]
fn a_fully_blocked_socket_reports_the_deadline_through_the_real_write_path() {
    // Guards the real call path (`set_write_timeout` + `write_all` + mapping)
    // rather than only the extracted mapping. A connected peer that never
    // reads, with a payload far larger than any socket buffer, is the shape
    // that makes the kernel report `WouldBlock` instead of accepting the whole
    // body; the call must end as `deadline_exceeded` and not as `io_error`.
    // The assertion is on the code the mapping produces, never on which
    // `io::ErrorKind` the host happened to use, so this is the same contract
    // on Linux (`WouldBlock`) and on platforms that report `TimedOut`.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a loopback listener");
    let address = listener.local_addr().expect("read the bound address");
    let accepted = std::thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept the client");
        // Never read. Hold the connection open past the client's write
        // deadline, then let the socket close. It must not close early: an
        // early close turns the blocked write into `BrokenPipe`, which is
        // `io_error` by design, so the hold is the point of the test. Two
        // seconds is several times the write that was actually observed here
        // (~0.6 s) and leaves headroom for a slower host.
        std::thread::sleep(Duration::from_secs(2));
        drop(stream);
    });
    let mut stream = TcpStream::connect(address).expect("connect to the listener");
    // A payload far past any socket buffer the host configures, so the write
    // is guaranteed to block rather than land in the buffer. Loopback
    // send/recv buffers autotune to a few MiB; the issue's own measurement
    // against this host reported the first `WouldBlock` at ~2.6 MB written.
    // 64 MiB stays past even a generously tuned host, and costs only the
    // allocation, because the 250 ms write deadline expires long before the
    // payload could be drained.
    let payload = vec![0_u8; 64 * 1024 * 1024];
    let error = write_with_deadline(
        &mut stream,
        &payload,
        Instant::now() + Duration::from_millis(250),
    )
    .expect_err("a peer that never reads must exhaust the write deadline");
    assert_eq!(error.code, "deadline_exceeded", "surfaced: {error}");
    drop(stream);
    accepted.join().expect("the accept thread must finish");
}
