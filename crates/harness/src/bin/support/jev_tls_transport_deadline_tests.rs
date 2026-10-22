// SPDX-License-Identifier: MIT

use super::*;
use std::cell::Cell;
use std::net::TcpListener;
use std::process::{Child, Command, Output, Stdio};

fn expired() -> Instant {
    Instant::now() - super::super::deadline() - Duration::from_secs(1)
}

fn short_budget() -> Instant {
    Instant::now() - super::super::deadline() + Duration::from_millis(200)
}

fn sockets() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let client = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (peer, _) = listener.accept().expect("accept");
    (client, peer)
}

#[test]
fn expired_budget_refuses_before_resolving_or_connecting() {
    let called = Cell::new(false);
    let result = super::super::post_after_admission(b"{}", "test-only", expired(), || {
        called.set(true);
        Err("the resolver must not run".into())
    });
    assert_eq!(result.expect_err("expired").to_string(), EXPIRED);
    assert!(!called.get(), "expiry must precede the resolver boundary");
    assert_eq!(
        remaining(expired()).expect_err("no allowance").kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn each_socket_read_uses_the_original_budget_even_with_queued_bytes() {
    let (client, mut peer) = sockets();
    peer.write_all(b"ab").expect("queue control bytes");
    let mut socket = DeadlineSocket::new(client, Instant::now());
    let mut byte = [0_u8];
    assert_eq!(socket.read(&mut byte).expect("fresh control"), 1);
    assert_eq!(byte, *b"a");
    // Advance the same private clock seam; the peer's second byte is already readable.
    socket.started = expired();
    assert_eq!(
        socket.read(&mut byte).expect_err("expired read").kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(socket.stream.read(&mut byte).expect("unconsumed byte"), 1);
    assert_eq!(byte, *b"b");
}

#[test]
fn expired_write_and_flush_refuse_without_sending_bytes() {
    let (client, mut peer) = sockets();
    let mut socket = DeadlineSocket::new(client, expired());
    assert_eq!(
        socket.write(b"request").expect_err("expired write").kind(),
        io::ErrorKind::TimedOut
    );
    assert_eq!(
        socket.flush().expect_err("expired flush").kind(),
        io::ErrorKind::TimedOut
    );
    peer.set_nonblocking(true).expect("nonblocking peer");
    let mut byte = [0_u8];
    assert_eq!(
        peer.read(&mut byte).expect_err("zero sent bytes").kind(),
        io::ErrorKind::WouldBlock
    );
}

const CHILD_CASE: &str = "STS2_JEV_DEADLINE_CHILD_CASE";
const CHILD_TEST: &str = "tls::budget::tests::deadline_subprocess_entry";

struct OwnedChild(Option<Child>);

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn child_case(case: &str) -> Output {
    let child = Command::new(std::env::current_exe().expect("test executable"))
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env(CHILD_CASE, case)
        .env_remove("TYPESAFE_API_KEY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn owned test child");
    let mut owned = OwnedChild(Some(child));
    let started = Instant::now();
    loop {
        if owned
            .0
            .as_mut()
            .expect("owned child")
            .try_wait()
            .expect("inspect child")
            .is_some()
        {
            return owned
                .0
                .take()
                .expect("owned child")
                .wait_with_output()
                .expect("capture child");
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "deadline child did not terminate"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_stalled_resolver_is_bounded_by_the_actual_post_guard() {
    let output = child_case("stalled-resolution");
    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("resolver-entered"));
    assert!(!stdout.contains("unexpected-return"));
    assert!(!stdout.contains("\"decision\""));
}

#[test]
fn a_completed_exchange_cancels_and_joins_its_watchdog() {
    let output = child_case("completed");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("survived-cancelled-deadline"));
}

#[test]
fn deadline_subprocess_entry() -> Result<(), Box<dyn std::error::Error>> {
    let Ok(case) = std::env::var(CHILD_CASE) else {
        return Ok(());
    };
    match case.as_str() {
        "stalled-resolution" => {
            let _ = super::super::post_after_admission(b"{}", "test-only", short_budget(), || {
                println!("resolver-entered");
                loop {
                    thread::park();
                }
            });
            Err("unexpected-return from stalled resolution".into())
        }
        "completed" => {
            let value = within(short_budget(), || Ok(17)).expect("complete before deadline");
            assert_eq!(value, 17);
            // A watchdog left armed would terminate this child before the marker.
            thread::sleep(Duration::from_millis(400));
            println!("survived-cancelled-deadline");
            Ok(())
        }
        other => Err(format!("unknown test case {other}").into()),
    }
}
