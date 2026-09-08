// SPDX-License-Identifier: MIT

//! Linux executable image proof held by the worker transport.

#![cfg(target_os = "linux")]

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use rustix::fs::{FileType, Mode, OFlags, fstat, openat};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::time::Instant;

use super::LinuxTransportError;
use super::worker_local_linux_fs::{
    FileIdentity, HeldDirectories, MAX_IMAGE_BYTES, compare_path_identity, ensure_deadline,
    open_held_file, owner_allowed,
};

const READ_CHUNK_BYTES: usize = 16 * 1024;

pub(super) struct HeldImage {
    _directories: Option<HeldDirectories>,
    _file: File,
    identity: FileIdentity,
}

impl HeldImage {
    pub(super) fn open(
        path: &Path,
        owner_uid: u32,
        expected_digest: &[u8; 32],
    ) -> Result<Self, LinuxTransportError> {
        let (directories, mut file, identity, stat) =
            open_held_file(path, owner_uid, LinuxTransportError::Peer)?;
        validate_image_stat(&stat, owner_uid)?;
        hash_image(&mut file, expected_digest, None)?;
        let after = fstat(&file).map_err(|_| LinuxTransportError::Peer)?;
        if FileIdentity::from_stat(&after) != identity {
            return Err(LinuxTransportError::Peer);
        }
        compare_path_identity(
            directories.parent()?,
            directories.leaf(),
            identity,
            LinuxTransportError::Peer,
        )?;
        Ok(Self {
            _directories: Some(directories),
            _file: file,
            identity,
        })
    }

    /// Opens the image bound by the kernel's `/proc/<pid>/exe` magic link.
    ///
    /// This is intentionally separate from [`Self::open`]. The generated proc
    /// path is the only symlink-like path followed by this adapter; it is not
    /// a caller-provided filesystem path. Opening it without `NOFOLLOW` gives
    /// a descriptor for the image the kernel reports as loaded, after which
    /// the descriptor is type-checked, identity-checked and hashed.
    pub(super) fn open_proc(
        pid: u32,
        expected_path: &Path,
        owner_uid: u32,
        expected_digest: &[u8; 32],
        deadline: Instant,
    ) -> Result<Self, LinuxTransportError> {
        if read_proc_executable(pid, deadline)? != expected_path {
            return Err(LinuxTransportError::Peer);
        }
        ensure_deadline(deadline)?;
        let mut file = open_proc_image(pid)?;
        ensure_deadline(deadline)?;
        let stat = fstat(&file).map_err(|_| LinuxTransportError::Peer)?;
        ensure_deadline(deadline)?;
        validate_image_stat(&stat, owner_uid)?;
        let identity = FileIdentity::from_stat(&stat);
        hash_image(&mut file, expected_digest, Some(deadline))?;
        let after = fstat(&file).map_err(|_| LinuxTransportError::Peer)?;
        ensure_deadline(deadline)?;
        if FileIdentity::from_stat(&after) != identity {
            return Err(LinuxTransportError::Peer);
        }
        Ok(Self {
            _directories: None,
            _file: file,
            identity,
        })
    }

    pub(super) fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub(super) fn verify_path(&self) -> Result<(), LinuxTransportError> {
        let directories = self
            ._directories
            .as_ref()
            .ok_or(LinuxTransportError::Peer)?;
        compare_path_identity(
            directories.parent()?,
            directories.leaf(),
            self.identity,
            LinuxTransportError::Peer,
        )
    }
}

fn hash_image(
    file: &mut File,
    expected: &[u8; 32],
    deadline: Option<Instant>,
) -> Result<[u8; 32], LinuxTransportError> {
    let mut hasher = Sha256::new();
    let mut total = 0_usize;
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        if let Some(deadline) = deadline {
            ensure_deadline(deadline)?;
        }
        let count = file
            .read(&mut chunk)
            .map_err(|_| LinuxTransportError::Peer)?;
        if count == 0 {
            break;
        }
        total = total.checked_add(count).ok_or(LinuxTransportError::Peer)?;
        if total > MAX_IMAGE_BYTES {
            return Err(LinuxTransportError::Peer);
        }
        hasher.update(&chunk[..count]);
        if let Some(deadline) = deadline {
            ensure_deadline(deadline)?;
        }
    }
    if let Some(deadline) = deadline {
        ensure_deadline(deadline)?;
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if bool::from(digest.ct_eq(expected)) {
        Ok(digest)
    } else {
        Err(LinuxTransportError::Peer)
    }
}

pub(super) fn open_proc_image(pid: u32) -> Result<File, LinuxTransportError> {
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    let fd = openat(
        rustix::fs::ABS,
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| LinuxTransportError::Peer)?;
    Ok(File::from(fd))
}

pub(super) fn read_proc_executable(
    pid: u32,
    deadline: Instant,
) -> Result<PathBuf, LinuxTransportError> {
    ensure_deadline(deadline)?;
    let path = PathBuf::from(format!("/proc/{pid}/exe"));
    let executable = fs::read_link(path).map_err(|_| LinuxTransportError::Peer)?;
    ensure_deadline(deadline)?;
    Ok(executable)
}

pub(super) fn validate_image_stat(
    stat: &rustix::fs::Stat,
    owner_uid: u32,
) -> Result<(), LinuxTransportError> {
    if !FileType::from_raw_mode(stat.st_mode).is_file()
        || !owner_allowed(stat.st_uid, owner_uid)
        || stat.st_nlink != 1
        || stat.st_mode & 0o222 != 0
        || stat.st_mode & 0o400 == 0
        || stat.st_size < 0
        || stat.st_size as u64 > MAX_IMAGE_BYTES as u64
    {
        return Err(LinuxTransportError::Peer);
    }
    Ok(())
}

pub(super) fn process_image_identity(
    pid: u32,
    owner_uid: u32,
    deadline: Instant,
) -> Result<FileIdentity, LinuxTransportError> {
    ensure_deadline(deadline)?;
    let file = open_proc_image(pid)?;
    ensure_deadline(deadline)?;
    let stat = fstat(&file).map_err(|_| LinuxTransportError::Peer)?;
    ensure_deadline(deadline)?;
    validate_image_stat(&stat, owner_uid)?;
    ensure_deadline(deadline)?;
    Ok(FileIdentity::from_stat(&stat))
}
