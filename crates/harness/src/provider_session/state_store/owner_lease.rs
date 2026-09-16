// SPDX-License-Identifier: MIT

//! Exclusive process lifetime lease for one saved provider-session policy journal.

use super::ProviderSessionMetadataStoreError;
#[cfg(unix)]
use super::{restricted_directory, restricted_file};
#[cfg(unix)]
use std::ffi::OsString;
#[cfg(unix)]
use std::fs::File;
use std::path::Path;
#[cfg(unix)]
use std::path::{Component, PathBuf};

#[cfg(unix)]
const LEASE_ATTEMPTS: usize = 32;
#[cfg(unix)]
const LEASE_RETRY: std::time::Duration = std::time::Duration::from_millis(5);

pub(crate) struct PolicyOwnerLease {
    #[cfg(unix)]
    directory: File,
    #[cfg(unix)]
    lock_name: OsString,
    #[cfg(unix)]
    lock: File,
    #[cfg(unix)]
    directory_path: PathBuf,
}

#[cfg(unix)]
impl PolicyOwnerLease {
    pub fn acquire(path: &Path) -> Result<Self, ProviderSessionMetadataStoreError> {
        use rustix::fs::{Mode, OFlags, openat};
        use std::fs::TryLockError;

        let directory_path = path
            .parent()
            .ok_or(ProviderSessionMetadataStoreError::InvalidPath)?;
        let directory = open_private_directory(directory_path)?;
        let mut lock_name = path
            .file_name()
            .ok_or(ProviderSessionMetadataStoreError::InvalidPath)?
            .to_os_string();
        lock_name.push(".policy-owner.lock");
        let lock = File::from(
            openat(
                &directory,
                &lock_name,
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| ProviderSessionMetadataStoreError::InvalidPath)?,
        );
        if !private_lock(
            &lock
                .metadata()
                .map_err(|_| ProviderSessionMetadataStoreError::Io)?,
        ) {
            return Err(ProviderSessionMetadataStoreError::InvalidPath);
        }
        let mut acquired = false;
        for _ in 0..LEASE_ATTEMPTS {
            match lock.try_lock() {
                Ok(()) => {
                    acquired = true;
                    break;
                }
                Err(TryLockError::WouldBlock) => std::thread::sleep(LEASE_RETRY),
                Err(TryLockError::Error(_)) => {
                    return Err(ProviderSessionMetadataStoreError::Unsupported);
                }
            }
        }
        if !acquired {
            return Err(ProviderSessionMetadataStoreError::Busy);
        }
        directory
            .sync_all()
            .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
        let lease = Self {
            directory,
            lock_name,
            lock,
            directory_path: directory_path.to_owned(),
        };
        lease.verify()?;
        Ok(lease)
    }

    pub fn verify(&self) -> Result<(), ProviderSessionMetadataStoreError> {
        use rustix::fs::{Mode, OFlags, openat};

        let current_directory = open_private_directory(&self.directory_path)?;
        let current_lock = File::from(
            openat(
                &self.directory,
                &self.lock_name,
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ProviderSessionMetadataStoreError::InvalidPath)?,
        );
        if !same_inode(&current_directory, &self.directory)?
            || !same_inode(&current_lock, &self.lock)?
            || !private_lock(
                &current_lock
                    .metadata()
                    .map_err(|_| ProviderSessionMetadataStoreError::Io)?,
            )
        {
            return Err(ProviderSessionMetadataStoreError::InvalidPath);
        }
        Ok(())
    }
}

#[cfg(not(unix))]
impl PolicyOwnerLease {
    pub fn acquire(_: &Path) -> Result<Self, ProviderSessionMetadataStoreError> {
        Err(ProviderSessionMetadataStoreError::Unsupported)
    }

    pub fn verify(&self) -> Result<(), ProviderSessionMetadataStoreError> {
        Err(ProviderSessionMetadataStoreError::Unsupported)
    }
}

#[cfg(unix)]
fn open_private_directory(path: &Path) -> Result<File, ProviderSessionMetadataStoreError> {
    use rustix::fs::{Mode, OFlags, open, openat};

    if !path.is_absolute() {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    let directory_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut directory = File::from(
        open("/", directory_flags, Mode::empty())
            .map_err(|_| ProviderSessionMetadataStoreError::Io)?,
    );
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => {
                directory = File::from(
                    openat(&directory, name, directory_flags, Mode::empty())
                        .map_err(|_| ProviderSessionMetadataStoreError::InvalidPath)?,
                );
            }
            _ => return Err(ProviderSessionMetadataStoreError::InvalidPath),
        }
    }
    let metadata = directory
        .metadata()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    if !metadata.is_dir() || !restricted_directory(&metadata) {
        return Err(ProviderSessionMetadataStoreError::InvalidPath);
    }
    Ok(directory)
}

#[cfg(unix)]
fn private_lock(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    restricted_file(metadata) && metadata.nlink() == 1
}

#[cfg(unix)]
fn same_inode(left: &File, right: &File) -> Result<bool, ProviderSessionMetadataStoreError> {
    use std::os::unix::fs::MetadataExt;
    let left = left
        .metadata()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    let right = right
        .metadata()
        .map_err(|_| ProviderSessionMetadataStoreError::Io)?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}
