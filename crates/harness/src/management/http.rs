// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[path = "http_client.rs"]
mod client;
#[path = "http_failure_report.rs"]
mod failure_report;
#[path = "http_lifecycle.rs"]
mod lifecycle;
#[path = "http_lifecycle_log.rs"]
mod lifecycle_log;
#[path = "http_parse.rs"]
mod parse;
#[path = "http_response.rs"]
mod response;
#[path = "http_routes.rs"]
mod routes;
#[path = "http_routes_effective_limits.rs"]
mod routes_effective_limits;
#[path = "http_routes_lifecycle.rs"]
mod routes_lifecycle;
#[path = "http_routes_memory_owner.rs"]
mod routes_memory_owner;
#[path = "http_routes_policy.rs"]
mod routes_policy;
#[path = "http_routes_run.rs"]
mod routes_run;

pub use failure_report::{ManagementFailurePort, ManagementFailureSink};
use lifecycle::serve_reported;
use lifecycle_log::RequestLifecycleLog;
use response::{
    auth_http_error, io_http_error, parse_bearer, validate_loopback, wake_listener,
    write_raw_status,
};

pub use client::{ClientResponse, ManagementClient};

#[cfg(test)]
#[path = "http_connection_tests.rs"]
mod connection_tests;
#[cfg(test)]
#[path = "http_lifecycle_log_server_tests.rs"]
mod lifecycle_log_server_tests;

use super::auth::Authenticator;
use super::contract::{
    CommandRequest, DiffRequest, ExportRequest, InspectRequest, MAX_CONNECTIONS, MAX_HEADER_BYTES,
    MAX_JSON_BYTES, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, REQUEST_DEADLINE_MILLIS, ReplayRequest,
    RunRequest, TargetAdmissionRequest, ValidateRequest, decode_strict, validate_identifier,
};
use super::service::{ManagementError, ManagementService};

const READ_BUFFER_BYTES: usize = 2048;

#[derive(Clone, Debug)]
pub struct HttpLimits {
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub max_response_bytes: usize,
    pub max_path_bytes: usize,
    pub max_connections: usize,
    pub deadline: Duration,
}

impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_header_bytes: MAX_HEADER_BYTES,
            max_body_bytes: MAX_JSON_BYTES,
            max_response_bytes: MAX_RESPONSE_BYTES,
            max_path_bytes: MAX_PATH_BYTES,
            max_connections: MAX_CONNECTIONS,
            deadline: Duration::from_millis(REQUEST_DEADLINE_MILLIS),
        }
    }
}

impl HttpLimits {
    fn validate(&self) -> Result<(), HttpError> {
        if self.max_header_bytes == 0
            || self.max_header_bytes > MAX_HEADER_BYTES
            || self.max_body_bytes == 0
            || self.max_body_bytes > MAX_JSON_BYTES
            || self.max_response_bytes == 0
            || self.max_response_bytes > MAX_RESPONSE_BYTES
            || self.max_path_bytes == 0
            || self.max_path_bytes > MAX_PATH_BYTES
            || self.max_connections == 0
            || self.max_connections > MAX_CONNECTIONS
            || self.deadline.is_zero()
        {
            return Err(HttpError::new(
                "invalid_limits",
                "management HTTP limits are outside the admitted bounds",
            ));
        }
        Ok(())
    }
}

pub struct ServerConfig {
    pub listen: SocketAddr,
    pub authenticator: Arc<dyn Authenticator>,
    pub limits: HttpLimits,
    /// The port an undeliverable management response is reported through.
    pub failure_sink: ManagementFailureSink,
    /// Per-request lifecycle reporting (#820). Real stderr by default; the in-crate
    /// tests inject a sink. Not `pub`: a test seam, not a public knob.
    pub(in crate::management::http) lifecycle_log: RequestLifecycleLog,
}

impl ServerConfig {
    pub fn new(
        listen: SocketAddr,
        authenticator: Arc<dyn Authenticator>,
    ) -> Result<Self, HttpError> {
        validate_loopback(listen)?;
        let limits = HttpLimits::default();
        limits.validate()?;
        Ok(Self {
            listen,
            authenticator,
            limits,
            failure_sink: ManagementFailureSink::default(),
            lifecycle_log: RequestLifecycleLog::default(),
        })
    }

    /// Attaches the reporting port. See [`ManagementFailureSink`].
    pub fn with_failure_sink(mut self, failure_sink: ManagementFailureSink) -> Self {
        self.failure_sink = failure_sink;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpError {
    pub code: String,
    pub message: String,
    status: u16,
}

impl HttpError {
    fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            status: 400,
        }
    }

    /// The status the peer is told. Read by the lifecycle log so its terminal marker
    /// reports what the client saw, rather than re-deriving it and drifting.
    pub(crate) fn status(&self) -> u16 {
        self.status
    }
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

pub struct ServerHandle {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<Result<(), HttpError>>>,
}

impl ServerHandle {
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    pub fn shutdown(mut self) -> Result<(), HttpError> {
        self.stop.store(true, Ordering::Release);
        wake_listener(self.address);
        let join = self.join.take().ok_or_else(|| {
            HttpError::new(
                "server_join_missing",
                "management server join handle is missing",
            )
        })?;
        join.join().map_err(|_| {
            HttpError::new(
                "server_thread_panicked",
                "management server thread terminated unexpectedly",
            )
        })?
    }

    pub fn wait(mut self) -> Result<(), HttpError> {
        let join = self.join.take().ok_or_else(|| {
            HttpError::new(
                "server_join_missing",
                "management server join handle is missing",
            )
        })?;
        join.join().map_err(|_| {
            HttpError::new(
                "server_thread_panicked",
                "management server thread terminated unexpectedly",
            )
        })?
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        wake_listener(self.address);
    }
}

pub struct ManagementServer;

impl ManagementServer {
    pub fn start(
        config: ServerConfig,
        service: Arc<ManagementService>,
    ) -> Result<ServerHandle, HttpError> {
        config.limits.validate()?;
        validate_loopback(config.listen)?;
        let listener = TcpListener::bind(config.listen).map_err(io_http_error)?;
        listener.set_nonblocking(true).map_err(io_http_error)?;
        let address = listener.local_addr().map_err(io_http_error)?;
        validate_loopback(address)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let limits = config.limits;
        let authenticator = Arc::clone(&config.authenticator);
        let failure_sink = config.failure_sink;
        let lifecycle_log = config.lifecycle_log.clone();
        let join = thread::Builder::new()
            .name("sts2-management-server".to_owned())
            .spawn(move || {
                run_server_loop(
                    listener,
                    service,
                    authenticator,
                    limits,
                    thread_stop,
                    failure_sink,
                    lifecycle_log,
                )
            })
            .map_err(io_http_error)?;
        Ok(ServerHandle {
            address,
            stop,
            join: Some(join),
        })
    }
}

fn run_server_loop(
    listener: TcpListener,
    service: Arc<ManagementService>,
    authenticator: Arc<dyn Authenticator>,
    limits: HttpLimits,
    stop: Arc<AtomicBool>,
    failure_sink: ManagementFailureSink,
    lifecycle_log: RequestLifecycleLog,
) -> Result<(), HttpError> {
    let active = Arc::new(AtomicUsize::new(0));
    let workers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match listener.accept() {
            Ok((stream, peer)) => {
                let current = active.load(Ordering::Acquire);
                if current >= limits.max_connections {
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
                let worker = thread::Builder::new()
                    .name("sts2-management-connection".to_owned())
                    .spawn(move || {
                        // The only evidence that the owner could not deliver an answer it had
                        // already computed: the socket is gone, so the peer can no longer be told,
                        // and that silence is what made #816 undiagnosable.
                        let failed = serve_reported(
                            stream,
                            &service,
                            &*authenticator,
                            &worker_limits,
                            &worker_log,
                        );
                        if let Err((request, error)) = failed {
                            worker_failure_sink.report_connection(peer, request.as_deref(), &error);
                        }
                        active_for_worker.fetch_sub(1, Ordering::AcqRel);
                    })
                    .map_err(io_http_error)?;
                if let Ok(mut handles) = workers.lock() {
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
    Ok(())
}

#[derive(Clone, Debug)]
struct HttpRequest {
    method: String,
    path: String,
    query: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug)]
struct HttpResponse {
    status: u16,
    reason: &'static str,
    body: Vec<u8>,
}
