// SPDX-License-Identifier: MIT

mod locks;
mod markers;
mod tree;

pub(super) use locks::{lock_policy_roots, verify_policy_lock, verify_policy_root};
pub(super) use markers::{
    create_private_child, create_private_file, read_marker, same_file, verify_attempt,
    verify_private_directory, verify_private_file, write_marker,
};
pub(super) use tree::{
    policy_attempt_names, remove_attempt, remove_children, scan_attempts, scan_policy_root,
    scan_run,
};

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, mkdirat, open, openat, unlinkat};
use rustix::io::Errno;

use serde::{Deserialize, Serialize};

use super::{OWNER_LOCK_NAME, OWNER_MARKER_NAME};

const PRIVATE_DIR_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const PRIVATE_BITS: u32 = 0o077;
const SPECIAL_BITS: u32 = 0o7000;
const POLICY_LOCK_RECORD: &str = "sts2.exo-private-policy-lock-v1";
pub(super) const MAX_POLICY_LOCK_BYTES: u64 = 256;
pub(super) const MAX_TREE_ENTRIES: usize = 8192;
pub(super) const MAX_TREE_DEPTH: usize = 16;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub uid: u32,
    pub mode: u32,
}

impl FileIdentity {
    pub(super) fn from_file(file: &File) -> Result<Self, &'static str> {
        let metadata = file.metadata().map_err(|_| "exo_private_path")?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            uid: metadata.uid(),
            mode: metadata.mode() & 0o7777,
        })
    }

    fn matches_file(&self, file: &File) -> Result<bool, &'static str> {
        Ok(self == &Self::from_file(file)?)
    }
}

pub(super) struct PolicyRoot {
    pub path: PathBuf,
    pub file: File,
    pub identity: FileIdentity,
    pub ancestors: Vec<FileIdentity>,
}

pub(super) struct PolicyLock {
    pub file: File,
    pub identity: FileIdentity,
    pub policy_digest: String,
}

pub(super) struct AttemptDirectory {
    pub name: OsString,
    pub path: PathBuf,
    pub file: File,
    pub identity: FileIdentity,
    pub lock: File,
    pub marker: File,
}

pub(super) fn open_policy_root(path: &Path, create_leaf: bool) -> Result<PolicyRoot, &'static str> {
    if !path.is_absolute() {
        return Err("exo_private_path");
    }
    let components = path.components().collect::<Vec<_>>();
    if components.first() != Some(&Component::RootDir)
        || components.len() < 2
        || components.iter().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        return Err("exo_private_path");
    }
    let mut directory = File::from(
        open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| "exo_private_path")?,
    );
    let service_uid = rustix::process::geteuid().as_raw();
    let mut ancestors = vec![FileIdentity::from_file(&directory)?];
    let normal = components.iter().filter_map(|component| match component {
        Component::Normal(name) => Some(*name),
        _ => None,
    });
    let normal = normal.collect::<Vec<_>>();
    if normal.len() + 1 != components.len() {
        return Err("exo_private_path");
    }
    for (index, component) in normal.iter().enumerate() {
        let final_component = index + 1 == normal.len();
        let next = match openat(
            &directory,
            *component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(Errno::NOENT) if final_component && create_leaf => {
                mkdirat(
                    &directory,
                    *component,
                    Mode::from_bits_retain(PRIVATE_DIR_MODE),
                )
                .map_err(|_| "exo_private_root_create")?;
                let created = File::from(
                    openat(
                        &directory,
                        *component,
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    )
                    .map_err(|_| "exo_private_root_create")?,
                );
                rustix::fs::fchmod(&created, Mode::from_bits_retain(PRIVATE_DIR_MODE))
                    .map_err(|_| "exo_private_root_create")?;
                created
            }
            Err(_) => return Err("exo_private_path"),
        };
        let identity = FileIdentity::from_file(&next)?;
        if final_component {
            if identity.uid != service_uid
                || identity.mode != PRIVATE_DIR_MODE
                || !next.metadata().map_err(|_| "exo_private_path")?.is_dir()
            {
                return Err("exo_private_root_owner");
            }
        } else if !trusted_ancestor(&identity) {
            return Err("exo_private_ancestor");
        }
        ancestors.push(identity);
        directory = next;
    }
    let identity = FileIdentity::from_file(&directory)?;
    Ok(PolicyRoot {
        path: path.to_owned(),
        file: directory,
        identity,
        ancestors,
    })
}

fn trusted_ancestor(identity: &FileIdentity) -> bool {
    let trusted_owner = identity.uid == 0 || identity.uid == rustix::process::geteuid().as_raw();
    let ordinary = identity.mode & (0o022 | SPECIAL_BITS) == 0;
    let root_sticky_tmp = identity.uid == 0 && identity.mode == 0o1777;
    trusted_owner && (ordinary || root_sticky_tmp)
}

pub(super) fn create_attempt(
    root: &PolicyRoot,
    name: &OsStr,
) -> Result<AttemptDirectory, &'static str> {
    mkdirat(&root.file, name, Mode::from_bits_retain(PRIVATE_DIR_MODE))
        .map_err(|_| "exo_private_attempt_create")?;
    let file = openat(
        &root.file,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_attempt_open")?;
    if rustix::fs::fchmod(&file, Mode::from_bits_retain(PRIVATE_DIR_MODE)).is_err()
        || verify_private_directory(&file).is_err()
    {
        return Err("exo_private_attempt_mode");
    }
    let lock = match create_private_file(&file, OsStr::new(OWNER_LOCK_NAME)) {
        Ok(lock) => lock,
        Err(error) => {
            remove_new_directory(root, name, &file)?;
            return Err(error);
        }
    };
    if rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive).is_err() {
        remove_new_directory(root, name, &file)?;
        return Err("exo_private_owner_lock");
    }
    let marker = match create_private_file(&file, OsStr::new(OWNER_MARKER_NAME)) {
        Ok(marker) => marker,
        Err(error) => {
            remove_new_directory(root, name, &file)?;
            return Err(error);
        }
    };
    let identity = FileIdentity::from_file(&file)?;
    Ok(AttemptDirectory {
        name: name.to_owned(),
        path: root.path.join(name),
        file,
        identity,
        lock,
        marker,
    })
}

fn remove_new_directory(root: &PolicyRoot, name: &OsStr, owned: &File) -> Result<(), &'static str> {
    let current = openat(
        &root.file,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_partial_cleanup")?;
    verify_private_directory(&current)?;
    if !same_file(owned, &current)? {
        return Err("exo_private_partial_cleanup");
    }
    let mut entries = 0_usize;
    remove_children(&current, 0, &mut entries)?;
    let reopened = openat(
        &root.file,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_partial_cleanup")?;
    if !same_file(owned, &reopened)? {
        return Err("exo_private_partial_cleanup");
    }
    unlinkat(&root.file, name, AtFlags::REMOVEDIR).map_err(|_| "exo_private_partial_cleanup")
}

pub(super) fn open_existing_attempt(
    root: &PolicyRoot,
    name: &OsStr,
) -> Result<AttemptDirectory, &'static str> {
    let file = openat(
        &root.file,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_attempt_open")?;
    verify_private_directory(&file)?;
    let lock = openat(
        &file,
        OWNER_LOCK_NAME,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_owner_lock")?;
    verify_private_file(&lock, true)?;
    rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| "exo_private_owner_lock_busy")?;
    let marker = openat(
        &file,
        OWNER_MARKER_NAME,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_marker")?;
    verify_private_file(&marker, true)?;
    let identity = FileIdentity::from_file(&file)?;
    Ok(AttemptDirectory {
        name: name.to_owned(),
        path: root.path.join(name),
        file,
        identity,
        lock,
        marker,
    })
}
