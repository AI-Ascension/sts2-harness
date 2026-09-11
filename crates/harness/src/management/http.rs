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

use parse::read_request;
use response::{
    auth_http_error, io_http_error, parse_bearer, validate_loopback, wake_listener,
    write_raw_status, write_response,
};
use routes::dispatch;

pub use client::{ClientResponse, ManagementClient};

use super::auth::Authenticator;
use super::contract::{
    CommandRequest, DiffRequest, ExportRequest, InspectRequest, MAX_CONNECTIONS, MAX_HEADER_BYTES,
    MAX_JSON_BYTES, MAX_PATH_BYTES, MAX_RESPONSE_BYTES, REQUEST_DEADLINE_MILLIS, ReplayRequest,
    RunRequest, ValidateRequest, decode_strict, validate_identifier,
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
        let join = thread::Builder::new()
            .name("sts2-management-server".to_owned())
            .spawn(move || run_server_loop(listener, service, authenticator, limits, thread_stop))
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
) -> Result<(), HttpError> {
    let active = Arc::new(AtomicUsize::new(0));
    let workers: Arc<Mutex<Vec<JoinHandle<()>>>> = Arc::new(Mutex::new(Vec::new()));
    loop {
        if stop.load(Ordering::Acquire) {
            break;
        }
        match listener.accept() {
            Ok((stream, _peer)) => {
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
                let worker = thread::Builder::new()
                    .name("sts2-management-connection".to_owned())
                    .spawn(move || {
                        let _ =
                            handle_connection(stream, &service, &*authenticator, &worker_limits);
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
    let deadline = Instant::now() + limits.deadline;
    let response = match read_request(&mut stream, deadline, limits) {
        Ok(request) => dispatch(request, service, authenticator),
        Err(error) => Err(error),
    };
    write_response(&mut stream, response, deadline, limits)
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
