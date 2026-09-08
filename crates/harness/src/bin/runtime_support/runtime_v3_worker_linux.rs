// SPDX-License-Identifier: MIT

//! Native command loop and its one scoped execution owner.

use std::future::Future;
use std::thread::{Scope, ScopedJoinHandle};
use std::time::Duration;
use sts2_harness::worker_frame_io::ConnectionDeadline;
use sts2_harness::worker_linux_exchange::{LinuxWorkerExchange, WorkerEndpointPolicy};
use sts2_harness::worker_local_linux::{LinuxTransportError, LinuxWorkerListener};
use sts2_harness::worker_runtime::{WorkerExecutionCompletion, WorkerStartOutcome};
use sts2_harness::worker_runtime_store::{SharedExecutionStore, begin_quarantine};

use super::super::super::config::RuntimeConfig;
use super::super::super::runtime_v3_settings::RuntimeV3Settings;
use super::super::launch_options::RuntimeV3LaunchOptions;
use super::WorkerRuntime;

#[path = "runtime_v3_worker_linux_settings.rs"]
mod settings;
use settings::ListenerSettings;

const REAP_INTERVAL: Duration = Duration::from_millis(25);

struct ExecutionInputs {
    config: RuntimeConfig,
    settings: RuntimeV3Settings,
    options: RuntimeV3LaunchOptions,
}

/// On unwinding, close admission before the enclosing scope joins execution.
/// This is a safety fence, not proof that an in-flight effect was cancelled.
struct ExitFence {
    store: SharedExecutionStore,
    cancellation: sts2_harness::ExecutionCancellation,
}

impl Drop for ExitFence {
    fn drop(&mut self) {
        let _ = begin_quarantine(&self.store);
        self.cancellation.cancel();
    }
}

pub(super) fn run(
    worker: &mut WorkerRuntime,
    config: RuntimeConfig,
    bootstrap: &sts2_harness::worker_bootstrap::WorkerBootstrap,
) -> Result<(), String> {
    let listener = ListenerSettings::from_environment(bootstrap)?;
    let inputs = ExecutionInputs {
        config,
        settings: RuntimeV3Settings::from_environment()?,
        options: RuntimeV3LaunchOptions::from_environment()?,
    };
    super::super::durable::validate_worker_configuration(
        &inputs.config,
        &inputs.settings,
        worker.fingerprint(),
    )?;
    let executor = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| String::from("cannot start worker command executor"))?;
    std::thread::scope(|scope| {
        executor.block_on(async {
            // Register shutdown before binding. A partially configured process
            // must never expose an endpoint without its shutdown owner.
            let mut terminate =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .map_err(|_| String::from("cannot register worker termination signal"))?;
            let mut interrupt =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())
                    .map_err(|_| String::from("cannot register worker interrupt signal"))?;
            let endpoint = listener
                .transport
                .bind()
                .map_err(|error| error.to_string())?;
            let shutdown = async {
                tokio::select! {
                    _ = terminate.recv() => {},
                    _ = interrupt.recv() => {},
                }
            };
            serve(
                scope,
                worker,
                &endpoint,
                &inputs,
                listener.exchange_timeout,
                shutdown,
            )
            .await
        })
    })
}

async fn serve<'scope, 'env>(
    scope: &'scope Scope<'scope, 'env>,
    worker: &mut WorkerRuntime,
    listener: &LinuxWorkerListener,
    inputs: &ExecutionInputs,
    timeout: Duration,
    shutdown: impl Future<Output = ()>,
) -> Result<(), String> {
    let mut exit_fence = ExitFence {
        store: worker.store().clone(),
        cancellation: inputs.options.cancellation.clone(),
    };
    tokio::pin!(shutdown);
    let mut execution: Option<ScopedJoinHandle<'scope, WorkerExecutionCompletion>> = None;
    let mut execution_failure = None;
    loop {
        reap(worker, &mut execution, &mut execution_failure)?;
        let deadline = ConnectionDeadline::start(timeout).map_err(|error| error.to_string())?;
        let accepted = tokio::select! {
            biased;
            _ = &mut shutdown => break,
            accepted = LinuxWorkerExchange::accept_with_policy(listener, deadline, WorkerEndpointPolicy::WatchdogOwner) => accepted,
        };
        match accepted {
            Ok(exchange) => {
                // Invalid semantics or a busy store deny this exchange, not
                // another attempt's execution. Admission accounts for any
                // reservation whose response was lost or cancelled.
                if let Ok(outcome) = exchange.admit(worker).await
                    && let WorkerStartOutcome::Started(running) = outcome.start
                {
                    start_execution(
                        scope,
                        worker,
                        inputs,
                        &mut execution,
                        *running,
                        &mut exit_fence.cancellation,
                    )?;
                }
            }
            Err(LinuxTransportError::Configuration) => {
                return Err(String::from("worker listener lost its native transport"));
            }
            Err(_) => {}
        }
        // Bound rejection traffic and completion polling without a request queue.
        tokio::time::sleep(REAP_INTERVAL).await;
    }
    let fence = fence_shutdown(worker);
    // Admission/uncertainty is fenced first. Cancellation is not a receipt and
    // must not turn an in-flight provider reservation into unused budget.
    exit_fence.cancellation.cancel();
    while execution.is_some() {
        reap(worker, &mut execution, &mut execution_failure)?;
        if execution.is_some() {
            tokio::time::sleep(REAP_INTERVAL).await;
        }
    }
    match (fence, execution_failure) {
        (Ok(()), None) => Ok(()),
        (Err(error), None) | (Ok(()), Some(error)) => Err(error),
        (Err(fence), Some(execution)) => Err(format!("{execution}; {fence}")),
    }
}

fn start_execution<'scope, 'env>(
    scope: &'scope Scope<'scope, 'env>,
    worker: &mut WorkerRuntime,
    inputs: &ExecutionInputs,
    execution: &mut Option<ScopedJoinHandle<'scope, WorkerExecutionCompletion>>,
    running: sts2_harness::StoredWorkerHandoff,
    cancellation: &mut sts2_harness::ExecutionCancellation,
) -> Result<(), String> {
    let handoff_id = running.tuple.handoff_id.clone();
    if execution.is_some() {
        return Err(retain_failure(
            worker,
            &handoff_id,
            String::from("worker execution owner is already occupied"),
        ));
    }
    let prepared = worker
        .prepare_started(
            running,
            inputs.config.clone(),
            inputs.settings.clone(),
            inputs.options.clone(),
        )
        .map_err(|error| retain_failure(worker, &handoff_id, error))?;
    *cancellation = prepared.cancellation().clone();
    *execution = Some(
        std::thread::Builder::new()
            .name(String::from("harness-worker-execution"))
            .spawn_scoped(scope, move || prepared.run())
            .map_err(|_| String::from("cannot start owned worker execution thread"))?,
    );
    Ok(())
}

fn reap(
    worker: &mut WorkerRuntime,
    execution: &mut Option<ScopedJoinHandle<'_, WorkerExecutionCompletion>>,
    failure: &mut Option<String>,
) -> Result<(), String> {
    if execution
        .as_ref()
        .is_some_and(ScopedJoinHandle::is_finished)
        && let Some(joined) = execution.take()
    {
        let completion = joined
            .join()
            .map_err(|_| String::from("owned worker execution thread failed"))?;
        // Failure retains UNKNOWN and keeps the lane fenced. Historical
        // lookup/probe/stop must remain available in that controlled state.
        if let Err(error) = worker.complete_execution(completion) {
            *failure = Some(error);
        }
    }
    Ok(())
}

fn fence_shutdown(worker: &WorkerRuntime) -> Result<(), String> {
    begin_quarantine(worker.store())?;
    if let Some(tuple) = worker.active_tuple() {
        worker.retain_unknown(&tuple.handoff_id)?;
    }
    Ok(())
}

fn retain_failure(worker: &WorkerRuntime, handoff_id: &str, error: String) -> String {
    match worker.retain_unknown(handoff_id) {
        Ok(()) => error,
        Err(retention) => format!("{error}; {retention}"),
    }
}
