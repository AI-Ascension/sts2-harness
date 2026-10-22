// SPDX-License-Identifier: MIT

//! Bounded HTTP frontend. Only the dedicated helper owns or reaps backend descendants.

use std::io::Write;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::process::{Child, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};
use std::time::Instant;

use super::owned_deadline::Deadline;
use super::owned_proxy::{Proxy, Work, lost, unavailable, uncertain};
use super::owned_supervisor::{REAP_BUDGET, same_binary};
use super::owned_wire::{Frames, FromBackend, ToBackend, WIRE_VERSION};
use super::*;

#[path = "http_owned_frontend_bridge.rs"]
mod bridge;
use bridge::Bridge;

pub(super) fn serve(listen: SocketAddr, limits: HttpLimits) -> Result<(), HttpError> {
    serve_with_stop(listen, limits, &AtomicBool::new(false))
}

pub(super) fn serve_with_stop(
    listen: SocketAddr,
    limits: HttpLimits,
    stop: &AtomicBool,
) -> Result<(), HttpError> {
    limits.validate()?;
    validate_loopback(listen)?;
    let (work, requests) = mpsc::sync_channel(limits.max_connections);
    let (diagnostics, logs) = mpsc::sync_channel(64);
    let proxy = Proxy {
        work,
        diagnostics,
        lost_diagnostics: Arc::new(AtomicU64::new(0)),
        origin: Instant::now(),
    };
    let mut bridge = Bridge::spawn(
        requests,
        logs,
        Arc::clone(&proxy.lost_diagnostics),
        limits.clone(),
    )?;
    let mut workers = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        bridge.await_ready()?;
        let listener = TcpListener::bind(listen).map_err(io_http_error)?;
        listener.set_nonblocking(true).map_err(io_http_error)?;
        proxy.log(&format!(
            "sts2-management serving authenticated loopback API at {} mode=owned_backend",
            listener.local_addr().map_err(io_http_error)?
        ));
        let active = Arc::new(AtomicUsize::new(0));
        let log = RequestLifecycleLog::with_sink(Arc::new(proxy.clone()));
        let failure = ManagementFailureSink::new(Arc::new(proxy.clone()));
        loop {
            if stop.load(Ordering::Acquire) {
                return Ok(());
            }
            bridge.tick()?;
            reap_connections(&mut workers);
            match listener.accept() {
                Ok((mut stream, peer)) => {
                    if active.load(Ordering::Acquire) >= limits.max_connections {
                        // No synchronous peer write is allowed to delay backend supervision.
                        let _ = stream.set_nonblocking(true);
                        let _ = stream.write(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        continue;
                    }
                    let active_for_worker = Arc::clone(&active);
                    let worker_proxy = proxy.clone();
                    let worker_limits = limits.clone();
                    let worker_log = log.clone();
                    let worker_failure = failure.clone();
                    active.fetch_add(1, Ordering::AcqRel);
                    let worker = thread::Builder::new()
                        .name("sts2-owned-http".to_owned())
                        .spawn(move || {
                            let _active = Connection(active_for_worker);
                            if let Err((route, error)) = lifecycle::serve_owned_reported(
                                stream,
                                &worker_limits,
                                &worker_log,
                                &worker_proxy,
                            ) {
                                worker_failure.report_connection(peer, route.as_deref(), &error);
                            }
                        });
                    match worker {
                        Ok(worker) => workers.push(worker),
                        Err(error) => {
                            active.fetch_sub(1, Ordering::AcqRel);
                            return Err(io_http_error(error));
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(io_http_error(error)),
            }
            thread::sleep(Duration::from_millis(1));
        }
    }));
    // Closing the channel tells the helper to terminate and reap its generation, including if
    // this loop failed. No helper handle or connection join handle is discarded on return.
    bridge.shutdown();
    for worker in workers {
        let _ = worker.join();
    }
    result.unwrap_or_else(|_| Err(unavailable("owned_frontend_panicked")))
}

struct Connection(Arc<AtomicUsize>);
impl Drop for Connection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn reap_connections(workers: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < workers.len() {
        if workers[index].is_finished() {
            let _ = workers.swap_remove(index).join();
        } else {
            index += 1;
        }
    }
}

struct Waiting {
    deadline: Deadline,
    effect: route_effects::RouteEffect,
    recovery: bool,
    reply: SyncSender<Result<HttpResponse, HttpError>>,
}

struct Helper {
    child: Child,
    frames: Option<Frames>,
}

impl Helper {
    fn spawn(capacity: usize) -> Result<Self, HttpError> {
        let (socket, inherited) = UnixStream::pair().map_err(io_http_error)?;
        let mut frames = Frames::new(socket).map_err(|_| unavailable("owned_helper_channel"))?;
        frames
            .queue(&ToBackend::Hello {
                version: WIRE_VERSION,
                generation: 1,
                capacity,
            })
            .map_err(|_| unavailable("owned_helper_channel"))?;
        let descriptor: OwnedFd = inherited.into();
        let mut command = same_binary("supervisor")?;
        command.stdin(Stdio::from(descriptor));
        let child = command
            .spawn()
            .map_err(|_| unavailable("owned_helper_spawn"))?;
        Ok(Self {
            child,
            frames: Some(frames),
        })
    }

    fn channel(&mut self) -> Result<&mut Frames, HttpError> {
        self.frames
            .as_mut()
            .ok_or_else(|| unavailable("owned_helper_closed"))
    }

    fn shutdown(&mut self) {
        self.frames = None;
        // The helper itself owns draining and remains alive if descendant reaping is pending.
        // An embedding host never signals/reaps arbitrary descendants or spawns a replacement.
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) | Err(_) => thread::sleep(Duration::from_millis(1)),
            }
        }
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        self.shutdown();
    }
}
