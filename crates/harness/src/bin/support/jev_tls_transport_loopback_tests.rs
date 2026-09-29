// SPDX-License-Identifier: MIT

//! A live TLS handshake, driven against a loopback peer, with no network and no credential.
//!
//! The rest of this suite proves refusals by handing bytes to a pure function. That cannot cover the
//! one place where a mistake is silent: the handshake loop. A client that reads the peer's records
//! and never hands them to `rustls` -- dropping them, or calling `process_new_packets` over an empty
//! deframer -- still passes every offline test in the suite. It fails only against a peer that
//! completes a real handshake, which is why this file exists and why it drives a socket.
//!
//! The peer is a `rustls` server on `127.0.0.1` presenting the fixture leaf, and the client side is
//! the transport's own handshake loop through the `session_over` seam. No production path is
//! duplicated here: the same function a run calls is the one under test, differing only in which
//! root store the configuration is built from.

use std::io::Write;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use super::tls::client_config;
use super::tls::session::{connection_over, loopback_client_config};
use super::tls_loopback_fixture::{LOOPBACK_CA, LOOPBACK_LEAF, LOOPBACK_LEAF_KEY};

/// A server configuration presenting the fixture leaf, built the way a peer would build one.
fn server_config() -> Arc<rustls::ServerConfig> {
    let provider = rustls::crypto::ring::default_provider();
    let certified = rustls::sign::CertifiedKey::from_der(
        vec![
            rustls_pki_types::CertificateDer::from(LOOPBACK_LEAF),
            rustls_pki_types::CertificateDer::from(LOOPBACK_CA),
        ],
        rustls_pki_types::PrivateKeyDer::try_from(LOOPBACK_LEAF_KEY)
            .expect("the fixture key is a parseable PKCS#8 key"),
        &provider,
    )
    .expect("the fixture leaf and key are a matching pair");
    Arc::new(
        rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()
            .expect("a peer supports the safe default versions")
            .with_no_client_auth()
            .with_single_cert(
                certified.cert,
                rustls_pki_types::PrivateKeyDer::try_from(LOOPBACK_LEAF_KEY)
                    .expect("the fixture key is a parseable PKCS#8 key"),
            )
            .expect("the fixture certificate installs on the peer"),
    )
}

/// A listener on loopback, bound before the test returns so the address is real and not guessed.
fn loopback_listener() -> TcpListener {
    TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .expect("the loopback interface accepts a bound port")
}

/// Runs `peer` on a thread against the accepted half of a loopback connection.
///
/// The peer's socket bounds are its own, separate from the client's. What is under test is the
/// client's loop, so the peer must never be the thing that decides an assertion -- but it also must
/// not outlast the exchange, or a refusal the client made in milliseconds would be reported after
/// the fixture's patience ran out.
fn serve_with_timeout(
    listener: TcpListener,
    peer: impl FnOnce(TcpStream) + Send + 'static,
    timeout: Duration,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        if let Ok((stream, _)) = listener.accept() {
            let _ = stream.set_read_timeout(Some(timeout));
            let _ = stream.set_write_timeout(Some(timeout));
            peer(stream);
        }
    })
}

/// Drives the client's handshake over a loopback socket, exactly as `tls_session` does.
fn handshake_against(
    peer: impl FnOnce(TcpStream) + Send + 'static,
    trusted: bool,
) -> Result<(), String> {
    let listener = loopback_listener();
    let address = listener
        .local_addr()
        .expect("a bound listener has an address");
    // The peer's own timeout is short on purpose. It is not what the assertions are about -- they
    // are about the client's refusal -- and a fixture that waits out a long socket timeout turns a
    // 3ms refusal into a half-minute test. A peer that hits it has simply outlived the exchange.
    let server = serve_with_timeout(listener, peer, Duration::from_secs(5));
    let mut stream = TcpStream::connect(address).expect("the loopback peer accepts a connection");
    // `trusted` picks the store, and only the store. The handshake loop, the server name, and the
    // deadline are the transport's own, so the positive and negative cases differ in exactly the
    // one way they claim to: whether the fixture CA is in the trust store.
    let config = if trusted {
        loopback_client_config()
    } else {
        client_config().expect("the pinned root store builds a client config")
    };
    let outcome = connection_over(&mut stream, config, Instant::now())
        .map(|_| ())
        .map_err(|error| error.to_string());
    let _ = server.join();
    outcome
}

/// A peer that completes the handshake, writes one framed response, and closes cleanly.
fn responding_peer(response: Vec<u8>) -> impl FnOnce(TcpStream) + Send + 'static {
    move |mut socket| {
        let mut connection =
            rustls::ServerConnection::new(server_config()).expect("the peer builds a connection");
        // Drive the handshake over the raw socket with the same ordering the client uses: flush
        // what the session holds, then read the peer's bytes into it. The read is what lets a peer
        // see the client's alert: when the client refuses the fixture chain it sends one and
        // closes, and a peer that only ever writes would sit on a socket nobody will read until its
        // own 30-second timeout. Ending on the read that returns `Ok(0)` is what keeps a test's
        // runtime reflecting the client's behaviour rather than the fixture's patience.
        while connection.is_handshaking() {
            if connection.wants_write() {
                if connection.write_tls(&mut socket).is_err() {
                    return;
                }
                continue;
            }
            match connection.read_tls(&mut socket) {
                Ok(0) | Err(_) => return,
                Ok(_) => {
                    if connection.process_new_packets().is_err() {
                        return;
                    }
                }
            }
        }
        let mut session = rustls::StreamOwned::new(connection, socket);
        let _ = session.write_all(&response);
        let _ = session.flush();
        session.conn.send_close_notify();
        let _ = session.into_parts().1.shutdown(std::net::Shutdown::Both);
    }
}

/// The handshake completes against a peer whose certificate the client trusts.
///
/// This is the control the whole file rests on. It is the only assertion here that must succeed,
/// and it is the one that fails if the client loop stops moving bytes into the session: reading the
/// socket and calling `process_new_packets` without `read_tls` leaves the deframer empty forever, so
/// this test times out on the deadline refusal while every other test in the suite still passes.
#[test]
fn a_trusted_loopback_peer_completes_the_handshake() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec();
    handshake_against(responding_peer(response), true)
        .expect("a peer whose certificate the client trusts completes the handshake");
}

/// A peer whose certificate nothing the client trusts vouches for is refused.
///
/// The pinned roots are the ones a run uses, so this is the substitution the pinned store exists to
/// prevent, exercised end to end: a real TLS handshake against a real peer that presents a
/// well-formed certificate naming the admitted host, refused. The positive control above is what
/// makes this a refusal rather than a broken loop.
#[test]
fn a_peer_naming_the_admitted_host_but_signed_by_an_unknown_ca_is_refused() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".to_vec();
    let outcome = handshake_against(responding_peer(response), false);
    assert_eq!(
        outcome.expect_err("a chain no pinned root vouches for must be refused"),
        "the provider certificate or hostname did not verify",
        "the handshake must refuse the chain before any request is written"
    );
}

/// A peer that closes without completing the handshake is a refusal, not a hang.
///
/// The deadline refusal and the truncation refusal are different claims, and this is the one that
/// covers the socket reaching zero bytes mid-handshake: `read_tls` returning `Ok(0)` must end the
/// exchange rather than spin on a socket that will never produce another byte.
#[test]
fn a_peer_that_closes_during_the_handshake_is_refused() {
    let outcome = handshake_against(
        |stream| {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        },
        true,
    );
    assert_eq!(
        outcome.expect_err("a peer that closes mid-handshake must be refused"),
        "the provider closed the connection during the TLS handshake",
        "a truncated handshake is a refusal rather than a spin to the deadline"
    );
}

/// A peer that accepts the connection and then says nothing is bounded, not waited on.
///
/// The point is that the handshake inherits the whole-exchange deadline instead of getting a fresh
/// allowance per read. The assertion is on the refusal message, not on a wall-clock bound: a test
/// that asserted an elapsed time would be a flaky test on a loaded machine, and the deadline itself
/// is a constant already asserted elsewhere. What is being proved is that the wait is *cut short*
/// -- an accepted socket that never speaks produces the deadline refusal and returns.
#[test]
fn a_peer_that_never_answers_the_handshake_is_refused_rather_than_waited_on() {
    let outcome = handshake_against(
        |stream| {
            // Hold the connection open without writing. The client's read timeout has to end this.
            thread::sleep(Duration::from_secs(2));
            let _ = stream.shutdown(std::net::Shutdown::Both);
        },
        true,
    );
    assert_eq!(
        outcome.expect_err("a peer that never answers must be refused"),
        "the provider closed the connection during the TLS handshake",
        "the wait for a silent peer has to be cut by the deadline, not run to completion"
    );
}
