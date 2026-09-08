// SPDX-License-Identifier: MIT

#![allow(unsafe_code)]

use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{addr_of_mut, null, null_mut};

use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_READ_ATTRIBUTES,
    FILE_READ_DATA, FILE_SHARE_MODE, FILE_SHARE_READ, FileIdInfo, GetDriveTypeW,
    GetFileInformationByHandle, GetFileInformationByHandleEx, OPEN_EXISTING, ReadFile,
};

use super::security::validate_private_acl;
use super::{MAX_HASH_CHUNK, MAX_IMAGE_BYTES, MAX_PATH_UTF16};
use crate::MAX_CREDENTIAL_BYTES;
use crate::policy::validate_local_path;
use crate::transport::TransportError;

/// Owned Win32 handle.  `CloseHandle` is called exactly once, including all
/// partial-construction paths, and the invalid-handle sentinel is never closed.
pub(super) struct Handle(HANDLE);

// A HANDLE is an opaque kernel reference.  Ownership remains exclusive in
// `Handle`; the shared connection state serializes every operation that can
// release it and joins pending overlapped I/O before the final drop.  This
// marker permits the listener and a returned connection to coordinate across
// threads without exposing a raw handle through the safe API.
unsafe impl Send for Handle {}

impl Handle {
    pub(super) fn new(raw: HANDLE) -> Result<Self, TransportError> {
        if raw.is_null() || raw == INVALID_HANDLE_VALUE {
            Err(TransportError::Os)
        } else {
            Ok(Self(raw))
        }
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // The handle was validated on construction and is owned exclusively.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct FileIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

/// A regular file plus all validated ancestor handles.  The final handle is
/// opened with sharing that denies write and delete, and every held ancestor
/// was opened with `OPEN_REPARSE_POINT` and checked as a non-reparse directory.
pub(super) struct ProtectedFile {
    handle: Handle,
    _ancestors: Vec<Handle>,
    identity: FileIdentity,
}

impl ProtectedFile {
    pub(super) fn open(
        path: &Path,
        access: u32,
        share: FILE_SHARE_MODE,
    ) -> Result<Self, TransportError> {
        let path_wide = local_path(path)?;
        let ancestors = open_ancestors(path)?;
        let handle = unsafe {
            CreateFileW(
                path_wide.as_ptr(),
                access,
                share,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT,
                0 as HANDLE,
            )
        };
        let handle = Handle::new(handle)?;
        let info = file_info(handle.raw())?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
            || info.nNumberOfLinks != 1
        {
            return Err(TransportError::Credential);
        }
        let identity = file_identity(handle.raw())?;
        Ok(Self {
            handle,
            _ancestors: ancestors,
            identity,
        })
    }

    pub(super) fn raw(&self) -> HANDLE {
        self.handle.raw()
    }

    pub(super) fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub(super) fn verify_identity(&self) -> Result<(), TransportError> {
        if file_identity(self.raw())? == self.identity {
            Ok(())
        } else {
            Err(TransportError::Credential)
        }
    }
}

pub(super) struct HeldImage {
    pub(super) file: ProtectedFile,
    _sha256: [u8; 32],
}

impl HeldImage {
    pub(super) fn open(path: &Path, expected: &[u8; 32]) -> Result<Self, TransportError> {
        let file =
            ProtectedFile::open(path, FILE_READ_DATA | FILE_READ_ATTRIBUTES, FILE_SHARE_READ)?;
        let mut digest = Sha256::new();
        let mut chunk = [0_u8; MAX_HASH_CHUNK];
        let mut total = 0_u64;
        loop {
            let count = read_sync(file.raw(), &mut chunk[..])?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(u64::try_from(count).map_err(|_| TransportError::Configuration)?)
                .ok_or(TransportError::Configuration)?;
            if total > MAX_IMAGE_BYTES {
                return Err(TransportError::Configuration);
            }
            digest.update(&chunk[..count]);
        }
        let actual: [u8; 32] = digest.finalize().into();
        if !bool::from(actual.ct_eq(expected)) {
            return Err(TransportError::Identity);
        }
        file.verify_identity()
            .map_err(|_| TransportError::Identity)?;
        Ok(Self {
            file,
            _sha256: actual,
        })
    }
}

pub(super) struct ProtectedCredential {
    _file: ProtectedFile,
    bytes: Zeroizing<Vec<u8>>,
}

impl ProtectedCredential {
    pub(super) fn open(path: &Path, worker_sid: &str) -> Result<Self, TransportError> {
        let file =
            ProtectedFile::open(path, FILE_READ_DATA | FILE_READ_ATTRIBUTES, FILE_SHARE_READ)?;
        validate_private_acl(file.raw(), worker_sid)?;
        let mut bytes = Zeroizing::new(Vec::with_capacity(MAX_CREDENTIAL_BYTES));
        let mut chunk = Zeroizing::new([0_u8; MAX_CREDENTIAL_BYTES]);
        loop {
            let count = read_sync(file.raw(), &mut chunk[..])?;
            if count == 0 {
                break;
            }
            if bytes.len().saturating_add(count) > MAX_CREDENTIAL_BYTES {
                return Err(TransportError::Credential);
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        if bytes.is_empty()
            || !bytes
                .iter()
                .all(|byte| (0x21..=0x7e).contains(byte) && *byte != b' ')
        {
            return Err(TransportError::Credential);
        }
        file.verify_identity()?;
        Ok(Self { _file: file, bytes })
    }

    pub(super) fn matches(&self, candidate: &[u8]) -> bool {
        if candidate.is_empty() || candidate.len() > MAX_CREDENTIAL_BYTES {
            return false;
        }
        let mut expected = Zeroizing::new([0_u8; MAX_CREDENTIAL_BYTES]);
        let mut actual = Zeroizing::new([0_u8; MAX_CREDENTIAL_BYTES]);
        expected[..self.bytes.len()].copy_from_slice(&self.bytes);
        actual[..candidate.len()].copy_from_slice(candidate);
        let expected_len = (self.bytes.len() as u32).to_be_bytes();
        let actual_len = (candidate.len() as u32).to_be_bytes();
        bool::from(expected[..].ct_eq(&actual[..]) & expected_len.ct_eq(&actual_len))
    }
}
fn read_sync(handle: HANDLE, body: &mut [u8]) -> Result<usize, TransportError> {
    let length = u32::try_from(body.len()).map_err(|_| TransportError::Configuration)?;
    let mut count = 0_u32;
    let ok = unsafe {
        ReadFile(
            handle,
            body.as_mut_ptr(),
            length,
            addr_of_mut!(count),
            null_mut(),
        )
    };
    if ok == FALSE {
        Err(TransportError::Credential)
    } else {
        let count = usize::try_from(count).map_err(|_| TransportError::Credential)?;
        if count > body.len() {
            Err(TransportError::Credential)
        } else {
            Ok(count)
        }
    }
}

fn file_info(
    handle: HANDLE,
) -> Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION, TransportError> {
    let mut info = unsafe { zeroed() };
    if unsafe { GetFileInformationByHandle(handle, addr_of_mut!(info)) } == FALSE {
        return Err(TransportError::Credential);
    }
    Ok(info)
}

fn file_identity(handle: HANDLE) -> Result<FileIdentity, TransportError> {
    let mut info: FILE_ID_INFO = unsafe { zeroed() };
    if unsafe {
        GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            addr_of_mut!(info).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    } == FALSE
    {
        return Err(TransportError::Credential);
    }
    Ok(FileIdentity {
        volume_serial: info.VolumeSerialNumber,
        file_id: info.FileId.Identifier,
    })
}

fn open_ancestors(path: &Path) -> Result<Vec<Handle>, TransportError> {
    let mut paths = Vec::<PathBuf>::new();
    let mut current = path.parent();
    while let Some(parent) = current {
        paths.push(parent.to_path_buf());
        if parent.parent() == Some(parent) {
            break;
        }
        current = parent.parent();
    }
    paths.reverse();
    let mut handles = Vec::with_capacity(paths.len());
    for ancestor in paths {
        let wide = local_path(&ancestor)?;
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                0 as HANDLE,
            )
        };
        let handle = Handle::new(raw)?;
        let info = file_info(handle.raw())?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
            || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(TransportError::Credential);
        }
        handles.push(handle);
    }
    Ok(handles)
}
fn local_path(path: &Path) -> Result<Vec<u16>, TransportError> {
    validate_local_path(path)?;
    let text = path.to_str().ok_or(TransportError::Configuration)?;
    let bytes = text.as_bytes();
    let root = [u16::from(bytes[0]), u16::from(b':'), u16::from(b'\\'), 0];
    let drive_type = unsafe { GetDriveTypeW(root.as_ptr()) };
    // Win32 DRIVE_UNKNOWN=0, DRIVE_NO_ROOT_DIR=1 and DRIVE_REMOTE=4.
    if matches!(drive_type, 0 | 1 | 4) {
        return Err(TransportError::Configuration);
    }
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    if wide.len() >= MAX_PATH_UTF16 {
        return Err(TransportError::Configuration);
    }
    wide.push(0);
    Ok(wide)
}
