// SPDX-License-Identifier: MIT

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::config::RuntimeConfig;

const MAX_BODY_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 8 * 1024;

pub(crate) struct GatewayClient {
    address: SocketAddr,
    token: String,
}

impl GatewayClient {
    pub(crate) fn new(config: &RuntimeConfig) -> Result<Self, String> {
        let address = parse_address(&config.gateway_address)?;
        if config.gateway_token.is_empty()
            || config.gateway_token.len() > 256
            || !config
                .gateway_token
                .bytes()
                .all(|byte| byte.is_ascii_graphic())
        {
            return Err(String::from("gateway token is empty, unsafe, or oversized"));
        }
        Ok(Self {
            address,
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
        self.exchange(method, path, body, extra_headers, Duration::from_secs(5))
    }

    fn exchange(
        &self,
        method: &str,
        path: &str,
        body: &Value,
        extra_headers: BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let deadline = Instant::now() + timeout;
        validate_request(method, path, &extra_headers)?;
        let bytes = if body.is_null() {
            Vec::new()
        } else {
            serde_json::to_vec(body)
                .map_err(|error| format!("request serialization failed: {error}"))?
        };
        if bytes.len() > MAX_BODY_BYTES {
            return Err(String::from("gateway request body exceeds the bound"));
        }
        let mut headers = extra_headers;
        headers.insert(
            String::from("Authorization"),
            format!("Bearer {}", self.token),
        );
        headers.insert(String::from("Host"), self.address.to_string());
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
        if request.len() > MAX_HEADER_BYTES {
            return Err(String::from("gateway request headers exceed the bound"));
        }
        let mut stream = TcpStream::connect_timeout(&self.address, remaining(deadline)?)
            .map_err(|_| String::from("gateway connection failed"))?;
        write_deadline(&mut stream, request.as_bytes(), deadline)?;
        write_deadline(&mut stream, &bytes, deadline)?;
        let response = read_response(&mut stream, deadline)?;
        if !(200..300).contains(&response.status) {
            return Err(format!("gateway returned HTTP {}", response.status));
        }
        serde_json::from_slice(&response.body)
            .map_err(|_| String::from("gateway response was not JSON"))
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn parse_address(value: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = value
        .parse()
        .map_err(|_| String::from("gateway address must be a numeric loopback socket address"))?;
    if !address.ip().is_loopback() || address.port() == 0 {
        return Err(String::from(
            "gateway address must be a numeric loopback socket address",
        ));
    }
    Ok(address)
}

fn header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn header_value(value: &str) -> bool {
    value.len() <= 2048
        && value
            .bytes()
            .all(|byte| byte == b' ' || byte.is_ascii_graphic())
}

fn validate_request(
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
) -> Result<(), String> {
    if !matches!(method, "GET" | "POST" | "DELETE")
        || !path.starts_with('/')
        || path.len() > 2048
        || !path.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return Err(String::from("gateway request target is invalid"));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut header_bytes = 0_usize;
    for (name, value) in headers {
        let normalized = name.to_ascii_lowercase();
        if !header_name(name)
            || !header_value(value)
            || !names.insert(normalized.clone())
            || matches!(
                normalized.as_str(),
                "authorization"
                    | "host"
                    | "content-length"
                    | "content-type"
                    | "connection"
                    | "transfer-encoding"
            )
        {
            return Err(String::from("gateway request header is invalid"));
        }
        header_bytes += name.len() + value.len() + 4;
        if header_bytes > MAX_HEADER_BYTES {
            return Err(String::from("gateway request headers exceed the bound"));
        }
    }
    Ok(())
}

fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| String::from("gateway exchange deadline expired"))
}

fn write_deadline(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
) -> Result<(), String> {
    while !bytes.is_empty() {
        stream
            .set_write_timeout(Some(remaining(deadline)?))
            .map_err(|_| String::from("gateway write timeout setup failed"))?;
        let written = stream
            .write(bytes)
            .map_err(|_| String::from("gateway request failed or timed out"))?;
        if written == 0 {
            return Err(String::from("gateway closed during request"));
        }
        bytes = &bytes[written..];
    }
    remaining(deadline)?;
    Ok(())
}

fn read_deadline(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    deadline: Instant,
) -> Result<usize, String> {
    stream
        .set_read_timeout(Some(remaining(deadline)?))
        .map_err(|_| String::from("gateway read timeout setup failed"))?;
    let read = stream
        .read(bytes)
        .map_err(|_| String::from("gateway response read failed or timed out"))?;
    remaining(deadline)?;
    Ok(read)
}

fn response_length<'a>(lines: impl Iterator<Item = &'a str>) -> Result<usize, String> {
    let mut length = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| String::from("gateway response header was invalid"))?;
        if !header_name(name)
            || !header_value(value)
            || name.eq_ignore_ascii_case("transfer-encoding")
        {
            return Err(String::from(
                "gateway response header was invalid or unsupported",
            ));
        }
        if name.eq_ignore_ascii_case("content-length") {
            let value = value.trim_matches(' ');
            if length.is_some()
                || value.is_empty()
                || !value.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err(String::from(
                    "gateway response content length was invalid or duplicated",
                ));
            }
            length = Some(
                value
                    .parse::<usize>()
                    .map_err(|_| String::from("gateway response content length was invalid"))?,
            );
        }
    }
    length.ok_or_else(|| String::from("gateway response omitted content length"))
}

fn read_response(stream: &mut TcpStream, deadline: Instant) -> Result<HttpResponse, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 2048];
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            if end + 4 > MAX_HEADER_BYTES {
                return Err(String::from("gateway response headers exceed the bound"));
            }
            break end;
        }
        if bytes.len() >= MAX_HEADER_BYTES {
            return Err(String::from("gateway response headers exceed the bound"));
        }
        let read = read_deadline(stream, &mut buffer, deadline)?;
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
    let code = parts
        .next()
        .ok_or_else(|| String::from("gateway response omitted status code"))?;
    if code.len() != 3 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(String::from("gateway status was invalid"));
    }
    let status = code
        .parse::<u16>()
        .map_err(|error| format!("gateway status was invalid: {error}"))?;
    if !(100..600).contains(&status) || !header_value(status_line) {
        return Err(String::from("gateway status was invalid"));
    }
    let content_length = response_length(lines)?;
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
        let read = read_deadline(stream, &mut buffer[..read_capacity], deadline)?;
        if read == 0 {
            return Err(String::from("gateway closed before response body"));
        }
        body.extend_from_slice(&buffer[..read]);
    }
    Ok(HttpResponse { status, body })
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
