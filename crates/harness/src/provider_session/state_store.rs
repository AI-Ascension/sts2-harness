// SPDX-License-Identifier: MIT

//! Explicit storage boundary for the broker-owned provider-session metadata journal.
//!
//! This adapter never claims to encrypt or contain a native provider runtime. It protects only the
//! bounded `BrokerSnapshot` journal owned by this crate. Native persistent operation must still
//! supply an independently verified encrypted state boundary before an enabled profile is allowed.

mod atomic_replace;
mod owner_lease;
mod state_store_io;
mod store_error;
use atomic_replace::atomic_write;
pub(crate) use owner_lease::PolicyOwnerLease;
use state_store_io::{read_restricted_file, validate_store_path};
pub(crate) use state_store_io::{restricted_directory, restricted_file};
pub use store_error::ProviderSessionMetadataStoreError;

use super::{
    MAX_HISTORY_BYTES, NativeCapabilities, ProviderSessionBroker, ProviderSessionPolicy,
    SessionScope,
};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use getrandom::fill as fill_random;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

const STORE_MAGIC: &[u8] = b"ASCENSION-PROVIDER-METADATA-ENC1\0";
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const MAX_ENVELOPE_BYTES: usize = STORE_MAGIC.len() + NONCE_BYTES + TAG_BYTES + MAX_HISTORY_BYTES;
const AAD_PREFIX: &[u8] = b"ascension.provider-session.metadata.v1\0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderSessionMetadataMode {
    Volatile,
    EncryptedPersistent,
}

pub struct ProviderSessionMetadataStore {
    scope: SessionScope,
    mode: ProviderSessionMetadataMode,
    path: Option<PathBuf>,
    key: Option<Zeroizing<[u8; 32]>>,
}

impl std::fmt::Debug for ProviderSessionMetadataStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProviderSessionMetadataStore")
            .field("scope", &self.scope)
            .field("mode", &self.mode)
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl ProviderSessionMetadataStore {
    pub fn volatile(scope: SessionScope) -> Result<Self, ProviderSessionMetadataStoreError> {
        if !scope.valid() {
            return Err(ProviderSessionMetadataStoreError::InvalidScope);
        }
        Ok(Self {
            scope,
            mode: ProviderSessionMetadataMode::Volatile,
            path: None,
            key: None,
        })
    }

    pub fn encrypted(
        path: impl AsRef<Path>,
        key: [u8; 32],
        scope: SessionScope,
    ) -> Result<Self, ProviderSessionMetadataStoreError> {
        let path = path.as_ref().to_owned();
        if !scope.valid() {
            return Err(ProviderSessionMetadataStoreError::InvalidScope);
        }
        if key.iter().all(|byte| *byte == 0) {
            return Err(ProviderSessionMetadataStoreError::InvalidKey);
        }
        validate_store_path(&path)?;
        Ok(Self {
            scope,
            mode: ProviderSessionMetadataMode::EncryptedPersistent,
            path: Some(path),
            key: Some(Zeroizing::new(key)),
        })
    }

    #[must_use]
    pub fn mode(&self) -> ProviderSessionMetadataMode {
        self.mode
    }

    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Persists only the bounded broker metadata snapshot. Volatile mode intentionally performs no
    /// filesystem operation and therefore cannot be rehydrated after process exit.
    pub fn save(
        &self,
        broker: &ProviderSessionBroker,
    ) -> Result<(), ProviderSessionMetadataStoreError> {
        if broker.scope() != &self.scope {
            return Err(ProviderSessionMetadataStoreError::ScopeMismatch);
        }
        if self.mode == ProviderSessionMetadataMode::Volatile {
            return Ok(());
        }
        let path = self
            .path
            .as_deref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        validate_store_path(path)?;
        let plaintext = broker.snapshot_json()?;
        if plaintext.len() > MAX_HISTORY_BYTES {
            return Err(ProviderSessionMetadataStoreError::Capacity);
        }
        let envelope = self.encrypt(&plaintext)?;
        atomic_write(path, &envelope)
    }

    /// Loads an encrypted metadata snapshot only after matching the currently approved profile.
    pub fn load(
        &self,
        owner_token: impl Into<String>,
        expected_policy: &ProviderSessionPolicy,
        expected_capabilities: &NativeCapabilities,
    ) -> Result<ProviderSessionBroker, ProviderSessionMetadataStoreError> {
        if self.mode == ProviderSessionMetadataMode::Volatile {
            return Err(ProviderSessionMetadataStoreError::Unsupported);
        }
        let path = self
            .path
            .as_deref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        validate_store_path(path)?;
        // Read through a no-follow descriptor and re-check the resulting file metadata on that
        // descriptor, so a symlink or permission swap between path validation and read cannot
        // redirect the load to attacker-selected bytes.
        let envelope = read_restricted_file(path)?;
        if envelope.len() > MAX_ENVELOPE_BYTES {
            return Err(ProviderSessionMetadataStoreError::Capacity);
        }
        let plaintext = self.decrypt(&envelope)?;
        ProviderSessionBroker::from_snapshot_json_checked(
            &plaintext,
            owner_token,
            &self.scope,
            expected_policy,
            expected_capabilities,
        )
        .map_err(ProviderSessionMetadataStoreError::from)
    }

    /// Imports an already bounded, descriptor-read envelope without reopening its pathname.
    pub(crate) fn load_envelope(
        &self,
        envelope: &[u8],
        owner_token: impl Into<String>,
        expected_policy: &ProviderSessionPolicy,
        expected_capabilities: &NativeCapabilities,
    ) -> Result<ProviderSessionBroker, ProviderSessionMetadataStoreError> {
        if envelope.len() > MAX_ENVELOPE_BYTES {
            return Err(ProviderSessionMetadataStoreError::Capacity);
        }
        let plaintext = self.decrypt(envelope)?;
        ProviderSessionBroker::from_snapshot_json_checked(
            &plaintext,
            owner_token,
            &self.scope,
            expected_policy,
            expected_capabilities,
        )
        .map_err(ProviderSessionMetadataStoreError::from)
    }

    /// Persists a bounded policy-owner journal with the same encrypted,
    /// scope-bound envelope used for broker metadata. The caller owns the
    /// journal schema; this storage boundary supplies no policy semantics.
    pub(crate) fn save_owner_journal(
        &self,
        bytes: &[u8],
    ) -> Result<(), ProviderSessionMetadataStoreError> {
        if self.mode == ProviderSessionMetadataMode::Volatile {
            return Err(ProviderSessionMetadataStoreError::Unsupported);
        }
        if bytes.len() > MAX_HISTORY_BYTES {
            return Err(ProviderSessionMetadataStoreError::Capacity);
        }
        let path = self
            .path
            .as_deref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        validate_store_path(path)?;
        atomic_write(path, &self.encrypt(bytes)?)
    }

    pub(crate) fn load_owner_journal(&self) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
        if self.mode == ProviderSessionMetadataMode::Volatile {
            return Err(ProviderSessionMetadataStoreError::Unsupported);
        }
        let path = self
            .path
            .as_deref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        validate_store_path(path)?;
        let envelope = read_restricted_file(path)?;
        let bytes = self.decrypt(&envelope)?;
        if bytes.len() > MAX_HISTORY_BYTES {
            return Err(ProviderSessionMetadataStoreError::Capacity);
        }
        Ok(bytes)
    }

    pub(crate) fn acquire_owner_journal_lease(
        &self,
    ) -> Result<PolicyOwnerLease, ProviderSessionMetadataStoreError> {
        if self.mode == ProviderSessionMetadataMode::Volatile {
            return Err(ProviderSessionMetadataStoreError::Unsupported);
        }
        let path = self
            .path
            .as_deref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        validate_store_path(path)?;
        PolicyOwnerLease::acquire(path)
    }
    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
        let key = self
            .key
            .as_ref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        let mut nonce = [0_u8; NONCE_BYTES];
        fill_random(&mut nonce).map_err(|_| ProviderSessionMetadataStoreError::Crypto)?;
        let cipher = XChaCha20Poly1305::new(&Key::from(**key));
        let ciphertext = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plaintext,
                    aad: &self.aad(),
                },
            )
            .map_err(|_| ProviderSessionMetadataStoreError::Crypto)?;
        let mut envelope = Vec::with_capacity(STORE_MAGIC.len() + nonce.len() + ciphertext.len());
        envelope.extend_from_slice(STORE_MAGIC);
        envelope.extend_from_slice(&nonce);
        envelope.extend_from_slice(&ciphertext);
        Ok(envelope)
    }

    fn decrypt(&self, envelope: &[u8]) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
        if envelope.len() < STORE_MAGIC.len() + NONCE_BYTES + TAG_BYTES
            || !envelope.starts_with(STORE_MAGIC)
        {
            return Err(ProviderSessionMetadataStoreError::Corrupt);
        }
        let key = self
            .key
            .as_ref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        let nonce_start = STORE_MAGIC.len();
        let nonce_end = nonce_start + NONCE_BYTES;
        let nonce: &XNonce = (&envelope[nonce_start..nonce_end])
            .try_into()
            .map_err(|_| ProviderSessionMetadataStoreError::Corrupt)?;
        let cipher = XChaCha20Poly1305::new(&Key::from(**key));
        cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &envelope[nonce_end..],
                    aad: &self.aad(),
                },
            )
            .map_err(|_| ProviderSessionMetadataStoreError::Crypto)
    }

    fn aad(&self) -> Vec<u8> {
        let mut aad = Vec::with_capacity(AAD_PREFIX.len() + 64);
        aad.extend_from_slice(AAD_PREFIX);
        aad.extend_from_slice(super::digest_scope(&self.scope).as_bytes());
        aad
    }
}
