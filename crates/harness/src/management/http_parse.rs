// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::net::TcpStream;
use std::time::Instant;

use super::response::{find_header_end, read_with_deadline};
use super::*;

pub(super) fn read_request(
    stream: &mut TcpStream,
    deadline: Instant,
    limits: &HttpLimits,
) -> Result<HttpRequest, HttpError> {
    let mut bytes = Vec::with_capacity(READ_BUFFER_BYTES);
    let header_end = loop {
        if let Some(end) = find_header_end(&bytes) {
            break end;
        }
        if bytes.len() >= limits.max_header_bytes {
            return Err(HttpError::new(
                "headers_too_large",
                "request headers exceed the bound",
            ));
        }
        let mut chunk = [0_u8; READ_BUFFER_BYTES];
        let read = read_with_deadline(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(HttpError::new(
                "incomplete_request",
                "request ended before headers completed",
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    if header_end > limits.max_header_bytes {
        return Err(HttpError::new(
            "headers_too_large",
            "request headers exceed the bound",
        ));
    }
    let header_text = std::str::from_utf8(&bytes[..header_end])
        .map_err(|_| HttpError::new("invalid_headers", "request headers are not UTF-8"))?;
    let (method, target, headers) = parse_headers(header_text)?;
    if target.len() > limits.max_path_bytes {
        return Err(HttpError::new(
            "path_too_large",
            "request path exceeds the bound",
        ));
    }
    let (path, query) = parse_target(target)?;
    let content_length = parse_content_length(&headers)?;
    if headers.contains_key("transfer-encoding") {
        return Err(HttpError::new(
            "transfer_encoding_forbidden",
            "Transfer-Encoding is not accepted by the bounded adapter",
        ));
    }
    if content_length > limits.max_body_bytes {
        return Err(HttpError::new(
            "body_too_large",
            "request body exceeds the bound",
        ));
    }
    if method == "GET" && content_length != 0 {
        return Err(HttpError::new(
            "body_not_allowed",
            "GET requests cannot carry a body",
        ));
    }
    if method == "POST"
        && headers.get("content-type").map(String::as_str) != Some("application/json")
    {
        return Err(HttpError::new(
            "content_type_required",
            "POST requests require Content-Type: application/json",
        ));
    }
    let mut body = bytes[header_end + 4..].to_vec();
    if body.len() > content_length {
        body.truncate(content_length);
    }
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let mut chunk = vec![0_u8; remaining.min(READ_BUFFER_BYTES)];
        let read = read_with_deadline(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(HttpError::new(
                "incomplete_body",
                "request body ended early",
            ));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    if headers.contains_key("origin") {
        return Err(HttpError::new(
            "origin_forbidden",
            "browser-origin requests are not accepted",
        )
        .with_status(403));
    }
    Ok(HttpRequest {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn parse_headers(header_text: &str) -> Result<(String, &str, BTreeMap<String, String>), HttpError> {
    let mut lines = header_text.split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| HttpError::new("invalid_request_line", "request line is missing"))?;
    let parts = request_line.split_ascii_whitespace().collect::<Vec<_>>();
    if parts.len() != 3 || parts[2] != "HTTP/1.1" {
        return Err(HttpError::new(
            "invalid_request_line",
            "only HTTP/1.1 requests are accepted",
        ));
    }
    let method = parts[0].to_owned();
    if method != "GET" && method != "POST" {
        return Err(HttpError::new(
            "method_not_allowed",
            "HTTP method is not supported",
        ));
    }
    let target = parts[1];
    let mut headers = BTreeMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| HttpError::new("invalid_header", "header is missing a colon"))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_owned();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            || value.len() > MAX_HEADER_BYTES
        {
            return Err(HttpError::new(
                "invalid_header",
                "header name or value is invalid",
            ));
        }
        if headers.insert(name, value).is_some() {
            return Err(HttpError::new(
                "duplicate_header",
                "duplicate headers are not accepted",
            ));
        }
    }
    Ok((method, target, headers))
}

fn parse_target(target: &str) -> Result<(String, BTreeMap<String, String>), HttpError> {
    if !target.starts_with('/')
        || target.starts_with("//")
        || target.contains("\\")
        || target.contains("..")
        || target.contains('@')
        || target.contains('#')
        || target.contains('%')
    {
        return Err(HttpError::new(
            "invalid_path",
            "request target is not an admitted local path",
        ));
    }
    let (path, query_text) = target.split_once('?').unwrap_or((target, ""));
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(HttpError::new(
            "invalid_path",
            "request path is outside the bound",
        ));
    }
    let mut query = BTreeMap::new();
    if !query_text.is_empty() {
        for item in query_text.split('&') {
            let (key, value) = item
                .split_once('=')
                .ok_or_else(|| HttpError::new("invalid_query", "query parameter is missing '='"))?;
            if key.is_empty()
                || value.is_empty()
                || query.insert(key.to_owned(), value.to_owned()).is_some()
            {
                return Err(HttpError::new(
                    "invalid_query",
                    "query parameters are duplicated or empty",
                ));
            }
        }
    }
    Ok((path.to_owned(), query))
}

fn parse_content_length(headers: &BTreeMap<String, String>) -> Result<usize, HttpError> {
    headers
        .get("content-length")
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(HttpError::new(
                    "invalid_content_length",
                    "Content-Length is invalid",
                ));
            }
            value.parse::<usize>().map_err(|_| {
                HttpError::new(
                    "invalid_content_length",
                    "Content-Length is outside the bound",
                )
            })
        })
        .transpose()
        .map(|length| length.unwrap_or(0))
}
