// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;

const MAX_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 256 * 1024;

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
    let max_response_bytes = MAX_RESPONSE_HEADER_BYTES
        .checked_add(MAX_RESPONSE_BODY_BYTES)
        .and_then(|size| size.checked_add(4))
        .ok_or_else(|| String::from("HTTP response size limit overflowed"))?;
    let read_limit = max_response_bytes
        .checked_add(1)
        .ok_or_else(|| String::from("HTTP response read limit overflowed"))?;
    let mut bytes = Vec::new();
    stream
        .take(u64::try_from(read_limit).map_err(|_| "HTTP response limit is invalid")?)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {path}: {error}"))?;
    if bytes.len() > max_response_bytes {
        return Err(format!("HTTP response exceeded {max_response_bytes} bytes"));
    }
    parse_response(&bytes)
}

fn parse_response(bytes: &[u8]) -> Result<(u16, Value), String> {
    let marker = b"\r\n\r\n";
    let split = bytes
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| String::from("HTTP response omitted header terminator"))?;
    if split > MAX_RESPONSE_HEADER_BYTES {
        return Err(String::from("HTTP response headers oversized"));
    }
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
    if body.len() > MAX_RESPONSE_BODY_BYTES {
        return Err(String::from("HTTP response body oversized"));
    }
    let value = serde_json::from_slice(body)
        .map_err(|error| format!("HTTP response body was not JSON: {error}"))?;
    Ok((status, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_response_body_is_rejected() {
        let mut response = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
        response.extend(std::iter::repeat_n(b'x', MAX_RESPONSE_BODY_BYTES + 1));
        assert_eq!(
            parse_response(&response).map_err(|error| error.to_string()),
            Err(String::from("HTTP response body oversized"))
        );
    }
}
