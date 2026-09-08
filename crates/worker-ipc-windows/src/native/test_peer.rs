// SPDX-License-Identifier: MIT
#![allow(unsafe_code)]

//! Finite named-pipe peer used by native transport tests.

use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::ptr::{addr_of_mut, null, null_mut};
use std::thread;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    FALSE, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING, ReadFile, WriteFile,
};

use super::resources::Handle;
use super::test_support::pipe_name;
use crate::{AUTH_MAGIC, MAX_FRAME_BYTES, TransportError};

pub(super) fn spawn_peer(
    mode: &str,
    endpoint_nonce: [u8; 16],
    credential: &Path,
) -> Result<Child, TransportError> {
    let executable = std::env::current_exe().map_err(|_| TransportError::Os)?;
    let credential = credential.to_str().ok_or(TransportError::Configuration)?;
    Command::new(executable)
        .args(["--nocapture", "--exact", "native::tests::peer_client"])
        .env("WORKER_IPC_TEST_PIPE_NAME", pipe_name(endpoint_nonce))
        .env("WORKER_IPC_TEST_CREDENTIAL_PATH", credential)
        .env("WORKER_IPC_TEST_PEER_MODE", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| TransportError::Os)
}

fn write_sync(handle: HANDLE, bytes: &[u8]) -> bool {
    let Ok(length) = u32::try_from(bytes.len()) else {
        return false;
    };
    let mut written = 0_u32;
    unsafe {
        WriteFile(
            handle,
            bytes.as_ptr(),
            length,
            addr_of_mut!(written),
            null_mut(),
        ) != FALSE
            && written == length
    }
}

fn read_sync(handle: HANDLE, bytes: &mut [u8]) -> bool {
    let Ok(length) = u32::try_from(bytes.len()) else {
        return false;
    };
    let mut read = 0_u32;
    unsafe {
        ReadFile(
            handle,
            bytes.as_mut_ptr(),
            length,
            addr_of_mut!(read),
            null_mut(),
        ) != FALSE
            && read == length
    }
}

fn client_handle(name: &str) -> Option<Handle> {
    let wide = super::wide_string(name).ok()?;
    for _ in 0..200 {
        let raw = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            )
        };
        if !raw.is_null() && raw != INVALID_HANDLE_VALUE {
            return Handle::new(raw).ok();
        }
        thread::sleep(Duration::from_millis(5));
    }
    None
}

pub(super) fn peer_exchange(mode: &str, name: &str, credential: &Path) -> bool {
    let Some(handle) = client_handle(name) else {
        return mode == "connect-only";
    };
    if mode == "connect-only" {
        return false;
    }
    let Ok(secret) = fs::read(credential) else {
        return false;
    };
    let mut auth = Vec::with_capacity(AUTH_MAGIC.len() + secret.len());
    auth.extend_from_slice(AUTH_MAGIC);
    auth.extend_from_slice(&secret);
    let auth_length = (auth.len() as u32).to_be_bytes();
    match mode {
        "oversized-auth" => write_sync(handle.raw(), &u32::MAX.to_be_bytes()),
        "wrong-magic" => {
            auth[..AUTH_MAGIC.len()].fill(0);
            write_sync(handle.raw(), &auth_length) && write_sync(handle.raw(), &auth)
        }
        "slow-auth" => write_sync(handle.raw(), &[0]),
        "slow-frame" => {
            let request_length = 2_u32.to_be_bytes();
            write_sync(handle.raw(), &auth_length)
                && write_sync(handle.raw(), &auth)
                && write_sync(handle.raw(), &request_length[..1])
                && {
                    thread::sleep(Duration::from_millis(300));
                    true
                }
        }
        "connect-only" => false,
        _ => {
            let request = b"{}";
            let request_length = (request.len() as u32).to_be_bytes();
            if !write_sync(handle.raw(), &auth_length)
                || !write_sync(handle.raw(), &auth)
                || !write_sync(handle.raw(), &request_length)
                || !write_sync(handle.raw(), request)
            {
                return false;
            }
            let mut response_length = [0_u8; 4];
            if !read_sync(handle.raw(), &mut response_length) {
                return mode != "valid";
            }
            let length = u32::from_be_bytes(response_length) as usize;
            if length == 0 || length > MAX_FRAME_BYTES {
                return false;
            }
            let mut response = vec![0_u8; length];
            read_sync(handle.raw(), &mut response)
        }
    }
}
