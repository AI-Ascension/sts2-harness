// SPDX-License-Identifier: MIT
//! Owned duplex subprocess adapter for the additive lookup agent protocol.
use std::process::Stdio;
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::ExoProcessConfig;
use crate::exo_lookup_wire::{
    EXO_LOOKUP_FRAME_BYTES, EXO_LOOKUP_WIRE, ExoLookupFrame, ExoLookupPayload,
};
use crate::game_information::{
    LookupAgentInput, LookupAgentPort, LookupError, LookupFeedback, LookupTurn,
};

/// One bounded model turn with read-only tool round trips. Dropping joins the owned supervisor.
pub struct ExoLookupProcess {
    request_id: String,
    turn_id: String,
    request: serde_json::Value,
    sequence: u64,
    closed: bool,
    sender: Option<SyncSender<Vec<u8>>>,
    receiver: Receiver<Result<Vec<u8>, LookupError>>,
    worker: Option<JoinHandle<()>>,
    cancel: tokio::sync::watch::Sender<bool>,
    binding: Option<crate::game_information::LookupBinding>,
}

impl ExoLookupProcess {
    pub fn new(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
    ) -> Result<Self, LookupError> {
        if timeout.is_zero() || timeout > Duration::from_secs(120) {
            return Err(LookupError::Bounds);
        }
        crate::parse_bridge_request(
            &serde_json::to_vec(&request).map_err(|_| LookupError::Invalid)?,
            131_072,
        )
        .map_err(|_| LookupError::Invalid)?;
        ExoLookupFrame {
            wire_version: EXO_LOOKUP_WIRE.into(),
            request_id: request_id.clone(),
            turn_id: turn_id.clone(),
            sequence: 0,
            payload: ExoLookupPayload::Start {
                request: request.clone(),
            },
        }
        .encode()?;
        let (sender, commands) = sync_channel(1);
        let (responses, receiver) = sync_channel(1);
        let (cancel, cancelled) = tokio::sync::watch::channel(false);
        let deadline = Instant::now() + timeout;
        let worker = std::thread::Builder::new()
            .name("exo-lookup".into())
            .spawn(move || {
                let result = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|_| LookupError::Transport)
                    .and_then(|runtime| {
                        runtime
                            .block_on(supervise(config, commands, &responses, cancelled, deadline))
                    });
                if let Err(error) = result {
                    let _ = responses.try_send(Err(error));
                }
            })
            .map_err(|_| LookupError::Transport)?;
        Ok(Self {
            request_id,
            turn_id,
            request,
            sequence: 0,
            closed: false,
            sender: Some(sender),
            receiver,
            worker: Some(worker),
            cancel,
            binding: None,
        })
    }

    fn exchange(&mut self, payload: ExoLookupPayload) -> Result<ExoLookupPayload, LookupError> {
        if self.closed {
            return Err(LookupError::Invalid);
        }
        let frame = ExoLookupFrame {
            wire_version: EXO_LOOKUP_WIRE.into(),
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence: self.sequence,
            payload,
        };
        self.sender
            .as_ref()
            .ok_or(LookupError::Transport)?
            .send(frame.encode()?)
            .map_err(|_| LookupError::Transport)?;
        let bytes = self.receiver.recv().map_err(|_| LookupError::Transport)??;
        let response = ExoLookupFrame::parse(&bytes)?;
        self.sequence += 1;
        response.assert_identity(&self.request_id, &self.turn_id, self.sequence)?;
        Ok(response.payload)
    }
}

impl LookupAgentPort for ExoLookupProcess {
    fn next_turn(&mut self, input: LookupAgentInput<'_>) -> Result<LookupTurn, LookupError> {
        let result = (|| {
            if self
                .binding
                .as_ref()
                .is_some_and(|binding| binding != input.binding)
                || self.request["generation"].as_u64() != Some(input.legal_actions.generation())
                || self.request["state_id"].as_str() != Some(input.legal_actions.state_id())
                || self.request["legal_action_ids"]
                    != serde_json::json!(
                        input
                            .legal_actions
                            .actions()
                            .iter()
                            .map(|a| a.action_id())
                            .collect::<Vec<_>>()
                    )
            {
                return Err(LookupError::Scope);
            }
            let payload = if self.sequence == 0 {
                if *input.feedback != LookupFeedback::Start {
                    return Err(LookupError::Scope);
                }
                self.binding = Some(input.binding.clone());
                ExoLookupPayload::Start {
                    request: self.request.clone(),
                }
            } else {
                ExoLookupPayload::Feedback {
                    value: crate::exo_lookup_wire::feedback_value(input.feedback)?,
                }
            };
            match self.exchange(payload)? {
                ExoLookupPayload::Query { arguments } => {
                    crate::exo_lookup_wire::query_turn(arguments, &input)
                }
                ExoLookupPayload::ReadRetained {
                    record_ordinal,
                    offset,
                } if record_ordinal < 256 && offset <= 65_536 => Ok(LookupTurn::ReadRetained {
                    record_ordinal,
                    offset,
                }),
                ExoLookupPayload::Decision { action_id }
                    if input
                        .legal_actions
                        .actions()
                        .iter()
                        .any(|a| a.action_id() == action_id) =>
                {
                    self.closed = true;
                    self.sender.take();
                    Ok(LookupTurn::Decide { action_id })
                }
                _ => Err(LookupError::Invalid),
            }
        })();
        if result.is_err() {
            self.closed = true;
            self.sender.take();
        }
        result
    }
}

impl Drop for ExoLookupProcess {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

async fn supervise(
    config: ExoProcessConfig,
    commands: Receiver<Vec<u8>>,
    responses: &SyncSender<Result<Vec<u8>, LookupError>>,
    mut cancelled: tokio::sync::watch::Receiver<bool>,
    deadline: Instant,
) -> Result<(), LookupError> {
    let mut command = tokio::process::Command::new(config.executable());
    command
        .args(config.arguments())
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    command.process_group(0);
    if let Some(cwd) = config.working_directory() {
        command.current_dir(cwd);
    }
    for name in config.inherited_environment() {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let mut child = command.spawn().map_err(|_| LookupError::Transport)?;
    let mut stdin = child.stdin.take().ok_or(LookupError::Transport)?;
    let mut stdout = child.stdout.take().ok_or(LookupError::Transport)?;
    let result = async {
        while let Ok(bytes) =
            commands.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            let io = tokio::time::timeout_at(deadline.into(), async {
                stdin
                    .write_all(&bytes)
                    .await
                    .map_err(|_| LookupError::Transport)?;
                stdin.flush().await.map_err(|_| LookupError::Transport)?;
                let mut line = Vec::new();
                loop {
                    let byte = stdout.read_u8().await.map_err(|_| LookupError::Transport)?;
                    if byte == b'\n' {
                        return Ok(line);
                    }
                    if line.len() >= EXO_LOOKUP_FRAME_BYTES {
                        return Err(LookupError::Bounds);
                    }
                    line.push(byte);
                }
            });
            let response = tokio::select! {
                result = io => result.map_err(|_|LookupError::Transport)??,
                _ = cancelled.changed() => return Err(LookupError::Transport),
            };
            responses
                .send(Ok(response))
                .map_err(|_| LookupError::Transport)?;
        }
        Ok(())
    }
    .await;
    drop(stdin);
    // EOF lets the bridge cancel and reap its separately owned executor group.
    tokio::time::sleep(Duration::from_millis(250)).await;
    #[cfg(target_os = "linux")]
    if let Some(pid) = child
        .id()
        .and_then(|id| i32::try_from(id).ok())
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
    }
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
    result
}
