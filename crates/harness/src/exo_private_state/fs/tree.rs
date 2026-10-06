// SPDX-License-Identifier: MIT

use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;

use rustix::fs::{AtFlags, FileType, Mode, OFlags, openat, statat, unlinkat};

use super::{
    AttemptDirectory, MAX_TREE_DEPTH, MAX_TREE_ENTRIES, PolicyRoot, verify_attempt,
    verify_policy_root, verify_private_directory, verify_private_file,
};

pub(in crate::exo_private_state) fn policy_attempt_names(
    root: &PolicyRoot,
    skip_policy_lock: bool,
) -> Result<Vec<OsString>, &'static str> {
    verify_policy_root(root)?;
    let mut names = Vec::new();
    for name in names_in(&root.file)? {
        if skip_policy_lock && name == OsStr::new(".sts2-policy.lock") {
            continue;
        }
        let stat = statat(&root.file, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "exo_private_tree_changed")?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Err("exo_private_foreign_entry");
        }
        let child = openat(
            &root.file,
            &name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| "exo_private_tree_changed")?;
        verify_private_directory(&child)?;
        let metadata = child.metadata().map_err(|_| "exo_private_tree_changed")?;
        if metadata.dev() != stat.st_dev || metadata.ino() != stat.st_ino {
            return Err("exo_private_tree_changed");
        }
        names.push(name);
        if names.len() > MAX_TREE_ENTRIES {
            return Err("exo_private_entry_bound");
        }
    }
    Ok(names)
}

pub(in crate::exo_private_state) fn scan_policy_root(
    root: &PolicyRoot,
    skip_policy_lock: bool,
    entries: &mut usize,
    total: &mut u64,
    maximum_bytes: u64,
) -> Result<(), &'static str> {
    verify_policy_root(root)?;
    for name in names_in(&root.file)? {
        if skip_policy_lock && name == OsStr::new(".sts2-policy.lock") {
            continue;
        }
        *entries = entries.checked_add(1).ok_or("exo_private_entry_bound")?;
        if *entries > MAX_TREE_ENTRIES {
            return Err("exo_private_entry_bound");
        }
        let stat = statat(&root.file, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "exo_private_tree_changed")?;
        if FileType::from_raw_mode(stat.st_mode) != FileType::Directory {
            return Err("exo_private_foreign_entry");
        }
        let child = openat(
            &root.file,
            &name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(|_| "exo_private_tree_changed")?;
        verify_private_directory(&child)?;
        let metadata = child.metadata().map_err(|_| "exo_private_tree_changed")?;
        if metadata.dev() != stat.st_dev || metadata.ino() != stat.st_ino {
            return Err("exo_private_tree_changed");
        }
        scan_directory(&child, 0, entries, total, maximum_bytes)?;
    }
    Ok(())
}

pub(in crate::exo_private_state) fn scan_run(
    attempts: &[AttemptDirectory],
    maximum_bytes: u64,
) -> Result<u64, &'static str> {
    let mut total = 0_u64;
    let mut entries = 0_usize;
    for attempt in attempts {
        verify_private_directory(&attempt.file)?;
        scan_directory(&attempt.file, 0, &mut entries, &mut total, maximum_bytes)?;
    }
    Ok(total)
}

fn scan_directory(
    directory: &File,
    depth: usize,
    entries: &mut usize,
    total: &mut u64,
    maximum_bytes: u64,
) -> Result<(), &'static str> {
    if depth > MAX_TREE_DEPTH {
        return Err("exo_private_depth_bound");
    }
    for name in names_in(directory)? {
        *entries = entries.checked_add(1).ok_or("exo_private_entry_bound")?;
        if *entries > MAX_TREE_ENTRIES {
            return Err("exo_private_entry_bound");
        }
        let stat = statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "exo_private_tree_changed")?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let child = openat(
                    directory,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(|_| "exo_private_tree_changed")?;
                verify_private_directory(&child)?;
                let metadata = child.metadata().map_err(|_| "exo_private_tree_changed")?;
                if metadata.dev() != stat.st_dev || metadata.ino() != stat.st_ino {
                    return Err("exo_private_tree_changed");
                }
                scan_directory(&child, depth + 1, entries, total, maximum_bytes)?;
            }
            FileType::RegularFile => {
                let file = openat(
                    directory,
                    &name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(|_| "exo_private_tree_changed")?;
                verify_private_file(&file, false)?;
                let metadata = file.metadata().map_err(|_| "exo_private_tree_changed")?;
                if metadata.dev() != stat.st_dev
                    || metadata.ino() != stat.st_ino
                    || metadata.nlink() != 1
                {
                    return Err("exo_private_tree_changed");
                }
                *total = total
                    .checked_add(metadata.len())
                    .ok_or("exo_private_quota")?;
                if *total > maximum_bytes {
                    return Err("exo_private_quota");
                }
            }
            _ => return Err("exo_private_tree_entry"),
        }
    }
    Ok(())
}

pub(in crate::exo_private_state) fn remove_attempt(
    root: &PolicyRoot,
    attempt: &AttemptDirectory,
    maximum_bytes: u64,
) -> Result<(), &'static str> {
    verify_policy_root(root)?;
    verify_attempt(root, attempt)?;
    let mut entries = 0_usize;
    let mut total = 0_u64;
    scan_directory(&attempt.file, 0, &mut entries, &mut total, maximum_bytes)?;
    remove_children(&attempt.file, 0, &mut entries)?;
    unlinkat(&root.file, &attempt.name, AtFlags::REMOVEDIR).map_err(|_| "exo_private_cleanup")
}

pub(in crate::exo_private_state) fn remove_children(
    directory: &File,
    depth: usize,
    entries: &mut usize,
) -> Result<(), &'static str> {
    if depth > MAX_TREE_DEPTH {
        return Err("exo_private_depth_bound");
    }
    for name in names_in(directory)? {
        *entries = entries.checked_add(1).ok_or("exo_private_entry_bound")?;
        if *entries > MAX_TREE_ENTRIES * 2 {
            return Err("exo_private_entry_bound");
        }
        let stat = statat(directory, &name, AtFlags::SYMLINK_NOFOLLOW)
            .map_err(|_| "exo_private_tree_changed")?;
        match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => {
                let child = openat(
                    directory,
                    &name,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(|_| "exo_private_tree_changed")?;
                let metadata = child.metadata().map_err(|_| "exo_private_tree_changed")?;
                if metadata.dev() != stat.st_dev || metadata.ino() != stat.st_ino {
                    return Err("exo_private_tree_changed");
                }
                verify_private_directory(&child)?;
                remove_children(&child, depth + 1, entries)?;
                unlinkat(directory, &name, AtFlags::REMOVEDIR)
                    .map_err(|_| "exo_private_cleanup")?;
            }
            FileType::RegularFile => {
                let file = openat(
                    directory,
                    &name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                )
                .map(File::from)
                .map_err(|_| "exo_private_tree_changed")?;
                let metadata = file.metadata().map_err(|_| "exo_private_tree_changed")?;
                if metadata.dev() != stat.st_dev || metadata.ino() != stat.st_ino {
                    return Err("exo_private_tree_changed");
                }
                verify_private_file(&file, false)?;
                unlinkat(directory, &name, AtFlags::empty()).map_err(|_| "exo_private_cleanup")?;
            }
            _ => return Err("exo_private_tree_entry"),
        }
    }
    Ok(())
}

fn names_in(directory: &File) -> Result<Vec<OsString>, &'static str> {
    let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
    let entries = fs::read_dir(path).map_err(|_| "exo_private_tree_read")?;
    let mut names = Vec::with_capacity(MAX_TREE_ENTRIES);
    for entry in entries {
        if names.len() >= MAX_TREE_ENTRIES {
            return Err("exo_private_entry_bound");
        }
        names.push(entry.map_err(|_| "exo_private_tree_read")?.file_name());
    }
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::names_in;
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::unix::fs::DirBuilderExt;

    #[test]
    fn oversized_directory_is_refused_during_enumeration() -> io::Result<()> {
        let root = std::env::temp_dir().join(format!(
            "sts2-private-tree-bound-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        for index in 0..=super::MAX_TREE_ENTRIES {
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(root.join(index.to_string()))?;
        }

        let result = names_in(&File::open(&root)?);
        fs::remove_dir_all(root)?;
        assert_eq!(result.err(), Some("exo_private_entry_bound"));
        Ok(())
    }
}
