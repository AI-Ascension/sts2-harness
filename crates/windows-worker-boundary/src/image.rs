// SPDX-License-Identifier: MIT

use crate::handle::{NativeHandle, last_error, wide_path};
use sha2::{Digest as _, Sha256};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::Arc;
use windows_sys::Win32::Foundation::{GENERIC_READ, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, GetFileInformationByHandle, OPEN_EXISTING, ReadFile,
};

pub(crate) const MAX_PATH_BYTES: usize = 4 * 1024;
pub(crate) const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

pub struct VerifiedExecutable(Arc<VerifiedExecutableInner>);

struct VerifiedExecutableInner {
    path: PathBuf,
    digest: String,
    // The mutex makes the retained handle movable through Arc without making
    // a mutable Win32 handle generally shareable.
    retained_image: Arc<std::sync::Mutex<HeldFile>>,
}

impl Clone for VerifiedExecutable {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl std::fmt::Debug for VerifiedExecutable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedExecutable")
            .field("path", &"<protected-reference>")
            .field("digest", &self.0.digest)
            .finish()
    }
}

impl VerifiedExecutable {
    pub fn open(path: &Path, expected_digest: Option<&str>) -> Result<Self, String> {
        validate_windows_path(path, "worker runtime executable")?;
        let image = open_hashed_file(path, "worker runtime executable")?;
        let canonical = std::fs::canonicalize(path)
            .map_err(|_| String::from("worker runtime executable is unavailable"))?;
        if !same_path(path, &canonical) {
            return Err(String::from(
                "worker runtime executable is not a canonical non-reparse path",
            ));
        }
        if let Some(expected) = expected_digest {
            validate_digest(expected)?;
            if image.digest != expected {
                return Err(String::from(
                    "worker runtime executable digest is not approved",
                ));
            }
        }
        let digest = image.digest.clone();
        Ok(Self(Arc::new(VerifiedExecutableInner {
            path: canonical,
            digest,
            retained_image: Arc::new(std::sync::Mutex::new(image)),
        })))
    }

    #[must_use]
    pub fn command_path(&self) -> &Path {
        &self.0.path
    }

    #[must_use]
    pub fn digest(&self) -> &str {
        &self.0.digest
    }

    pub fn verify_current(&self) -> Result<(), String> {
        let current = open_hashed_file(&self.0.path, "worker runtime executable")?;
        let retained = self
            .0
            .retained_image
            .lock()
            .map_err(|_| String::from("worker runtime image state is unavailable"))?;
        if current.identity != retained.identity || current.digest != self.0.digest {
            return Err(String::from(
                "worker runtime executable changed after admission",
            ));
        }
        Ok(())
    }
}

pub(crate) struct HeldFile {
    pub(crate) handle: NativeHandle,
    pub(crate) identity: FileIdentity,
    pub(crate) digest: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FileIdentity {
    volume_serial: u32,
    index_high: u32,
    index_low: u32,
}

pub(crate) fn open_hashed_file(path: &Path, label: &str) -> Result<HeldFile, String> {
    let wide = wide_path(path, label)?;
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            null_mut(),
        )
    };
    let handle = NativeHandle::new(raw, "CreateFileW(worker image)")?;
    let identity = file_identity(handle.raw())?;
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(handle.raw(), &raw mut information) } == 0 {
        return Err(last_error("GetFileInformationByHandle(worker image)"));
    }
    if information.dwFileAttributes & (FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT) != 0
    {
        return Err(format!("{label} is not a regular non-reparse file"));
    }
    let digest = hash_file(handle.raw(), label)?;
    Ok(HeldFile {
        handle,
        identity,
        digest,
    })
}

pub(crate) fn file_identity(handle: HANDLE) -> Result<FileIdentity, String> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(handle, &raw mut information) } == 0 {
        return Err(last_error("GetFileInformationByHandle"));
    }
    Ok(FileIdentity {
        volume_serial: information.dwVolumeSerialNumber,
        index_high: information.nFileIndexHigh,
        index_low: information.nFileIndexLow,
    })
}

fn hash_file(handle: HANDLE, label: &str) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 32 * 1024];
    loop {
        let mut read = 0_u32;
        let count = u32::try_from(buffer.len())
            .map_err(|_| String::from("worker image buffer size overflow"))?;
        if unsafe {
            ReadFile(
                handle,
                buffer.as_mut_ptr().cast(),
                count,
                &raw mut read,
                null_mut(),
            )
        } == 0
        {
            return Err(format!("{label} could not be hashed"));
        }
        if read == 0 {
            break;
        }
        if read > count {
            return Err(String::from("worker image read count is invalid"));
        }
        total = total
            .checked_add(u64::from(read))
            .ok_or_else(|| String::from("worker image size overflow"))?;
        if total > MAX_IMAGE_BYTES {
            return Err(String::from("worker image exceeds its size bound"));
        }
        hasher.update(
            &buffer[..usize::try_from(read)
                .map_err(|_| String::from("worker image read count overflow"))?],
        );
    }
    Ok(crate::hex_bytes(hasher.finalize()))
}

pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy()
        .trim_start_matches(r"\\?\")
        .replace('/', "\\")
        .to_ascii_lowercase()
}

pub(crate) fn validate_windows_path(path: &Path, label: &str) -> Result<(), String> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("{label} is not Unicode"))?;
    let bytes = value.as_bytes();
    if value.len() < 3
        || value.len() > MAX_PATH_BYTES
        || !path.is_absolute()
        || value.contains('\0')
        || bytes[1] != b':'
        || bytes[2] != b'\\'
        || bytes[2..].contains(&b':')
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(format!("{label} path is invalid"));
    }
    Ok(())
}

pub(crate) fn validate_digest(value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(String::from("worker executable digest is invalid"));
    }
    Ok(())
}
