// SPDX-License-Identifier: MIT

use super::lease::Lease;
use super::types::{JournalSnapshot, MAGIC, MAX_ENVELOPE};
use crate::exo_lifecycle::{JournalConfig, LifecycleError};
use crate::provider_session::{MAX_HISTORY_BYTES, MAX_JSON_DEPTH};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::io::{Read, Write};

fn aad(config: &JournalConfig) -> Vec<u8> {
    let mut aad = b"ascension.provider-session.owner-journal.v2\0".to_vec();
    for value in [
        super::super::digest_scope(&config.scope),
        config.store_id.clone(),
    ] {
        aad.extend_from_slice(&(value.len() as u64).to_be_bytes());
        aad.extend_from_slice(value.as_bytes());
    }
    aad
}

#[cfg(all(test, unix))]
pub(super) fn seal_test_plaintext(
    config: &JournalConfig,
    key: &[u8; 32],
    plain: &[u8],
) -> Result<Vec<u8>, LifecycleError> {
    let nonce = [1; 24];
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let encrypted = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: plain,
                aad: &aad(config),
            },
        )
        .map_err(|_| LifecycleError::Corrupt)?;
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&encrypted);
    Ok(bytes)
}

pub fn encode(
    config: &JournalConfig,
    key: &[u8; 32],
    snapshot: &JournalSnapshot,
) -> Result<Vec<u8>, LifecycleError> {
    snapshot.validate(config)?;
    let plain = serde_json::to_vec(snapshot).map_err(|_| LifecycleError::Invalid)?;
    if plain.len() > MAX_HISTORY_BYTES {
        return Err(LifecycleError::Capacity);
    }
    let mut nonce = [0; 24];
    getrandom::fill(&mut nonce).map_err(|_| LifecycleError::Unavailable)?;
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let encrypted = cipher
        .encrypt(
            (&nonce).into(),
            Payload {
                msg: &plain,
                aad: &aad(config),
            },
        )
        .map_err(|_| LifecycleError::Corrupt)?;
    let mut bytes = MAGIC.to_vec();
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&encrypted);
    Ok(bytes)
}

pub fn decode(
    config: &JournalConfig,
    key: &[u8; 32],
    bytes: &[u8],
) -> Result<JournalSnapshot, LifecycleError> {
    if bytes.len() < MAGIC.len() + 40 || bytes.len() > MAX_ENVELOPE || !bytes.starts_with(MAGIC) {
        return Err(LifecycleError::Corrupt);
    }
    let end_nonce = MAGIC.len() + 24;
    let nonce: &XNonce = (&bytes[MAGIC.len()..end_nonce])
        .try_into()
        .map_err(|_| LifecycleError::Corrupt)?;
    let cipher = XChaCha20Poly1305::new(&Key::from(*key));
    let plain = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &bytes[end_nonce..],
                aad: &aad(config),
            },
        )
        .map_err(|_| LifecycleError::Corrupt)?;
    let snapshot: JournalSnapshot = super::super::protocol::parse_strict_json_bounded(
        &plain,
        MAX_HISTORY_BYTES,
        MAX_JSON_DEPTH,
    )
    .map_err(|_| LifecycleError::Corrupt)?;
    snapshot.validate(config)?;
    Ok(snapshot)
}

#[cfg(unix)]
pub fn read(lease: &Lease) -> Result<Vec<u8>, LifecycleError> {
    use rustix::fs::{Mode, OFlags, openat};
    let file = File::from(
        openat(
            &lease.directory,
            "journal.enc",
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| LifecycleError::Corrupt)?,
    );
    let metadata = file.metadata().map_err(|_| LifecycleError::Io)?;
    if !super::lease::private(&metadata, false) || metadata.len() > MAX_ENVELOPE as u64 {
        return Err(LifecycleError::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(MAX_ENVELOPE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| LifecycleError::Io)?;
    if bytes.len() > MAX_ENVELOPE {
        return Err(LifecycleError::Capacity);
    }
    Ok(bytes)
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CommitStage {
    BeforeWrite,
    AfterPartialWrite,
    AfterWrite,
    AfterFileSync,
    AfterRename,
    AfterDirectorySync,
}

#[cfg(unix)]
pub fn write(lease: &Lease, bytes: &[u8]) -> Result<(), LifecycleError> {
    use rustix::fs::{AtFlags, Mode, OFlags, openat, renameat, unlinkat};
    if bytes.len() > MAX_ENVELOPE {
        return Err(LifecycleError::Capacity);
    }
    let mut random = [0; 16];
    getrandom::fill(&mut random).map_err(|_| LifecycleError::Unavailable)?;
    let name = format!(".journal-{}.tmp", crate::sha256_hex(random));
    let mut file = File::from(
        openat(
            &lease.directory,
            name.as_str(),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| LifecycleError::Io)?,
    );
    let result = (|| {
        checkpoint(CommitStage::BeforeWrite)?;
        let middle = bytes.len() / 2;
        file.write_all(&bytes[..middle])
            .map_err(|_| LifecycleError::Io)?;
        checkpoint(CommitStage::AfterPartialWrite)?;
        file.write_all(&bytes[middle..])
            .map_err(|_| LifecycleError::Io)?;
        checkpoint(CommitStage::AfterWrite)?;
        file.sync_all().map_err(|_| LifecycleError::Io)?;
        checkpoint(CommitStage::AfterFileSync)?;
        renameat(
            &lease.directory,
            name.as_str(),
            &lease.directory,
            "journal.enc",
        )
        .map_err(|_| LifecycleError::Io)?;
        checkpoint(CommitStage::AfterRename)?;
        lease.directory.sync_all().map_err(|_| LifecycleError::Io)?;
        checkpoint(CommitStage::AfterDirectorySync)
    })();
    if result.is_err() {
        let _ = unlinkat(&lease.directory, name.as_str(), AtFlags::empty());
    }
    result
}

#[cfg(all(test, unix))]
thread_local! { pub(crate) static FAILURE: std::cell::Cell<Option<CommitStage>> = const {
    std::cell::Cell::new(None)
}; }

#[cfg(all(test, unix))]
thread_local! { pub(crate) static FAILURE_SKIP: std::cell::Cell<usize> = const {
    std::cell::Cell::new(0)
}; }

#[cfg(all(test, unix))]
thread_local! { static TERMINAL_PROCESS_BARRIER: std::cell::RefCell<Option<std::path::PathBuf>> = const {
    std::cell::RefCell::new(None)
}; }

#[cfg(all(test, unix))]
pub(super) fn inject_terminal_process_barrier(marker: std::path::PathBuf) {
    TERMINAL_PROCESS_BARRIER.with(|barrier| *barrier.borrow_mut() = Some(marker));
}

#[cfg(all(test, unix))]
pub(super) fn terminal_process_barrier() -> Result<(), LifecycleError> {
    let marker = TERMINAL_PROCESS_BARRIER.with(|barrier| barrier.borrow_mut().take());
    if let Some(marker) = marker {
        signal_parent(&marker)?;
        wait_for_parent_kill()?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
fn signal_parent(marker: &std::path::Path) -> Result<(), LifecycleError> {
    use std::io::Write;
    let pending = marker.with_extension("pending");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)
        .map_err(|_| LifecycleError::Io)?;
    file.write_all(b"terminal journal commit reached\n")
        .map_err(|_| LifecycleError::Io)?;
    file.sync_all().map_err(|_| LifecycleError::Io)?;
    std::fs::rename(&pending, marker).map_err(|_| LifecycleError::Io)?;
    std::fs::File::open(marker.parent().ok_or(LifecycleError::Invalid)?)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| LifecycleError::Io)
}

#[cfg(all(test, unix))]
fn wait_for_parent_kill() -> Result<(), LifecycleError> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(LifecycleError::Io)
}

#[cfg(unix)]
fn checkpoint(_stage: CommitStage) -> Result<(), LifecycleError> {
    #[cfg(test)]
    if FAILURE.get() == Some(_stage) {
        if FAILURE_SKIP.get() > 0 {
            FAILURE_SKIP.set(FAILURE_SKIP.get() - 1);
            return Ok(());
        }
        return Err(LifecycleError::Io);
    }
    Ok(())
}

#[cfg(not(unix))]
pub fn read(_: &Lease) -> Result<Vec<u8>, LifecycleError> {
    Err(LifecycleError::Unsupported)
}
#[cfg(not(unix))]
pub fn write(_: &Lease, _: &[u8]) -> Result<(), LifecycleError> {
    Err(LifecycleError::Unsupported)
}
