// SPDX-License-Identifier: MIT

//! Linux owner-local path and descriptor identity for the worker endpoint.

#![cfg(target_os = "linux")]

use std::ffi::OsString;
use std::fs::File;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixListener as StdUnixListener;
use std::path::{Component, Path};

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, Mode, OFlags, chmodat, fstat, openat, statat, unlinkat};
use tokio::net::UnixListener;
use tokio::time::Instant;

use super::LinuxTransportError;

const MAX_PATH_BYTES: usize = 4_096;
pub(super) const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;

pub(super) struct HeldEndpoint {
    directories: HeldDirectories,
    _leaf: OwnedFd,
    identity: FileIdentity,
    listener: Option<StdUnixListener>,
}

impl Drop for HeldEndpoint {
    fn drop(&mut self) {
        self.remove_if_owned();
    }
}

impl HeldEndpoint {
    pub(super) fn bind(path: &Path, owner_uid: u32) -> Result<Self, LinuxTransportError> {
        let directories = HeldDirectories::open(path, owner_uid)?;
        let parent = directories.parent()?;
        if statat(parent, directories.leaf(), AtFlags::SYMLINK_NOFOLLOW).is_ok() {
            return Err(LinuxTransportError::Configuration);
        }
        let listener = StdUnixListener::bind(path).map_err(|_| LinuxTransportError::Io)?;
        listener
            .set_nonblocking(true)
            .map_err(|_| LinuxTransportError::Io)?;
        let listener_stat = fstat(&listener).map_err(|_| LinuxTransportError::Io)?;
        if !FileType::from_raw_mode(listener_stat.st_mode).is_socket()
            || !owner_allowed(listener_stat.st_uid, owner_uid)
        {
            return Err(LinuxTransportError::Configuration);
        }
        chmodat(
            parent,
            directories.leaf(),
            Mode::from_raw_mode(0o600),
            AtFlags::empty(),
        )
        .map_err(|_| LinuxTransportError::Io)?;
        let leaf = openat(
            parent,
            directories.leaf(),
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| LinuxTransportError::Configuration)?;
        let leaf_stat = fstat(&leaf).map_err(|_| LinuxTransportError::Configuration)?;
        validate_endpoint_stat(&leaf_stat, owner_uid)?;
        let identity = FileIdentity::from_stat(&leaf_stat);
        Ok(Self {
            directories,
            _leaf: leaf,
            identity,
            listener: Some(listener),
        })
    }

    pub(super) fn listener(&mut self) -> Result<UnixListener, LinuxTransportError> {
        let listener = self.listener.take().ok_or(LinuxTransportError::Closed)?;
        UnixListener::from_std(listener).map_err(|_| LinuxTransportError::Io)
    }

    /// Duplicates the protected parent directory and returns the stable leaf
    /// identity/name used by the out-of-process verifier. The verifier repeats
    /// the `statat(..., NOFOLLOW)` check so accepted-connection work does not
    /// perform blocking endpoint filesystem calls on the async parent.
    pub(super) fn duplicate_verifier_path(
        &self,
    ) -> Result<(OwnedFd, FileIdentity, Vec<u8>), LinuxTransportError> {
        let parent = rustix::io::dup(self.directories.parent()?)
            .map_err(|_| LinuxTransportError::Configuration)?;
        Ok((
            parent,
            self.identity,
            self.directories.leaf().as_bytes().to_vec(),
        ))
    }

    fn remove_if_owned(&self) {
        let Ok(parent) = self.directories.parent() else {
            return;
        };
        let Ok(stat) = statat(parent, self.directories.leaf(), AtFlags::SYMLINK_NOFOLLOW) else {
            return;
        };
        if FileIdentity::from_stat(&stat) == self.identity {
            let _ = unlinkat(parent, self.directories.leaf(), AtFlags::empty());
        }
    }
}

pub(super) struct HeldDirectories {
    directories: Vec<OwnedFd>,
    leaf: OsString,
}

impl HeldDirectories {
    pub(super) fn open(path: &Path, owner_uid: u32) -> Result<Self, LinuxTransportError> {
        if !is_canonical_absolute(path) {
            return Err(LinuxTransportError::Configuration);
        }
        let components = path.components().collect::<Vec<_>>();
        let leaf = match components.last() {
            Some(Component::Normal(leaf)) => leaf.to_os_string(),
            _ => return Err(LinuxTransportError::Configuration),
        };
        let root = openat(
            rustix::fs::ABS,
            "/",
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| LinuxTransportError::Configuration)?;
        validate_directory_fd(&root, owner_uid)?;
        let mut directories = vec![root];
        for component in components
            .iter()
            .skip(1)
            .take(components.len().saturating_sub(2))
        {
            let Component::Normal(component) = component else {
                return Err(LinuxTransportError::Configuration);
            };
            let parent = directories
                .last()
                .ok_or(LinuxTransportError::Configuration)?;
            let next = openat(
                parent,
                *component,
                OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                Mode::empty(),
            )
            .map_err(|_| LinuxTransportError::Configuration)?;
            validate_directory_fd(&next, owner_uid)?;
            directories.push(next);
        }
        Ok(Self { directories, leaf })
    }

    pub(super) fn parent(&self) -> Result<&OwnedFd, LinuxTransportError> {
        self.directories
            .last()
            .ok_or(LinuxTransportError::Configuration)
    }

    pub(super) fn leaf(&self) -> &OsString {
        &self.leaf
    }
}

pub(super) fn open_held_file(
    path: &Path,
    owner_uid: u32,
    error: LinuxTransportError,
) -> Result<(HeldDirectories, File, FileIdentity, rustix::fs::Stat), LinuxTransportError> {
    let directories = HeldDirectories::open(path, owner_uid)?;
    let fd = openat(
        directories.parent()?,
        directories.leaf(),
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| error)?;
    let stat = fstat(&fd).map_err(|_| error)?;
    let identity = FileIdentity::from_stat(&stat);
    compare_path_identity(directories.parent()?, directories.leaf(), identity, error)?;
    Ok((directories, File::from(fd), identity, stat))
}

pub(super) fn ensure_deadline(deadline: Instant) -> Result<(), LinuxTransportError> {
    if Instant::now() >= deadline {
        Err(LinuxTransportError::Deadline)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

impl FileIdentity {
    pub(super) fn from_stat(stat: &rustix::fs::Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode: stat.st_ino,
        }
    }

    pub(super) fn parts(self) -> (u64, u64) {
        (self.device, self.inode)
    }

    pub(super) const fn from_parts(device: u64, inode: u64) -> Self {
        Self { device, inode }
    }
}

pub(super) fn compare_path_identity(
    parent: &OwnedFd,
    leaf: &OsString,
    expected: FileIdentity,
    error: LinuxTransportError,
) -> Result<(), LinuxTransportError> {
    let stat = statat(parent, leaf, AtFlags::SYMLINK_NOFOLLOW).map_err(|_| error)?;
    if FileIdentity::from_stat(&stat) != expected {
        return Err(error);
    }
    Ok(())
}

fn validate_directory_fd(fd: &OwnedFd, owner_uid: u32) -> Result<(), LinuxTransportError> {
    let stat = fstat(fd).map_err(|_| LinuxTransportError::Configuration)?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    let writable = stat.st_mode & 0o022 != 0;
    let sticky = stat.st_mode & 0o1000 != 0;
    if !file_type.is_dir()
        || !owner_allowed(stat.st_uid, owner_uid)
        || (writable && !(stat.st_uid == 0 && sticky))
    {
        return Err(LinuxTransportError::Configuration);
    }
    Ok(())
}

pub(super) fn validate_credential_stat(
    stat: &rustix::fs::Stat,
    owner_uid: u32,
) -> Result<(), LinuxTransportError> {
    if !FileType::from_raw_mode(stat.st_mode).is_file()
        || !owner_allowed(stat.st_uid, owner_uid)
        || stat.st_nlink != 1
        || stat.st_mode & 0o077 != 0
        || stat.st_mode & 0o400 == 0
        || stat.st_mode & 0o7000 != 0
    {
        return Err(LinuxTransportError::Credential);
    }
    Ok(())
}

fn validate_endpoint_stat(
    stat: &rustix::fs::Stat,
    owner_uid: u32,
) -> Result<(), LinuxTransportError> {
    if !FileType::from_raw_mode(stat.st_mode).is_socket()
        || !owner_allowed(stat.st_uid, owner_uid)
        || stat.st_mode & 0o077 != 0
    {
        return Err(LinuxTransportError::Configuration);
    }
    Ok(())
}

pub(super) fn owner_allowed(owner: u32, expected: u32) -> bool {
    owner == expected || owner == 0
}

pub(super) fn is_canonical_absolute(path: &Path) -> bool {
    if !path.is_absolute() || path.as_os_str().as_bytes().len() > MAX_PATH_BYTES {
        return false;
    }
    let mut has_leaf = false;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => {
                has_leaf = true;
                if part.as_bytes().is_empty() || part.as_bytes().contains(&0) {
                    return false;
                }
            }
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => return false,
        }
    }
    has_leaf
}
