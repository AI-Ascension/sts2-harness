// SPDX-License-Identifier: MIT

use super::*;
use crate::management::http::owned_wire::Request;
use crate::management::http::route_effects::RouteEffect;

#[path = "http_owned_supervisor_retirement.rs"]
mod retirement;

struct Pending {
    deadline: Deadline,
    effect: RouteEffect,
    recovery: bool,
    request: Option<Request>,
    sent: bool,
}

pub(super) struct Supervisor {
    scope: ChildScope,
    parent: Frames,
    parent_generation: u64,
    capacity: usize,
    last_sequence: u64,
    next_generation: u64,
    generation: Option<Generation>,
    pending: BTreeMap<u64, Pending>,
    diagnostics: BTreeMap<u64, Deadline>,
    announced: bool,
}

impl Supervisor {
    pub(super) fn new(scope: ChildScope, parent: Frames, generation: u64, capacity: usize) -> Self {
        Self {
            scope,
            parent,
            parent_generation: generation,
            capacity,
            last_sequence: 0,
            next_generation: 1,
            generation: None,
            pending: BTreeMap::new(),
            diagnostics: BTreeMap::new(),
            announced: false,
        }
    }

    pub(super) fn serve(&mut self) -> Result<(), HttpError> {
        self.start_generation()?;
        loop {
            if let Some(message) = self
                .parent
                .receive()
                .map_err(|_| fault("owned_parent_closed"))?
            {
                if matches!(message, ToBackend::Stop { generation } if generation == self.parent_generation)
                {
                    return Ok(());
                }
                self.accept(message)?;
            }
            self.tick_generation()?;
            self.parent
                .flush()
                .map_err(|_| fault("owned_parent_closed"))?;
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn start_generation(&mut self) -> Result<(), HttpError> {
        let number = self.next_generation;
        self.next_generation = number
            .checked_add(1)
            .ok_or_else(|| fault("owned_generation_limit"))?;
        self.generation = Some(Generation::spawn(number, self.capacity)?);
        Ok(())
    }

    fn accept(&mut self, message: ToBackend) -> Result<(), HttpError> {
        match message {
            ToBackend::Dispatch {
                generation,
                sequence,
                deadline_nanos,
                request,
            } => {
                self.sequence(generation, sequence)?;
                let deadline = Deadline::from_wire(deadline_nanos, Duration::from_secs(300))
                    .ok_or_else(|| fault("owned_dispatch_budget"))?;
                let request = request
                    .into_http(&HttpLimits::default())
                    .map_err(|_| fault("owned_request_bounds"))?;
                let effect = route_effects::classify(&request);
                let recovery = route_effects::is_submission_recovery(&request);
                let ordinary = self
                    .pending
                    .values()
                    .filter(|pending| !pending.recovery)
                    .count();
                if self.pending.len() >= self.capacity
                    || (!recovery && ordinary >= self.capacity.saturating_sub(1).max(1))
                    || deadline.expired()
                    || self.generation.as_ref().is_some_and(|g| g.retiring)
                {
                    return self.refuse(sequence, "dispatch_not_started");
                }
                self.pending.insert(
                    sequence,
                    Pending {
                        deadline,
                        effect,
                        recovery,
                        request: Some(request.into()),
                        sent: false,
                    },
                );
                if self.generation.is_none() {
                    self.start_generation()?;
                }
            }
            ToBackend::Diagnostic {
                generation,
                sequence,
                line,
            } => {
                self.sequence(generation, sequence)?;
                if self.diagnostics.len() >= MAX_DIAGNOSTICS
                    || self
                        .generation
                        .as_ref()
                        .is_none_or(|g| !g.ready || g.retiring)
                {
                    return self.emit(FromBackend::DiagnosticLost {
                        generation: self.parent_generation,
                        sequence,
                    });
                }
                let deadline = Deadline::after(DIAGNOSTIC_BUDGET)
                    .ok_or_else(|| fault("owned_diagnostic_budget"))?;
                if let Some(current) = self.generation.as_mut() {
                    current
                        .frames
                        .queue(&ToBackend::Diagnostic {
                            generation: current.number,
                            sequence,
                            line,
                        })
                        .map_err(|_| fault("owned_backend_channel"))?;
                }
                self.diagnostics.insert(sequence, deadline);
            }
            #[cfg(test)]
            ToBackend::PanicForTest { generation } if generation == self.parent_generation => {
                crate::management::http::owned_process_tests::panic_helper();
            }
            _ => return Err(fault("owned_parent_protocol")),
        }
        Ok(())
    }

    fn sequence(&mut self, generation: u64, sequence: u64) -> Result<(), HttpError> {
        if generation != self.parent_generation || sequence <= self.last_sequence {
            return Err(fault("owned_parent_identity"));
        }
        self.last_sequence = sequence;
        Ok(())
    }

    fn tick_generation(&mut self) -> Result<(), HttpError> {
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.deadline.expired())
            .map(|(sequence, pending)| (*sequence, pending.sent))
            .collect();
        for (sequence, sent) in expired {
            if sent {
                self.retire()?;
                return Ok(());
            }
            self.pending.remove(&sequence);
            self.refuse(sequence, "dispatch_not_started")?;
        }
        if self.diagnostics.values().any(|deadline| deadline.expired()) {
            self.retire()?;
            return Ok(());
        }
        let Some(current) = self.generation.as_mut() else {
            return Ok(());
        };
        if current.retiring || (!current.ready && current.startup.expired()) {
            self.retire()?;
            return Ok(());
        }
        let received = current.frames.receive::<FromBackend>();
        if let Ok(Some(reply)) = received {
            self.reply(reply)?;
        } else if received.is_err() {
            self.retire()?;
            return Ok(());
        }
        self.forward_pending()?;
        if self
            .generation
            .as_mut()
            .is_some_and(|current| current.frames.flush().is_err())
        {
            self.retire()?;
        }
        Ok(())
    }

    fn forward_pending(&mut self) -> Result<(), HttpError> {
        let Some(current) = self.generation.as_mut().filter(|g| g.ready && !g.retiring) else {
            return Ok(());
        };
        for (sequence, pending) in &mut self.pending {
            if let Some(request) = pending.request.take() {
                current
                    .frames
                    .queue(&ToBackend::Dispatch {
                        generation: current.number,
                        sequence: *sequence,
                        deadline_nanos: pending.deadline.wire(),
                        request,
                    })
                    .map_err(|_| fault("owned_backend_channel"))?;
                pending.sent = true;
            }
        }
        Ok(())
    }

    fn reply(&mut self, reply: FromBackend) -> Result<(), HttpError> {
        let number = self
            .generation
            .as_ref()
            .ok_or_else(|| fault("owned_backend_identity"))?
            .number;
        match reply {
            FromBackend::Ready {
                version: WIRE_VERSION,
                generation,
            } if generation == number => {
                let current = self
                    .generation
                    .as_mut()
                    .ok_or_else(|| fault("owned_backend_identity"))?;
                if current.ready {
                    return Err(fault("owned_backend_identity"));
                }
                current.ready = true;
                if !self.announced {
                    self.emit(FromBackend::Ready {
                        version: WIRE_VERSION,
                        generation: self.parent_generation,
                    })?;
                    self.announced = true;
                }
            }
            FromBackend::Response {
                generation,
                sequence,
                status,
                body,
            } if generation == number => {
                let pending = self
                    .pending
                    .remove(&sequence)
                    .ok_or_else(|| fault("owned_backend_identity"))?;
                if !pending.sent {
                    return Err(fault("owned_backend_identity"));
                }
                owned_wire::into_response(status, body.clone(), &HttpLimits::default())
                    .map_err(|_| fault("owned_backend_response"))?;
                self.emit(FromBackend::Response {
                    generation: self.parent_generation,
                    sequence,
                    status,
                    body,
                })?;
            }
            FromBackend::DiagnosticWritten {
                generation,
                sequence,
            } if generation == number => {
                if self.diagnostics.remove(&sequence).is_none() {
                    return Err(fault("owned_backend_identity"));
                }
                self.emit(FromBackend::DiagnosticWritten {
                    generation: self.parent_generation,
                    sequence,
                })?;
            }
            FromBackend::DiagnosticLost {
                generation,
                sequence,
            } if generation == number => {
                if self.diagnostics.remove(&sequence).is_none() {
                    return Err(fault("owned_backend_identity"));
                }
                self.emit(FromBackend::DiagnosticLost {
                    generation: self.parent_generation,
                    sequence,
                })?;
            }
            _ => return Err(fault("owned_backend_identity")),
        }
        Ok(())
    }
}
