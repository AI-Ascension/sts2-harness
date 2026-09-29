// SPDX-License-Identifier: MIT

//! The TLS session for the System One transport.
//!
//! Split from the transport so the socket, the credential, and the response framing stay in one
//! file and everything that decides *whether this peer is who it claims to be* stays in another.
//! That is the boundary worth having: a certificate refusal must happen before a request is built,
//! and keeping the handshake behind one small surface makes that ordering visible rather than
//! incidental.
//!
//! The trust anchor is `webpki-roots`, pinned by version in `Cargo.toml` and carried in
//! `Cargo.lock`. It is compiled into this binary rather than read from the system store, so a run
//! record can state which roots verified the peer instead of saying "it used the system roots".

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use super::{DEADLINE, PROVIDER_HOST};

/// Builds the client TLS configuration from the pinned root store.
///
/// The provider is verified for [`PROVIDER_HOST`] only. A peer presenting a certificate that
/// chains to the pinned roots but names a different host is refused here, in the handshake,
/// rather than after the request has been sent.
pub(crate) fn client_config() -> Result<Arc<rustls::ClientConfig>, Box<dyn std::error::Error>> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| "no supported TLS protocol version could be selected")?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Wraps the connected socket in a client session bound to the admitted host and handshakes.
///
/// The handshake is driven here rather than left to the first `read` or `write`, so that the
/// certificate and hostname are verified before the request is built and sent. That ordering is
/// the point: a refusal at this stage means nothing about the request was disclosed to the peer.
pub(crate) fn tls_session(
    mut stream: TcpStream,
    started: Instant,
) -> Result<rustls::StreamOwned<rustls::ClientConnection, TcpStream>, Box<dyn std::error::Error>> {
    let name = rustls_pki_types::ServerName::try_from(PROVIDER_HOST.to_owned())
        .map_err(|_| "the admitted provider host is not a valid server name")?;
    let mut connection = rustls::ClientConnection::new(client_config()?, name)
        .map_err(|_| "the provider TLS session could not be created")?;
    handshake(&mut connection, &mut stream, started)?;
    Ok(rustls::StreamOwned::new(connection, stream))
}

/// Drives the handshake to completion, refusing a certificate or hostname that does not verify.
///
/// `rustls` performs verification while it processes the handshake transcript, so the whole
/// handshake is a read/write loop over the raw socket: `process_new_packets` advances and verifies,
/// and `wants_write` says whether the last transcript produced bytes that still need sending. A
/// peer that stalls makes a socket read or write time out, which is the same deadline refusal the
/// response path reports.
fn handshake(
    connection: &mut rustls::ClientConnection,
    stream: &mut TcpStream,
    started: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    while connection.is_handshaking() {
        if started.elapsed() >= DEADLINE {
            return Err("the provider exchange exceeded its deadline".into());
        }
        while connection.wants_write() {
            // `wants_write` says the session is holding records to emit, and `write_tls` drains
            // them into a buffer. A session that claims to want to write and then produces
            // nothing would spin this loop forever, so an empty drain is treated as a session
            // that cannot make progress.
            let mut outgoing = Vec::new();
            connection
                .write_tls(&mut outgoing)
                .map_err(|_| "the provider TLS handshake could not be prepared")?;
            if outgoing.is_empty() {
                return Err("the provider TLS session made no handshake progress".into());
            }
            stream
                .write_all(&outgoing)
                .map_err(|_| "the provider TLS handshake could not be sent")?;
            stream
                .flush()
                .map_err(|_| "the provider TLS handshake could not be sent")?;
        }
        let mut buffer = [0_u8; 4096];
        let read = stream
            .read(&mut buffer)
            .map_err(|_| "the provider TLS handshake could not be completed")?;
        if read == 0 {
            return Err("the provider closed the connection during the TLS handshake".into());
        }
        connection
            .process_new_packets()
            .map_err(|_| "the provider certificate or hostname did not verify")?;
    }
    Ok(())
}
