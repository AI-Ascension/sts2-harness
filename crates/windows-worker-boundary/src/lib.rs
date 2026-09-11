// SPDX-License-Identifier: MIT

//! Small Windows-only boundary for the harness worker endpoint.
//!
//! The parent `sts2-harness` package keeps its coordination and worker
//! admission logic free of FFI.  This package owns the reviewed Win32 handle,
//! named-pipe, process-identity, and protected-file operations required by
//! that adapter.  It exposes bounded, typed operations rather than raw
//! handles or Windows structs.

#![cfg(windows)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]

mod credential;
mod handle;
mod identity;
mod image;
mod pipe;
mod pipe_stream;
mod stdin;

pub use credential::read_protected_credential;
pub use identity::PeerExpectation;
pub use image::VerifiedExecutable;
pub use pipe::WorkerPipeListener;
pub use pipe_stream::WorkerPipeStream;
pub use stdin::read_bootstrap_stdin;

pub(crate) fn hex_bytes(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}
