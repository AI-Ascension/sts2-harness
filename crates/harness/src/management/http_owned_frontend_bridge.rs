// SPDX-License-Identifier: MIT

use super::*;

pub(super) struct Bridge {
    helper: Helper,
    requests: Receiver<Work>,
    diagnostics: Receiver<String>,
    lost_diagnostics: Arc<AtomicU64>,
    limits: HttpLimits,
    next_sequence: u64,
    waiting: BTreeMap<u64, Waiting>,
    diagnostic_sequences: std::collections::BTreeSet<u64>,
    ready: bool,
}

impl Bridge {
    pub(super) fn spawn(
        requests: Receiver<Work>,
        diagnostics: Receiver<String>,
        lost_diagnostics: Arc<AtomicU64>,
        limits: HttpLimits,
    ) -> Result<Self, HttpError> {
        Ok(Self {
            helper: Helper::spawn(limits.max_connections)?,
            requests,
            diagnostics,
            lost_diagnostics,
            limits,
            next_sequence: 1,
            waiting: BTreeMap::new(),
            diagnostic_sequences: std::collections::BTreeSet::new(),
            ready: false,
        })
    }

    pub(super) fn await_ready(&mut self) -> Result<(), HttpError> {
        let deadline = Deadline::after(Duration::from_secs(30))
            .ok_or_else(|| unavailable("owned_helper_clock"))?;
        while !self.ready {
            self.tick()?;
            if deadline.expired() {
                return Err(unavailable("owned_helper_startup"));
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }

    pub(super) fn tick(&mut self) -> Result<(), HttpError> {
        if self.ready {
            self.accept_work()?;
            self.accept_diagnostics()?;
        }
        self.helper
            .channel()?
            .flush()
            .map_err(|_| unavailable("owned_helper_channel"))?;
        for _ in 0..self.limits.max_connections + 64 {
            let message = self
                .helper
                .channel()?
                .receive::<FromBackend>()
                .map_err(|_| unavailable("owned_helper_channel"))?;
            let Some(message) = message else {
                break;
            };
            self.reply(message)?;
        }
        let expired: Vec<_> = self
            .waiting
            .iter()
            .filter(|(_, waiting)| waiting.deadline.expired())
            .map(|(sequence, _)| *sequence)
            .collect();
        for sequence in expired {
            if let Some(waiting) = self.waiting.remove(&sequence) {
                let _ = waiting.reply.send(Err(uncertain(
                    waiting.effect,
                    "dispatch_termination_pending",
                )));
            }
        }
        if self
            .helper
            .child
            .try_wait()
            .map_err(io_http_error)?
            .is_some()
        {
            return Err(unavailable("owned_helper_exited"));
        }
        Ok(())
    }

    fn sequence(&mut self) -> Result<u64, HttpError> {
        let value = self.next_sequence;
        self.next_sequence = value
            .checked_add(1)
            .ok_or_else(|| unavailable("owned_request_limit"))?;
        Ok(value)
    }

    fn accept_work(&mut self) -> Result<(), HttpError> {
        for _ in 0..self.limits.max_connections {
            let work = match self.requests.try_recv() {
                Ok(work) => work,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            let recovery = route_effects::is_submission_recovery(&work.request);
            let ordinary = self
                .waiting
                .values()
                .filter(|waiting| !waiting.recovery)
                .count();
            if work.deadline.expired()
                || self.waiting.len() >= self.limits.max_connections
                || (!recovery && ordinary >= self.limits.max_connections.saturating_sub(1).max(1))
            {
                let _ = work.reply.send(Err(unavailable("dispatch_not_started")));
                continue;
            }
            let sequence = self.sequence()?;
            let effect = route_effects::classify(&work.request);
            self.helper
                .channel()?
                .queue(&ToBackend::Dispatch {
                    generation: 1,
                    sequence,
                    deadline_nanos: work.deadline.wire(),
                    request: work.request.into(),
                })
                .map_err(|_| unavailable("owned_helper_bounds"))?;
            let deadline = Deadline::after(
                work.deadline.remaining() + REAP_BUDGET + Duration::from_millis(100),
            )
            .ok_or_else(|| unavailable("owned_dispatch_clock"))?;
            self.waiting.insert(
                sequence,
                Waiting {
                    deadline,
                    effect,
                    recovery,
                    reply: work.reply,
                },
            );
        }
        Ok(())
    }

    fn accept_diagnostics(&mut self) -> Result<(), HttpError> {
        for _ in 0..64 {
            let line = match self.diagnostics.try_recv() {
                Ok(line) => line,
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            };
            if self.diagnostic_sequences.len() >= 64 {
                lost(&self.lost_diagnostics);
                continue;
            }
            let sequence = self.sequence()?;
            self.helper
                .channel()?
                .queue(&ToBackend::Diagnostic {
                    generation: 1,
                    sequence,
                    line,
                })
                .map_err(|_| unavailable("owned_helper_bounds"))?;
            self.diagnostic_sequences.insert(sequence);
        }
        Ok(())
    }

    fn reply(&mut self, message: FromBackend) -> Result<(), HttpError> {
        match message {
            FromBackend::Ready {
                version: WIRE_VERSION,
                generation: 1,
            } if !self.ready => {
                self.ready = true;
            }
            FromBackend::Response {
                generation: 1,
                sequence,
                status,
                body,
            } => {
                let response = owned_wire::into_response(status, body, &self.limits)
                    .map_err(|_| unavailable("owned_helper_response"))?;
                if let Some(waiting) = self.waiting.remove(&sequence) {
                    let _ = waiting.reply.send(Ok(response));
                } else if sequence >= self.next_sequence {
                    return Err(unavailable("owned_helper_identity"));
                }
                // A late response for an already-timed-out sequence cannot adopt another request.
            }
            FromBackend::DiagnosticWritten {
                generation: 1,
                sequence,
            } => {
                if !self.diagnostic_sequences.remove(&sequence) {
                    return Err(unavailable("owned_helper_identity"));
                }
            }
            FromBackend::DiagnosticLost {
                generation: 1,
                sequence,
            } => {
                if !self.diagnostic_sequences.remove(&sequence) {
                    return Err(unavailable("owned_helper_identity"));
                }
                lost(&self.lost_diagnostics);
            }
            FromBackend::BackendRetired { generation: 1, .. } => {}
            FromBackend::SupervisorFailed {
                generation: 1,
                code,
            } => {
                let code = if code == "owned_supervisor_panicked" {
                    "owned_supervisor_panicked"
                } else {
                    "owned_supervisor_failed"
                };
                for (_, waiting) in std::mem::take(&mut self.waiting) {
                    let _ = waiting.reply.send(Err(uncertain(waiting.effect, code)));
                }
                return Err(unavailable(code));
            }
            _ => return Err(unavailable("owned_helper_identity")),
        }
        Ok(())
    }

    pub(super) fn shutdown(&mut self) {
        for (_, waiting) in std::mem::take(&mut self.waiting) {
            let _ = waiting.reply.send(Err(uncertain(
                waiting.effect,
                "dispatch_termination_pending",
            )));
        }
        for _ in std::mem::take(&mut self.diagnostic_sequences) {
            lost(&self.lost_diagnostics);
        }
        self.helper.shutdown();
    }
}
