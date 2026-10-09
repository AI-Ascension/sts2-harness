// SPDX-License-Identifier: MIT

//! Descriptor-confined exact-artifact I/O. Linux publication requires unnamed temporary files;
//! unsupported platforms/filesystems fail closed rather than using path-following fallbacks.

use std::path::Path;

use super::{ExactArtifactStore, ExactCheckpointError};

impl ExactArtifactStore {
    pub(super) fn read_verified(&self, path: &Path) -> Result<Vec<u8>, ExactCheckpointError> {
        if path.strip_prefix(&self.root).is_err() {
            return Err(ExactCheckpointError::InvalidDigest);
        }
        confined::read(path)
    }

    pub(super) fn write_atomic(
        &self,
        path: &Path,
        bytes: &[u8],
    ) -> Result<(), ExactCheckpointError> {
        if path.strip_prefix(&self.root).is_err() {
            return Err(ExactCheckpointError::InvalidDigest);
        }
        confined::write(path, bytes)
    }
}

#[cfg(target_os = "linux")]
mod confined {
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::path::{Component, Path};

    use rustix::fs::{
        AtFlags, Mode, OFlags, PROC_SUPER_MAGIC, fstatfs, linkat, mkdirat, open, openat,
    };
    use rustix::io::Errno;

    use super::super::{MAX_EXACT_BLOB_BYTES, persistence};
    use super::ExactCheckpointError;

    const DIRECTORY: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const PROCESS_DIRECTORY: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::CLOEXEC);
    const READ_FILE: OFlags = OFlags::RDONLY
        .union(OFlags::NOFOLLOW)
        .union(OFlags::NONBLOCK)
        .union(OFlags::CLOEXEC);

    pub(super) fn read(path: &Path) -> Result<Vec<u8>, ExactCheckpointError> {
        let (directories, name) = parent(path, false)?;
        let directory = directories
            .last()
            .ok_or(ExactCheckpointError::InvalidDigest)?;
        let file =
            File::from(openat(directory, name, READ_FILE, Mode::empty()).map_err(read_error)?);
        read_file(&file)
    }

    pub(super) fn write(path: &Path, bytes: &[u8]) -> Result<(), ExactCheckpointError> {
        if bytes.len() > MAX_EXACT_BLOB_BYTES {
            return Err(ExactCheckpointError::Oversized);
        }
        let (directories, name) = parent(path, true)?;
        let directory = directories
            .last()
            .ok_or(ExactCheckpointError::InvalidDigest)?;
        match openat(directory, name, READ_FILE, Mode::empty()) {
            Ok(fd) => return existing(File::from(fd), bytes, directory),
            Err(Errno::NOENT) => {}
            Err(error) => return Err(write_error(error)),
        }
        publish(directory, name, bytes)
    }

    fn publish(directory: &File, name: &OsStr, bytes: &[u8]) -> Result<(), ExactCheckpointError> {
        // The temporary has no pathname an attacker could replace between write and publication.
        let mut temporary = File::from(
            openat(
                directory,
                ".",
                OFlags::WRONLY | OFlags::TMPFILE | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(write_error)?,
        );
        temporary.write_all(bytes).map_err(persistence)?;
        temporary.sync_all().map_err(persistence)?;
        match linkat(&temporary, "", directory, name, AtFlags::EMPTY_PATH) {
            Ok(()) => sync_directory(directory),
            Err(Errno::EXIST) => existing_at(directory, name, bytes),
            Err(Errno::NOENT) => publish_via_procfs(&temporary, directory, name, bytes),
            Err(error) => Err(write_error(error)),
        }
    }

    fn publish_via_procfs(
        temporary: &File,
        directory: &File,
        name: &OsStr,
        bytes: &[u8],
    ) -> Result<(), ExactCheckpointError> {
        let proc_fds = proc_fd_directory()?;
        // Keep the unnamed file open while procfs resolves its kernel-owned descriptor entry.
        let temporary_name = temporary.as_raw_fd().to_string();
        match linkat(
            &proc_fds,
            temporary_name.as_str(),
            directory,
            name,
            AtFlags::SYMLINK_FOLLOW,
        ) {
            Ok(()) => sync_directory(directory),
            Err(Errno::EXIST) => existing_at(directory, name, bytes),
            Err(error) => Err(write_error(error)),
        }
    }

    fn proc_fd_directory() -> Result<File, ExactCheckpointError> {
        let proc_root = File::from(open("/proc", DIRECTORY, Mode::empty()).map_err(write_error)?);
        verify_procfs(&proc_root)?;
        // Resolve only procfs's kernel-owned self entry after verifying the held proc root.
        let process = File::from(
            openat(&proc_root, "self", PROCESS_DIRECTORY, Mode::empty()).map_err(write_error)?,
        );
        let descriptors =
            File::from(openat(&process, "fd", DIRECTORY, Mode::empty()).map_err(write_error)?);
        verify_procfs(&descriptors)?;
        Ok(descriptors)
    }

    fn verify_procfs(directory: &File) -> Result<(), ExactCheckpointError> {
        let filesystem = fstatfs(directory).map_err(write_error)?;
        if filesystem.f_type != PROC_SUPER_MAGIC {
            return Err(persistence(std::io::Error::other(
                "descriptor publication requires procfs",
            )));
        }
        Ok(())
    }

    fn existing_at(
        directory: &File,
        name: &OsStr,
        bytes: &[u8],
    ) -> Result<(), ExactCheckpointError> {
        let file =
            File::from(openat(directory, name, READ_FILE, Mode::empty()).map_err(write_error)?);
        existing(file, bytes, directory)
    }

    fn existing(file: File, bytes: &[u8], directory: &File) -> Result<(), ExactCheckpointError> {
        if read_file(&file)? != bytes {
            return Err(ExactCheckpointError::DigestMismatch);
        }
        file.sync_all().map_err(persistence)?;
        sync_directory(directory)
    }

    fn read_file(mut file: &File) -> Result<Vec<u8>, ExactCheckpointError> {
        let metadata = file.metadata().map_err(persistence)?;
        if !metadata.is_file() {
            return Err(persistence(std::io::Error::other(
                "artifact is not a regular file",
            )));
        }
        if metadata.len() > MAX_EXACT_BLOB_BYTES as u64 {
            return Err(ExactCheckpointError::Oversized);
        }
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_EXACT_BLOB_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(persistence)?;
        if bytes.len() > MAX_EXACT_BLOB_BYTES {
            return Err(ExactCheckpointError::Oversized);
        }
        Ok(bytes)
    }

    fn parent(path: &Path, create: bool) -> Result<(Vec<File>, &OsStr), ExactCheckpointError> {
        let name = path
            .file_name()
            .ok_or(ExactCheckpointError::InvalidDigest)?;
        let parent = path.parent().ok_or(ExactCheckpointError::InvalidDigest)?;
        let mut directories = vec![File::from(
            open(
                if parent.is_absolute() { "/" } else { "." },
                DIRECTORY,
                Mode::empty(),
            )
            .map_err(|error| path_error(error, create))?,
        )];
        for component in parent.components() {
            let name = match component {
                Component::RootDir | Component::CurDir => continue,
                Component::Normal(name) => name,
                _ => return Err(ExactCheckpointError::InvalidDigest),
            };
            if directories.len() >= 256 {
                return Err(ExactCheckpointError::InvalidDigest);
            }
            let directory = directories
                .last()
                .ok_or(ExactCheckpointError::InvalidDigest)?;
            let next = match openat(directory, name, DIRECTORY, Mode::empty()) {
                Ok(next) => next,
                Err(Errno::NOENT) if create => {
                    match mkdirat(directory, name, Mode::RWXU) {
                        Ok(()) => sync_directory(directory)?,
                        Err(Errno::EXIST) => {}
                        Err(error) => return Err(write_error(error)),
                    }
                    openat(directory, name, DIRECTORY, Mode::empty()).map_err(write_error)?
                }
                Err(error) => return Err(path_error(error, create)),
            };
            directories.push(File::from(next));
        }
        Ok((directories, name))
    }

    fn sync_directory(directory: &File) -> Result<(), ExactCheckpointError> {
        directory.sync_all().map_err(persistence)
    }

    fn read_error(error: Errno) -> ExactCheckpointError {
        if error == Errno::NOENT {
            ExactCheckpointError::Missing
        } else {
            persistence(error.into())
        }
    }

    fn path_error(error: Errno, creating: bool) -> ExactCheckpointError {
        if creating {
            write_error(error)
        } else {
            read_error(error)
        }
    }

    fn write_error(error: Errno) -> ExactCheckpointError {
        persistence(error.into())
    }
}

#[cfg(not(target_os = "linux"))]
mod confined {
    use super::{ExactCheckpointError, Path};

    pub(super) fn read(_: &Path) -> Result<Vec<u8>, ExactCheckpointError> {
        Err(ExactCheckpointError::Persistence(
            "confined exact store requires Linux".to_owned(),
        ))
    }

    pub(super) fn write(_: &Path, _: &[u8]) -> Result<(), ExactCheckpointError> {
        Err(ExactCheckpointError::Persistence(
            "confined exact store requires Linux".to_owned(),
        ))
    }
}
