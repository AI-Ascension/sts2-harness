// SPDX-License-Identifier: MIT

//! Deadline-bound, single-use stdin pipe consumption for worker startup only.

use std::os::fd::AsFd;
use std::time::{Duration, Instant};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::OwnedFd;
use rustix::fs::{FileType, OFlags, fcntl_getfl, fcntl_setfl, fstat, fstatfs};
use rustix::io::{Errno, dup, read};

use crate::worker_bootstrap::{
    BOOTSTRAP_MAGIC, ExpectedBootstrapPeer, MAX_BOOTSTRAP_BYTES, WorkerBootstrap,
};

pub const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootstrapReadError {
    Invalid,
    Deadline,
    Io,
}
impl std::fmt::Display for BootstrapReadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid worker bootstrap pipe or frame",
            Self::Deadline => "worker bootstrap deadline expired",
            Self::Io => "worker bootstrap pipe failed",
        })
    }
}
impl std::error::Error for BootstrapReadError {}

/// Reserve stdin at worker startup, before any other stdin consumer or runtime
/// threads exist. Replacing descriptor zero with /dev/null closes the original
/// pipe reference; the duplicate is owned and closed on every result path.
pub fn read_stdin() -> Result<WorkerBootstrap, BootstrapReadError> {
    let input = std::io::stdin();
    let pipe = dup(input.as_fd()).map_err(|_| BootstrapReadError::Io)?;
    let null = std::fs::File::open("/dev/null").map_err(|_| BootstrapReadError::Io)?;
    rustix::stdio::dup2_stdin(&null).map_err(|_| BootstrapReadError::Io)?;
    read_owned_pipe(pipe, BOOTSTRAP_TIMEOUT)
}

/// Consume one frame without waiting for EOF. Nonblocking reads and one
/// absolute deadline prevent a writer from extending startup by trickling.
/// This validates expected policy only, never connected-peer authentication.
pub fn read_owned_pipe(
    pipe: OwnedFd,
    timeout: Duration,
) -> Result<WorkerBootstrap, BootstrapReadError> {
    if timeout.is_zero() || timeout > BOOTSTRAP_TIMEOUT {
        return Err(BootstrapReadError::Invalid);
    }
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(BootstrapReadError::Deadline)?;
    let stat = fstat(&pipe).map_err(|_| BootstrapReadError::Io)?;
    // Linux UAPI PIPEFS_MAGIC: a named filesystem FIFO is not the dedicated
    // anonymous bootstrap channel required by the launch contract.
    let filesystem = fstatfs(&pipe).map_err(|_| BootstrapReadError::Io)?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::Fifo || filesystem.f_type != 0x5049_5045 {
        return Err(BootstrapReadError::Invalid);
    }
    let flags = fcntl_getfl(&pipe).map_err(|_| BootstrapReadError::Io)?;
    fcntl_setfl(&pipe, flags | OFlags::NONBLOCK).map_err(|_| BootstrapReadError::Io)?;
    let mut prefix = [0_u8; 12];
    read_exact(&pipe, &mut prefix, deadline)?;
    if &prefix[..8] != BOOTSTRAP_MAGIC {
        return Err(BootstrapReadError::Invalid);
    }
    let length = u32::from_be_bytes(
        prefix[8..]
            .try_into()
            .map_err(|_| BootstrapReadError::Invalid)?,
    );
    let length = usize::try_from(length).map_err(|_| BootstrapReadError::Invalid)?;
    if length == 0 || length > MAX_BOOTSTRAP_BYTES {
        return Err(BootstrapReadError::Invalid);
    }
    let mut frame = vec![0_u8; 12 + length];
    frame[..12].copy_from_slice(&prefix);
    read_exact(&pipe, &mut frame[12..], deadline)?;
    // Do not wait for EOF. If already-buffered trailing input exists it cannot
    // become a second bootstrap frame; reject it and close the pipe.
    let mut extra = [0_u8; 1];
    match read(&pipe, &mut extra) {
        Ok(0) | Err(Errno::AGAIN) => {}
        _ => return Err(BootstrapReadError::Invalid),
    }
    let result = WorkerBootstrap::decode(&frame).map_err(|_| BootstrapReadError::Invalid)?;
    if !matches!(result.expected_peer(), ExpectedBootstrapPeer::Linux { .. }) {
        return Err(BootstrapReadError::Invalid);
    }
    if Instant::now() >= deadline {
        return Err(BootstrapReadError::Deadline);
    }
    Ok(result)
}

fn read_exact(
    pipe: &OwnedFd,
    bytes: &mut [u8],
    deadline: Instant,
) -> Result<(), BootstrapReadError> {
    let mut offset = 0;
    while offset < bytes.len() {
        if Instant::now() >= deadline {
            return Err(BootstrapReadError::Deadline);
        }
        match read(pipe, &mut bytes[offset..]) {
            Ok(0) => return Err(BootstrapReadError::Invalid),
            Ok(count) => offset += count,
            Err(Errno::INTR) => continue,
            Err(Errno::AGAIN) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let timeout =
                    Timespec::try_from(remaining).map_err(|_| BootstrapReadError::Deadline)?;
                let mut fds = [PollFd::new(pipe, PollFlags::IN)];
                match poll(&mut fds, Some(&timeout)) {
                    Ok(0) => return Err(BootstrapReadError::Deadline),
                    Ok(_) | Err(Errno::INTR) => {}
                    Err(_) => return Err(BootstrapReadError::Io),
                }
            }
            Err(_) => return Err(BootstrapReadError::Io),
        }
    }
    Ok(())
}
