// SPDX-License-Identifier: MIT

//! Bounded connection admission and ownership.

use super::*;

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) struct ServerContext {
    pub(super) service: Arc<ManagementService>,
    pub(super) authenticator: Arc<dyn Authenticator>,
    pub(super) limits: HttpLimits,
    pub(super) stop: Arc<AtomicBool>,
    pub(super) failure_sink: ManagementFailureSink,
    pub(super) lifecycle_log: RequestLifecycleLog,
    pub(super) dispatch: Arc<dispatch_executor::DispatchExecutor>,
}

pub(super) fn run_server_loop(
    listener: TcpListener,
    context: ServerContext,
) -> Result<(), HttpError> {
    let mut workers = Vec::new();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        accept_connections(&listener, &context, &mut workers)
    }));
    context.stop.store(true, Ordering::Release);
    // Closing transport does not discard accepted work. This supervisor remains in the bounded
    // process registry until both connection and dispatch worker handles have been joined.
    drop(listener);
    for handle in workers {
        let _ = handle.join();
    }
    context.dispatch.drain();
    result.unwrap_or_else(|_| {
        Err(HttpError::new(
            "server_accept_panicked",
            "management admission terminated unexpectedly",
        ))
    })
}

fn accept_connections(
    listener: &TcpListener,
    context: &ServerContext,
    workers: &mut Vec<JoinHandle<()>>,
) -> Result<(), HttpError> {
    let active = Arc::new(AtomicUsize::new(0));
    loop {
        if context.stop.load(Ordering::Acquire) {
            break;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                if context.stop.load(Ordering::Acquire) {
                    break;
                }
                let current = active.load(Ordering::Acquire);
                if current >= context.limits.max_connections {
                    stream
                        .set_write_timeout(Some(context.limits.deadline))
                        .map_err(io_http_error)?;
                    let _ = write_raw_status(stream, 503, "Service Unavailable", &[]);
                    continue;
                }
                active.fetch_add(1, Ordering::AcqRel);
                let service = Arc::clone(&context.service);
                let authenticator = Arc::clone(&context.authenticator);
                let active_for_worker = Arc::clone(&active);
                let worker_limits = context.limits.clone();
                let worker_failure_sink = context.failure_sink.clone();
                let worker_log = context.lifecycle_log.clone();
                let worker_dispatch = Arc::clone(&context.dispatch);
                let worker = thread::Builder::new()
                    .name("sts2-management-connection".to_owned())
                    .spawn(move || {
                        let _active_connection = ActiveConnection(active_for_worker);
                        // The only evidence that the owner could not deliver an answer it had
                        // already computed: the socket is gone, so the peer can no longer be told,
                        // and that silence is what made #816 undiagnosable.
                        let failed = serve_reported(
                            stream,
                            service,
                            authenticator,
                            &worker_limits,
                            &worker_log,
                            &worker_dispatch,
                        );
                        if let Err((request, error)) = failed {
                            worker_failure_sink.report_connection(peer, request.as_deref(), &error);
                        }
                    })
                    .map_err(io_http_error)?;
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].is_finished() {
                        let _ = workers.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
                workers.push(worker);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(io_http_error(error)),
        }
    }
    Ok(())
}
