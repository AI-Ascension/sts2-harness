// SPDX-License-Identifier: MIT

#![cfg(target_os = "linux")]

use super::worker_local_linux_image::HeldImage;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::time::Instant;

static NEXT_IMAGE_FIXTURE: AtomicU64 = AtomicU64::new(1);

struct ChildGuard(std::process::Child);

impl ChildGuard {
    fn new(child: std::process::Child) -> Self {
        Self(child)
    }

    fn id(&self) -> u32 {
        self.0.id()
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn image_fixture_root() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let nonce = NEXT_IMAGE_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "ascension-harness-linux-image-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&root)?;
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    Ok(root)
}

fn image_digest(path: &Path) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = [0_u8; 16 * 1024];
    loop {
        let count = file.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        hasher.update(&chunk[..count]);
    }
    Ok(hasher.finalize().into())
}

#[test]
fn owner_writable_image_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let root = image_fixture_root()?;
    let path = root.join("image");
    let mut file = File::create(&path)?;
    file.write_all(b"synthetic release image")?;
    file.sync_all()?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    let digest = image_digest(&path)?;
    assert!(matches!(
        HeldImage::open(&path, rustix::process::getuid().as_raw(), &digest),
        Err(super::LinuxTransportError::Peer)
    ));
    fs::remove_dir_all(root)?;
    Ok(())
}

#[test]
fn cross_process_magic_link_checks_path_and_identity() -> Result<(), Box<dyn std::error::Error>> {
    let root = image_fixture_root()?;
    let source = fs::canonicalize("/bin/sh")?;
    let approved_path = root.join("approved-image");
    let live_path = root.join("live-image");
    fs::copy(&source, &approved_path)?;
    fs::copy(&source, &live_path)?;
    fs::set_permissions(&approved_path, fs::Permissions::from_mode(0o555))?;
    fs::set_permissions(&live_path, fs::Permissions::from_mode(0o555))?;
    let digest = image_digest(&approved_path)?;
    let owner_uid = rustix::process::getuid().as_raw();
    let approved = HeldImage::open(&approved_path, owner_uid, &digest)
        .map_err(|error| io::Error::other(format!("approved fixture image rejected: {error}")))?;
    let child = ChildGuard::new(
        Command::new(&live_path)
            .args(["-c", "read value"])
            .stdin(Stdio::piped())
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    let wrong_path = HeldImage::open_proc(child.id(), &approved_path, owner_uid, &digest, deadline);
    assert!(matches!(wrong_path, Err(super::LinuxTransportError::Peer)));
    let live = HeldImage::open_proc(child.id(), &live_path, owner_uid, &digest, deadline)
        .map_err(|error| live_image_diagnostic(error, child.id(), &live_path, owner_uid))?;
    assert!(live.identity() != approved.identity());
    drop(live);
    drop(approved);
    fs::remove_dir_all(root)?;
    Ok(())
}

// Report bounded structural facts, not private paths or environment contents,
// when a hosted kernel rejects an otherwise locally passing image fixture.
fn live_image_diagnostic(
    error: super::LinuxTransportError,
    pid: u32,
    expected_path: &Path,
    expected_uid: u32,
) -> io::Error {
    let proc_path = PathBuf::from(format!("/proc/{pid}/exe"));
    let path_matches = fs::read_link(&proc_path).map(|path| path == expected_path);
    let identity = fs::metadata(&proc_path).map(|metadata| {
        (
            metadata.uid(),
            metadata.mode(),
            metadata.nlink(),
            metadata.len(),
        )
    });
    io::Error::other(format!(
        "live fixture image rejected: {error}; path_matches={path_matches:?}; expected_uid={expected_uid}; identity={identity:?}"
    ))
}
