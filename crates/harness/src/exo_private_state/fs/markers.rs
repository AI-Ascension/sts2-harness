// SPDX-License-Identifier: MIT

use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;

use rustix::fs::{AtFlags, FlockOperation, Mode, OFlags, mkdirat, openat};

use super::{
    AttemptDirectory, FileIdentity, PRIVATE_BITS, PRIVATE_DIR_MODE, PRIVATE_FILE_MODE, PolicyRoot,
    SPECIAL_BITS,
};
use crate::exo_private_state::{OWNER_LOCK_NAME, OWNER_MARKER_NAME};

pub(super) fn create_private_child(parent: &File, name: &OsStr) -> Result<File, &'static str> {
    mkdirat(parent, name, Mode::from_bits_retain(PRIVATE_DIR_MODE))
        .map_err(|_| "exo_private_child_create")?;
    let file = openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_child_open")?;
    rustix::fs::fchmod(&file, Mode::from_bits_retain(PRIVATE_DIR_MODE))
        .map_err(|_| "exo_private_child_mode")?;
    verify_private_directory(&file)?;
    Ok(file)
}

pub(super) fn create_private_file(parent: &File, name: &OsStr) -> Result<File, &'static str> {
    let file = openat(
        parent,
        name,
        OFlags::RDWR
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::NOFOLLOW
            | OFlags::CLOEXEC
            | OFlags::NONBLOCK,
        Mode::from_bits_retain(PRIVATE_FILE_MODE),
    )
    .map(File::from)
    .map_err(|_| "exo_private_marker_create")?;
    rustix::fs::fchmod(&file, Mode::from_bits_retain(PRIVATE_FILE_MODE))
        .map_err(|_| "exo_private_marker_create")?;
    verify_private_file(&file, true)?;
    Ok(file)
}

pub(super) fn write_marker(file: &mut File, bytes: &[u8]) -> Result<(), &'static str> {
    verify_private_file(file, true)?;
    file.set_len(0).map_err(|_| "exo_private_marker_write")?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| "exo_private_marker_write")?;
    file.write_all(bytes)
        .map_err(|_| "exo_private_marker_write")?;
    file.sync_all().map_err(|_| "exo_private_marker_write")
}

pub(super) fn read_marker(directory: &File) -> Result<Vec<u8>, &'static str> {
    let mut file = openat(
        directory,
        OWNER_MARKER_NAME,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_marker")?;
    verify_private_file(&file, true)?;
    let mut bytes = Vec::new();
    file.take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "exo_private_marker")?;
    if bytes.len() > 16 * 1024 {
        return Err("exo_private_marker_bound");
    }
    Ok(bytes)
}

pub(super) fn verify_private_directory(file: &File) -> Result<(), &'static str> {
    let identity = FileIdentity::from_file(file)?;
    if identity.uid != rustix::process::geteuid().as_raw()
        || identity.mode != PRIVATE_DIR_MODE
        || !file.metadata().map_err(|_| "exo_private_path")?.is_dir()
    {
        return Err("exo_private_directory_mode");
    }
    Ok(())
}

pub(super) fn verify_private_file(file: &File, exact_mode: bool) -> Result<(), &'static str> {
    let metadata = file.metadata().map_err(|_| "exo_private_file")?;
    let mode = metadata.mode() & 0o7777;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || mode & (PRIVATE_BITS | SPECIAL_BITS) != 0
        || (exact_mode && mode != PRIVATE_FILE_MODE)
        || metadata.nlink() != 1
    {
        return Err("exo_private_file_mode");
    }
    Ok(())
}

pub(super) fn same_file(left: &File, right: &File) -> Result<bool, &'static str> {
    let left = left.metadata().map_err(|_| "exo_private_path")?;
    let right = right.metadata().map_err(|_| "exo_private_path")?;
    Ok(left.dev() == right.dev() && left.ino() == right.ino())
}

pub(super) fn verify_attempt(
    root: &PolicyRoot,
    attempt: &AttemptDirectory,
) -> Result<(), &'static str> {
    let reopened = openat(
        &root.file,
        &attempt.name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_attempt_changed")?;
    if !attempt.identity.matches_file(&reopened)? || !same_file(&attempt.file, &reopened)? {
        return Err("exo_private_attempt_changed");
    }
    verify_named_file(&reopened, OWNER_LOCK_NAME, &attempt.lock)?;
    verify_named_file(&reopened, OWNER_MARKER_NAME, &attempt.marker)?;
    Ok(())
}

fn verify_named_file(directory: &File, name: &str, expected: &File) -> Result<(), &'static str> {
    let current = openat(
        directory,
        name,
        OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(|_| "exo_private_attempt_changed")?;
    verify_private_file(&current, true)?;
    if !same_file(expected, &current)? {
        return Err("exo_private_attempt_changed");
    }
    Ok(())
}
