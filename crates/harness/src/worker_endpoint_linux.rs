// SPDX-License-Identifier: MIT

//! Harness-owned consumer of watchdog worker endpoint namespace v1.
//! Resolution has no filesystem effects and does not authorize stale-socket removal.

use crate::worker_bootstrap::WorkerBootstrap;
use std::path::{Component, Path, PathBuf};

/// Rejected static namespace policy. Diagnostics never include the supplied path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EndpointError;

impl std::fmt::Display for EndpointError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("invalid worker endpoint namespace")
    }
}

impl std::error::Error for EndpointError {}

/// Derive a Linux socket path from approved directory policy and the validated
/// startup nonce. Existing endpoints are never inspected, adopted, or deleted.
///
/// # Errors
/// Rejects ambiguous/nonabsolute directories and endpoints exceeding 100 UTF-8 bytes.
pub fn from_bootstrap(
    namespace: &str,
    bootstrap: &WorkerBootstrap,
) -> Result<PathBuf, EndpointError> {
    let path = Path::new(namespace);
    if namespace.len() > 100
        || !path.is_absolute()
        || namespace.contains('\\')
        || namespace.chars().any(char::is_control)
        || namespace
            .split('/')
            .skip(1)
            .any(|part| matches!(part, "" | "." | ".."))
        || path
            .components()
            .any(|part| !matches!(part, Component::RootDir | Component::Normal(_)))
    {
        return Err(EndpointError);
    }
    let selected = path.join(format!(
        "ascension-worker-{}.sock",
        bootstrap.launch_nonce()
    ));
    if selected.as_os_str().len() > 100 {
        return Err(EndpointError);
    }
    Ok(selected)
}
