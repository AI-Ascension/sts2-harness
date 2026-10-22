// SPDX-License-Identifier: MIT

//! Bounded connection admission and ownership.

use super::*;

struct ActiveConnection(Arc<AtomicUsize>);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(super) fn run_server_loop(
    listener: TcpListener,
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    limits: HttpLimits,
    stop: Arc<AtomicBool>,
    failure_sink: ManagementFailureSink,
    lifecycle_log: RequestLifecycleLog,
) -> Result<(), HttpError> {
    let active = Arc::new(AtomicUsize::new(0));
    let dispatch_executor = Arc::new(dispatch_executor::DispatchExecutor::new(
        limits.max_connections,
    ));
    let workers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let current = active.load(Ordering::Acquire);
                if current >= limits.max_connections {
                    stream
                        .set_write_timeout(Some(limits.deadline))
                        .map_err(io_http_error)?;
                    let _ = write_raw_status(stream, 503, "Service Unavailable", &[]);
                    continue;
                }
                active.fetch_add(1, Ordering::AcqRel);
                let service = Arc::clone(&service);
                let authenticator = Arc::clone(&authenticator);
                let active_for_worker = Arc::clone(&active);
                let worker_limits = limits.clone();
                let worker_failure_sink = failure_sink.clone();
                let worker_log = lifecycle_log.clone();
                let worker_dispatch = Arc::clone(&dispatch_executor);
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
                if let Ok(mut handles) = workers.lock() {
                    let mut index = 0;
                    while index < handles.len() {
                        if handles[index].is_finished() {
                            let _ = handles.swap_remove(index).join();
                        } else {
                            index += 1;
                        }
                    }
                    handles.push(worker);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(io_http_error(error)),
        }
    }
    if let Ok(mut handles) = workers.lock() {
        for handle in handles.drain(..) {
            let _ = handle.join();
        }
    }
    dispatch_executor.shutdown()
}
