// SPDX-License-Identifier: MIT

//! Bounded, operator-visible diagnostics for an Exo transport child that failed or could not start.
//!
//! Everything here is deliberately separate from the exchange itself: a diagnostic must never
//! change the transport verdict, and the bytes it prints come from an untrusted child.

use std::process::ExitStatus;

use tokio::io::{AsyncRead, AsyncReadExt};

/// Bounded tail of a failed child's stderr kept for the operator-visible diagnostic.
pub(super) const MAX_CHILD_STDERR_BYTES: usize = 4_096;

/// Reads a child stream to its end and keeps only the last `maximum` bytes.
///
/// The tail is a diagnostic, so it must neither fail the exchange nor grow without bound, and the
/// stream has to keep draining or a verbose child cannot exit.
pub(super) async fn read_tail(mut stream: impl AsyncRead + Unpin, maximum: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0_u8; 4_096];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return kept,
            Ok(count) => {
                kept.extend_from_slice(&chunk[..count]);
                if kept.len() > maximum {
                    let excess = kept.len() - maximum;
                    kept.drain(..excess);
                }
            }
        }
    }
}

/// Names why a transport child failed, using only what is bounded and safe to print.
pub(super) fn child_failure_diagnostic(status: &ExitStatus, tail: &[u8]) -> String {
    let tail = String::from_utf8_lossy(tail);
    let tail = tail.trim_end();
    if tail.is_empty() {
        format!("exo transport child failed with {status} and wrote no stderr")
    } else {
        format!("exo transport child failed with {status}; stderr tail:\n{tail}")
    }
}

/// Names why a transport child could not be started at all.
pub(super) fn start_failure_diagnostic(error: &std::io::Error) -> String {
    format!("exo transport child could not be started: {error}")
}
