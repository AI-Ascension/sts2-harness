// SPDX-License-Identifier: MIT

//! Decrypted bytes can be returned without socket I/O; they still require a live deadline.

use super::*;
use std::io::Cursor;
use std::net::TcpListener;

const RESPONSE: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}";

fn buffered_response(
    started: Instant,
) -> rustls::StreamOwned<rustls::ClientConnection, DeadlineSocket> {
    let mut client = rustls::ClientConnection::new(
        session::loopback_client_config(),
        rustls_pki_types::ServerName::try_from(PROVIDER_HOST.to_owned()).expect("provider name"),
    )
    .expect("client");
    let mut server =
        rustls::ServerConnection::new(crate::tls_loopback_tests::server_config()).expect("server");
    // Drive the real verifier and TLS record parser in memory to preload plaintext and a clean
    // close. The socket below intentionally has no response bytes available to read.
    for _ in 0..16 {
        let mut outgoing = Vec::new();
        client.write_tls(&mut outgoing).expect("client flight");
        if !outgoing.is_empty() {
            server
                .read_tls(&mut Cursor::new(outgoing))
                .expect("server read");
            server.process_new_packets().expect("server process");
        }
        let mut outgoing = Vec::new();
        server.write_tls(&mut outgoing).expect("server flight");
        if !outgoing.is_empty() {
            client
                .read_tls(&mut Cursor::new(outgoing))
                .expect("client read");
            client.process_new_packets().expect("client process");
        }
        if !client.is_handshaking()
            && !server.is_handshaking()
            && !client.wants_write()
            && !server.wants_write()
        {
            break;
        }
    }
    assert!(!client.is_handshaking() && !server.is_handshaking());
    assert!(!client.wants_write() && !server.wants_write());
    server.writer().write_all(RESPONSE).expect("write response");
    server.send_close_notify();
    let mut wire = Vec::new();
    server.write_tls(&mut wire).expect("response records");
    client
        .read_tls(&mut Cursor::new(wire))
        .expect("response input");
    let state = client.process_new_packets().expect("decrypted response");
    assert_eq!(state.plaintext_bytes_to_read(), RESPONSE.len());
    assert!(state.peer_has_closed());

    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
    let socket = TcpStream::connect(listener.local_addr().expect("address")).expect("connect");
    let (peer, _) = listener.accept().expect("accept");
    drop(peer);
    rustls::StreamOwned::new(client, DeadlineSocket::new(socket, started))
}

#[test]
fn a_complete_buffered_response_and_close_are_accepted_within_budget() {
    let started = Instant::now();
    let mut session = buffered_response(started);
    let raw = read_response(&mut session, started).expect("fresh response control");
    assert_eq!(framing::parse_body(&raw).expect("framed body"), b"{}");
}

#[test]
fn a_complete_buffered_response_and_close_are_refused_after_deadline() {
    let started = Instant::now() - deadline() - Duration::from_secs(1);
    let mut session = buffered_response(started);
    let error = read_response(&mut session, started).expect_err("late response refused");
    assert_eq!(error.to_string(), budget::EXPIRED);
}
