// SPDX-License-Identifier: MIT

//! Refusal vocabulary for the broker-owned provider-session metadata store.
//!
//! Every way the store can refuse a call, and the stable `reason_code` each one reports. Kept
//! apart from the store's envelope and key handling so the set of refusals can be read, and
//! extended, as one list.

use super::super::SessionError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderSessionMetadataStoreError {
    InvalidScope,
    InvalidKey,
    InvalidPath,
    ScopeMismatch,
    NotFound,
    Capacity,
    Corrupt,
    Crypto,
    /// The exclusive policy-owner lease is held by another live owner.
    Busy,
    Io,
    Unsupported,
    Session(SessionError),
}

impl std::fmt::Display for ProviderSessionMetadataStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScope => formatter.write_str("provider metadata store scope is invalid"),
            Self::InvalidKey => formatter.write_str("provider metadata store key is invalid"),
            Self::InvalidPath => formatter.write_str("provider metadata store path is invalid"),
            Self::ScopeMismatch => {
                formatter.write_str("provider metadata store scope does not match")
            }
            Self::NotFound => formatter.write_str("provider metadata store journal was not found"),
            Self::Capacity => formatter.write_str("provider metadata store is over its bound"),
            Self::Corrupt => formatter.write_str("provider metadata store envelope is corrupt"),
            Self::Crypto => formatter.write_str("provider metadata store authentication failed"),
            Self::Busy => formatter.write_str("provider metadata journal already has an owner"),
            Self::Io => formatter.write_str("provider metadata store filesystem operation failed"),
            Self::Unsupported => formatter.write_str("provider metadata store mode is unsupported"),
            Self::Session(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProviderSessionMetadataStoreError {}

impl From<SessionError> for ProviderSessionMetadataStoreError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}
