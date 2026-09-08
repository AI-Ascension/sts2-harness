// SPDX-License-Identifier: MIT

//! Safe transport API and lifecycle state machine.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use crate::native;
use crate::{EndpointPolicy, MAX_FRAME_BYTES};

const MAX_DEADLINE: Duration = Duration::from_secs(5);

/// Fixed redacted transport categories.  OS messages, paths, PIDs, frame
/// contents and credential bytes are intentionally absent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransportError {
    Configuration,
    Identity,
    Credential,
    Deadline,
    Framing,
    Busy,
    Closed,
    Os,
}

impl std::fmt::Display for TransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Configuration => "invalid worker transport configuration",
            Self::Identity => "worker peer identity verification failed",
            Self::Credential => "worker credential authentication failed",
            Self::Deadline => "worker connection deadline exceeded",
            Self::Framing => "worker transport frame rejected",
            Self::Busy => "worker transport is busy",
            Self::Closed => "worker transport is closed",
            Self::Os => "worker transport operating-system failure",
        })
    }
}

impl std::error::Error for TransportError {}

/// One monotonic budget shared by accept, peer verification, auth and both
/// bounded JSON frames.  The budget is never reset by partial I/O.
#[derive(Clone, Copy, Debug)]
pub struct Deadline(Instant);

impl Deadline {
    /// Starts a deadline no longer than the five-second transport maximum.
    pub fn new(timeout: Duration) -> Result<Self, TransportError> {
        if timeout.is_zero() || timeout > MAX_DEADLINE {
            return Err(TransportError::Configuration);
        }
        let instant = Instant::now()
            .checked_add(timeout)
            .ok_or(TransportError::Configuration)?;
        Ok(Self(instant))
    }

    /// Creates a deadline from an already-created monotonic instant.  This is
    /// useful when the caller starts its budget before entering `accept`.
    pub fn at(instant: Instant) -> Result<Self, TransportError> {
        let now = Instant::now();
        let remaining = instant
            .checked_duration_since(now)
            .ok_or(TransportError::Deadline)?;
        if remaining.is_zero() {
            return Err(TransportError::Deadline);
        }
        if remaining > MAX_DEADLINE {
            return Err(TransportError::Configuration);
        }
        Ok(Self(instant))
    }

    pub fn instant(self) -> Instant {
        self.0
    }
}

impl From<&Deadline> for Deadline {
    fn from(value: &Deadline) -> Self {
        *value
    }
}

#[cfg(target_os = "windows")]
impl Deadline {
    pub(crate) fn check(self) -> Result<(), TransportError> {
        if Instant::now() >= self.instant() {
            Err(TransportError::Deadline)
        } else {
            Ok(())
        }
    }

    pub(crate) fn remaining_millis(self) -> Result<u32, TransportError> {
        let duration = self
            .instant()
            .checked_duration_since(Instant::now())
            .ok_or(TransportError::Deadline)?;
        let millis = duration.as_millis().saturating_add(1);
        u32::try_from(millis.min(u128::from(u32::MAX))).map_err(|_| TransportError::Deadline)
    }
}

/// Opaque proof that the native endpoint authenticated one exact peer for the
/// owning connection.  It has no fields, accessors, serialization or clone;
/// only the transport can construct it.
pub struct PeerWitness {
    _sealed: (),
}

impl PeerWitness {
    pub(crate) fn verified() -> Self {
        Self { _sealed: () }
    }
}

/// A server-side named-pipe listener with one active exchange at a time.
pub struct WorkerListener {
    inner: native::Listener,
}

impl WorkerListener {
    /// Creates the first named-pipe instance with the exact owner DACL and
    /// first-instance flag.  Stolen endpoint names fail closed.
    pub fn bind(policy: EndpointPolicy) -> Result<Self, TransportError> {
        native::Listener::bind(policy).map(|inner| Self { inner })
    }

    /// Accepts and authenticates exactly one connection under the supplied
    /// absolute deadline.  The generic input accepts either [`Deadline`] or a
    /// borrowed deadline created by [`Deadline::new`] or [`Deadline::at`].
    pub fn accept_authenticated<D: Into<Deadline>>(
        &mut self,
        deadline: D,
    ) -> Result<AuthenticatedConnection, TransportError> {
        self.inner
            .accept_authenticated(deadline.into())
            .map(|inner| AuthenticatedConnection { inner })
    }

    /// Stops accepting new peers and releases the listener.  Native pending
    /// I/O is cancelled and joined before this method returns.
    pub fn shutdown(&mut self) -> Result<(), TransportError> {
        self.inner.shutdown()
    }
}

impl Drop for WorkerListener {
    fn drop(&mut self) {
        let _ = self.inner.shutdown();
    }
}

/// One authenticated request/response exchange.  The native connection keeps
/// the limited-query process handle and both held image/ancestor resources
/// alive until the exchange is closed.
pub struct AuthenticatedConnection {
    inner: native::Connection,
}

impl AuthenticatedConnection {
    /// Returns the non-forgeable peer proof for this connection.
    pub fn peer_witness(&self) -> &PeerWitness {
        self.inner.peer_witness()
    }

    /// Reads exactly one bounded, framed handoff request after authentication.
    /// JSON decoding and all semantic/admission checks remain caller-owned.
    pub fn read_request(&mut self) -> Result<Vec<u8>, TransportError> {
        self.inner.read_frame(MAX_FRAME_BYTES)
    }

    /// Writes exactly one bounded, framed handoff response and closes the
    /// exchange.  A failed or cancelled write poisons the connection.
    pub fn write_response(&mut self, body: &[u8]) -> Result<(), TransportError> {
        self.inner.write_frame(body, MAX_FRAME_BYTES)
    }

    /// Explicitly closes the connection.  Dropping it has the same effect.
    pub fn close(&mut self) -> Result<(), TransportError> {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_is_bounded_and_monotonic() {
        assert!(matches!(
            Deadline::new(Duration::ZERO),
            Err(TransportError::Configuration)
        ));
        assert!(matches!(
            Deadline::new(Duration::from_secs(6)),
            Err(TransportError::Configuration)
        ));
        assert!(Deadline::new(Duration::from_millis(1)).is_ok());
        assert!(matches!(
            Deadline::at(Instant::now()),
            Err(TransportError::Deadline)
        ));
    }

    #[test]
    fn transport_errors_are_fixed_and_redacted() {
        let text = TransportError::Credential.to_string();
        assert_eq!(text, "worker credential authentication failed");
        assert!(!text.contains("\\"));
        assert!(!text.contains("pid"));
    }
}
