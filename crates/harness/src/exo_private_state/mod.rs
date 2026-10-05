// SPDX-License-Identifier: MIT
//! Owned private roots for the explicitly selected Exo bridge v2 profile.
//!
//! This is a bridge-level ownership and observational quota boundary. It is not a kernel quota or
//! a sandbox against another process with the same service UID.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::ExoPrivateStatePolicy;

#[cfg(target_os = "linux")]
mod fs;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
mod process;

#[cfg(not(target_os = "linux"))]
mod unsupported;

#[cfg(target_os = "linux")]
pub use linux::GuardedRun;
#[cfg(target_os = "linux")]
pub use process::BridgeChildScope;
#[cfg(not(target_os = "linux"))]
pub use unsupported::{BridgeChildScope, GuardedRun};

/// The per-attempt roots passed only over the bridge's private stdin pipe.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunPaths {
    pub state_root: PathBuf,
    pub cache_root: PathBuf,
    pub temp_root: PathBuf,
    pub config_root: PathBuf,
}

/// Independently checkable identity passed to the isolated executor after ownership is recorded.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorPrivateState {
    pub version: String,
    pub attempt_id: String,
    pub config_digest: String,
    pub policy_digest: String,
    pub policy: ExoPrivateStatePolicy,
    pub service_uid: u32,
    pub process: ProcessIdentity,
    pub paths: RunPaths,
}

/// Linux process identity used for conservative owned-group cleanup after bridge restart.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub boot_id: String,
    pub pid: u32,
    pub parent_pid: u32,
    pub process_group: u32,
    pub session: u32,
    pub start_time_ticks: u64,
    pub uid: u32,
}

pub const OWNER_MARKER_NAME: &str = ".sts2-owner.json";
pub const OWNER_LOCK_NAME: &str = ".sts2-owner.lock";
pub const EXECUTOR_STATE_VERSION: &str = "sts2.exo-private-state-executor-v2";

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
