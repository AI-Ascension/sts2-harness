// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

use super::config::RuntimeConfig;

const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

pub(crate) struct GatewayClient {
    address: String,
    token: String,
}

impl GatewayClient {
    pub(crate) fn new(config: &RuntimeConfig) -> Result<Self, String> {
        if config.gateway_address.is_empty() || config.gateway_token.is_empty() {
            return Err(String::from("gateway address and token are required"));
        }
        Ok(Self {
            address: config.gateway_address.clone(),
            token: config.gateway_token.clone(),
        })
    }

    pub(crate) fn request(
        &self,
        method: &str,
        path: &str,
        body: &Value,
        extra_headers: BTreeMap<String, String>,
    ) -> Result<Value, String> {
        let bytes = if body.is_null() {
            Vec::new()
        } else {
            serde_json::to_vec(body)
                .map_err(|error| format!("request serialization failed: {error}"))?
        };
        if bytes.len() > MAX_BODY_BYTES {
            return Err(String::from("gateway request body exceeds the bound"));
        }
        let address = self
            .address
            .to_socket_addrs()
            .map_err(|error| format!("gateway address failed: {error}"))?
            .next()
            .ok_or_else(|| String::from("gateway address has no socket target"))?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2))
            .map_err(|error| format!("gateway connection failed: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| format!("gateway read timeout setup failed: {error}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| format!("gateway write timeout setup failed: {error}"))?;
        let mut headers = extra_headers;
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", self.token),
        );
        headers.insert(String::from("Host"), self.address.clone());
        headers.insert(String::from("Content-Length"), bytes.len().to_string());
        if !bytes.is_empty() {
            headers.insert(
                String::from("Content-Type"),
                String::from("application/json"),
            );
        }
        let mut request = format!("{method} {path} HTTP/1.1\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("Connection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .and_then(|_| stream.write_all(&bytes))
            .map_err(|error| format!("gateway request failed: {error}"))?;
        let response = read_response(&mut stream)?;
        let value: Value = serde_json::from_slice(&response.body)
            .map_err(|error| format!("gateway response was not JSON: {error}"))?;
        if !(200..300).contains(&response.status) {
            return Err(format!("gateway returned {}: {value}", response.status));
        }
        Ok(value)
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn read_response(stream: &mut TcpStream) -> Result<HttpResponse, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if bytes.len() >= 8 * 1024 {
            return Err(String::from("gateway response headers exceed the bound"));
        }
        let read = stream
            .read(&mut buffer)
            .map_err(|error| format!("gateway response read failed: {error}"))?;
        if read == 0 {
            return Err(String::from("gateway closed before response headers"));
        }
        bytes.extend_from_slice(&buffer[..read]);
    };
    let header = std::str::from_utf8(&bytes[..header_end])
        .map_err(|error| format!("gateway response headers were not UTF-8: {error}"))?;
    let mut lines = header.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| String::from("gateway response omitted status"))?;
    let mut parts = status_line.split_ascii_whitespace();
    if parts.next() != Some("HTTP/1.1") {
        return Err(String::from(
            "gateway response used an unsupported HTTP version",
        ));
    }
    let status = parts
        .next()
        .ok_or_else(|| String::from("gateway response omitted status code"))?
        .parse::<u16>()
        .map_err(|error| format!("gateway status was invalid: {error}"))?;
    let content_length = lines
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .ok_or_else(|| String::from("gateway response omitted content length"))?;
    if content_length > MAX_RESPONSE_BYTES {
        return Err(String::from("gateway response body exceeds the bound"));
    }
    let body_start = header_end + 4;
    let available = bytes.len().saturating_sub(body_start);
    if available > content_length {
        return Err(String::from("gateway response contained trailing bytes"));
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < content_length {
        let remaining = content_length - body.len();
        let read_capacity = remaining.min(buffer.len());
        let read = stream
            .read(&mut buffer[..read_capacity])
            .map_err(|error| format!("gateway response body read failed: {error}"))?;
        if read == 0 {
            return Err(String::from("gateway closed before response body"));
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpResponse { status, body })
}
