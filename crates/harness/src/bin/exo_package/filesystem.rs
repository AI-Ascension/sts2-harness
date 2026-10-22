// SPDX-License-Identifier: MIT

use super::Result;
use rustix::fs::{Mode, OFlags, mkdirat, open, openat};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path};

const DIRECTORY_FLAGS: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub(super) fn open_directory(path: &Path, private: bool) -> Result<File> {
    if !path.is_absolute() {
        return Err("exo_package_path");
    }
    let mut directory =
        File::from(open("/", DIRECTORY_FLAGS, Mode::empty()).map_err(|_| "exo_package_path")?);
    for part in path.components() {
        match part {
            Component::RootDir => (),
            Component::Normal(name) => {
                check_directory(&directory, false)?;
                directory = File::from(
                    openat(&directory, name, DIRECTORY_FLAGS, Mode::empty())
                        .map_err(|_| "exo_package_path")?,
                );
            }
            _ => return Err("exo_package_path"),
        }
    }
    check_directory(&directory, private)?;
    Ok(directory)
}

pub(super) fn child_directory(parent: &File, name: &str) -> Result<File> {
    let directory = File::from(
        openat(parent, name, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|_| "exo_package_directory")?,
    );
    check_directory(&directory, true)?;
    Ok(directory)
}

pub(super) fn create_directory(parent: &File, name: &str) -> Result<File> {
    mkdirat(parent, name, Mode::RWXU)
        .map_err(|_| "exo_package_destination_exists_or_unwritable")?;
    let directory = File::from(
        openat(parent, name, DIRECTORY_FLAGS, Mode::empty())
            .map_err(|_| "exo_package_directory")?,
    );
    rustix::fs::fchmod(&directory, Mode::RWXU).map_err(|_| "exo_package_permissions")?;
    check_directory(&directory, true)?;
    parent.sync_all().map_err(|_| "exo_package_sync")?;
    Ok(directory)
}

pub(super) fn check_directory(directory: &File, private: bool) -> Result<()> {
    let metadata = directory.metadata().map_err(|_| "exo_package_metadata")?;
    let uid = rustix::process::geteuid().as_raw();
    let mode = metadata.mode() & 0o7777;
    let safe = if private {
        metadata.uid() == uid && mode == 0o700
    } else {
        (metadata.uid() == 0 || metadata.uid() == uid)
            && (mode & 0o7022 == 0 || (metadata.uid() == 0 && mode == 0o1777))
    };
    if !metadata.is_dir() || !safe {
        return Err("exo_package_directory_permissions");
    }
    Ok(())
}

pub(super) fn open_file(parent: &File, name: &str, private: bool) -> Result<File> {
    let file = File::from(
        openat(
            parent,
            name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| "exo_package_file")?,
    );
    check_file(&file, private)?;
    Ok(file)
}

pub(super) fn check_file(file: &File, private: bool) -> Result<()> {
    let metadata = file.metadata().map_err(|_| "exo_package_metadata")?;
    let uid = rustix::process::geteuid().as_raw();
    let mode = metadata.mode() & 0o7777;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || (metadata.uid() != uid && (private || metadata.uid() != 0))
        || mode & (if private { 0o7077 } else { 0o7022 }) != 0
    {
        return Err("exo_package_file_permissions");
    }
    Ok(())
}

pub(super) fn check_executable(file: &File, executable: bool) -> Result<()> {
    let mode = file.metadata().map_err(|_| "exo_package_metadata")?.mode();
    if (mode & 0o100 != 0) != executable {
        return Err("exo_package_executable_mode");
    }
    Ok(())
}

pub(super) fn names(directory: &File, maximum: usize) -> Result<Vec<String>> {
    let mut names = Vec::new();
    // The descriptor anchors enumeration to the opened directory, including after a rename.
    let entries = fs::read_dir(format!("/proc/self/fd/{}", directory.as_raw_fd()))
        .map_err(|_| "exo_package_inventory")?;
    for entry in entries {
        if names.len() >= maximum {
            return Err("exo_package_inventory_bound");
        }
        names.push(
            entry
                .map_err(|_| "exo_package_inventory")?
                .file_name()
                .into_string()
                .map_err(|_| "exo_package_inventory")?,
        );
    }
    names.sort();
    Ok(names)
}

pub(super) fn hash(mut file: File, maximum: u64) -> Result<(u64, String)> {
    transfer(&mut file, &mut std::io::sink(), maximum)
}

pub(super) fn transfer(
    input: &mut File,
    output: &mut impl Write,
    maximum: u64,
) -> Result<(u64, String)> {
    let mut buffer = [0_u8; 64 * 1024];
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    loop {
        let count = input.read(&mut buffer).map_err(|_| "exo_package_read")?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or("exo_package_file_bound")?;
        if total > maximum {
            return Err("exo_package_file_bound");
        }
        digest.update(&buffer[..count]);
        output
            .write_all(&buffer[..count])
            .map_err(|_| "exo_package_write")?;
    }
    let mut encoded = String::with_capacity(64);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for byte in digest.finalize() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 15)]));
    }
    Ok((total, encoded))
}

pub(super) fn read(file: File, maximum: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "exo_package_read")?;
    if bytes.len() > maximum {
        return Err("exo_package_file_bound");
    }
    Ok(bytes)
}

pub(super) fn read_absolute(path: &Path, maximum: usize, private: bool) -> Result<Vec<u8>> {
    let parent = open_directory(path.parent().ok_or("exo_package_path")?, false)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("exo_package_path")?;
    read(open_file(&parent, name, private)?, maximum)
}

pub(super) fn write_absolute_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = open_directory(path.parent().ok_or("exo_package_path")?, false)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("exo_package_path")?;
    write_new(&parent, name, bytes, false)
}

pub(super) fn create_file(parent: &File, name: &str) -> Result<File> {
    let file = File::from(
        openat(
            parent,
            name,
            OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| "exo_package_destination_exists_or_unwritable")?,
    );
    rustix::fs::fchmod(&file, Mode::RUSR | Mode::WUSR).map_err(|_| "exo_package_permissions")?;
    check_file(&file, true)?;
    Ok(file)
}

pub(super) fn seal(file: &File, executable: bool) -> Result<()> {
    let mode = if executable {
        Mode::RUSR | Mode::XUSR
    } else {
        Mode::RUSR
    };
    rustix::fs::fchmod(file, mode).map_err(|_| "exo_package_permissions")?;
    file.sync_all().map_err(|_| "exo_package_sync")
}

pub(super) fn write_new(parent: &File, name: &str, bytes: &[u8], executable: bool) -> Result<()> {
    let mut file = create_file(parent, name)?;
    file.write_all(bytes).map_err(|_| "exo_package_write")?;
    seal(&file, executable)?;
    parent.sync_all().map_err(|_| "exo_package_sync")
}
