// SPDX-License-Identifier: MIT

mod path;

pub(super) use path::{attempt_path, path_for_kind, private_path_for_kind, validate_policy_path};

use super::*;

pub(super) fn open_private_root(path: &Path) -> Result<OpenRoot, &'static str> {
    validate_policy_path(path)?;
    let mut directory = File::from(
        open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| "exo_private_root")?,
    );
    let uid = rustix::process::geteuid().as_raw();
    let mut ancestors = vec![file_identity(&directory)?];
    let components = path.components().filter_map(|component| match component {
        Component::Normal(name) => Some(name),
        _ => None,
    });
    let components = components.collect::<Vec<_>>();
    if components.len() > MAX_COMPONENTS {
        return Err("exo_private_path_bound");
    }
    for (index, component) in components.iter().enumerate() {
        let child = openat(
            &directory,
            *component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| "exo_private_root")?;
        let identity = file_identity(&child)?;
        if index + 1 == components.len() {
            if identity.uid != uid || identity.mode != PRIVATE_DIRECTORY_MODE {
                return Err("exo_private_root_owner");
            }
        } else if !trusted_ancestor(&identity, uid) {
            return Err("exo_private_ancestor");
        }
        ancestors.push(identity);
        directory = child;
    }
    let identity = file_identity(&directory)?;
    Ok(OpenRoot {
        file: directory,
        identity,
        ancestors,
    })
}

pub(super) fn verify_policy_lock(root: &OpenRoot, policy_digest: &str) -> Result<(), &'static str> {
    let stat = statat(&root.file, POLICY_LOCK_NAME, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| "exo_private_policy_lock")?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o7777 != PRIVATE_FILE_MODE
        || stat.st_nlink != 1
    {
        return Err("exo_private_policy_lock");
    }
    let lock = openat(
        &root.file,
        POLICY_LOCK_NAME,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_policy_lock")?;
    let opened_identity = file_identity(&lock)?;
    let opened_metadata = lock.metadata().map_err(|_| "exo_private_policy_lock")?;
    if opened_identity.device != stat.st_dev
        || opened_identity.inode != stat.st_ino
        || opened_identity.uid != rustix::process::geteuid().as_raw()
        || opened_identity.mode != PRIVATE_FILE_MODE
        || opened_metadata.nlink() != 1
    {
        return Err("exo_private_policy_lock");
    }
    let mut contents = Vec::new();
    (&lock)
        .take((MAX_POLICY_LOCK_BYTES + 1) as u64)
        .read_to_end(&mut contents)
        .map_err(|_| "exo_private_policy_lock")?;
    let expected = format!("{POLICY_LOCK_RECORD}\n{policy_digest}\n");
    if contents.len() > MAX_POLICY_LOCK_BYTES || contents != expected.as_bytes() {
        return Err("exo_private_policy_owner");
    }
    match rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Err(rustix::io::Errno::WOULDBLOCK) => {}
        Ok(()) => {
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::Unlock)
                .map_err(|_| "exo_private_policy_lock")?;
            return Err("exo_private_policy_lock_unheld");
        }
        Err(_) => return Err("exo_private_policy_lock"),
    }
    Ok(())
}

pub(super) fn verify_attempt_lock(directory: &File) -> Result<(), &'static str> {
    let stat = statat(directory, ".sts2-owner.lock", AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|_| "exo_private_owner_lock")?;
    if FileType::from_raw_mode(stat.st_mode) != FileType::RegularFile
        || stat.st_uid != rustix::process::geteuid().as_raw()
        || stat.st_mode & 0o7777 != PRIVATE_FILE_MODE
        || stat.st_nlink != 1
    {
        return Err("exo_private_owner_lock");
    }
    let lock = openat(
        directory,
        ".sts2-owner.lock",
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_owner_lock")?;
    let opened_identity = file_identity(&lock)?;
    let opened_metadata = lock.metadata().map_err(|_| "exo_private_owner_lock")?;
    if opened_identity.device != stat.st_dev
        || opened_identity.inode != stat.st_ino
        || opened_identity.uid != rustix::process::geteuid().as_raw()
        || opened_identity.mode != PRIVATE_FILE_MODE
        || opened_metadata.nlink() != 1
    {
        return Err("exo_private_owner_lock");
    }
    match rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(()),
        Ok(()) => {
            rustix::fs::flock(&lock, rustix::fs::FlockOperation::Unlock)
                .map_err(|_| "exo_private_owner_lock")?;
            Err("exo_private_owner_lock_unheld")
        }
        Err(_) => Err("exo_private_owner_lock"),
    }
}

pub(super) fn read_marker(directory: &File) -> Result<OwnerMarker, &'static str> {
    let mut file = openat(
        directory,
        MARKER_NAME,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_marker")?;
    let metadata = file.metadata().map_err(|_| "exo_private_marker")?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o7777 != PRIVATE_FILE_MODE
        || metadata.nlink() != 1
    {
        return Err("exo_private_marker");
    }
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_MARKER_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "exo_private_marker")?;
    if bytes.len() > MAX_MARKER_BYTES {
        return Err("exo_private_marker_bound");
    }
    serde_json::from_slice(&bytes).map_err(|_| "exo_private_marker")
}

pub(super) fn verify_marker(
    marker: &OwnerMarker,
    private: &PrivateState,
    kind: &str,
    base_identity: &FileIdentity,
    attempt_identity: &FileIdentity,
    expected_proofs: &[RootProof],
) -> Result<(), &'static str> {
    let proof = marker
        .root_proofs
        .iter()
        .find(|proof| proof.kind == kind)
        .ok_or("exo_private_marker_identity")?;
    if marker.schema != MARKER_SCHEMA
        || marker.attempt_id != private.attempt_id
        || marker.config_digest != private.config_digest
        || marker.policy_digest != private.policy_digest
        || marker.service_uid != private.service_uid
        || marker.boot_id != private.process.boot_id
        || marker.root_proofs != expected_proofs
        || marker.root_kind != kind
        || marker.phase != MarkerPhase::Running
        || marker.process.as_ref() != Some(&private.process)
        || marker.root_identity != *attempt_identity
        || proof.base_identity != *base_identity
        || proof.attempt_identity != *attempt_identity
    {
        return Err("exo_private_marker_identity");
    }
    Ok(())
}

pub(super) fn same_shared_marker(left: &OwnerMarker, right: &OwnerMarker) -> bool {
    left.schema == right.schema
        && left.attempt_id == right.attempt_id
        && left.config_digest == right.config_digest
        && left.policy_digest == right.policy_digest
        && left.service_uid == right.service_uid
        && left.created_at_unix_seconds == right.created_at_unix_seconds
        && left.boot_id == right.boot_id
        && left.root_proofs == right.root_proofs
        && left.phase == right.phase
        && left.process == right.process
}

pub(super) fn reject_root_aliases(roots: &[(&str, OpenRoot)]) -> Result<(), &'static str> {
    for left in 0..roots.len() {
        for right in (left + 1)..roots.len() {
            let left_identity = &roots[left].1.identity;
            let right_identity = &roots[right].1.identity;
            if left_identity.device == right_identity.device
                && left_identity.inode == right_identity.inode
                || roots[left].1.ancestors.iter().any(|identity| {
                    identity.device == right_identity.device
                        && identity.inode == right_identity.inode
                })
                || roots[right].1.ancestors.iter().any(|identity| {
                    identity.device == left_identity.device && identity.inode == left_identity.inode
                })
            {
                return Err("exo_private_root_alias");
            }
        }
    }
    Ok(())
}

pub(super) fn verify_directory(directory: &File, uid: u32) -> Result<(), &'static str> {
    let metadata = directory.metadata().map_err(|_| "exo_private_path")?;
    if !metadata.is_dir()
        || metadata.uid() != uid
        || metadata.mode() & 0o7777 != PRIVATE_DIRECTORY_MODE
    {
        return Err("exo_private_directory_mode");
    }
    Ok(())
}

pub(super) fn trusted_ancestor(identity: &FileIdentity, uid: u32) -> bool {
    let owner = identity.uid == 0 || identity.uid == uid;
    let ordinary = identity.mode & (0o022 | 0o7000) == 0;
    let root_sticky = identity.uid == 0 && identity.mode == 0o1777;
    owner && (ordinary || root_sticky)
}

pub(super) fn file_identity(file: &File) -> Result<FileIdentity, &'static str> {
    let metadata = file.metadata().map_err(|_| "exo_private_path")?;
    Ok(FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        uid: metadata.uid(),
        mode: metadata.mode() & 0o7777,
    })
}

pub(super) fn file_identity_path(path: &Path) -> Result<FileIdentity, &'static str> {
    let file = open_absolute_directory(path)?;
    file_identity(&file)
}

pub(super) fn open_absolute_directory(path: &Path) -> Result<File, &'static str> {
    validate_policy_path(path)?;
    let mut directory = File::from(
        open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| "exo_private_path")?,
    );
    let mut count = 0_usize;
    for component in path.components() {
        let Component::Normal(name) = component else {
            continue;
        };
        count += 1;
        if count > MAX_COMPONENTS {
            return Err("exo_private_path_bound");
        }
        directory = File::from(
            openat(
                &directory,
                name,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| "exo_private_path")?,
        );
    }
    Ok(directory)
}
