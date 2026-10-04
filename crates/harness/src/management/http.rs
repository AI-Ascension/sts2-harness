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

#[path = "http_diagnostics.rs"]
mod diagnostics;

#[path = "http_limits.rs"]
mod limits_module;

use parse::read_request;
use response::{
    auth_http_error, io_http_error, parse_bearer, validate_loopback, wake_listener,
    write_raw_status, write_response,
};
use routes::dispatch;

pub use diagnostics::{DiagnosticsSink, StderrDiagnostics, UndeliveredResponse};
pub use limits_module::HttpLimits;

// The sibling `http_*` modules reached the admitted bounds through this
// module, so they keep resolving here even though `HttpLimits` itself moved.
pub use limits_module::{
    MAX_HEADER_BYTES, MAX_JSON_BYTES, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, REQUEST_DEADLINE_MILLIS,
};
pub use client::{ClientResponse, ManagementClient};

#[cfg(test)]
#[path = "http_connection_tests.rs"]
mod connection_tests;

#[cfg(test)]
#[path = "http_diagnostics_tests.rs"]
mod diagnostics_tests;

use super::auth::Authenticator;
use super::contract::{
    CommandRequest, DiffRequest, ExportRequest, InspectRequest, ReplayRequest, RunRequest,
    TargetAdmissionRequest, ValidateRequest, decode_strict, validate_identifier,
};
use super::service::{ManagementError, ManagementService};

const READ_BUFFER_BYTES: usize = 2048;

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

impl ManagementServer {
    pub fn start(
        config: ServerConfig,
        service: Arc<ManagementService>,
    ) -> Result<ServerHandle, HttpError> {
        Self::start_with_diagnostics(config, service, Arc::new(StderrDiagnostics))
    }

    /// Start the server with an explicit diagnostics sink. `start` delegates
    /// here with `StderrDiagnostics`, so the production path is unchanged; the
    /// seam exists so the undelivered-response report can be asserted in a test
    /// rather than only reviewed (harness#819).
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
        let join = thread::Builder::new()
            .name("sts2-management-server".to_owned())
            .spawn(move || {
                run_server_loop(listener, service, authenticator, limits, thread_stop, diagnostics)
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
                        // answer it had already computed: the peer can no longer
                        // be told, so the fact must not vanish (#816, #819).
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
