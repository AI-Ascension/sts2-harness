// SPDX-License-Identifier: MIT

//! Executable configuration and gameplay adapter for the library-owned worker admission state.

use super::super::config::RuntimeConfig;
use super::super::worker_settings::WorkerSettings;
use super::worker_store::share_store;
use sts2_harness::worker_runtime::WorkerRuntime as WorkerCore;
use sts2_harness::{ExecutionStore, ExecutionStoreConfig};

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
}

/// Starts worker mode with a stopped durable boot and a native authenticated command loop.
pub(crate) fn run(config: RuntimeConfig) -> Result<(), String> {
    let bootstrap = read_bootstrap()?;
    let settings = WorkerSettings::from_environment(&config, &bootstrap)?;
    let runtime = WorkerRuntime::open(settings)?;
    #[cfg(target_os = "linux")]
    let mut runtime = runtime;
    #[cfg(target_os = "linux")]
    let result = linux::run(&mut runtime, config, &bootstrap);
    #[cfg(not(target_os = "linux"))]
    let result: Result<(), String> = Err(String::from(
        "native worker listener is not integrated on this platform",
    ));
    let close = runtime.close();
    match close {
        Ok(()) => result,
        Err(error) => match result {
            Ok(()) => Err(error),
            Err(original) => Err(format!("{original}; {error}")),
        },
    }
}

fn read_bootstrap() -> Result<sts2_harness::worker_bootstrap::WorkerBootstrap, String> {
    #[cfg(target_os = "linux")]
    {
        sts2_harness::worker_bootstrap_linux::read_stdin().map_err(|error| error.to_string())
    }
    #[cfg(not(target_os = "linux"))]
    {
        Err(String::from(
            "native worker bootstrap transport is not integrated on this platform",
        ))
    }
}

#[cfg(target_os = "linux")]
#[path = "runtime_v3_worker_runtime_execution.rs"]
mod execution;

#[cfg(target_os = "linux")]
#[path = "runtime_v3_worker_linux.rs"]
mod linux;
