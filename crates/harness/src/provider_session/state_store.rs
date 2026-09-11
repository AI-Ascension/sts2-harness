// SPDX-License-Identifier: MIT

//! Explicit storage boundary for the broker-owned provider-session metadata journal.
//!
//! This adapter never claims to encrypt or contain a native provider runtime. It protects only the
//! bounded `BrokerSnapshot` journal owned by this crate. Native persistent operation must still
//! supply an independently verified encrypted state boundary before an enabled profile is allowed.

use super::{
    MAX_HISTORY_BYTES, NativeCapabilities, ProviderSessionBroker, ProviderSessionPolicy,
    SessionError, SessionScope,
};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use getrandom::fill as fill_random;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroizing;

const STORE_MAGIC: &[u8] = b"ASCENSION-PROVIDER-METADATA-ENC1\0";
const NONCE_BYTES: usize = 24;
const TAG_BYTES: usize = 16;
const MAX_ENVELOPE_BYTES: usize = STORE_MAGIC.len() + NONCE_BYTES + TAG_BYTES + MAX_HISTORY_BYTES;
const AAD_PREFIX: &[u8] = b"ascension.provider-session.metadata.v1\0";
static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderSessionMetadataMode {
    Volatile,
    EncryptedPersistent,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderSessionMetadataStoreError {
    InvalidScope,
    InvalidKey,
    InvalidPath,
    ScopeMismatch,
    Capacity,
    Corrupt,
    Crypto,
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
            Self::Capacity => formatter.write_str("provider metadata store is over its bound"),
            Self::Corrupt => formatter.write_str("provider metadata store envelope is corrupt"),
            Self::Crypto => formatter.write_str("provider metadata store authentication failed"),
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

    fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
        let key = self
            .key
            .as_ref()
            .ok_or(ProviderSessionMetadataStoreError::Unsupported)?;
        let mut nonce = [0_u8; NONCE_BYTES];
        fill_random(&mut nonce).map_err(|_| ProviderSessionMetadataStoreError::Crypto)?;
        let cipher = XChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
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
        let cipher = XChaCha20Poly1305::new(Key::from_slice(key.as_ref()));
        cipher
            .decrypt(
                XNonce::from_slice(&envelope[nonce_start..nonce_end]),
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

#[cfg(unix)]
fn read_restricted_file(path: &Path) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
    use rustix::fs::{Mode, OFlags, open};

    let descriptor = open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|_| ProviderSessionMetadataStoreError::InvalidPath)?;
    let mut file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    if !metadata.file_type().is_file() || !restricted_file(&metadata) {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_restricted_file(path: &Path) -> Result<Vec<u8>, ProviderSessionMetadataStoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    if !metadata.file_type().is_file() {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    fs::read(path).map_err(|_| ProviderSessionMetadataStoreError::Io)
}

fn validate_store_path(path: &Path) -> Result<(), ProviderSessionMetadataStoreError> {
    if !path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let Some(parent) = path.parent() else {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    };
    if !safe_directory(parent) {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.file_type().is_file() || !restricted_file(&metadata))
    {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    Ok(())
}

fn atomic_write(path: &Path, envelope: &[u8]) -> Result<(), ProviderSessionMetadataStoreError> {
    let parent = path
        .parent()
        .ok_or(ProviderSessionMetadataStoreError::InvalidPath)?;
    if !safe_directory(parent) || envelope.len() > MAX_ENVELOPE_BYTES {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(ProviderSessionMetadataStoreError::InvalidPath)?;
    let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(".{file_name}.tmp-{}-{counter}", std::process::id()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    set_private_file_mode(&file).map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    file.write_all(envelope)
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    file.sync_all()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    drop(file);
    if fs::symlink_metadata(path)
        .is_ok_and(|metadata| !metadata.file_type().is_file() || !restricted_file(&metadata))
    {
        let _ = fs::remove_file(&temporary);
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    fs::rename(&temporary, path).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        ProviderSessionMetadataStoreError::Io
    })?;
    Ok(())
}

fn set_private_file_mode(file: &File) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn safe_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir() && !has_symlink_component(path) && restricted_directory(&metadata)
    })
}

#[cfg(unix)]
fn restricted_file(metadata: &fs::Metadata) -> bool {
    use rustix::process::geteuid;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == geteuid().as_raw() && metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn restricted_file(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn restricted_directory(metadata: &fs::Metadata) -> bool {
    use rustix::process::geteuid;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == geteuid().as_raw() && metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn restricted_directory(_metadata: &fs::Metadata) -> bool {
    false
}

fn has_symlink_component(path: &Path) -> bool {
    let mut current = PathBuf::new();
    path.components().any(|component| {
        current.push(component.as_os_str());
        fs::symlink_metadata(&current).is_ok_and(|metadata| metadata.file_type().is_symlink())
    })
}
