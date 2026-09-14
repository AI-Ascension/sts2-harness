// SPDX-License-Identifier: MIT

//! Private persistence owned by the provider-session broker.

mod io;
mod lease;
#[cfg(all(test, unix))]
mod tests;
pub(crate) mod types;

use crate::exo_lifecycle::{JournalConfig, LifecycleError};
use lease::Lease;
pub(crate) use types::JournalSnapshot;

#[cfg(all(test, unix))]
pub(crate) use io::CommitStage;

#[cfg(all(test, unix))]
pub(crate) fn inject_commit_failure(stage: Option<CommitStage>, skip: usize) {
    io::FAILURE.set(stage);
    io::FAILURE_SKIP.set(skip);
}

#[cfg(unix)]
pub(crate) fn read_legacy(path: &std::path::Path) -> Result<Vec<u8>, LifecycleError> {
    use rustix::fs::{Mode, OFlags, openat};
    use std::io::Read;
    let parent = lease::open_directory(path.parent().ok_or(LifecycleError::Invalid)?)?;
    let file = std::fs::File::from(
        openat(
            &parent,
            path.file_name().ok_or(LifecycleError::Invalid)?,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| LifecycleError::Corrupt)?,
    );
    let metadata = file.metadata().map_err(|_| LifecycleError::Io)?;
    if !lease::private(&metadata, false) || metadata.len() > types::MAX_ENVELOPE as u64 {
        return Err(LifecycleError::Corrupt);
    }
    let mut bytes = Vec::new();
    file.take(types::MAX_ENVELOPE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| LifecycleError::Io)?;
    if bytes.len() > types::MAX_ENVELOPE {
        return Err(LifecycleError::Capacity);
    }
    Ok(bytes)
}

#[cfg(not(unix))]
pub(crate) fn read_legacy(_: &std::path::Path) -> Result<Vec<u8>, LifecycleError> {
    Err(LifecycleError::Unsupported)
}

/// A live exclusive lease; never reconstructed from serialized metadata.
pub(crate) struct OwnerJournal {
    config: JournalConfig,
    key: zeroize::Zeroizing<[u8; 32]>,
    lease: Lease,
    epoch: u64,
    revision: u64,
    poisoned: bool,
}

impl OwnerJournal {
    pub fn create(
        config: JournalConfig,
        key: [u8; 32],
        snapshot: &JournalSnapshot,
    ) -> Result<Self, LifecycleError> {
        if key.iter().all(|byte| *byte == 0) {
            return Err(LifecycleError::Invalid);
        }
        let bytes = io::encode(&config, &key, snapshot)?;
        let lease = Lease::acquire(&config, true)?;
        io::write(&lease, &bytes)?;
        lease.verify(&config)?;
        Ok(Self {
            config,
            key: zeroize::Zeroizing::new(key),
            lease,
            epoch: snapshot.claim_epoch,
            revision: snapshot.revision,
            poisoned: false,
        })
    }

    pub fn open(
        config: JournalConfig,
        key: [u8; 32],
    ) -> Result<(Self, JournalSnapshot), LifecycleError> {
        if key.iter().all(|byte| *byte == 0) {
            return Err(LifecycleError::Invalid);
        }
        let lease = Lease::acquire(&config, false)?;
        let snapshot = io::decode(&config, &key, &io::read(&lease)?)?;
        lease.verify(&config)?;
        let journal = Self {
            config,
            key: zeroize::Zeroizing::new(key),
            lease,
            epoch: snapshot.claim_epoch,
            revision: snapshot.revision,
            poisoned: false,
        };
        Ok((journal, snapshot))
    }

    pub fn check(&self) -> Result<(), LifecycleError> {
        if self.poisoned {
            return Err(LifecycleError::Poisoned);
        }
        self.lease.verify(&self.config)?;
        let disk = io::decode(&self.config, &self.key, &io::read(&self.lease)?)?;
        if disk.claim_epoch != self.epoch || disk.revision != self.revision {
            return Err(LifecycleError::Stale);
        }
        Ok(())
    }

    /// All uncertainty is sticky, including a failure reported after rename or fsync.
    pub fn commit(&mut self, snapshot: &JournalSnapshot) -> Result<(), LifecycleError> {
        let result = self.commit_checked(snapshot);
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }

    fn commit_checked(&mut self, snapshot: &JournalSnapshot) -> Result<(), LifecycleError> {
        self.check()?;
        if snapshot.revision
            != self
                .revision
                .checked_add(1)
                .ok_or(LifecycleError::Capacity)?
            || snapshot.claim_epoch < self.epoch
            || snapshot.claim_epoch > self.epoch.checked_add(1).ok_or(LifecycleError::Capacity)?
        {
            return Err(LifecycleError::Stale);
        }
        let bytes = io::encode(&self.config, &self.key, snapshot)?;
        io::write(&self.lease, &bytes)?;
        self.lease.verify(&self.config)?;
        self.epoch = snapshot.claim_epoch;
        self.revision = snapshot.revision;
        Ok(())
    }
}
