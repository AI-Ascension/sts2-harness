// SPDX-License-Identifier: MIT

use super::*;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::thread::{self, JoinHandle};

/// Sendable command proxy that leaves RuntimeV3Port and its owner-local
/// `Rc<RefCell<ExecutionStore>>` exclusively on one worker thread.
pub(crate) struct RuntimeV3SessionWorker {
    commands: SyncSender<WorkerCommand>,
    join: Option<JoinHandle<()>>,
}

enum WorkerCommand {
    Launch(SyncSender<Result<(), PortError>>),
    LeaseBinding(SyncSender<Result<RuntimeLeaseBinding, PortError>>),
    Observe(SyncSender<Result<EpisodeObservation, PortError>>),
    Projection(String, SyncSender<Result<EpisodeObservation, PortError>>),
    LegalActions(
        String,
        u64,
        SyncSender<Result<EpisodeLegalActionSet, PortError>>,
    ),
    Dispatch(
        ActionIdentity,
        EpisodeLegalAction,
        SyncSender<Result<TransitionReceipt, PortError>>,
    ),
    Wait(String, u32, SyncSender<Result<WaitSample, BarrierError>>),
    Reconcile(String, SyncSender<Result<TransitionReceipt, RecoveryError>>),
    Receipt(
        ReceiptQueryIdentity,
        SyncSender<Result<ReceiptQueryResult, RecoveryError>>,
    ),
    Release(SyncSender<Result<(), RecoveryError>>),
    Stop(SyncSender<Result<(), RecoveryError>>),
    Shutdown(SyncSender<()>),
}

impl RuntimeV3SessionWorker {
    pub(crate) fn start(config: RuntimeConfig) -> Result<Self, String> {
        let (commands, receiver) = sync_channel(8);
        let (ready_tx, ready_rx) = sync_channel(1);
        let join = thread::Builder::new()
            .name("sts2-runtime-v3-session".to_owned())
            .spawn(move || worker_main(config, receiver, ready_tx))
            .map_err(|error| format!("cannot start runtime-v3 session worker: {error}"))?;
        ready_rx
            .recv()
            .map_err(|_| String::from("runtime-v3 session worker exited before startup"))??;
        Ok(Self {
            commands,
            join: Some(join),
        })
    }

    fn call<T>(&self, command: WorkerCommand, reply: Receiver<T>) -> Result<T, PortError> {
        self.commands
            .try_send(command)
            .map_err(|error| match error {
                TrySendError::Full(_) => PortError::new(
                    "runtime_worker_overloaded",
                    "runtime worker queue is full",
                    true,
                ),
                TrySendError::Disconnected(_) => PortError::new(
                    "runtime_worker_unavailable",
                    "runtime worker is unavailable",
                    true,
                ),
            })?;
        reply.recv().map_err(|_| {
            PortError::new(
                "runtime_worker_unavailable",
                "runtime worker ended before returning a result",
                true,
            )
        })
    }
}

fn worker_main(
    config: RuntimeConfig,
    receiver: Receiver<WorkerCommand>,
    ready: SyncSender<Result<(), String>>,
) {
    let settings = match RuntimeV3Settings::from_environment(&config) {
        Ok(settings) => settings,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let context = match TelemetryContext::new(TelemetryContextInput {
        run_id: &config.run_id,
        episode_id: &config.episode_id,
        trajectory_id: &config.trajectory_id,
        trace_id: &config.trace_id,
        instance_id: &config.instance_id,
        session_id: &config.session_id,
        runtime_profile: &config.runtime_profile,
        provider_revision: &settings.exo.revision,
    }) {
        Ok(context) => context,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let telemetry = RuntimeV3Telemetry::new(context);
    let durable = match durable::DurableHandle::open(&config, &settings, false) {
        Ok((durable, _)) => durable,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    let mut port = match RuntimeV3Port::new_with_store(config, telemetry.handle(), durable) {
        Ok(port) => {
            let _ = ready.send(Ok(()));
            port
        }
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };
    while let Ok(command) = receiver.recv() {
        match command {
            WorkerCommand::Launch(reply) => {
                let _ = reply.send(port.launch());
            }
            WorkerCommand::LeaseBinding(reply) => {
                let _ = reply.send(port.allocated_lease_binding());
            }
            WorkerCommand::Observe(reply) => {
                let _ = reply.send(port.observe());
            }
            WorkerCommand::Projection(reference, reply) => {
                let _ = reply.send(port.observe_projection(&reference));
            }
            WorkerCommand::LegalActions(state, generation, reply) => {
                let _ = reply.send(port.legal_actions(&state, generation));
            }
            WorkerCommand::Dispatch(identity, action, reply) => {
                let _ = reply.send(port.dispatch_action(&identity, &action));
            }
            WorkerCommand::Wait(operation, millis, reply) => {
                let _ = reply.send(port.wait_for_transition(&operation, millis));
            }
            WorkerCommand::Reconcile(operation, reply) => {
                let _ = reply.send(port.reconcile(&operation));
            }
            WorkerCommand::Receipt(identity, reply) => {
                let _ = reply.send(port.query_receipt(&identity));
            }
            WorkerCommand::Release(reply) => {
                let _ = reply.send(RecoveryPort::release_lease(&mut port));
            }
            WorkerCommand::Stop(reply) => {
                let _ = reply.send(port.stop_episode());
            }
            WorkerCommand::Shutdown(reply) => {
                let _ = ShutdownPort::release_lease(&mut port);
                let _ = port.close_mcp();
                let _ = port.close_gateway();
                if let Some(durable) = port.durable_handle() {
                    let _ = durable.close();
                }
                let _ = reply.send(());
                break;
            }
        }
    }
}

impl EpisodeRuntimePort for RuntimeV3SessionWorker {
    fn launch(&mut self) -> Result<(), PortError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Launch(tx), rx)?
    }
    fn current_lease_binding(&mut self) -> Result<RuntimeLeaseBinding, PortError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::LeaseBinding(tx), rx)?
    }
    fn observe(&mut self) -> Result<EpisodeObservation, PortError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Observe(tx), rx)?
    }
    fn observe_projection(&mut self, reference: &str) -> Result<EpisodeObservation, PortError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Projection(reference.to_owned(), tx), rx)?
    }
    fn legal_actions(
        &mut self,
        state: &str,
        generation: u64,
    ) -> Result<EpisodeLegalActionSet, PortError> {
        let (tx, rx) = sync_channel(1);
        self.call(
            WorkerCommand::LegalActions(state.to_owned(), generation, tx),
            rx,
        )?
    }
    fn dispatch_action(
        &mut self,
        identity: &ActionIdentity,
        action: &EpisodeLegalAction,
    ) -> Result<TransitionReceipt, PortError> {
        let (tx, rx) = sync_channel(1);
        self.call(
            WorkerCommand::Dispatch(identity.clone(), action.clone(), tx),
            rx,
        )?
    }
}

impl BarrierPort for RuntimeV3SessionWorker {
    fn wait_for_transition(
        &mut self,
        operation: &str,
        millis: u32,
    ) -> Result<WaitSample, BarrierError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Wait(operation.to_owned(), millis, tx), rx)
            .map_err(|_| BarrierError::PortFailure)?
    }
}

impl RecoveryPort for RuntimeV3SessionWorker {
    fn reobserve(&mut self) -> Result<EpisodeObservation, RecoveryError> {
        self.observe().map_err(|_| RecoveryError::PortFailure)
    }
    fn reconcile(&mut self, operation: &str) -> Result<TransitionReceipt, RecoveryError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Reconcile(operation.to_owned(), tx), rx)
            .map_err(|_| RecoveryError::PortFailure)?
    }
    fn query_receipt(
        &mut self,
        identity: &ReceiptQueryIdentity,
    ) -> Result<ReceiptQueryResult, RecoveryError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Receipt(identity.clone(), tx), rx)
            .map_err(|_| RecoveryError::PortFailure)?
    }
    fn release_lease(&mut self) -> Result<(), RecoveryError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Release(tx), rx)
            .map_err(|_| RecoveryError::PortFailure)?
    }
    fn stop_episode(&mut self) -> Result<(), RecoveryError> {
        let (tx, rx) = sync_channel(1);
        self.call(WorkerCommand::Stop(tx), rx)
            .map_err(|_| RecoveryError::PortFailure)?
    }
}

impl ShutdownPort for RuntimeV3SessionWorker {
    fn release_lease(&mut self) -> Result<(), ShutdownError> {
        RecoveryPort::release_lease(self).map_err(|_| ShutdownError::ReleaseFailed)
    }
    fn close_mcp(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
    fn close_gateway(&mut self) -> Result<(), ShutdownError> {
        Ok(())
    }
}

impl Drop for RuntimeV3SessionWorker {
    fn drop(&mut self) {
        let (tx, rx) = sync_channel(1);
        let _ = self.commands.try_send(WorkerCommand::Shutdown(tx));
        let _ = rx.recv();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
