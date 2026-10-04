// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

#[path = "http_client.rs"]
mod client;
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

use parse::read_request;
use response::{
    auth_http_error, io_http_error, parse_bearer, validate_loopback, wake_listener,
    write_raw_status, write_response,
};
use routes::dispatch;

pub use client::{ClientResponse, ManagementClient};

#[cfg(test)]
#[path = "http_connection_tests.rs"]
mod connection_tests;

#[cfg(test)]
#[path = "http_diagnostics_tests.rs"]
mod diagnostics_tests;

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
        })
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

/// A connection failure the peer can no longer be told about.
///
/// Once `handle_connection` has failed, the socket is gone and the only place
/// the fact can still go is the process's own stderr. That makes the report
/// write-only and awkward to assert on: a test can capture stderr, but the
/// obvious way to break this -- deleting the `eprintln!` -- passes every other
/// test in the suite, because nothing else observes it. Routing the report
/// through this sink is what makes the line testable at all (harness#819).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndeliveredResponse {
    /// The peer's address, so a report can be tied to a connection.
    pub peer: SocketAddr,
    /// The management error's stable code, e.g. `deadline_exceeded`.
    pub code: String,
    /// The error's human-readable message.
    pub message: String,
}

/// Where an undelivered-response report goes.
///
/// Production writes to stderr. Tests install a sink so the line is asserted
/// rather than merely reviewed.
pub trait DiagnosticsSink: Send + Sync + 'static {
    fn report(&self, report: &UndeliveredResponse);
}

/// The default sink: one attributed line on stderr.
#[derive(Clone, Copy, Debug, Default)]
pub struct StderrDiagnostics;

impl DiagnosticsSink for StderrDiagnostics {
    fn report(&self, report: &UndeliveredResponse) {
        // Attribution is the whole point of this line: without the peer it is
        // a fact about *a* connection, not about a request, and cannot be tied
        // back to anything when a run goes red (harness#819).
        eprintln!(
            "sts2-management peer={} code={} response not delivered: {}",
            report.peer, report.code, report.message
        );
    }
}

impl ManagementServer {
    pub fn start(
        config: ServerConfig,
        service: Arc<ManagementService>,
    ) -> Result<ServerHandle, HttpError> {
        Self::start_with_diagnostics(config, service, Arc::new(StderrDiagnostics))
    }

    /// Start the server with an explicit diagnostics sink.
    ///
    /// `start` delegates here with `StderrDiagnostics`, so the production path
    /// is unchanged; the seam exists so the undelivered-response report can be
    /// asserted in a test rather than only reviewed.
    pub fn start_with_diagnostics(
        config: ServerConfig,
        service: Arc<ManagementService>,
        diagnostics: Arc<dyn DiagnosticsSink>,
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
        let worker_diagnostics = Arc::clone(&diagnostics);
        let join = thread::Builder::new()
            .name("sts2-management-server".to_owned())
            .spawn(move || {
                run_server_loop(
                    listener,
                    service,
                    authenticator,
                    limits,
                    thread_stop,
                    worker_diagnostics,
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
    diagnostics: Arc<dyn DiagnosticsSink>,
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
                let worker_sink = Arc::clone(&diagnostics);
                let worker = thread::Builder::new()
                    .name("sts2-management-connection".to_owned())
                    .spawn(move || {
                        // The only evidence that the owner could not deliver an
                        // answer it had already computed. The peer can no longer
                        // be told -- the socket is gone -- but the fact must not
                        // vanish; that silence is what made #816 undiagnosable.
                        if let Err(error) =
                            handle_connection(stream, &service, &*authenticator, &worker_limits)
                        {
                            worker_sink.report(&UndeliveredResponse {
                                peer,
                                code: error.code.clone(),
                                message: error.message.clone(),
                            });
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

fn handle_connection(
    mut stream: TcpStream,
    service: &ManagementService,
    authenticator: &dyn Authenticator,
    limits: &HttpLimits,
) -> Result<(), HttpError> {
    // Each phase gets its own budget rather than sharing one taken before the
    // read. A shared deadline meant an overran read left the write with an
    // expired budget, which `write_with_deadline` refuses *before* any syscall:
    // the peer got nothing instead of the computed response, and saw a silent
    // close (`ECONNRESET`) for a request the owner had answered. That is what
    // made Studio's `submission_refused_502` undiagnosable (#816, studio #214).
    // The bound itself is unchanged -- `limits.deadline` still caps each phase.
    let read_deadline = Instant::now() + limits.deadline;
    let response = match read_request(&mut stream, read_deadline, limits) {
        Ok(request) => dispatch(request, service, authenticator),
        Err(error) => Err(error),
    };
    write_response(
        &mut stream,
        response,
        Instant::now() + limits.deadline,
        limits,
    )
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
