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
        if method != "GET" && method != "POST" && method != "PUT" {
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
        if (method == "POST" || method == "PUT") && body.is_empty() {
            return Err(HttpError::new(
                "body_required",
                "POST and PUT client requests require a JSON body",
            ));
        }
        let deadline = Instant::now() + self.deadline;
        let mut stream =
            TcpStream::connect_timeout(&self.address, self.deadline).map_err(io_http_error)?;
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nAccept: application/json\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.address,
            self.bearer_token,
            body.len()
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
