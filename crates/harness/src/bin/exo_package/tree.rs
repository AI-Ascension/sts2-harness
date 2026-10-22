// SPDX-License-Identifier: MIT

use super::{Result, filesystem as fs};
use rustix::fs::{AtFlags, FileType, statat};
use std::fs::File;
use std::os::unix::fs::MetadataExt;

/// Read-only quota observation. No lease, marker, retained operation or private byte is rewritten.
pub(super) fn private_usage(private: &File, quota: u64) -> Result<u64> {
    if fs::names(private, 3)? != ["cache", "state", "temp"] {
        return Err("exo_package_private_inventory");
    }
    let mut entries = 0_usize;
    let mut bytes = 0_u64;
    for name in ["state", "cache", "temp"] {
        let directory = fs::child_directory(private, name)?;
        scan(&directory, 0, &mut entries, &mut bytes, quota)?;
    }
    Ok(bytes)
}

fn scan(
    directory: &File,
    depth: usize,
    entries: &mut usize,
    bytes: &mut u64,
    quota: u64,
) -> Result<()> {
    if depth > 16 {
        return Err("exo_package_private_depth");
    }
    for name in fs::names(directory, 8192)? {
        *entries += 1;
        if *entries > 8192 {
            return Err("exo_package_private_entries");
        }
        let metadata = statat(directory, name.as_str(), AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "exo_package_private_changed")?;
        match FileType::from_raw_mode(metadata.st_mode) {
            FileType::Directory => {
                let child = fs::child_directory(directory, &name)?;
                same(&child, metadata.st_dev, metadata.st_ino)?;
                scan(&child, depth + 1, entries, bytes, quota)?;
            }
            FileType::RegularFile => {
                let file = fs::open_file(directory, &name, true)?;
                same(&file, metadata.st_dev, metadata.st_ino)?;
                *bytes = bytes
                    .checked_add(file.metadata().map_err(|_| "exo_package_metadata")?.len())
                    .ok_or("exo_package_private_quota")?;
                if *bytes > quota {
                    return Err("exo_package_private_quota");
                }
            }
            _ => return Err("exo_package_private_entry"),
        }
    }
    Ok(())
}

fn same(file: &File, device: u64, inode: u64) -> Result<()> {
    let metadata = file.metadata().map_err(|_| "exo_package_metadata")?;
    if metadata.dev() != device || metadata.ino() != inode {
        return Err("exo_package_private_changed");
    }
    Ok(())
}
