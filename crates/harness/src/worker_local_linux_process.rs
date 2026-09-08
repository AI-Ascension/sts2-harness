// SPDX-License-Identifier: MIT

//! Linux kernel/process identity proof for the worker transport.

#![cfg(target_os = "linux")]

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::OwnedFd;
use rustix::fs::{OFlags, fstat};
use rustix::net::sockopt::socket_peercred;
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use subtle::ConstantTimeEq;
use tokio::net::UnixStream;
use tokio::time::Instant;

use super::worker_local_linux_fs::{
    FileIdentity, HeldDirectories, compare_path_identity, ensure_deadline, open_held_file,
};
use super::worker_local_linux_image::{HeldImage, process_image_identity, read_proc_executable};
use super::{LinuxPeerIdentity, LinuxPeerWitness, LinuxTransportError, MAX_CREDENTIAL_BYTES};

const MAX_PROCESS_STAT_BYTES: usize = 4_096;
const READ_CHUNK_BYTES: usize = 16 * 1024;

pub(super) struct ProtectedCredential {
    _directories: HeldDirectories,
    _file: File,
    bytes: zeroize::Zeroizing<Vec<u8>>,
    _identity: FileIdentity,
}

impl ProtectedCredential {
    pub(super) fn open(path: &Path, owner_uid: u32) -> Result<Self, LinuxTransportError> {
        let (directories, mut file, identity, stat) =
            open_held_file(path, owner_uid, LinuxTransportError::Credential)?;
        super::worker_local_linux_fs::validate_credential_stat(&stat, owner_uid)?;
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(MAX_CREDENTIAL_BYTES));
        let mut chunk = zeroize::Zeroizing::new([0_u8; READ_CHUNK_BYTES]);
        loop {
            let count = file
                .read(&mut *chunk)
                .map_err(|_| LinuxTransportError::Credential)?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > MAX_CREDENTIAL_BYTES {
                return Err(LinuxTransportError::Credential);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        let after = fstat(&file).map_err(|_| LinuxTransportError::Credential)?;
        super::worker_local_linux_fs::validate_credential_stat(&after, owner_uid)?;
        if FileIdentity::from_stat(&after) != identity || !valid_credential(&bytes) {
            return Err(LinuxTransportError::Credential);
        }
        compare_path_identity(
            directories.parent()?,
            directories.leaf(),
            identity,
            LinuxTransportError::Credential,
        )?;
        Ok(Self {
            _directories: directories,
            _file: file,
            bytes,
            _identity: identity,
        })
    }

    pub(super) fn matches(&self, candidate: &[u8]) -> bool {
        if candidate.is_empty() || candidate.len() > MAX_CREDENTIAL_BYTES {
            return false;
        }
        let mut expected = zeroize::Zeroizing::new([0_u8; MAX_CREDENTIAL_BYTES]);
        let mut actual = zeroize::Zeroizing::new([0_u8; MAX_CREDENTIAL_BYTES]);
        expected[..self.bytes.len()].copy_from_slice(&self.bytes);
        actual[..candidate.len()].copy_from_slice(candidate);
        let expected_len = (self.bytes.len() as u32).to_be_bytes();
        let actual_len = (candidate.len() as u32).to_be_bytes();
        bool::from(expected[..].ct_eq(&actual[..]) & expected_len.ct_eq(&actual_len))
    }
}

fn valid_credential(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.len() <= MAX_CREDENTIAL_BYTES
        && bytes.iter().all(|byte| (0x21..=0x7e).contains(byte))
}

pub(super) fn verify_peer(
    stream: &UnixStream,
    expected: &LinuxPeerIdentity,
    approved_image: &HeldImage,
    deadline: Instant,
) -> Result<LinuxPeerWitness, LinuxTransportError> {
    if Instant::now() >= deadline {
        return Err(LinuxTransportError::Deadline);
    }
    let credentials = socket_peercred(stream).map_err(|_| LinuxTransportError::Peer)?;
    ensure_deadline(deadline)?;
    if credentials.uid.as_raw() != expected.uid
        || credentials.pid.as_raw_pid() <= 0
        || u32::try_from(credentials.pid.as_raw_pid()).ok() != Some(expected.pid)
    {
        return Err(LinuxTransportError::Peer);
    }
    let pid = Pid::from_raw(credentials.pid.as_raw_pid()).ok_or(LinuxTransportError::Peer)?;
    let pidfd = pidfd_open(pid, PidfdFlags::empty()).map_err(|_| LinuxTransportError::Peer)?;
    ensure_deadline(deadline)?;
    ensure_pidfd_live(&pidfd)?;
    ensure_deadline(deadline)?;
    let before = process_start_token(expected.pid, deadline)?;
    if before != expected.start_token {
        return Err(LinuxTransportError::Peer);
    }
    let actual_image = HeldImage::open_proc(
        expected.pid,
        &expected.executable,
        expected.uid,
        &expected.executable_sha256,
        deadline,
    )?;
    if actual_image.identity() != approved_image.identity() || approved_image.verify_path().is_err()
    {
        return Err(LinuxTransportError::Peer);
    }
    ensure_deadline(deadline)?;
    let after = process_start_token(expected.pid, deadline)?;
    let executable_after = read_proc_executable(expected.pid, deadline)?;
    let actual_identity_after = process_image_identity(expected.pid, expected.uid, deadline)?;
    ensure_pidfd_live(&pidfd)?;
    ensure_deadline(deadline)?;
    if before != after
        || executable_after != expected.executable
        || actual_image.identity() != approved_image.identity()
        || actual_identity_after != actual_image.identity()
        || approved_image.verify_path().is_err()
        || Instant::now() >= deadline
    {
        return Err(if Instant::now() >= deadline {
            LinuxTransportError::Deadline
        } else {
            LinuxTransportError::Peer
        });
    }
    Ok(LinuxPeerWitness {
        _pidfd: pidfd,
        _pid: expected.pid,
        _uid: expected.uid,
        _image: actual_image,
    })
}

fn ensure_pidfd_live(pidfd: &OwnedFd) -> Result<(), LinuxTransportError> {
    let mut descriptor = PollFd::new(
        pidfd,
        PollFlags::IN | PollFlags::ERR | PollFlags::HUP | PollFlags::NVAL,
    );
    poll(
        std::slice::from_mut(&mut descriptor),
        Some(&Timespec::default()),
    )
    .map_err(|_| LinuxTransportError::Peer)?;
    if descriptor.revents().is_empty() {
        Ok(())
    } else {
        Err(LinuxTransportError::Peer)
    }
}

fn process_start_token(pid: u32, deadline: Instant) -> Result<u64, LinuxTransportError> {
    if Instant::now() >= deadline {
        return Err(LinuxTransportError::Deadline);
    }
    let path = format!("/proc/{pid}/stat");
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(nonblocking_cloexec_flags())
        .open(path)
        .map_err(|_| LinuxTransportError::Peer)?;
    let mut bytes = Vec::with_capacity(MAX_PROCESS_STAT_BYTES);
    let mut chunk = [0_u8; 512];
    loop {
        if Instant::now() >= deadline {
            return Err(LinuxTransportError::Deadline);
        }
        let count = file
            .read(&mut chunk)
            .map_err(|_| LinuxTransportError::Peer)?;
        if count == 0 {
            break;
        }
        if bytes.len().saturating_add(count) > MAX_PROCESS_STAT_BYTES {
            return Err(LinuxTransportError::Peer);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if Instant::now() >= deadline {
            return Err(LinuxTransportError::Deadline);
        }
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| LinuxTransportError::Peer)?;
    let close = text.rfind(')').ok_or(LinuxTransportError::Peer)?;
    text.get(close + 2..)
        .and_then(|suffix| suffix.split_whitespace().nth(19))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|token| *token != 0)
        .ok_or(LinuxTransportError::Peer)
}

fn nonblocking_cloexec_flags() -> i32 {
    let flags = (OFlags::NONBLOCK | OFlags::CLOEXEC).bits();
    flags as i32
}
