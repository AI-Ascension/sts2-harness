// SPDX-License-Identifier: MIT

use crate::ExoPrivateStatePolicy;

use super::{ExecutorPrivateState, RunPaths};
use std::sync::OnceLock;

pub struct BridgeChildScope;

impl BridgeChildScope {
    pub fn enable() -> Result<Self, &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn drain(&self) -> Result<bool, &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn confirm_spawn_error(&self) -> Result<(), &'static str> {
        Err("exo_private_platform_unsupported")
    }
}

pub struct GuardedRun {
    _private: (),
}

static EMPTY_PATHS: OnceLock<RunPaths> = OnceLock::new();

impl GuardedRun {
    pub fn create(
        _policy: &ExoPrivateStatePolicy,
        _config_digest: &str,
    ) -> Result<Self, &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn paths(&self) -> &RunPaths {
        EMPTY_PATHS.get_or_init(|| RunPaths {
            state_root: std::path::PathBuf::new(),
            cache_root: std::path::PathBuf::new(),
            temp_root: std::path::PathBuf::new(),
            config_root: std::path::PathBuf::new(),
        })
    }

    pub const fn policy_digest(&self) -> &str {
        ""
    }

    pub const fn attempt_id(&self) -> &str {
        ""
    }

    pub fn begin_spawn(&mut self) -> Result<(), &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn spawn_failed(&mut self, _child_scope: &BridgeChildScope) -> Result<(), &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn record_child(&mut self, _pid: u32) -> Result<ExecutorPrivateState, &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn verify_quota(&self) -> Result<u64, &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn finish_child(&mut self, _child_scope: &BridgeChildScope) -> Result<bool, &'static str> {
        Err("exo_private_platform_unsupported")
    }

    pub fn finish(&mut self) -> Result<(), &'static str> {
        Err("exo_private_platform_unsupported")
    }
}
