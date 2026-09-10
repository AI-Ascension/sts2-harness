// SPDX-License-Identifier: MIT

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::super::super::contract::{MAX_STORE_BYTES, PersistedStore};
use super::super::StoreError;

pub(crate) fn persist(path: &Path, state: &PersistedStore) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| StoreError::new("store_encode", error.to_string()))?;
    if bytes.len() > MAX_STORE_BYTES {
        return Err(StoreError::new(
            "store_too_large",
            "workflow store exceeds the supported bound",
        ));
    }
    let temporary = temporary_path(path);
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(io_store_error)?;
    if let Err(error) = write_and_sync(&mut file, &bytes) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(io_store_error(error));
    }
    Ok(())
}

fn write_and_sync(file: &mut File, bytes: &[u8]) -> Result<(), StoreError> {
    file.write_all(bytes).map_err(io_store_error)?;
    file.sync_all().map_err(io_store_error)
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    PathBuf::from(temporary)
}

pub(crate) fn io_store_error(error: io::Error) -> StoreError {
    StoreError::new("store_io", error.to_string())
}
