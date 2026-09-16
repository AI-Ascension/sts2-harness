// SPDX-License-Identifier: MIT
//! Opt-in private SQLite opening for runtime-owned policy/corpus stores.
//!
//! The parent directory is opened component-by-component without following links and must be
//! owned/private at the store boundary. SQLite then opens the already checked leaf with
//! SQLITE_OPEN_NOFOLLOW. The retained directory and file descriptors are revalidated before
//! owner-store operations. This protects against other-UID pathname replacement; a hostile
//! process running as the same UID or an administrator is outside this boundary.

#[cfg(unix)]
mod unix {
    use rusqlite::{Connection, OpenFlags};
    use rustix::fs::{Mode, OFlags, open, openat};
    use rustix::io::Errno;
    use std::ffi::OsString;
    use std::fs::File;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::{Component, Path, PathBuf};

    const PRIVATE_MODE_MASK: u32 = 0o077;
    const WRITABLE_MODE_MASK: u32 = 0o022;
    const STICKY_MODE: u32 = 0o1000;
    const OWNER_MODE_MASK: u32 = 0o777;

    pub(crate) struct PrivateSqliteGuard {
        directory: File,
        database: File,
        directory_path: PathBuf,
        name: OsString,
        max_bytes: u64,
    }

    impl PrivateSqliteGuard {
        pub(crate) fn open(path: &Path, max_bytes: u64) -> Result<(Connection, Self), ()> {
            if !path.is_absolute() || max_bytes == 0 {
                return Err(());
            }
            let directory_path = path.parent().ok_or(())?.to_owned();
            let name = path.file_name().ok_or(())?.to_owned();
            if name.is_empty() || name == "." || name == ".." {
                return Err(());
            }
            let directory = open_private_directory(&directory_path)?;
            let database = open_database(&directory, &name, max_bytes)?;
            let guard = Self {
                directory,
                database,
                directory_path,
                name,
                max_bytes,
            };
            guard.check_sidecars()?;

            // NOFOLLOW is essential: SQLite must not follow a swapped leaf symlink. The parent
            // directory is retained/private, so other UIDs cannot replace this leaf or its
            // journal siblings between this preflight and SQLite's pathname open.
            let connection = Connection::open_with_flags(
                path,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_NOFOLLOW,
            )
            .map_err(|_| ())?;
            guard.verify()?;
            Ok((connection, guard))
        }

        pub(crate) fn verify(&self) -> Result<(), ()> {
            let current_directory = open_private_directory(&self.directory_path)?;
            if !same_inode(&current_directory, &self.directory)? {
                return Err(());
            }
            let current_database = open_database(&self.directory, &self.name, self.max_bytes)?;
            if !same_inode(&current_database, &self.database)? {
                return Err(());
            }
            self.check_sidecars()
        }

        fn check_sidecars(&self) -> Result<(), ()> {
            for suffix in ["-journal", "-wal", "-shm"] {
                let mut name = self.name.clone();
                name.push(suffix);
                match openat(
                    &self.directory,
                    &name,
                    OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
                    Mode::empty(),
                ) {
                    Ok(fd) => {
                        let file = File::from(fd);
                        check_private_regular(&file, self.max_bytes)?;
                    }
                    Err(Errno::NOENT) => {}
                    Err(_) => return Err(()),
                }
            }
            Ok(())
        }
    }

    fn open_private_directory(path: &Path) -> Result<File, ()> {
        if !path.is_absolute() {
            return Err(());
        }
        let mut directory = File::from(
            open(
                "/",
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| ())?,
        );
        let components = path.components().collect::<Vec<_>>();
        if components.is_empty() || components[0] != Component::RootDir {
            return Err(());
        }
        let normal = components
            .iter()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(*value),
                Component::RootDir => None,
                _ => Some(std::ffi::OsStr::new("")),
            })
            .collect::<Vec<_>>();
        if normal.is_empty() || normal.iter().any(|component| component.is_empty()) {
            return Err(());
        }
        let effective_uid = rustix::process::geteuid().as_raw();
        for (index, component) in normal.iter().enumerate() {
            let next = File::from(
                openat(
                    &directory,
                    *component,
                    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .map_err(|_| ())?,
            );
            let metadata = next.metadata().map_err(|_| ())?;
            let mode = metadata.permissions().mode();
            let final_parent = index + 1 == normal.len();
            if !metadata.is_dir()
                || (final_parent
                    && (metadata.uid() != effective_uid
                        || mode & OWNER_MODE_MASK != 0o700
                        || mode & 0o7000 != 0))
                || (!final_parent && metadata.uid() != effective_uid && metadata.uid() != 0)
                || (mode & WRITABLE_MODE_MASK != 0
                    && !(metadata.uid() == 0 && mode & STICKY_MODE != 0 && !final_parent))
            {
                return Err(());
            }
            directory = next;
        }
        Ok(directory)
    }

    fn open_database(directory: &File, name: &std::ffi::OsStr, max_bytes: u64) -> Result<File, ()> {
        let file = File::from(
            openat(
                directory,
                name,
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC
                    | OFlags::NONBLOCK,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| ())?,
        );
        check_private_regular(&file, max_bytes)?;
        let mode = file.metadata().map_err(|_| ())?.permissions().mode();
        if mode & OWNER_MODE_MASK != 0o600 || mode & 0o7000 != 0 {
            return Err(());
        }
        Ok(file)
    }

    fn check_private_regular(file: &File, max_bytes: u64) -> Result<(), ()> {
        let metadata = file.metadata().map_err(|_| ())?;
        if !metadata.is_file()
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & PRIVATE_MODE_MASK != 0
            || metadata.permissions().mode() & 0o7000 != 0
            || metadata.len() > max_bytes
        {
            return Err(());
        }
        Ok(())
    }

    fn same_inode(left: &File, right: &File) -> Result<bool, ()> {
        let left = left.metadata().map_err(|_| ())?;
        let right = right.metadata().map_err(|_| ())?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
}

#[cfg(unix)]
pub(crate) use unix::PrivateSqliteGuard;

#[cfg(not(unix))]
pub(crate) struct PrivateSqliteGuard;

#[cfg(not(unix))]
impl PrivateSqliteGuard {
    pub(crate) fn open(
        _path: &std::path::Path,
        _max_bytes: u64,
    ) -> Result<(rusqlite::Connection, Self), ()> {
        Err(())
    }

    pub(crate) fn verify(&self) -> Result<(), ()> {
        Err(())
    }
}

#[cfg(all(test, unix))]
#[path = "private_sqlite_tests.rs"]
mod tests;
