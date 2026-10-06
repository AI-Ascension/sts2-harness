// SPDX-License-Identifier: MIT

use std::net::TcpStream;
use std::time::{Duration, Instant};

use super::response::{
    find_header_end, io_http_error, read_with_deadline, validate_loopback, write_with_deadline,
};
use super::*;

pub struct ManagementClient {
    address: SocketAddr,
    bearer_token: String,
    deadline: Duration,
}

impl ManagementClient {
    pub fn new(address: SocketAddr, bearer_token: impl Into<String>) -> Result<Self, HttpError> {
        validate_loopback(address)?;
        let bearer_token = bearer_token.into();
        if bearer_token.is_empty() || bearer_token.bytes().any(|byte| byte.is_ascii_whitespace()) {
            return Err(HttpError::new(
                "invalid_token",
                "client credential is invalid",
            ));
        }
        Ok(Self {
            address,
            bearer_token,
            deadline: Duration::from_millis(REQUEST_DEADLINE_MILLIS),
        })
    }

    pub fn request_json(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<ClientResponse, HttpError> {
        self.request_json_inner(method, path, body, None)
    }

    /// Sends a policy-owner mutation with the bounded idempotency header used
    /// by durable owner commands.
    pub fn request_json_with_idempotency_key(
        &self,
        method: &str,
        path: &str,
        body: &[u8],
        idempotency_key: &str,
    ) -> Result<ClientResponse, HttpError> {
        if idempotency_key.is_empty()
            || idempotency_key.len() > 128
            || !idempotency_key.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric() || (index > 0 && b"._:-".contains(&byte))
            })
        {
            return Err(HttpError::new(
                "invalid_idempotency_key",
                "idempotency key is outside the supported bound",
            ));
        }
        self.request_json_inner(method, path, Some(body), Some(idempotency_key))
    }

    fn request_json_inner(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
        idempotency_key: Option<&str>,
    ) -> Result<ClientResponse, HttpError> {
        if method != "GET" && method != "POST" && method != "PUT" && method != "PATCH" {
            return Err(HttpError::new(
                "method_not_allowed",
                "HTTP method is not supported",
            ));
        }
        if path.len() > MAX_PATH_BYTES
            || !path.starts_with('/')
            || path.contains('@')
            || path.contains("..")
        {
            return Err(HttpError::new(
                "invalid_path",
                "client path is outside the bound",
            ));
        }
        let body = body.unwrap_or(&[]);
        if body.len() > MAX_JSON_BYTES {
            return Err(HttpError::new(
                "body_too_large",
                "client body exceeds the bound",
            ));
        }
        if (method == "POST" || method == "PUT" || method == "PATCH") && body.is_empty() {
            return Err(HttpError::new(
                "body_required",
                "POST, PUT, and PATCH client requests require a JSON body",
            ));
        }
        let deadline = Instant::now() + self.deadline;
        let mut stream =
            TcpStream::connect_timeout(&self.address, self.deadline).map_err(io_http_error)?;
        let head = request_head(
            self.address,
            &self.bearer_token,
            method,
            path,
            idempotency_key,
            body.len(),
        );
        let mut request = head.into_bytes();
        request.extend_from_slice(body);
        write_with_deadline(&mut stream, &request, deadline)?;
        read_client_response(&mut stream, deadline)
    }
}

#[derive(Clone, Debug)]
pub struct ClientResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Build the request head this client puts on the wire.
///
/// # The invariant
///
/// Every header this client sends is admitted by **the server it is pointed at** — and
/// today that server is the harness management listener, which runs no allow-list of its
/// own. This is deliberately not "every header here is gateway-admissible": one is not, and
/// pretending otherwise is how #598 happened. A caller pointed at a different server has to
/// re-check this list against that server, not inherit this one's conclusion.
///
/// # Why `accept` is gone
///
/// `accept` is **not** in the gateway's `header_is_allowed` list, and the gateway pins the
/// refusal with a test of its own (`service_auth_tests.rs`), so sending it made every
/// management request over the gateway hop fail `400 unsupported_header` — see issue #560.
/// The client reads a whole JSON body and the gateway answers with a single JSON
/// representation, so there is no content negotiation to perform here: `Accept:
/// application/json` was a statement with no recipient. Add a header only once something on
/// the far side actually admits it.
///
/// # Why `Idempotency-Key` stays
///
/// It is the one header emitted here that the gateway refuses, and it is required by the
/// server that actually receives it: `http_routes_memory_owner.rs` answers
/// `idempotency_key_required` when a policy mutation arrives without it, and 16 operations
/// across `contracts/context-memory/memory-api.openapi.json` (10) and
/// `contracts/context-control/control-api.openapi.json` (6) declare it `"in": "header",
/// "required": true`. Deleting it would convert those mutations into 400s.
///
/// The two header sets never mix because the two hops never mix: keyed callers are
/// policy-owner mutations against the harness management listener on loopback, and every
/// other caller crosses the gateway and passes no key. Note that all nine
/// `request_json_with_idempotency_key` call sites are `#[cfg(test)]`-gated today, so the
/// header currently rides only on test traffic — it is not dead code, because the listener
/// requires it and the first production policy-owner caller will need it.
///
/// If a keyed caller is ever pointed at the gateway, that header is the one that will be
/// refused, and the fix is at the call site, not here. See issues #597 and #598.
fn request_head(
    address: SocketAddr,
    bearer_token: &str,
    method: &str,
    path: &str,
    idempotency_key: Option<&str>,
    body_len: usize,
) -> String {
    format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {bearer_token}\r\nContent-Type: application/json\r\n{}Content-Length: {body_len}\r\nConnection: close\r\n\r\n",
        idempotency_key.map_or_else(String::new, |key| format!("Idempotency-Key: {key}\r\n")),
    )
}

fn read_client_response(
    stream: &mut TcpStream,
    deadline: Instant,
) -> Result<ClientResponse, HttpError> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            break end;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(HttpError::new(
                "headers_too_large",
                "response headers exceed the bound",
            ));
        }
        let mut chunk = [0_u8; READ_BUFFER_BYTES];
        let read = read_with_deadline(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(HttpError::new(
                "incomplete_response",
                "response ended before headers completed",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| HttpError::new("invalid_response", "response headers are not UTF-8"))?;
    let mut lines = header_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| HttpError::new("invalid_response", "response status is missing"))?;
    let parts = status_line.split_ascii_whitespace().collect::<Vec<_>>();
    if parts.len() < 3 || parts[0] != "HTTP/1.1" {
        return Err(HttpError::new(
            "invalid_response",
            "response status line is invalid",
        ));
    }
    let status = parts[1]
        .parse::<u16>()
        .map_err(|_| HttpError::new("invalid_response", "response status is invalid"))?;
    let mut content_length = None;
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| HttpError::new("invalid_response", "response header is malformed"))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "transfer-encoding" {
            return Err(HttpError::new(
                "transfer_encoding_forbidden",
                "response uses Transfer-Encoding",
            ));
        }
        if name == "content-length" {
            if content_length.is_some() {
                return Err(HttpError::new(
                    "duplicate_header",
                    "response has duplicate Content-Length",
                ));
            }
            content_length = Some(value.parse::<usize>().map_err(|_| {
                HttpError::new("invalid_response", "response Content-Length is invalid")
            })?);
        }
    }
    let content_length = content_length
        .ok_or_else(|| HttpError::new("invalid_response", "response Content-Length is missing"))?;
    if content_length > MAX_RESPONSE_BYTES {
        return Err(HttpError::new(
            "response_too_large",
            "response body exceeds the bound",
        ));
    }
    let mut body = bytes[header_end + 4..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let mut chunk = vec![0_u8; (content_length - body.len()).min(READ_BUFFER_BYTES)];
        let read = read_with_deadline(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(HttpError::new(
                "incomplete_response",
                "response body ended early",
            ));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    Ok(ClientResponse { status, body })
}

#[cfg(test)]
#[path = "http_client_tests.rs"]
mod tests;
