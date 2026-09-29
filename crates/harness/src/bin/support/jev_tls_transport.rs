// SPDX-License-Identifier: MIT

//! Bounded in-process HTTPS transport for the System One bridge.
//!
//! This is the migration target ADR 0053 named: one digest-pinned artifact performs the exchange
//! instead of an operator-owned second executable, so a run carries one verified digest rather
//! than two, and the operator no longer has to install and pin a transport beside the bridge.
//!
//! Everything that has to stay reviewable stays in the bridge. This module owns only the socket,
//! the TLS session, and the framing of one `POST`; it never sees the action catalog, the
//! confidence gate, or the decision. It performs exactly one exchange and returns the response
//! body, never an HTTP message.
//!
//! The refusals are the contract. A non-`200` status -- including the provider's `429` and `529` --
//! a certificate or hostname that does not verify against the pinned roots, a body above the
//! bound, a truncated or malformed frame, and a missed deadline are all the same outcome: an
//! `Err`, no decision, and no fallback. A TLS failure is never a downgrade to plaintext and never
//! a retry, because a retry policy belongs to the caller and a downgrade would make a run's
//! evidence unable to say what terminated the connection.
//!
//! Trust anchor: `webpki-roots`, pinned by version in `Cargo.toml` and carried in `Cargo.lock`. It
//! is compiled into this binary rather than read from the system store, so a run record can state
//! which roots verified the peer instead of saying "it used the system roots".

/// What counts as a whole response, kept beside the socket rather than folded into it.
#[path = "jev_tls_transport_framing.rs"]
pub(super) mod framing;

/// The TLS session, behind one small surface so a certificate refusal precedes the request.
#[path = "jev_tls_transport_session.rs"]
pub(super) mod session;

use session::tls_session;

// The refusal tests reach the framer and the verifier the way the transport does. Re-exported
// under `cfg(test)` so production does not carry an import it does not use.
#[cfg(test)]
pub(super) use {framing::parse_body, session::client_config};

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// Provider host this lane is admitted against.
///
/// The certificate is verified for exactly this name, so a redirect or a substituted host is a
/// verification failure rather than a connection the bridge follows.
pub(super) const PROVIDER_HOST: &str = "api.typesafe.ai";

/// Provider route this lane posts to.
pub(super) const PROVIDER_PATH: &str = "/v1/systemone";

/// Largest response body this transport will read, in bytes.
///
/// Matches the bridge's own request, response, and decision bound, so a response cannot be larger
/// than the decision the bridge is willing to hold. This bounds the *body*: the header block is
/// bounded separately by [`MAX_HEADER_BYTES`] and the two are never added together, because a
/// response at the body bound is accepted even when its headers are also at their bound.
pub(super) const MAX_RESPONSE_BYTES: usize = 128 * 1024;

/// Largest header block this transport will accept, in bytes.
pub(super) const MAX_HEADER_BYTES: usize = 8192;

/// Largest bearer credential this transport will send, in bytes.
///
/// A credential this long is not a credential. Refusing it before connecting keeps a
/// misconfiguration from becoming a request the peer has to reject.
pub(super) const MAX_CREDENTIAL_BYTES: usize = 4096;

/// How long the whole exchange may take, covering connect, handshake, write, and read.
///
/// Read by [`deadline`] rather than named here so the session module can reach it without this
/// module's private constant being duplicated across a `use`.
fn deadline() -> Duration {
    Duration::from_secs(100)
}

/// How long one connection attempt may take before it is a refusal.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Name of the environment variable carrying the bearer credential.
///
/// The credential is read by name, never by argument, so it is not a captured byte in a process
/// table, an argument vector, or a record. It is held only for the one `POST`.
const CREDENTIAL_VARIABLE: &str = "TYPESAFE_API_KEY";

/// Performs exactly one bounded, verified HTTPS `POST` and returns the response body.
///
/// # Errors
///
/// Returns an opaque `&'static str` for every refusal. The message names the class of failure and
/// never carries the credential, the request body, or the response body, so an error surfaced in
/// a log, a `cause:` line, or a capture cannot leak any of them.
pub(super) fn post(body: &[u8]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // Read and validate the credential first. This is deliberately before resolution, connect,
    // and handshake: a run with no credential must refuse without opening a socket at all, and
    // must never build a header line out of a credential that was not there.
    let credential = credential()?;
    let started = Instant::now();
    let mut session = tls_session(connect(started)?, started)?;

    let request = build_request(body, &credential);
    // The write timeout was last set at connect time, against the deadline as it stood then. It is
    // refreshed against the same original deadline so a slow handshake cannot hand the writer a
    // stale allowance that outlives the bound the run promised.
    bound_socket(session.get_mut(), started)?;
    session
        .write_all(&request)
        .map_err(|_| "the provider connection failed while sending the request")?;
    session
        .flush()
        .map_err(|_| "the provider connection failed while sending the request")?;

    let raw = read_response(&mut session, started)?;
    framing::parse_body(&raw)
}

/// Reads and validates the bearer credential from the environment.
///
/// # Errors
///
/// Refuses a missing, empty, oversized, or header-unsafe credential. Each is refused before any
/// socket is opened, so none of them can appear in a provider log as a request that was sent.
fn credential() -> Result<String, Box<dyn std::error::Error>> {
    let credential = std::env::var(CREDENTIAL_VARIABLE)
        .map_err(|_| "no provider credential is present in the environment")?;
    if credential.is_empty() {
        return Err("the provider credential is empty".into());
    }
    if credential.len() > MAX_CREDENTIAL_BYTES {
        return Err("the provider credential is longer than this transport will send".into());
    }
    // A credential is placed in a header line, so a control character or a space in it would end
    // the line early and let the rest of the value be read as further header lines. No character
    // matched here is legal in a bearer token, so this is refused rather than escaped: escaping
    // would send a credential the provider cannot authenticate and would hide the misconfiguration.
    if let Some(character) = credential
        .chars()
        .find(|character| character.is_control() || character.is_whitespace())
    {
        return Err(format!(
            "the provider credential carries U+{:04X}, which cannot appear in a header",
            character as u32
        )
        .into());
    }
    Ok(credential)
}

/// Connects to the admitted host, refusing a missing route rather than following a redirect.
///
/// # Errors
///
/// Refuses a host that does not resolve, a connection that cannot be opened inside both the
/// per-attempt cap and the whole-exchange deadline, and a socket whose options cannot be set.
fn connect(started: Instant) -> Result<TcpStream, Box<dyn std::error::Error>> {
    let address = (PROVIDER_HOST, 443)
        .to_socket_addrs()
        .map_err(|_| "the provider host did not resolve")?
        .next()
        .ok_or("the provider host did not resolve")?;
    // The per-attempt cap and the whole-exchange deadline are both honoured, whichever is
    // shorter, so no phase of the exchange can outlast the bound the run promised.
    let allowance = CONNECT_TIMEOUT.min(remaining(started));
    if allowance.is_zero() {
        return Err("the provider exchange exceeded its deadline".into());
    }
    let stream = TcpStream::connect_timeout(&address, allowance)
        .map_err(|_| "the provider connection could not be opened")?;
    stream
        .set_nodelay(true)
        .map_err(|_| "the provider connection could not be configured")?;
    // The handshake is driven on this socket directly, so the read and write timeouts have to be
    // present before the first byte moves, not only before the first read of the response.
    bound_socket(&stream, started)?;
    Ok(stream)
}

/// Bounds both directions of the socket for the rest of the exchange.
///
/// A timeout is set in both directions deliberately. Read-only bounds still let a peer that stops
/// reading leave the bridge blocked in `write` indefinitely, because a full socket buffer blocks
/// the writer rather than returning a timeout, and that is the shape a stalled provider takes.
///
/// `started` is the whole-exchange start, not the present moment. Re-deriving the allowance from
/// `Instant::now()` on every call would hand each phase a fresh 100 seconds, which is the same
/// unbounded wait the bound exists to prevent -- a peer dribbling bytes inside every individual
/// interval could keep a run open forever. Every refresh therefore computes what is left of the
/// one deadline.
pub(super) fn bound_socket(
    stream: &TcpStream,
    started: Instant,
) -> Result<(), Box<dyn std::error::Error>> {
    stream
        .set_read_timeout(Some(remaining(started)))
        .map_err(|_| "the provider connection timeout could not be set")?;
    stream
        .set_write_timeout(Some(remaining(started)))
        .map_err(|_| "the provider connection timeout could not be set")?;
    Ok(())
}

/// How much of the whole-exchange deadline is left, never below one millisecond.
///
/// A zero duration passed to `set_read_timeout` means "block forever" on some platforms and
/// "fail immediately" on others, so the remainder is floored rather than allowed to reach zero.
fn remaining(started: Instant) -> Duration {
    deadline()
        .saturating_sub(started.elapsed())
        .max(Duration::from_millis(1))
}

/// Builds the one request this transport is allowed to send.
///
/// The credential is placed in the header here and nowhere else. The request is fully buffered
/// before the first byte is written, so a write failure cannot leave a partial header line, and
/// the length is declared rather than left to a connection close. The credential is validated
/// before it reaches this function, so nothing that could terminate the header line is here.
pub(super) fn build_request(body: &[u8], credential: &str) -> Vec<u8> {
    let mut request = format!(
        "POST {PROVIDER_PATH} HTTP/1.1\r\n\
         Host: {PROVIDER_HOST}\r\n\
         Authorization: Bearer {credential}\r\n\
         Content-Type: application/json\r\n\
         Accept: application/json\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n",
        body.len(),
    )
    .into_bytes();
    request.extend_from_slice(body);
    request
}

/// Reads one response, bounded in size, in time, and in bytes.
fn read_response(
    session: &mut rustls::StreamOwned<rustls::ClientConnection, TcpStream>,
    started: Instant,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    // The bound is the body bound plus a full header block, so a response that is at the body
    // bound is still accepted while one byte past the body bound -- even with a maximal header
    // block -- is still refused. The two bounds are enforced separately in `parse_body`; this cap
    // exists only so an endless response is detected while it is being read rather than after.
    let cap = MAX_RESPONSE_BYTES + MAX_HEADER_BYTES + 1;
    let mut raw = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        // The timeout is refreshed on every read so the bound is the whole exchange rather than
        // a fresh allowance per read: without this a peer dribbling one byte at a time inside
        // each interval could keep a run open indefinitely.
        session
            .get_mut()
            .set_read_timeout(Some(remaining(started)))
            .map_err(|_| "the provider connection timeout could not be set")?;
        let read = session
            .read(&mut chunk)
            .map_err(|error| read_failure(&error))?;
        match read {
            0 => break,
            read => {
                if raw.len() + read > cap {
                    return Err("the provider response exceeded its bound".into());
                }
                raw.extend_from_slice(&chunk[..read]);
            }
        }
    }
    // `Ok(0)` from a `rustls` reader is a clean shutdown: the peer sent `close_notify` and every
    // byte it sent has been delivered. A TCP close without `close_notify` reaches here as
    // `UnexpectedEof`, which `read_failure` already turns into a refusal, so a truncated response
    // is never mistaken for a complete one. The check below is the belt to that braces: it
    // refuses a clean close that arrived before a whole frame was read, which is the other way a
    // peer can hand back a body with the tail missing.
    if raw.is_empty() {
        return Err("the provider closed the connection without sending a response".into());
    }
    Ok(raw)
}

/// Whether a read failure is a timeout, mapped to the deadline refusal.
///
/// A socket timeout during the response means the exchange outlasted its bound, and an unexpected
/// end of file means the peer stopped without a `close_notify`. Both are translated into the
/// deadline and truncation refusals rather than being read as an end of the message, so the
/// deadline has one representation whatever phase ran out and a truncation is never accepted.
fn read_failure(error: &std::io::Error) -> Box<dyn std::error::Error> {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        "the provider closed the connection without a TLS close notification".into()
    } else if is_timeout(error) {
        "the provider exchange exceeded its deadline".into()
    } else {
        "the provider response could not be read".into()
    }
}

/// Whether a socket failure is a timeout on this platform, spelled the portable way.
fn is_timeout(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}
