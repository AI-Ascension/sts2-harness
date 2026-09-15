// SPDX-License-Identifier: MIT

use crate::exo_lifecycle::{JournalConfig, LifecycleError};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::path::{Component, Path};

/// Non-blocking attempts made before a live owner is reported as `LifecycleError::Busy`.
///
/// An `flock` belongs to the open file description rather than to the process, and a spawn gives
/// the child a copy of the parent descriptor table, so a descriptor this process has already
/// closed keeps the lock alive until the child reaches `execve` and drops its `CLOEXEC` copy. In a
/// binary whose other tests spawn children at the same time, an immediate non-blocking attempt can
/// therefore observe `EWOULDBLOCK` while no owner holds the lease. Retrying tolerates that window
/// and can never admit a second owner: a lock held by a live `Lease` or by another process is not
/// released by waiting, so it still ends in `Busy`. `map/bundle_store_io.rs` holds the map
/// publication lock the same way.
#[cfg(unix)]
const LEASE_ATTEMPTS: usize = 32;
#[cfg(unix)]
const LEASE_RETRY: std::time::Duration = std::time::Duration::from_millis(5);

/// The lock descriptor is private, never cloned and never names the replaceable journal.
pub(crate) struct Lease {
    #[cfg(unix)]
    pub directory: File,
    #[cfg(unix)]
    lock: File,
}

#[cfg(unix)]
impl Lease {
    pub fn acquire(config: &JournalConfig, create: bool) -> Result<Self, LifecycleError> {
        use rustix::fs::{Mode, OFlags, mkdirat, openat};
        config.validate()?;
        if create {
            let parent = config.directory.parent().ok_or(LifecycleError::Invalid)?;
            let parent = open_directory(parent)?;
            let name = config
                .directory
                .file_name()
                .ok_or(LifecycleError::Invalid)?;
            mkdirat(&parent, name, Mode::RWXU).map_err(|_| LifecycleError::Io)?;
            parent.sync_all().map_err(|_| LifecycleError::Io)?;
        }
        let directory = open_directory(&config.directory)?;
        let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC;
        if create {
            flags |= OFlags::CREATE | OFlags::EXCL;
        }
        let lock = File::from(
            openat(&directory, "owner.lock", flags, Mode::RUSR | Mode::WUSR)
                .map_err(|_| LifecycleError::Corrupt)?,
        );
        if !private(&lock.metadata().map_err(|_| LifecycleError::Io)?, false) {
            return Err(LifecycleError::Corrupt);
        }
        let mut acquired = false;
        for _ in 0..LEASE_ATTEMPTS {
            match lock.try_lock() {
                Ok(()) => {
                    acquired = true;
                    break;
                }
                Err(std::fs::TryLockError::WouldBlock) => std::thread::sleep(LEASE_RETRY),
                Err(std::fs::TryLockError::Error(_)) => return Err(LifecycleError::Unsupported),
            }
        }
        if !acquired {
            return Err(LifecycleError::Busy);
        }
        if create {
            directory.sync_all().map_err(|_| LifecycleError::Io)?;
        }
        let lease = Self { directory, lock };
        lease.verify(config)?;
        Ok(lease)
    }

    pub fn verify(&self, config: &JournalConfig) -> Result<(), LifecycleError> {
        use rustix::fs::{Mode, OFlags, openat};
        let current = open_directory(&config.directory)?;
        let lock = File::from(
            openat(
                &self.directory,
                "owner.lock",
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| LifecycleError::Corrupt)?,
        );
        if !same_inode(&current, &self.directory)?
            || !same_inode(&lock, &self.lock)?
            || !private(&lock.metadata().map_err(|_| LifecycleError::Io)?, false)
        {
            return Err(LifecycleError::Stale);
        }
        Ok(())
    }
}

#[cfg(unix)]
pub(crate) fn open_directory(path: &Path) -> Result<File, LifecycleError> {
    use rustix::fs::{Mode, OFlags, open, openat};
    if !path.is_absolute() {
        return Err(LifecycleError::Invalid);
    }
    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    let mut file = File::from(open("/", flags, Mode::empty()).map_err(|_| LifecycleError::Io)?);
    for part in path.components() {
        match part {
            Component::RootDir => (),
            Component::Normal(name) => {
                file = File::from(
                    openat(&file, name, flags, Mode::empty())
                        .map_err(|_| LifecycleError::Corrupt)?,
                );
            }
            _ => return Err(LifecycleError::Invalid),
        }
    }
    if !private(&file.metadata().map_err(|_| LifecycleError::Io)?, true) {
        return Err(LifecycleError::Corrupt);
    }
    Ok(file)
}

#[cfg(unix)]
pub(crate) fn private(metadata: &std::fs::Metadata, directory: bool) -> bool {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    metadata.uid() == rustix::process::geteuid().as_raw()
        && metadata.permissions().mode() & 0o077 == 0
        && if directory {
            metadata.is_dir()
        } else {
            metadata.is_file() && metadata.nlink() == 1
        }
}

#[cfg(unix)]
fn same_inode(a: &File, b: &File) -> Result<bool, LifecycleError> {
    use std::os::unix::fs::MetadataExt;
    let a = a.metadata().map_err(|_| LifecycleError::Io)?;
    let b = b.metadata().map_err(|_| LifecycleError::Io)?;
    Ok(a.dev() == b.dev() && a.ino() == b.ino())
}

#[cfg(not(unix))]
impl Lease {
    pub fn acquire(_: &JournalConfig, _: bool) -> Result<Self, LifecycleError> {
        Err(LifecycleError::Unsupported)
    }
    pub fn verify(&self, _: &JournalConfig) -> Result<(), LifecycleError> {
        Err(LifecycleError::Unsupported)
    }
}
