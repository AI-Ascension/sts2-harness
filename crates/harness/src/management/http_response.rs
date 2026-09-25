// SPDX-License-Identifier: MIT

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use super::super::auth::AuthError;
use super::super::contract::{ErrorClass, ErrorResponse};
use super::*;

pub(super) fn write_response(
    stream: &mut TcpStream,
    response: Result<HttpResponse, HttpError>,
    deadline: Instant,
    limits: &HttpLimits,
) -> Result<(), HttpError> {
    let response = match response {
        Ok(response) => response,
        Err(error) => error_response(error, limits.max_response_bytes)?,
    };
    if response.body.len() > limits.max_response_bytes {
        return Err(HttpError::new(
            "response_too_large",
            "response exceeds the bound",
        ));
    }
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        response.reason,
        response.body.len()
    );
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&response.body);
    write_with_deadline(stream, &bytes, deadline)
}

pub(super) fn error_response(
    error: HttpError,
    max_response_bytes: usize,
) -> Result<HttpResponse, HttpError> {
    let body = serde_json::to_vec(&error.error_response())
        .map_err(|encode| HttpError::new("response_encode", encode.to_string()))?;
    if body.len() > max_response_bytes {
        return Err(HttpError::new(
            "response_too_large",
            "error response exceeds the bound",
        ));
    }
    Ok(HttpResponse {
        status: error.status,
        reason: reason_phrase(error.status),
        body,
    })
}

pub(super) fn write_raw_status(
    mut stream: TcpStream,
    status: u16,
    reason: &'static str,
    body: &[u8],
) -> Result<(), HttpError> {
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(io_http_error)?;
    stream.write_all(body).map_err(io_http_error)
}

pub(super) fn parse_bearer(header: Option<&String>) -> Result<Option<&str>, HttpError> {
    let Some(header) = header else {
        return Ok(None);
    };
    let token = header.strip_prefix("Bearer ").ok_or_else(|| {
        HttpError::new(
            "invalid_authorization",
            "Authorization must use Bearer credentials",
        )
    })?;
    if token.is_empty() || token.bytes().any(|byte| byte.is_ascii_whitespace()) {
        return Err(HttpError::new(
            "invalid_authorization",
            "Authorization credential is invalid",
        ));
    }
    Ok(Some(token))
}

pub(super) fn read_with_deadline(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<usize, HttpError> {
    retry_transient(deadline, "request deadline exceeded", |timeout| {
        stream.set_read_timeout(Some(timeout))?;
        stream.read(buffer)
    })
}

// One syscall's worth of the read/write attempt is passed in so the retry
// policy here can be driven by a deterministic sequence of `io::Error`s in
// tests rather than by racing a real signal against a real socket.
fn retry_transient<T>(
    deadline: Instant,
    deadline_message: &'static str,
    mut attempt: impl FnMut(Duration) -> io::Result<T>,
) -> Result<T, HttpError> {
    loop {
        let timeout = deadline.saturating_duration_since(Instant::now());
        if timeout.is_zero() {
            return Err(deadline_exceeded(deadline_message));
        }
        match attempt(timeout) {
            // `Interrupted` is retryable by contract and is not a property of
            // the peer: a signal delivered to this thread mid-syscall makes the
            // kernel report the call as restartable even though the peer is
            // still healthy and still writing. Surfacing it turns a delivered
            // `SIGCHLD` into a fatal, status-400 management error for a request
            // that never failed. The deadline is re-derived from
            // `Instant::now()` on each pass, so this cannot spin: a peer that
            // never writes still terminates as `deadline_exceeded`.
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                return Err(deadline_exceeded(deadline_message));
            }
            result => return result.map_err(io_http_error),
        }
    }
}

fn deadline_exceeded(message: &'static str) -> HttpError {
    HttpError::new("deadline_exceeded", message)
}

pub(super) fn write_with_deadline(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), HttpError> {
    let timeout = deadline.saturating_duration_since(Instant::now());
    if timeout.is_zero() {
        return Err(HttpError::new(
            "deadline_exceeded",
            "response deadline exceeded",
        ));
    }
    stream
        .set_write_timeout(Some(timeout))
        .map_err(io_http_error)?;
    // `Interrupted` needs no arm here, unlike `read_with_deadline`: this call
    // is `Write::write_all`, whose default implementation already retries
    // `ErrorKind::Interrupted` and resumes the partial write, so a
    // `SIGCHLD` delivered mid-write cannot surface through it. Two independent
    // confirmations, because this is the whole reason the write path is left
    // alone: `std::net::TcpStream` does not override `write_all`, and the
    // default implementation in `std::io` carries `Err(ref e) if
    // e.is_interrupted() => {}`. A synthetic writer that returns `Interrupted`
    // for its first attempt returns `Ok(())` from `write_all` after exactly two
    // `write` calls. Adding a retry here would be uncovered code.
    stream.write_all(bytes).map_err(io_http_error)
}

pub(super) fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

pub(super) fn validate_loopback(address: SocketAddr) -> Result<(), HttpError> {
    match address.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => Ok(()),
        IpAddr::V6(ip) if ip.is_loopback() => Ok(()),
        _ => Err(HttpError::new(
            "non_loopback_bind",
            "management API must bind to numeric loopback only",
        )),
    }
}

pub(super) fn wake_listener(address: SocketAddr) {
    let _ = TcpStream::connect_timeout(&address, Duration::from_millis(50));
}

pub(super) fn io_http_error(error: io::Error) -> HttpError {
    HttpError::new("io_error", error.to_string())
}

pub(super) fn reason_phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        422 => "Unprocessable Entity",
        503 => "Service Unavailable",
        _ => "Error",
    }
}

impl HttpError {
    pub(super) fn from_management(error: ManagementError) -> Self {
        let status = match error.code.as_str() {
            "draft_not_found"
            | "definition_not_found"
            | "context_binding_not_recorded"
            | "context_control_receipt_not_recorded" => 404,
            _ => management_status(&error.class),
        };
        Self {
            code: error.code,
            message: error.message,
            status,
        }
    }

    fn error_response(&self) -> ErrorResponse {
        ErrorResponse {
            schema_version: super::super::contract::MANAGEMENT_SCHEMA_VERSION.to_owned(),
            error: super::super::contract::ErrorBody {
                class: match self.status {
                    401 => ErrorClass::Authentication,
                    403 => ErrorClass::Forbidden,
                    409 => ErrorClass::Conflict,
                    422 => ErrorClass::Replay,
                    503 => ErrorClass::Unavailable,
                    _ => ErrorClass::InvalidInput,
                },
                code: self.code.clone(),
                message: self.message.clone(),
            },
        }
    }

    pub(super) fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
}

impl From<AuthError> for HttpError {
    fn from(error: AuthError) -> Self {
        match error {
            AuthError::Missing | AuthError::InvalidCredentials => {
                Self::new("authentication_required", "authentication failed").with_status(401)
            }
            AuthError::Invalid(_) | AuthError::Configuration(_) => Self::new(
                "authentication_configuration",
                "authentication configuration is invalid",
            )
            .with_status(503),
        }
    }
}

pub(super) fn auth_http_error(error: AuthError) -> HttpError {
    error.into()
}

#[cfg(test)]
#[path = "http_response_tests.rs"]
mod tests;

fn management_status(class: &ErrorClass) -> u16 {
    match class {
        ErrorClass::InvalidInput => 400,
        ErrorClass::Authentication => 401,
        ErrorClass::Forbidden => 403,
        ErrorClass::Capability
        | ErrorClass::Conflict
        | ErrorClass::Unresolved
        | ErrorClass::Budget => 409,
        ErrorClass::Replay => 422,
        ErrorClass::Unavailable | ErrorClass::Store => 503,
    }
}
