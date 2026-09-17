// SPDX-License-Identifier: MIT
//! Owned duplex subprocess adapter for the additive lookup agent protocol.
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::ExoProcessConfig;
use crate::exo_lookup_wire::{
    EXO_LOOKUP_BOOTSTRAP_WIRE, EXO_LOOKUP_WIRE, ExoLookupFrame, ExoLookupPayload,
};
use crate::game_information::{
    LookupAgentInput, LookupAgentPort, LookupError, LookupFeedback, LookupTurn,
};

#[path = "exo_lookup_process_supervisor.rs"]
mod supervisor;

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
    byte_budget: Option<usize>,
    bootstrap_feedback_pending: bool,
    bootstrap_profile: bool,
}

impl ExoLookupProcess {
    pub fn new(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
    ) -> Result<Self, LookupError> {
        Self::new_mode(config, request_id, turn_id, request, timeout, false)
    }

    /// Explicit bootstrap-capable provider profile. Legacy `new` stays v1.
    pub fn new_bootstrap(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
    ) -> Result<Self, LookupError> {
        Self::new_mode(config, request_id, turn_id, request, timeout, true)
    }

    fn new_mode(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
        bootstrap_profile: bool,
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
            wire_version: if bootstrap_profile {
                EXO_LOOKUP_BOOTSTRAP_WIRE.into()
            } else {
                EXO_LOOKUP_WIRE.into()
            },
            request_id: request_id.clone(),
            turn_id: turn_id.clone(),
            sequence: 0,
            payload: ExoLookupPayload::Start {
                request: request.clone(),
                optional_byte_budget: crate::exo_lookup_wire::EXO_LOOKUP_FEEDBACK_BYTES,
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
                        runtime.block_on(supervisor::supervise(
                            config, commands, &responses, cancelled, deadline,
                        ))
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
            byte_budget: None,
            bootstrap_feedback_pending: false,
            bootstrap_profile,
        })
    }

    fn exchange(&mut self, payload: ExoLookupPayload) -> Result<ExoLookupPayload, LookupError> {
        if self.closed {
            return Err(LookupError::Invalid);
        }
        let bootstrap_wire = self.bootstrap_profile
            && matches!(&payload, ExoLookupPayload::Start { .. })
            || matches!(&payload, ExoLookupPayload::Bootstrap { .. })
            || (self.bootstrap_feedback_pending
                && matches!(&payload, ExoLookupPayload::Feedback { .. }));
        let frame = ExoLookupFrame {
            wire_version: if bootstrap_wire {
                EXO_LOOKUP_BOOTSTRAP_WIRE.into()
            } else {
                EXO_LOOKUP_WIRE.into()
            },
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence: self.sequence,
            payload,
        };
        if matches!(frame.payload, ExoLookupPayload::Bootstrap { .. }) {
            self.bootstrap_feedback_pending = true;
        } else if self.bootstrap_feedback_pending
            && matches!(frame.payload, ExoLookupPayload::Feedback { .. })
        {
            self.bootstrap_feedback_pending = false;
        }
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
                .is_some_and(|binding| !binding.same_owner(input.binding))
                || self
                    .byte_budget
                    .is_some_and(|budget| budget != input.optional_byte_budget)
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
                self.byte_budget = Some(input.optional_byte_budget);
                ExoLookupPayload::Start {
                    request: self.request.clone(),
                    optional_byte_budget: input.optional_byte_budget,
                }
            } else {
                ExoLookupPayload::Feedback {
                    value: crate::exo_lookup_wire::feedback_value(
                        input.feedback,
                        input.optional_byte_budget,
                    )?,
                }
            };
            match self.exchange(payload)? {
                ExoLookupPayload::Query { arguments } => {
                    crate::exo_lookup_wire::query_turn(arguments, &input)
                }
                ExoLookupPayload::Bootstrap { arguments } => {
                    if !self.bootstrap_profile {
                        return Err(LookupError::Invalid);
                    }
                    crate::exo_lookup_wire::bootstrap_turn(arguments)
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
