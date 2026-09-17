// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;

pub(crate) fn post(
    address: &str,
    path: &str,
    token: &str,
    capability: Option<&str>,
    body: &Value,
) -> Result<(u16, Value), String> {
    let (host, port) = address
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid peer address {address}"))?;
    let mut stream = TcpStream::connect((host, port.parse::<u16>().map_err(|_| "invalid port")?))
        .map_err(|error| format!("connect {address}: {error}"))?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .map_err(|error| format!("set HTTP timeout: {error}"))?;
    let payload = serde_json::to_vec(body).map_err(|error| format!("encode request: {error}"))?;
    let mut request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nAuthorization: Bearer {token}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        payload.len()
    );
    if let Some(capability) = capability {
        request.push_str(&format!("x-sts2-recovery-capability: {capability}\r\n"));
    }
    request.push_str("\r\n");
    stream
        .write_all(request.as_bytes())
        .and_then(|_| stream.write_all(&payload))
        .map_err(|error| format!("write {path}: {error}"))?;
    let mut bytes = Vec::new();
    stream
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {path}: {error}"))?;
    parse_response(&bytes)
}

fn parse_response(bytes: &[u8]) -> Result<(u16, Value), String> {
    let marker = b"\r\n\r\n";
    let split = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| String::from("HTTP response omitted header terminator"))?;
    let headers = std::str::from_utf8(&bytes[..split])
        .map_err(|_| String::from("HTTP response headers were not UTF-8"))?;
    let status = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or_else(|| String::from("HTTP response omitted status"))?
        .parse::<u16>()
        .map_err(|_| String::from("HTTP status was not numeric"))?;
    let body = &bytes[split + marker.len()..];
    let value = serde_json::from_slice(body)
        .map_err(|error| format!("HTTP response body was not JSON: {error}"))?;
    Ok((status, value))
}
