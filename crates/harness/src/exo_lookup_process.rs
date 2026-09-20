// SPDX-License-Identifier: MIT
//! Owned duplex subprocess adapter for the additive lookup agent protocol.
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::ExoProcessConfig;
use crate::exo_lookup_wire::{
    EXO_LOOKUP_BOOTSTRAP_WIRE, EXO_LOOKUP_HISTORY_WIRE, EXO_LOOKUP_WIRE, ExoLookupFrame,
    ExoLookupPayload,
};
use crate::game_information::{
    LookupAgentInput, LookupAgentPort, LookupError, LookupFeedback, LookupTurn,
};

#[path = "exo_lookup_process_supervisor.rs"]
mod supervisor;

#[path = "exo_lookup_process_port.rs"]
mod port;

/// The additive provider profile one process was explicitly selected under.
///
/// Selection is the caller's: a process built for one profile refuses a frame belonging to
/// another rather than widening its own surface, so a provider cannot obtain a capability the
/// caller did not select for it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExoLookupProfile {
    /// The closed terminal query/read relay.
    Terminal,
    /// The additive bootstrap-capable relay.
    Bootstrap,
    /// The additive history-capable relay.
    History,
}

impl ExoLookupProfile {
    /// The frame pin this profile opens on. Each additive turn carries its own pin.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Terminal => EXO_LOOKUP_WIRE,
            Self::Bootstrap => EXO_LOOKUP_BOOTSTRAP_WIRE,
            Self::History => EXO_LOOKUP_HISTORY_WIRE,
        }
    }
}

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
    profile: ExoLookupProfile,
    feedback_pending: bool,
}

impl ExoLookupProcess {
    pub fn new(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
    ) -> Result<Self, LookupError> {
        Self::new_mode(
            config,
            request_id,
            turn_id,
            request,
            timeout,
            ExoLookupProfile::Terminal,
        )
    }

    /// Explicit bootstrap-capable provider profile. Legacy `new` stays v1.
    pub fn new_bootstrap(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
    ) -> Result<Self, LookupError> {
        Self::new_mode(
            config,
            request_id,
            turn_id,
            request,
            timeout,
            ExoLookupProfile::Bootstrap,
        )
    }

    /// Explicit history-capable provider profile. The closed profiles above stay as they were.
    pub fn new_history(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
    ) -> Result<Self, LookupError> {
        Self::new_mode(
            config,
            request_id,
            turn_id,
            request,
            timeout,
            ExoLookupProfile::History,
        )
    }

    fn new_mode(
        config: ExoProcessConfig,
        request_id: String,
        turn_id: String,
        request: serde_json::Value,
        timeout: Duration,
        profile: ExoLookupProfile,
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
            wire_version: profile.wire().into(),
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
            profile,
            feedback_pending: false,
        })
    }

    fn exchange(&mut self, payload: ExoLookupPayload) -> Result<ExoLookupPayload, LookupError> {
        if self.closed {
            return Err(LookupError::Invalid);
        }
        // A turn adds its own pin; feedback for that turn stays on the pin it answered on, so a
        // provider cannot read an answer under a profile it did not select.
        let wire = match &payload {
            ExoLookupPayload::Bootstrap { .. } => EXO_LOOKUP_BOOTSTRAP_WIRE,
            ExoLookupPayload::History { .. } => EXO_LOOKUP_HISTORY_WIRE,
            ExoLookupPayload::Start { .. } => self.profile.wire(),
            ExoLookupPayload::Feedback { .. } if self.feedback_pending => self.profile.wire(),
            _ => EXO_LOOKUP_WIRE,
        };
        let frame = ExoLookupFrame {
            wire_version: wire.into(),
            request_id: self.request_id.clone(),
            turn_id: self.turn_id.clone(),
            sequence: self.sequence,
            payload,
        };
        if matches!(
            frame.payload,
            ExoLookupPayload::Bootstrap { .. } | ExoLookupPayload::History { .. }
        ) {
            self.feedback_pending = true;
        } else if self.feedback_pending
            && matches!(frame.payload, ExoLookupPayload::Feedback { .. })
        {
            self.feedback_pending = false;
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

impl Drop for ExoLookupProcess {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
