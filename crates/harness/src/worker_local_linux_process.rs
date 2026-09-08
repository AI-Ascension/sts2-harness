// SPDX-License-Identifier: MIT

//! Linux kernel/process identity proof for the worker transport.

#![cfg(target_os = "linux")]

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fd::OwnedFd;
use rustix::fs::{FileType, Mode, OFlags, fstat, openat};
use rustix::net::sockopt::socket_peercred;
use rustix::process::{Pid, PidfdFlags, pidfd_open};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tokio::net::UnixStream;
use tokio::time::Instant;

use super::worker_local_linux_fs::{FileIdentity, HeldDirectories, compare_path_identity};
use super::{LinuxPeerIdentity, LinuxPeerWitness, LinuxTransportError, MAX_CREDENTIAL_BYTES};

const MAX_PROCESS_STAT_BYTES: usize = 4_096;
const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
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

pub(super) struct HeldImage {
    _directories: HeldDirectories,
    _file: File,
    identity: FileIdentity,
}

impl HeldImage {
    pub(super) fn open(
        path: &Path,
        owner_uid: u32,
        expected_digest: Option<&[u8; 32]>,
    ) -> Result<Self, LinuxTransportError> {
        let (directories, mut file, identity, stat) =
            open_held_file(path, owner_uid, LinuxTransportError::Peer)?;
        validate_image_stat(&stat, owner_uid)?;
        if let Some(expected) = expected_digest {
            hash_image(&mut file, expected)?;
        }
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
            _directories: directories,
            _file: file,
            identity,
        })
    }

    pub(super) fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub(super) fn verify_path(&self) -> Result<(), LinuxTransportError> {
        compare_path_identity(
            self._directories.parent()?,
            self._directories.leaf(),
            self.identity,
            LinuxTransportError::Peer,
        )
    }
}

fn validate_image_stat(stat: &rustix::fs::Stat, owner_uid: u32) -> Result<(), LinuxTransportError> {
    if !FileType::from_raw_mode(stat.st_mode).is_file()
        || !super::worker_local_linux_fs::owner_allowed(stat.st_uid, owner_uid)
        || stat.st_nlink != 1
        || stat.st_mode & 0o022 != 0
        || stat.st_mode & 0o400 == 0
        || stat.st_size < 0
        || stat.st_size as u64 > MAX_IMAGE_BYTES as u64
    {
        return Err(LinuxTransportError::Peer);
    }
    Ok(())
}

fn hash_image(file: &mut File, expected: &[u8; 32]) -> Result<[u8; 32], LinuxTransportError> {
    let mut hasher = Sha256::new();
    let mut total = 0_usize;
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
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
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if bool::from(digest.ct_eq(expected)) {
        Ok(digest)
    } else {
        Err(LinuxTransportError::Peer)
    }
}

fn open_held_file(
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
    if credentials.uid.as_raw() != expected.uid
        || credentials.pid.as_raw_pid() <= 0
        || u32::try_from(credentials.pid.as_raw_pid()).ok() != Some(expected.pid)
    {
        return Err(LinuxTransportError::Peer);
    }
    let pid = Pid::from_raw(credentials.pid.as_raw_pid()).ok_or(LinuxTransportError::Peer)?;
    let pidfd = pidfd_open(pid, PidfdFlags::empty()).map_err(|_| LinuxTransportError::Peer)?;
    ensure_pidfd_live(&pidfd)?;
    let before = process_start_token(expected.pid, deadline)?;
    if before != expected.start_token {
        return Err(LinuxTransportError::Peer);
    }
    let proc_executable = PathBuf::from(format!("/proc/{}/exe", expected.pid));
    let executable = fs::read_link(&proc_executable).map_err(|_| LinuxTransportError::Peer)?;
    if executable != expected.executable {
        return Err(LinuxTransportError::Peer);
    }
    let actual_image = HeldImage::open(&executable, expected.uid, None)?;
    if actual_image.identity() != approved_image.identity() || approved_image.verify_path().is_err()
    {
        return Err(LinuxTransportError::Peer);
    }
    let after = process_start_token(expected.pid, deadline)?;
    let executable_after =
        fs::read_link(&proc_executable).map_err(|_| LinuxTransportError::Peer)?;
    ensure_pidfd_live(&pidfd)?;
    if before != after
        || executable != executable_after
        || actual_image.identity() != approved_image.identity()
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
