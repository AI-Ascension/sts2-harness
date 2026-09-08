// SPDX-License-Identifier: MIT

//! Authenticated, owner-local Windows named-pipe transport for the watchdog
//! worker handoff.
//!
//! This crate deliberately stops at transport authentication.  It does not
//! decode handoff JSON, access a store, launch a process, or make an episode
//! admission decision.  The endpoint is a one-request/one-response exchange;
//! callers must retain the opaque [`PeerWitness`] alongside the decoded
//! request until the harness performs its own capability and boot checks.

#![cfg(target_os = "windows")]

mod policy;
mod transport;

mod native;

pub use policy::{EndpointPolicy, ExpectedPeer, Sid};
pub use transport::{
    AuthenticatedConnection, Deadline, PeerWitness, TransportError, WorkerListener,
};

/// The fixed authentication profile prefix, including its NUL terminator.
pub const AUTH_MAGIC: &[u8; 25] = b"ascension-worker-auth-v1\0";
/// The smallest valid authentication body: magic plus one credential byte.
pub const MIN_AUTH_BODY_BYTES: usize = AUTH_MAGIC.len() + 1;
/// The largest valid authentication body: magic plus the bounded credential.
pub const MAX_AUTH_BODY_BYTES: usize = AUTH_MAGIC.len() + MAX_CREDENTIAL_BYTES;
/// Maximum credential bytes accepted by the transport.
pub const MAX_CREDENTIAL_BYTES: usize = 4_096;
/// Maximum encoded JSON request or response body.
pub const MAX_FRAME_BYTES: usize = 65_536;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_bounds_are_frozen() {
        assert_eq!(AUTH_MAGIC.len(), 25);
        assert_eq!(MIN_AUTH_BODY_BYTES, 26);
        assert_eq!(MAX_AUTH_BODY_BYTES, 4_121);
        assert_eq!(MAX_FRAME_BYTES, 65_536);
    }
}
