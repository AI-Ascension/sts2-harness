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

use std::io::Write;
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use super::{PROVIDER_HOST, deadline};

/// A client configuration trusting only the loopback fixture CA, for the offline handshake tests.
///
/// Present only under `cfg(test)` so production has exactly one way to build a configuration and
/// cannot reach this one by accident. The tests need it because a handshake that is only ever
/// refused proves nothing: the peer has to be able to complete one for the loop to be shown working.
#[cfg(test)]
pub(crate) fn loopback_client_config() -> Arc<rustls::ClientConfig> {
    let mut store = rustls::RootCertStore::empty();
    store
        .add(rustls_pki_types::CertificateDer::from(
            crate::tls_loopback_fixture::LOOPBACK_CA,
        ))
        .expect("the loopback fixture is a certificate a root store accepts");
    config_from_roots(store).expect("a store with one root builds a client config")
}

/// Builds the client TLS configuration from the pinned root store.
///
/// The provider is verified for [`PROVIDER_HOST`] only. A peer presenting a certificate that
/// chains to the pinned roots but names a different host is refused here, in the handshake,
/// rather than after the request has been sent.
pub(crate) fn client_config() -> Result<Arc<rustls::ClientConfig>, Box<dyn std::error::Error>> {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    config_from_roots(roots)
}

/// Builds a client configuration from an explicit root store.
///
/// `client_config` is the only caller in production and passes exactly the pinned roots. This seam
/// exists so the offline handshake tests can present a certificate signed by a fixture CA and be
/// accepted, which is what makes the negative cases above them non-vacuous: a loopback peer that
/// only ever gets refused proves nothing about the handshake loop.
fn config_from_roots(
    roots: rustls::RootCertStore,
) -> Result<Arc<rustls::ClientConfig>, Box<dyn std::error::Error>> {
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
    let connection = connection_over(&mut stream, client_config()?, started)?;
    Ok(rustls::StreamOwned::new(connection, stream))
}

/// Builds a connection for the admitted host and completes the handshake over an open socket.
///
/// The split from [`tls_session`] is the test seam: the loopback tests need a configuration whose
/// trust store contains their fixture CA, and they must go through the *same* handshake loop rather
/// than a parallel copy of it. A copy would be a different piece of code from the one a run uses,
/// so passing it would prove nothing about the production path.
pub(crate) fn connection_over(
    stream: &mut TcpStream,
    config: Arc<rustls::ClientConfig>,
    started: Instant,
) -> Result<rustls::ClientConnection, Box<dyn std::error::Error>> {
    let name = rustls_pki_types::ServerName::try_from(PROVIDER_HOST.to_owned())
        .map_err(|_| "the admitted provider host is not a valid server name")?;
    let mut connection = rustls::ClientConnection::new(config, name)
        .map_err(|_| "the provider TLS session could not be created")?;
    handshake(&mut connection, stream, started)?;
    Ok(connection)
}

/// Drives the handshake to completion, refusing a certificate or hostname that does not verify.
///
/// `rustls` verifies while it processes the handshake transcript, so the handshake is a read/write
/// loop over the raw socket: `read_tls` hands the socket's bytes to the session's deframer,
/// `process_new_packets` advances and verifies what was deframed, and `wants_write` says whether
/// the transcript produced records that still need sending.
///
/// The bytes have to travel through `read_tls` rather than being read and discarded. Reading the
/// socket directly would leave the deframer empty, so `process_new_packets` would advance nothing
/// and the loop would spin until the deadline -- a live handshake against a real provider would
/// never complete. The order below is the one the crate documents: `read_tls`, then
/// `process_new_packets` after every successful read, then drain whatever wants writing.
///
/// A peer that stalls makes a socket read or write time out, which is the same deadline refusal the
/// response path reports.
fn handshake(
    connection: &mut rustls::ClientConnection,
    stream: &mut TcpStream,
    started: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    while connection.is_handshaking() {
        if started.elapsed() >= deadline() {
            return Err("the provider exchange exceeded its deadline".into());
        }
        // Both directions are re-bounded from the same original deadline before every operation,
        // so no phase of the exchange can be handed a fresh allowance.
        super::bound_socket(stream, started)?;
        flush_records(connection, stream)?;
        // `read_tls` pulls from the socket itself and hands the bytes to the deframer, so the
        // transcript actually reaches the verifier. This loop never inspects a byte; it only
        // moves them from the socket into the session.
        let read = connection
            .read_tls(stream)
            .map_err(|_| "the provider TLS handshake could not be completed")?;
        if read == 0 {
            return Err("the provider closed the connection during the TLS handshake".into());
        }
        connection
            .process_new_packets()
            .map_err(|_| "the provider certificate or hostname did not verify")?;
    }
    // The final flight -- the client's Finished -- can be produced by the last
    // `process_new_packets` and still be unsent, so drain once more after the loop.
    super::bound_socket(stream, started)?;
    flush_records(connection, stream)?;
    Ok(())
}

/// Drains the records the session is holding into the socket.
///
/// `wants_write` says the session is holding records to emit, and `write_tls` drains them into a
/// buffer. A session that claims to want to write and then produces nothing would spin this loop
/// forever, so an empty drain is treated as a session that cannot make progress rather than as
/// something to retry.
fn flush_records(
    connection: &mut rustls::ClientConnection,
    stream: &mut TcpStream,
) -> Result<(), Box<dyn std::error::Error>> {
    while connection.wants_write() {
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
    Ok(())
}
