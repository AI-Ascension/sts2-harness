// SPDX-License-Identifier: MIT

//! Cancellable one-shot process effect for the durable Exo lifecycle owner.

use super::process_reap::{reap_handle, terminate};
use super::{EffectCompletion, EffectHandle, EffectPort, LifecycleError, SendPermit};
use crate::{
    ExecutionCancellation, ExoProcessConfig, encode_bridge_response, parse_bridge_request_envelope,
    sha256_hex,
};
use std::process::Stdio;
use std::sync::mpsc::{Receiver, TryRecvError, sync_channel};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

const MAX_RESPONSE_BYTES: usize = 8 * 1024;

/// An owned, cancellable process factory. The cancellation signal is external to the effect so an
/// authority owner can revoke an active turn without waiting for its model process to finish.
#[derive(Clone)]
pub struct LifecycleProcessEffect {
    config: ExoProcessConfig,
    cancellation: ExecutionCancellation,
    max_response_bytes: usize,
    timeout_millis: u32,
    completed_units: u64,
}

impl LifecycleProcessEffect {
    pub fn new(
        config: ExoProcessConfig,
        cancellation: ExecutionCancellation,
        max_response_bytes: usize,
        timeout_millis: u32,
        completed_units: u64,
    ) -> Result<Self, LifecycleError> {
        if max_response_bytes == 0
            || max_response_bytes > MAX_RESPONSE_BYTES
            || timeout_millis == 0
            || completed_units == 0
            || !matches!(
                config.arguments(),
                [mode, path, digest]
                    if mode == "--run-v2"
                        && std::path::Path::new(path).is_absolute()
                        && !digest.is_empty()
            )
        {
            return Err(LifecycleError::Invalid);
        }
        Ok(Self {
            config,
            cancellation,
            max_response_bytes,
            timeout_millis,
            completed_units,
        })
    }
}

pub struct LifecycleProcessHandle {
    result: Receiver<Result<EffectCompletion, LifecycleError>>,
    cancellation: ExecutionCancellation,
}

impl EffectPort for LifecycleProcessEffect {
    type Handle = LifecycleProcessHandle;

    fn try_start(
        &mut self,
        permit: SendPermit,
        input: &[u8],
    ) -> Result<Self::Handle, LifecycleError> {
        if self.cancellation.is_cancelled() {
            return Err(LifecycleError::Fenced);
        }
        let envelope = parse_bridge_request_envelope(input, 128 * 1024)
            .map_err(|_| LifecycleError::Invalid)?;
        let (sender, receiver) = sync_channel(1);
        // Returning a handle before its child exists makes cancellation ambiguous: the owner can
        // correctly retain Unknown, but callers cannot distinguish a cancelled launched process
        // from a scheduler race that cancelled before `Command::spawn`.  The short readiness
        // rendezvous proves one child was created after the durable send boundary without waiting
        // for provider work or its response.
        let (started_sender, started_receiver) = sync_channel(1);
        let config = self.config.clone();
        let cancellation = self.cancellation.clone();
        let bytes = input.to_vec();
        let maximum = self.max_response_bytes;
        let timeout = self.timeout_millis;
        let completed_units = self.completed_units;
        let operation_id = permit.operation_id().to_owned();
        let context = ExchangeContext {
            config,
            maximum,
            timeout_millis: timeout,
            cancellation: cancellation.clone(),
            request_id: envelope.request_id,
            turn_id: envelope.turn_id,
            operation_id,
            completed_units,
        };
        std::thread::Builder::new()
            .name(String::from("exo-lifecycle-effect"))
            .spawn(move || {
                let result = exchange(context, bytes, started_sender);
                let _ = sender.send(result);
            })
            .map_err(|_| LifecycleError::Unavailable)?;
        match started_receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error),
            Err(_) => return Err(LifecycleError::Unavailable),
        }
        Ok(LifecycleProcessHandle {
            result: receiver,
            cancellation: self.cancellation.clone(),
        })
    }
}

impl EffectHandle for LifecycleProcessHandle {
    fn poll(&mut self) -> Result<Option<EffectCompletion>, LifecycleError> {
        if self.cancellation.is_cancelled() {
            return Err(LifecycleError::Unknown);
        }
        match self.result.try_recv() {
            Ok(result) => result.map(Some),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(LifecycleError::Unknown),
        }
    }
}

struct ExchangeContext {
    config: ExoProcessConfig,
    maximum: usize,
    timeout_millis: u32,
    cancellation: ExecutionCancellation,
    request_id: String,
    turn_id: String,
    operation_id: String,
    completed_units: u64,
}

fn exchange(
    context: ExchangeContext,
    input: Vec<u8>,
    started: std::sync::mpsc::SyncSender<Result<(), LifecycleError>>,
) -> Result<EffectCompletion, LifecycleError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| LifecycleError::Unavailable)?;
    runtime.block_on(exchange_async(context, input, started))
}

async fn exchange_async(
    context: ExchangeContext,
    input: Vec<u8>,
    started: std::sync::mpsc::SyncSender<Result<(), LifecycleError>>,
) -> Result<EffectCompletion, LifecycleError> {
    let ExchangeContext {
        config,
        maximum,
        timeout_millis,
        cancellation,
        request_id,
        turn_id,
        operation_id,
        completed_units,
    } = context;
    let mut command = Command::new(config.executable());
    command
        .args(config.arguments())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .kill_on_drop(true);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.as_std_mut().process_group(0);
    }
    if let Some(directory) = config.working_directory() {
        command.current_dir(directory);
    }
    for name in config.inherited_environment() {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(_) => {
            let _ = started.send(Err(LifecycleError::Unavailable));
            return Err(LifecycleError::Unavailable);
        }
    };
    let pid = reap_handle(&child).ok_or(LifecycleError::Unavailable)?;
    let _ = started.send(Ok(()));
    let deadline = Instant::now() + Duration::from_millis(u64::from(timeout_millis));
    let result = tokio::select! {
        value = exchange_pipes(&mut child, &input, maximum) => value,
        _ = cancellation.cancelled() => Err(LifecycleError::Unknown),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            Err(LifecycleError::Unknown)
        }
    };
    let bytes = match result {
        Ok(bytes) => bytes,
        Err(error) => {
            terminate(&mut child, pid).await;
            return Err(error);
        }
    };
    let (_, native) = super::parse_lifecycle_response(&bytes, &request_id, &turn_id)
        .map_err(|_| LifecycleError::Invalid)?;
    let envelope: super::ExoLifecycleResponse =
        serde_json::from_slice(&bytes).map_err(|_| LifecycleError::Invalid)?;
    let decision = serde_json::to_vec(envelope.decision.as_ref().ok_or(LifecycleError::Invalid)?)
        .map_err(|_| LifecycleError::Invalid)?;
    let response = encode_bridge_response(
        &request_id,
        &turn_id,
        crate::ExoWireOutcome::Decision,
        Some(&decision),
        None,
    )
    .map_err(|_| LifecycleError::Invalid)?;
    Ok(EffectCompletion {
        response,
        result_ref: format!("exo-{}", sha256_hex(operation_id)),
        actual_units: Some(completed_units),
        native: Some(native),
    })
}

async fn exchange_pipes(
    child: &mut Child,
    input: &[u8],
    maximum: usize,
) -> Result<Vec<u8>, LifecycleError> {
    let mut stdin = child.stdin.take().ok_or(LifecycleError::Unavailable)?;
    let stdout = child.stdout.take().ok_or(LifecycleError::Unavailable)?;
    let write = async move {
        stdin
            .write_all(input)
            .await
            .map_err(|_| LifecycleError::Unavailable)?;
        stdin
            .shutdown()
            .await
            .map_err(|_| LifecycleError::Unavailable)
    };
    let read = async move {
        let mut bytes = Vec::new();
        stdout
            .take(u64::try_from(maximum + 1).map_err(|_| LifecycleError::Invalid)?)
            .read_to_end(&mut bytes)
            .await
            .map_err(|_| LifecycleError::Unavailable)?;
        if bytes.len() > maximum {
            return Err(LifecycleError::Invalid);
        }
        Ok(bytes)
    };
    let wait = async {
        let status = child
            .wait()
            .await
            .map_err(|_| LifecycleError::Unavailable)?;
        if status.success() {
            Ok(())
        } else {
            Err(LifecycleError::Unknown)
        }
    };
    let (_, bytes, ()) = tokio::try_join!(write, read, wait)?;
    Ok(bytes)
}
