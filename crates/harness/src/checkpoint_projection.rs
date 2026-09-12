// SPDX-License-Identifier: MIT

//! Public projection and keyed handles for exact checkpoints.
//!
//! Ordinary consumers must not receive exact-state digests, blob digests, compatibility digests, or
//! hidden gameplay values. A raw full-state digest is a dictionary oracle for low-entropy hidden
//! choices, so public surfaces carry only a keyed opaque handle plus permitted boundary and
//! assurance summaries. Deriving the handle requires the trusted projection key, which lets an
//! unauthorized caller compare nothing but tokens it cannot reproduce.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::execution::{ExactCheckpointId, ExactStateDigest};

use crate::exact_transition::OccurrenceId;

/// Serialized prefix of a keyed public checkpoint handle.
pub const HANDLE_PREFIX: &str = "ckpt-h1:";
/// Domain separator for the keyed handle.
pub const HANDLE_DOMAIN: &[u8] = b"AI-ASCENSION/PUBLIC-CHECKPOINT-HANDLE/v1\0";
/// Minimum accepted projection key length in bytes.
pub const MIN_HANDLE_KEY_BYTES: usize = 32;

/// Rejection reasons for the public projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectionError {
    /// The key is shorter than [`MIN_HANDLE_KEY_BYTES`].
    WeakKey,
    /// A boundary label or occurrence identifier is unusable.
    InvalidInput,
    /// A handle does not match the expected prefix or length.
    InvalidHandle,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WeakKey => "projection key is too short",
            Self::InvalidInput => "projection input is invalid",
            Self::InvalidHandle => "checkpoint handle is invalid",
        })
    }
}

impl std::error::Error for ProjectionError {}

/// Public summary safe to hand to an ordinary agent or public transcript.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublicCheckpointSummary {
    /// Keyed opaque handle; never the exact digest.
    pub handle: String,
    /// Occurrence this summary describes.
    pub occurrence: String,
    /// Boundary kind at capture.
    pub boundary_kind: String,
    /// Game phase at capture.
    pub boundary_phase: String,
    /// Assurance label; whether a restore was actually verified.
    pub restore_verified: bool,
}

/// Trusted projection key that issues and checks public checkpoint handles.
#[derive(Clone, Eq, PartialEq)]
pub struct ProjectionKey {
    secret: Vec<u8>,
}

impl ProjectionKey {
    /// Creates a projection key from at least [`MIN_HANDLE_KEY_BYTES`] secret bytes.
    pub fn new(secret: &[u8]) -> Result<Self, ProjectionError> {
        if secret.len() < MIN_HANDLE_KEY_BYTES {
            return Err(ProjectionError::WeakKey);
        }
        Ok(Self {
            secret: secret.to_vec(),
        })
    }

    /// Issues a keyed handle for one checkpoint occurrence.
    pub fn handle(
        &self,
        checkpoint: &ExactCheckpointId,
        state: &ExactStateDigest,
        occurrence: &OccurrenceId,
    ) -> Result<String, ProjectionError> {
        if occurrence.as_str().is_empty() {
            return Err(ProjectionError::InvalidInput);
        }
        let mut message = Vec::new();
        message.extend_from_slice(checkpoint.as_str().as_bytes());
        message.push(0);
        message.extend_from_slice(state.as_str().as_bytes());
        message.push(0);
        message.extend_from_slice(occurrence.as_str().as_bytes());
        Ok(format!(
            "{HANDLE_PREFIX}{}",
            to_hex(&hmac_sha256(&self.secret, HANDLE_DOMAIN, &message))
        ))
    }

    /// Builds the safe public summary for one checkpoint occurrence.
    pub fn project(
        &self,
        checkpoint: &ExactCheckpointId,
        state: &ExactStateDigest,
        occurrence: &OccurrenceId,
        boundary_kind: &str,
        boundary_phase: &str,
        restore_verified: bool,
    ) -> Result<PublicCheckpointSummary, ProjectionError> {
        if !valid_label(boundary_kind) || !valid_label(boundary_phase) {
            return Err(ProjectionError::InvalidInput);
        }
        Ok(PublicCheckpointSummary {
            handle: self.handle(checkpoint, state, occurrence)?,
            occurrence: occurrence.as_str().to_owned(),
            boundary_kind: boundary_kind.to_owned(),
            boundary_phase: boundary_phase.to_owned(),
            restore_verified,
        })
    }

    /// Reports whether a public handle was issued by this key for this occurrence.
    pub fn matches(
        &self,
        handle: &str,
        checkpoint: &ExactCheckpointId,
        state: &ExactStateDigest,
        occurrence: &OccurrenceId,
    ) -> Result<bool, ProjectionError> {
        let hex = handle
            .strip_prefix(HANDLE_PREFIX)
            .ok_or(ProjectionError::InvalidHandle)?;
        let lowercase = hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if hex.len() != 64 || !lowercase {
            return Err(ProjectionError::InvalidHandle);
        }
        Ok(self.handle(checkpoint, state, occurrence)? == handle)
    }
}

impl fmt::Debug for ProjectionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProjectionKey(<redacted>)")
    }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.contains('\0')
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn hmac_sha256(key: &[u8], domain: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut key_block = [0_u8; BLOCK];
    if key.len() > BLOCK {
        key_block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    let mut outer_pad = [0_u8; BLOCK];
    for index in 0..BLOCK {
        inner.update([key_block[index] ^ 0x36]);
        outer_pad[index] = key_block[index] ^ 0x5c;
    }
    inner.update(domain);
    inner.update(message);
    let inner_digest = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner_digest);
    outer.finalize().into()
}
