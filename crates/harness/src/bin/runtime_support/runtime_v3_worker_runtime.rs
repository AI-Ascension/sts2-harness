// SPDX-License-Identifier: MIT

//! Executable configuration and gameplay adapter for the library-owned worker admission state.

use super::super::config::RuntimeConfig;
use super::super::worker_settings::WorkerSettings;
use super::worker_store::share_store;
use sts2_harness::worker_runtime::WorkerRuntime as WorkerCore;
use sts2_harness::{ExecutionStore, ExecutionStoreConfig};

pub(super) const MISSING_TRANSPORT_ERROR: &str =
    "worker transport unavailable: authenticated native worker listener is not integrated";

pub struct WorkerRuntime {
    core: WorkerCore,
}

impl std::ops::Deref for WorkerRuntime {
    type Target = WorkerCore;
    fn deref(&self) -> &Self::Target {
        &self.core
    }
}
impl std::ops::DerefMut for WorkerRuntime {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.core
    }
}
impl WorkerRuntime {
    /// Opens the one store and persists a stopped, non-admitting worker boot before any listener
    /// could accept a dispatch.  No episode, job, or handoff is claimed by this constructor.
    pub fn open(settings: WorkerSettings) -> Result<Self, String> {
        let store_config =
            ExecutionStoreConfig::new(settings.store_path).with_approved_recovery_schema();
        let mut store = ExecutionStore::open(store_config)
            .map_err(|_| String::from("cannot open worker execution store"))?;
        if let Err(error) = store.start_worker_boot(&settings.boot) {
            let _ = store.close();
            return Err(format!("cannot persist worker boot: {error}"));
        }
        WorkerCore::from_shared_store(
            share_store(store),
            settings.command,
            settings.fingerprint,
            settings.boot.worker_boot_id,
        )
        .map(|core| Self { core })
    }

    fn transport_unavailable(&self) -> Result<(), String> {
        Err(String::from(MISSING_TRANSPORT_ERROR))
    }
}

/// Starts worker mode, persists its stopped boot, and fails closed until a native authenticated
/// listener is supplied by the platform-specific transport integration.
pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    let settings = WorkerSettings::from_environment(&config)?;
    let runtime = WorkerRuntime::open(settings)?;
    let result = runtime.transport_unavailable();
    let close = runtime.close();
    match close {
        Ok(()) => result,
        Err(error) => Err(format!("{MISSING_TRANSPORT_ERROR}; {error}")),
    }
}

fn combine_failure(original: String, quarantine: Result<(), String>) -> String {
    match quarantine {
        Ok(()) => original,
        Err(error) => format!("{original}; failed to retain unknown worker handoff: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::MISSING_TRANSPORT_ERROR;
    #[test]
    fn missing_transport_is_a_fixed_production_failure() {
        assert_eq!(
            MISSING_TRANSPORT_ERROR,
            "worker transport unavailable: authenticated native worker listener is not integrated"
        );
    }
}

#[path = "runtime_v3_worker_runtime_execution.rs"]
mod execution;
