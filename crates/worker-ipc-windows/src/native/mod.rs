// SPDX-License-Identifier: MIT

//! Private Windows FFI boundary.
//!
//! Unsafe calls are isolated in the child modules below. Each module keeps
//! raw pointers private, documents the owned resource that outlives every
//! call, and returns only typed safe values to the public transport wrapper.
//!
//! The native invariants are: owned handles are closed exactly once; pointers
//! are borrowed only while their typed owner remains alive; ACL storage uses a
//! four-byte-aligned allocation; token buffers use an eight-byte-aligned
//! allocation; UTF-16 arguments are NUL terminated and bounded; DWORD byte
//! lengths are checked before every call; and every overlapped owner remains
//! heap-stable until successful completion or confirmed cancellation. Native
//! errors are captured immediately and reduced to the fixed safe categories.

#![allow(unsafe_code)]

mod connection;
mod io;
mod io_operation;
mod listener;
mod process;
mod resources;
mod security;

#[cfg(test)]
mod test_peer;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;

pub(crate) use listener::{Connection, Listener};

pub(crate) const MAX_PATH_UTF16: usize = 32_768;
pub(crate) const MAX_HASH_CHUNK: usize = 32 * 1024;
pub(crate) const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;

pub(crate) const ACE_ALLOW: u8 = 0;
pub(crate) const ACE_DENY: u8 = 1;
pub(crate) const INHERITED_ACE: u8 = 16;
pub(crate) const GENERIC_WRITE: u32 = 0x4000_0000;
pub(crate) const GENERIC_ALL: u32 = 0x1000_0000;
pub(crate) const FILE_WRITE_DATA: u32 = 0x0000_0002;
pub(crate) const FILE_APPEND_DATA: u32 = 0x0000_0004;
pub(crate) const FILE_WRITE_EA: u32 = 0x0000_0010;
pub(crate) const FILE_WRITE_ATTRIBUTES: u32 = 0x0000_0100;
pub(crate) const FILE_DELETE: u32 = 0x0001_0000;
pub(crate) const FILE_WRITE_DAC: u32 = 0x0004_0000;
pub(crate) const FILE_WRITE_OWNER: u32 = 0x0008_0000;

pub(super) fn wide_string(value: &str) -> Result<Vec<u16>, crate::transport::TransportError> {
    if value.is_empty() || value.contains('\0') {
        return Err(crate::transport::TransportError::Configuration);
    }
    let mut wide: Vec<u16> = value.encode_utf16().collect();
    if wide.len() >= MAX_PATH_UTF16 {
        return Err(crate::transport::TransportError::Configuration);
    }
    wide.push(0);
    Ok(wide)
}
