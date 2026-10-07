// SPDX-License-Identifier: MIT

use std::path::Path;

use zeroize::Zeroizing;

use super::{
    Keyring, SeedDerivationKeyAuthority, SeedDerivationKeyReadiness, SeedKeyError, SeedKeyHandle,
};

#[cfg(target_os = "linux")]
use super::MAX_KEYRING_BYTES;

/// Read-only snapshot of a protected service keyring. The snapshot is immutable
/// for this process lifetime; rotating the selected version requires restart.
///
/// Its closed ASCII file format is one schema line, one `authority_id=...`, one
/// `current_version=...`, and one `key.<version>=<64 lowercase hex digits>` per
/// retained key. Unknown lines, duplicate fields, duplicate versions, CRLF,
/// comments, and a missing current key are rejected. The file is key material;
/// install it only in an owner-private directory with owner-private file mode.
pub struct FileSeedDerivationKeyAuthority {
    keyring: Keyring,
}

impl FileSeedDerivationKeyAuthority {
    /// Load an owner-private Linux keyring without following symlinks. Other
    /// platforms fail closed until equivalent descriptor/ACL checks are owned.
    pub fn open(path: &Path) -> Result<Self, SeedKeyError> {
        let bytes = read_private_keyring(path)?;
        Ok(Self {
            keyring: Keyring::parse(&bytes)?,
        })
    }
}

impl SeedDerivationKeyAuthority for FileSeedDerivationKeyAuthority {
    fn current_key(&self) -> Result<SeedKeyHandle, SeedKeyError> {
        self.keyring.current_key()
    }

    fn key_for(
        &self,
        authority_id: &str,
        version: &str,
    ) -> Result<Option<SeedKeyHandle>, SeedKeyError> {
        self.keyring.key_for(authority_id, version)
    }

    fn readiness(&self) -> SeedDerivationKeyReadiness {
        self.keyring.readiness()
    }
}

#[cfg(target_os = "linux")]
fn read_private_keyring(path: &Path) -> Result<Zeroizing<Vec<u8>>, SeedKeyError> {
    use std::fs::File;
    use std::io::Read;
    use std::os::fd::OwnedFd;
    use std::path::{Component, PathBuf};

    use rustix::fs::{FileType, Mode, OFlags, fstat, open, openat};
    use rustix::process::geteuid;

    if !path.is_absolute() {
        return Err(SeedKeyError::InsecureKeyring);
    }
    let components = path
        .components()
        .filter_map(|component| match component {
            Component::RootDir => None,
            Component::Normal(value) => value.to_str(),
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => None,
        })
        .collect::<Vec<_>>();
    if components.len() < 2
        || components.iter().any(|component| component.is_empty())
        || components.len() + 1 != path.components().count()
    {
        return Err(SeedKeyError::InsecureKeyring);
    }

    let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
    let root = open("/", flags, Mode::empty()).map_err(|_| SeedKeyError::Unavailable)?;
    let mut directory: OwnedFd = root;
    let owner = geteuid().as_raw();
    let mut traversed = PathBuf::from("/");
    validate_traversal_directory(&directory, &traversed, owner)?;
    for component in &components[..components.len() - 1] {
        let next = openat(&directory, *component, flags, Mode::empty())
            .map_err(|_| SeedKeyError::InsecureKeyring)?;
        traversed.push(component);
        validate_traversal_directory(&next, &traversed, owner)?;
        directory = next;
    }
    let directory_stat = fstat(&directory).map_err(|_| SeedKeyError::Unavailable)?;
    let directory_permissions = directory_stat.st_mode & 0o777;
    if FileType::from_raw_mode(directory_stat.st_mode) != FileType::Directory
        || directory_stat.st_uid != owner
        || directory_permissions & 0o077 != 0
        || directory_permissions & 0o500 != 0o500
    {
        return Err(SeedKeyError::InsecureKeyring);
    }

    let file = openat(
        &directory,
        components[components.len() - 1],
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| SeedKeyError::InsecureKeyring)?;
    let metadata = fstat(&file).map_err(|_| SeedKeyError::Unavailable)?;
    let permissions = metadata.st_mode & 0o777;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile
        || metadata.st_uid != owner
        || metadata.st_nlink != 1
        || permissions & 0o077 != 0
        || permissions & 0o444 == 0
        || permissions & 0o111 != 0
        || metadata.st_size <= 0
        || metadata.st_size as usize > MAX_KEYRING_BYTES
    {
        return Err(SeedKeyError::InsecureKeyring);
    }
    let mut file = File::from(file);
    let mut bytes = Zeroizing::new(Vec::new());
    bytes
        .try_reserve_exact(MAX_KEYRING_BYTES + 1)
        .map_err(|_| SeedKeyError::Unavailable)?;
    file.by_ref()
        .take((MAX_KEYRING_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| SeedKeyError::Unavailable)?;
    if bytes.len() != metadata.st_size as usize || bytes.len() > MAX_KEYRING_BYTES {
        return Err(SeedKeyError::InvalidKeyring);
    }
    Ok(bytes)
}

#[cfg(target_os = "linux")]
fn validate_traversal_directory(
    directory: &impl rustix::fd::AsFd,
    path: &Path,
    service_uid: u32,
) -> Result<(), SeedKeyError> {
    use rustix::fs::{FileType, fstat};

    let metadata = fstat(directory).map_err(|_| SeedKeyError::Unavailable)?;
    if FileType::from_raw_mode(metadata.st_mode) != FileType::Directory {
        return Err(SeedKeyError::InsecureKeyring);
    }
    let permissions = metadata.st_mode & 0o7777;
    if metadata.st_uid == 0 {
        let safe_root_owned_directory = if path == Path::new("/tmp") {
            permissions == 0o1777
        } else {
            permissions & 0o022 == 0 && permissions & 0o005 == 0o005
        };
        if safe_root_owned_directory {
            return Ok(());
        }
        return Err(SeedKeyError::InsecureKeyring);
    }
    if metadata.st_uid == service_uid && permissions & 0o077 == 0 && permissions & 0o500 == 0o500 {
        return Ok(());
    }
    Err(SeedKeyError::InsecureKeyring)
}

#[cfg(not(target_os = "linux"))]
fn read_private_keyring(_path: &Path) -> Result<Zeroizing<Vec<u8>>, SeedKeyError> {
    Err(SeedKeyError::UnsupportedPlatform)
}
