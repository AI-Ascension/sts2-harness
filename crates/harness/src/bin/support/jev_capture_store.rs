// SPDX-License-Identifier: MIT

//! Create-only, bounded local sidecars. Capture requires an existing private Unix directory.

use super::Error;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;

const MAX_SLOTS: usize = 4096;
const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_BRIDGE_BYTES: u64 = 128 * 1024 * 1024;

pub(super) struct Reservation {
    directory: PathBuf,
    slot: usize,
}

impl Reservation {
    pub(super) fn new(directory: &str, pending: &Value) -> Result<Self, Error> {
        Self::reserve(directory, pending, MAX_SLOTS)
    }

    fn reserve(directory: &str, pending: &Value, maximum: usize) -> Result<Self, Error> {
        let directory = private_directory(directory)?;
        let bytes = encoded(pending)?;
        for slot in 0..maximum.min(MAX_SLOTS) {
            let path = directory.join(format!("attempt-{slot:04}.pending.json"));
            let options = private_options()?;
            match options.open(path) {
                Ok(mut file) => {
                    // An occupied final path must be refused BEFORE a provider is called.
                    let final_path = directory.join(format!("attempt-{slot:04}.result.json"));
                    match std::fs::symlink_metadata(final_path) {
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        _ => return Err(Error::Storage),
                    }
                    write_record(&mut file, &bytes)?;
                    File::open(&directory)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| Error::Storage)?;
                    return Ok(Self { directory, slot });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err(Error::Storage),
            }
        }
        Err(Error::Quota)
    }

    pub(super) fn finish(&self, result: &Value) -> Result<(), Error> {
        let bytes = encoded(result)?;
        let path = self
            .directory
            .join(format!("attempt-{:04}.result.json", self.slot));
        let mut file = private_options()?.open(path).map_err(|_| Error::Storage)?;
        write_record(&mut file, &bytes)?;
        File::open(&self.directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::Storage)
    }
}

fn encoded(record: &Value) -> Result<Vec<u8>, Error> {
    let mut bytes = serde_json::to_vec(record).map_err(|_| Error::Evidence)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(Error::Evidence);
    }
    Ok(bytes)
}

fn write_record(file: &mut File, bytes: &[u8]) -> Result<(), Error> {
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| Error::Storage)
}

#[cfg(unix)]
fn private_options() -> Result<OpenOptions, Error> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true).mode(0o600);
    Ok(options)
}

#[cfg(not(unix))]
fn private_options() -> Result<OpenOptions, Error> {
    Err(Error::Platform)
}

#[cfg(unix)]
fn private_directory(directory: &str) -> Result<PathBuf, Error> {
    use std::os::unix::fs::PermissionsExt as _;
    let path = Path::new(directory);
    let actual = std::fs::canonicalize(path).map_err(|_| Error::Storage)?;
    let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Storage)?;
    if !path.is_absolute()
        || actual != path
        || !metadata.is_dir()
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(Error::Storage);
    }
    Ok(actual)
}

#[cfg(not(unix))]
fn private_directory(_directory: &str) -> Result<PathBuf, Error> {
    Err(Error::Platform)
}

pub(super) fn bridge_digest() -> Result<String, Error> {
    let path = std::env::current_exe().map_err(|_| Error::Storage)?;
    let mut file = File::open(path)
        .map_err(|_| Error::Storage)?
        .take(MAX_BRIDGE_BYTES + 1);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 65536];
    let mut length = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| Error::Storage)?;
        if count == 0 {
            break;
        }
        length += count as u64;
        if length > MAX_BRIDGE_BYTES {
            return Err(Error::Storage);
        }
        hasher.update(&buffer[..count]);
    }
    if length == 0 {
        return Err(Error::Storage);
    }
    Ok(sts2_harness::hex_bytes(hasher.finalize()))
}

#[cfg(all(test, unix))]
#[path = "jev_capture_store_tests.rs"]
mod tests;
