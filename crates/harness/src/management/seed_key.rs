// SPDX-License-Identifier: MIT

//! Management-owned, versioned HMAC keys for authored seed derivation.
//!
//! Key handles keep key bytes private and intentionally do not implement
//! `Clone`, `Debug`, or serialization. The HMAC implementation and its feature
//! graph do not establish erasure of every internal key-derived temporary, so
//! this module makes no such guarantee.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

#[path = "seed_key_file.rs"]
mod file;

pub use file::FileSeedDerivationKeyAuthority;

#[cfg(test)]
#[path = "seed_key_tests.rs"]
mod tests;

const KEYRING_SCHEMA_VERSION: &str = "ascension.seed-keyring/v1";
const KEY_COMMITMENT_DOMAIN: &str = "ascension.seed-key-commitment/v1";
const MAX_KEY_ID_BYTES: usize = 64;
const MAX_KEYRING_BYTES: usize = 64 * 1024;
const MAX_KEY_VERSIONS: usize = 64;
const SHA256_BYTES: usize = 32;
type HmacSha256 = Hmac<Sha256>;

/// Public, non-secret identity of one immutable key version.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SeedKeyIdentity {
    pub authority_id: String,
    pub version: String,
    /// Full lowercase hex HMAC commitment; never the key itself.
    pub commitment: String,
}

impl SeedKeyIdentity {
    fn new(
        authority_id: String,
        version: String,
        key: &[u8; SHA256_BYTES],
    ) -> Result<Self, SeedKeyError> {
        let message = framed(&[
            KEY_COMMITMENT_DOMAIN.as_bytes(),
            b"hmac-sha256-v1",
            authority_id.as_bytes(),
            version.as_bytes(),
        ])?;
        let commitment = hmac_sha256(key, &message)?;
        Ok(Self {
            authority_id,
            version,
            commitment: hex_lower(&commitment),
        })
    }

    /// Check bounded public identity fields without exposing secret material.
    pub fn validate(&self) -> Result<(), SeedKeyError> {
        validate_key_id(&self.authority_id)?;
        validate_key_id(&self.version)?;
        if decode_lower_hex::<SHA256_BYTES>(&self.commitment).is_none() {
            return Err(SeedKeyError::InvalidIdentity);
        }
        Ok(())
    }
}

struct SecretKey(Zeroizing<[u8; SHA256_BYTES]>);

/// A scoped handle to one immutable versioned key. The key bytes are never
/// returned; callers can only ask this handle to authenticate a bounded input.
pub struct SeedKeyHandle {
    identity: SeedKeyIdentity,
    key: Arc<SecretKey>,
}

impl SeedKeyHandle {
    fn new(identity: SeedKeyIdentity, key: Arc<SecretKey>) -> Self {
        Self { identity, key }
    }

    #[must_use]
    pub fn identity(&self) -> &SeedKeyIdentity {
        &self.identity
    }

    /// Return a full HMAC-SHA-256 output for a caller-framed, bounded message.
    ///
    /// The output is caller-owned derived material; key bytes remain private.
    pub(crate) fn authenticate(&self, message: &[u8]) -> Result<[u8; SHA256_BYTES], SeedKeyError> {
        hmac_sha256(&self.key.0[..], message)
    }

    /// Recompute a stored commitment with the pinned material and compare it
    /// through the HMAC implementation's constant-time verification operation.
    pub fn verifies_identity(&self, expected: &SeedKeyIdentity) -> Result<bool, SeedKeyError> {
        if self.identity.authority_id != expected.authority_id
            || self.identity.version != expected.version
        {
            return Ok(false);
        }
        let Some(expected_bytes) = decode_lower_hex::<SHA256_BYTES>(&expected.commitment) else {
            return Ok(false);
        };
        let message = framed(&[
            KEY_COMMITMENT_DOMAIN.as_bytes(),
            b"hmac-sha256-v1",
            expected.authority_id.as_bytes(),
            expected.version.as_bytes(),
        ])?;
        let mut mac = HmacSha256::new_from_slice(&self.key.0[..])
            .map_err(|_| SeedKeyError::InvalidKeyMaterial)?;
        mac.update(&message);
        Ok(mac.verify_slice(&expected_bytes).is_ok())
    }
}

/// Provides current material only for new operations and exact version lookups
/// for operations already pinned in durable storage.
pub trait SeedDerivationKeyAuthority: Send + Sync {
    /// Select the authority's startup-pinned current key for a new operation.
    fn current_key(&self) -> Result<SeedKeyHandle, SeedKeyError>;

    /// Load one exact historical key version. This must not silently substitute
    /// the current key when the requested identity is absent.
    fn key_for(
        &self,
        authority_id: &str,
        version: &str,
    ) -> Result<Option<SeedKeyHandle>, SeedKeyError>;
}

/// Bounded, non-reflecting key-authority failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SeedKeyError {
    Unavailable,
    UnsupportedPlatform,
    InvalidKeyring,
    InsecureKeyring,
    KeyVersionUnavailable,
    InvalidIdentity,
    InvalidKeyMaterial,
    MessageTooLarge,
}

impl fmt::Display for SeedKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "seed derivation key authority is unavailable",
            Self::UnsupportedPlatform => "seed derivation key file is unsupported on this platform",
            Self::InvalidKeyring => "seed derivation key file is invalid",
            Self::InsecureKeyring => "seed derivation key file permissions or type are unsafe",
            Self::KeyVersionUnavailable => "pinned seed derivation key version is unavailable",
            Self::InvalidIdentity => "seed derivation key identity is invalid",
            Self::InvalidKeyMaterial => "seed derivation key material is invalid",
            Self::MessageTooLarge => "seed derivation message exceeds its supported bound",
        })
    }
}

impl std::error::Error for SeedKeyError {}

pub(super) struct Keyring {
    authority_id: String,
    current_version: String,
    versions: BTreeMap<String, Arc<SecretKey>>,
}

impl Keyring {
    pub(super) fn parse(bytes: &[u8]) -> Result<Self, SeedKeyError> {
        if bytes.is_empty()
            || bytes.len() > MAX_KEYRING_BYTES
            || !bytes.is_ascii()
            || bytes.contains(&b'\r')
        {
            return Err(SeedKeyError::InvalidKeyring);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| SeedKeyError::InvalidKeyring)?;
        let mut lines = text.lines();
        let expected_schema = format!("schema={KEYRING_SCHEMA_VERSION}");
        if lines.next() != Some(expected_schema.as_str()) {
            return Err(SeedKeyError::InvalidKeyring);
        }
        let mut authority_id = None;
        let mut current_version = None;
        let mut versions = BTreeMap::new();
        for line in lines {
            let (name, value) = line.split_once('=').ok_or(SeedKeyError::InvalidKeyring)?;
            if value.is_empty() || value.len() > MAX_KEYRING_BYTES {
                return Err(SeedKeyError::InvalidKeyring);
            }
            match name {
                "authority_id" if authority_id.is_none() => {
                    validate_key_id(value)?;
                    authority_id = Some(value.to_owned());
                }
                "current_version" if current_version.is_none() => {
                    validate_key_id(value)?;
                    current_version = Some(value.to_owned());
                }
                _ if name.starts_with("key.") => {
                    let version = &name[4..];
                    validate_key_id(version)?;
                    if versions.len() >= MAX_KEY_VERSIONS || versions.contains_key(version) {
                        return Err(SeedKeyError::InvalidKeyring);
                    }
                    let mut key = Zeroizing::new([0_u8; SHA256_BYTES]);
                    if !decode_lower_hex_into(value, &mut key) {
                        return Err(SeedKeyError::InvalidKeyring);
                    }
                    versions.insert(version.to_owned(), Arc::new(SecretKey(key)));
                }
                _ => return Err(SeedKeyError::InvalidKeyring),
            }
        }
        let authority_id = authority_id.ok_or(SeedKeyError::InvalidKeyring)?;
        let current_version = current_version.ok_or(SeedKeyError::InvalidKeyring)?;
        if !versions.contains_key(&current_version) {
            return Err(SeedKeyError::InvalidKeyring);
        }
        Ok(Self {
            authority_id,
            current_version,
            versions,
        })
    }

    fn handle(&self, version: &str) -> Result<SeedKeyHandle, SeedKeyError> {
        let key = Arc::clone(
            self.versions
                .get(version)
                .ok_or(SeedKeyError::KeyVersionUnavailable)?,
        );
        let identity = SeedKeyIdentity::new(self.authority_id.clone(), version.to_owned(), &key.0)?;
        Ok(SeedKeyHandle::new(identity, key))
    }
}

impl SeedDerivationKeyAuthority for Keyring {
    fn current_key(&self) -> Result<SeedKeyHandle, SeedKeyError> {
        self.handle(&self.current_version)
    }

    fn key_for(
        &self,
        authority_id: &str,
        version: &str,
    ) -> Result<Option<SeedKeyHandle>, SeedKeyError> {
        if self.authority_id != authority_id || !self.versions.contains_key(version) {
            return Ok(None);
        }
        self.handle(version).map(Some)
    }
}

pub(super) fn framed(parts: &[&[u8]]) -> Result<Vec<u8>, SeedKeyError> {
    let total = parts.iter().try_fold(0_usize, |length, part| {
        let field_length = u32::try_from(part.len()).ok()? as usize;
        length.checked_add(4)?.checked_add(field_length)
    });
    let Some(total) = total.filter(|length| *length <= MAX_KEYRING_BYTES) else {
        return Err(SeedKeyError::MessageTooLarge);
    };
    let mut framed = Vec::new();
    framed
        .try_reserve_exact(total)
        .map_err(|_| SeedKeyError::Unavailable)?;
    for part in parts {
        let field_length = u32::try_from(part.len()).map_err(|_| SeedKeyError::MessageTooLarge)?;
        framed.extend_from_slice(&field_length.to_be_bytes());
        framed.extend_from_slice(part);
    }
    Ok(framed)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> Result<[u8; SHA256_BYTES], SeedKeyError> {
    if key.is_empty() {
        return Err(SeedKeyError::InvalidKeyMaterial);
    }
    if message.len() > MAX_KEYRING_BYTES {
        return Err(SeedKeyError::MessageTooLarge);
    }
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| SeedKeyError::InvalidKeyMaterial)?;
    mac.update(message);
    let output = mac.finalize().into_bytes();
    let mut bytes = [0_u8; SHA256_BYTES];
    bytes.copy_from_slice(&output);
    Ok(bytes)
}

pub(super) fn derive_seed(
    key: &SeedKeyHandle,
    framed_message: &[u8],
) -> Result<String, SeedKeyError> {
    let output = Zeroizing::new(key.authenticate(framed_message)?);
    Ok(hex_lower(&output[..16]))
}

#[path = "seed_key_encoding.rs"]
mod encoding;
pub(super) use encoding::hex_lower;
use encoding::{decode_lower_hex, decode_lower_hex_into, validate_key_id};
