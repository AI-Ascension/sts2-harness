// SPDX-License-Identifier: MIT

//! The HTTP hop the synthetic downstream speaks, kept beside the fixture so the
//! response table stays readable. Requests are read bounded and answered with
//! the fixture's own envelope, which is the only shape the gateway admits.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::Value;

use super::DownstreamRequest;

pub(super) fn read_request(
    stream: &mut TcpStream,
) -> Result<DownstreamRequest, Box<dyn std::error::Error>> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut bytes = Vec::new();
    let header_end = loop {
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break end;
        }
        if bytes.len() > 64 * 1024 {
            return Err("downstream request header bound exceeded".into());
        }
        let mut chunk = [0_u8; 2048];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("downstream request ended before headers".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let mut lines = std::str::from_utf8(&bytes[..header_end])?.split("\r\n");
    let request_line = lines.next().ok_or("downstream request line missing")?;
    let mut parts = request_line.split_ascii_whitespace();
    let method = parts.next().ok_or("downstream method missing")?.to_owned();
    let path = parts.next().ok_or("downstream path missing")?.to_owned();
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err("downstream request line invalid".into());
    }
    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or("downstream header invalid")?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .ok_or("downstream content length missing")?
        .parse::<usize>()?;
    if length > 128 * 1024 {
        return Err("downstream request body bound exceeded".into());
    }
    let body_start = header_end + 4;
    if bytes.len().saturating_sub(body_start) > length {
        return Err("downstream request has trailing bytes".into());
    }
    let mut body = bytes[body_start..].to_vec();
    while body.len() < length {
        let mut chunk = vec![0_u8; (length - body.len()).min(2048)];
        let count = stream.read(&mut chunk)?;
        if count == 0 {
            return Err("downstream request ended before body".into());
        }
        body.extend_from_slice(&chunk[..count]);
    }
    Ok(DownstreamRequest {
        method,
        path,
        headers,
        body: if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body)?
        },
        raw: body,
    })
}

pub(super) fn write_response(
    stream: &mut TcpStream,
    status: u16,
    body: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    let headers = format!(
        "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(&body)
}
