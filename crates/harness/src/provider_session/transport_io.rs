// SPDX-License-Identifier: MIT

use super::super::protocol::{NativeResponse, parse_native_frame};
use super::{NativeFrame, NativeTransportError, OwnedNativeTransport};
use std::io::{BufRead, Write};

impl OwnedNativeTransport {
    pub(super) fn write_frame(&mut self, frame: &NativeFrame) -> Result<(), NativeTransportError> {
        let bytes = frame
            .encode_line()
            .map_err(|_| NativeTransportError::Protocol)?;
        if bytes.len() > self.config.max_frame_bytes {
            return Err(NativeTransportError::Capacity);
        }
        let writer = self.writer.as_mut().ok_or(NativeTransportError::Closed)?;
        writer
            .write_all(&bytes)
            .map_err(|_| NativeTransportError::Unavailable)?;
        writer
            .flush()
            .map_err(|_| NativeTransportError::Unavailable)
    }

    pub(super) fn read_until(
        &mut self,
        request_id: u64,
    ) -> Result<serde_json::Value, NativeTransportError> {
        loop {
            let mut line = Vec::new();
            let reader = self.reader.as_mut().ok_or(NativeTransportError::Closed)?;
            let count = reader
                .read_until(b'\n', &mut line)
                .map_err(|_| NativeTransportError::Unavailable)?;
            if count == 0 {
                return Err(NativeTransportError::Ambiguous);
            }
            if line.len() > self.config.max_frame_bytes {
                self.fenced = true;
                return Err(NativeTransportError::Capacity);
            }
            match parse_native_frame(&line) {
                Ok(NativeResponse::Result { id, value }) if id == request_id => return Ok(value),
                Ok(NativeResponse::Error { id, error }) if id == Some(request_id) => {
                    return Err(if error.code == -32001 {
                        NativeTransportError::UnauthorizedServerRequest
                    } else {
                        NativeTransportError::Protocol
                    });
                }
                Ok(NativeResponse::Notification { sequence }) => {
                    let is_new = sequence
                        .is_some_and(|sequence| self.notification_sequences.insert(sequence));
                    if is_new {
                        self.notifications = self.notifications.saturating_add(1);
                        if self.notifications > super::MAX_OUTSTANDING * 16 {
                            self.fenced = true;
                            return Err(NativeTransportError::Capacity);
                        }
                    }
                }
                Ok(NativeResponse::ServerRequest { id, method: _ }) => {
                    self.fenced = true;
                    let denial = NativeFrame::error(Some(id), -32601, "server request denied");
                    let _ = self.write_frame(&denial);
                    return Err(NativeTransportError::UnauthorizedServerRequest);
                }
                Ok(NativeResponse::Result { .. }) | Ok(NativeResponse::Error { .. }) => {
                    self.fenced = true;
                    return Err(NativeTransportError::Protocol);
                }
                Err(_) => {
                    self.fenced = true;
                    return Err(NativeTransportError::Protocol);
                }
            }
        }
    }
}

pub(super) fn safe_directory(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.is_dir() && restricted_metadata(&metadata) && !has_symlink_component(path)
    })
}

pub(super) fn safe_executable(path: &std::path::Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| {
        metadata.file_type().is_file()
            && restricted_executable_metadata(&metadata)
            && !has_symlink_component(path)
    })
}

#[cfg(unix)]
fn restricted_metadata(metadata: &std::fs::Metadata) -> bool {
    use rustix::process::geteuid;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == geteuid().as_raw() && metadata.permissions().mode() & 0o077 == 0
}

#[cfg(not(unix))]
fn restricted_metadata(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn restricted_executable_metadata(metadata: &std::fs::Metadata) -> bool {
    use rustix::process::geteuid;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    metadata.uid() == geteuid().as_raw() && metadata.permissions().mode() & 0o022 == 0
}

#[cfg(not(unix))]
fn restricted_executable_metadata(_metadata: &std::fs::Metadata) -> bool {
    false
}

pub(super) fn has_symlink_component(path: &std::path::Path) -> bool {
    let mut current = std::path::PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return true;
        }
    }
    false
}
