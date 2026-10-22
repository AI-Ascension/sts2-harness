// SPDX-License-Identifier: MIT

//! Persistent service composition behind an inherited private socket.
//!
//! Only this backend constructs authentication, journals, runtime and provider ports. The
//! supervisor can terminate the entire backend without pretending Rust callbacks cooperated.

use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::Instant;

use super::owned_wire::{Frames, FromBackend, MAX_DIAGNOSTIC_BYTES, ToBackend, WIRE_VERSION};
use super::*;

pub(super) const BACKEND_ROLE: &str = "STS2_MANAGEMENT_OWNED_ROLE";
const MAX_DIAGNOSTICS: usize = 64;

pub(super) fn requested() -> bool {
    std::env::var(BACKEND_ROLE).as_deref() == Ok("backend")
}

pub(super) fn run(
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
) -> Result<(), HttpError> {
    if !requested() {
        return Err(fault("owned_backend_role"));
    }
    let socket = UnixStream::from(
        std::io::stdin()
            .as_fd()
            .try_clone_to_owned()
            .map_err(|_| fault("owned_backend_channel"))?,
    );
    if !socket
        .local_addr()
        .map_err(|_| fault("owned_backend_channel"))?
        .is_unnamed()
        || !socket
            .peer_addr()
            .map_err(|_| fault("owned_backend_channel"))?
            .is_unnamed()
    {
        return Err(fault("owned_backend_channel"));
    }
    let mut frames = Frames::new(socket).map_err(|_| fault("owned_backend_channel"))?;
    let (generation, capacity) = handshake(&mut frames)?;
    frames
        .queue(&FromBackend::Ready {
            version: WIRE_VERSION,
            generation,
        })
        .map_err(|_| fault("owned_backend_channel"))?;
    let (sender, receiver) = mpsc::sync_channel(capacity + MAX_DIAGNOSTICS);
    let mut backend = Backend {
        service,
        authenticator,
        limits: HttpLimits::default(),
        generation,
        sequence: 0,
        capacity,
        workers: Vec::new(),
        sender,
        receiver,
        stopping: false,
    };
    backend.serve(&mut frames)
}

fn handshake(frames: &mut Frames) -> Result<(u64, usize), HttpError> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match frames
            .receive()
            .map_err(|_| fault("owned_backend_handshake"))?
        {
            Some(ToBackend::Hello {
                version: WIRE_VERSION,
                generation,
                capacity,
            }) if generation != 0 && (1..=MAX_CONNECTIONS).contains(&capacity) => {
                return Ok((generation, capacity));
            }
            Some(_) => return Err(fault("owned_backend_handshake")),
            None if Instant::now() >= deadline => {
                return Err(fault("owned_backend_handshake"));
            }
            None => thread::sleep(Duration::from_millis(1)),
        }
    }
}

struct Backend {
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    limits: HttpLimits,
    generation: u64,
    sequence: u64,
    capacity: usize,
    workers: Vec<Worker>,
    sender: SyncSender<FromBackend>,
    receiver: Receiver<FromBackend>,
    stopping: bool,
}

struct Worker {
    diagnostic: bool,
    recovery: bool,
    handle: JoinHandle<()>,
}

impl Backend {
    fn serve(&mut self, frames: &mut Frames) -> Result<(), HttpError> {
        loop {
            self.reap()?;
            for _ in 0..self.capacity + MAX_DIAGNOSTICS {
                match self.receiver.try_recv() {
                    Ok(reply) => frames
                        .queue(&reply)
                        .map_err(|_| fault("owned_backend_reply_bounds"))?,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Err(fault("owned_backend_worker")),
                }
            }
            frames.flush().map_err(|_| fault("owned_backend_channel"))?;
            if self.stopping && self.workers.is_empty() && frames.output_empty() {
                return Ok(());
            }
            if let Some(message) = frames
                .receive()
                .map_err(|_| fault("owned_backend_channel"))?
            {
                self.accept(message)?;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn accept(&mut self, message: ToBackend) -> Result<(), HttpError> {
        match message {
            ToBackend::Dispatch {
                generation,
                sequence,
                deadline_nanos,
                request,
            } => {
                let deadline = super::owned_deadline::Deadline::from_wire(
                    deadline_nanos,
                    Duration::from_secs(300),
                )
                .ok_or_else(|| fault("owned_backend_deadline"))?;
                let request = request
                    .into_http(&self.limits)
                    .map_err(|_| fault("owned_backend_request_bounds"))?;
                let recovery = route_effects::is_submission_recovery(&request);
                if !self.admit(generation, sequence, false, recovery)? {
                    let response = response::error_response(
                        HttpError::from_management(ManagementError::unavailable(
                            "dispatch_capacity_exhausted",
                            "no additional management handler was started",
                        )),
                        self.limits.max_response_bytes,
                    )?;
                    return self.send(FromBackend::Response {
                        generation,
                        sequence,
                        status: response.status,
                        body: response.body,
                    });
                }
                let service = Arc::clone(&self.service);
                let auth = Arc::clone(&self.authenticator);
                let sender = self.sender.clone();
                let response_limit = self.limits.max_response_bytes;
                let worker = thread::Builder::new()
                    .name("sts2-owned-dispatch".to_owned())
                    .spawn(move || {
                        let response = if deadline.expired() {
                            Err(HttpError::from_management(ManagementError::unavailable(
                                "dispatch_not_started",
                                "the original dispatch deadline expired before handler admission",
                            )))
                        } else {
                            routes::dispatch(request, &service, &*auth)
                        }
                        .or_else(|error| response::error_response(error, response_limit));
                        if let Ok(response) = response {
                            let _ = sender.send(FromBackend::Response {
                                generation,
                                sequence,
                                status: response.status,
                                body: response.body,
                            });
                        }
                    })
                    .map_err(|_| fault("owned_backend_worker"))?;
                self.workers.push(Worker {
                    diagnostic: false,
                    recovery,
                    handle: worker,
                });
            }
            ToBackend::Diagnostic {
                generation,
                sequence,
                line,
            } => {
                if !self.admit(generation, sequence, true, false)? {
                    return self.send(FromBackend::DiagnosticLost {
                        generation,
                        sequence,
                    });
                }
                if line.len() > MAX_DIAGNOSTIC_BYTES || line.contains(['\r', '\n']) {
                    return Err(fault("owned_backend_diagnostic_bounds"));
                }
                let sender = self.sender.clone();
                let worker = thread::Builder::new()
                    .name("sts2-owned-diagnostic".to_owned())
                    .spawn(move || {
                        // The frontend never touches stderr or its process-global mutex. This
                        // potentially blocking write is inside the supervised process budget.
                        eprintln!("{line}");
                        let _ = sender.send(FromBackend::DiagnosticWritten {
                            generation,
                            sequence,
                        });
                    })
                    .map_err(|_| fault("owned_backend_worker"))?;
                self.workers.push(Worker {
                    diagnostic: true,
                    recovery: false,
                    handle: worker,
                });
            }
            ToBackend::Stop { generation } if generation == self.generation => {
                self.stopping = true;
            }
            _ => return Err(fault("owned_backend_identity")),
        }
        Ok(())
    }

    fn admit(
        &mut self,
        generation: u64,
        sequence: u64,
        diagnostic: bool,
        recovery: bool,
    ) -> Result<bool, HttpError> {
        let count = self
            .workers
            .iter()
            .filter(|worker| worker.diagnostic == diagnostic)
            .count();
        let maximum = if diagnostic {
            MAX_DIAGNOSTICS
        } else {
            self.capacity
        };
        if self.stopping || generation != self.generation || sequence <= self.sequence {
            return Err(fault("owned_backend_admission"));
        }
        self.sequence = sequence;
        let ordinary = self
            .workers
            .iter()
            .filter(|worker| !worker.diagnostic && !worker.recovery)
            .count();
        Ok(count < maximum
            && (diagnostic || recovery || ordinary < self.capacity.saturating_sub(1).max(1)))
    }

    fn send(&self, message: FromBackend) -> Result<(), HttpError> {
        self.sender
            .try_send(message)
            .map_err(|_| fault("owned_backend_reply_bounds"))
    }

    fn reap(&mut self) -> Result<(), HttpError> {
        let mut index = 0;
        while index < self.workers.len() {
            if self.workers[index].handle.is_finished() {
                self.workers
                    .swap_remove(index)
                    .handle
                    .join()
                    .map_err(|_| fault("owned_backend_panicked"))?;
            } else {
                index += 1;
            }
        }
        Ok(())
    }
}

fn fault(code: &'static str) -> HttpError {
    HttpError::from_management(ManagementError::unavailable(
        code,
        "owned management backend is unavailable",
    ))
}
