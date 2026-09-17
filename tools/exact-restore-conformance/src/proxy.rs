// SPDX-License-Identifier: MIT

use serde_json::Value;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const MAX_HEADER_BYTES: usize = 16 * 1024;
const MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_RESPONSE_BYTES: usize = MAX_HEADER_BYTES + MAX_BODY_BYTES + 4;

pub(crate) struct AllocationProxy {
    address: String,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl AllocationProxy {
    pub(crate) fn start(backend: &str, allocation: Value) -> Result<Self, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("bind allocation proxy: {error}"))?;
        listener
            .set_nonblocking(true)
            .map_err(|error| format!("configure allocation proxy: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("allocation proxy address: {error}"))?
            .to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&stop);
        let backend = backend.to_owned();
        let thread = thread::Builder::new()
            .name(String::from("exact-restore-allocation-proxy"))
            .spawn(move || {
                while !signal.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let _ = handle(stream, &backend, &allocation);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|error| format!("spawn allocation proxy: {error}"))?;
        Ok(Self {
            address,
            stop,
            thread: Some(thread),
        })
    }

    pub(crate) fn address(&self) -> &str {
        &self.address
    }
}

impl Drop for AllocationProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect(&self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle(mut client: TcpStream, backend: &str, allocation: &Value) -> Result<(), String> {
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|error| error.to_string())?;
    let request = read_request(&mut client)?;
    let first_line = request
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or_else(|| String::from("proxy request omitted request line"))?;
    let path = std::str::from_utf8(first_line)
        .map_err(|_| String::from("proxy request line was not UTF-8"))?
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| String::from("proxy request omitted path"))?;
    if path == "/v1/sessions/allocate" {
        let body = serde_json::to_vec(allocation).map_err(|error| error.to_string())?;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        client
            .write_all(response.as_bytes())
            .and_then(|_| client.write_all(&body))
            .map_err(|error| format!("write allocation response: {error}"))?;
        return Ok(());
    }
    let (host, port) = backend
        .rsplit_once(':')
        .ok_or_else(|| format!("invalid backend {backend}"))?;
    let mut upstream = TcpStream::connect((host, port.parse::<u16>().map_err(|_| "invalid port")?))
        .map_err(|error| format!("connect backend: {error}"))?;
    upstream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|error| error.to_string())?;
    let request = normalize_request(&request, backend)?;
    upstream
        .write_all(&request)
        .map_err(|error| format!("forward request: {error}"))?;
    let mut response = Vec::new();
    let read_limit = MAX_RESPONSE_BYTES
        .checked_add(1)
        .ok_or_else(|| String::from("proxy response read limit overflowed"))?;
    upstream
        .take(u64::try_from(read_limit).map_err(|_| "proxy response limit is invalid")?)
        .read_to_end(&mut response)
        .map_err(|error| format!("read backend response: {error}"))?;
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(String::from("backend response oversized"));
    }
    client
        .write_all(&response)
        .map_err(|error| format!("forward response: {error}"))?;
    Ok(())
}

fn normalize_request(request: &[u8], backend: &str) -> Result<Vec<u8>, String> {
    let marker = b"\r\n\r\n";
    let split = request
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| String::from("proxy request omitted header terminator"))?;
    let headers = std::str::from_utf8(&request[..split])
        .map_err(|_| String::from("proxy request headers were not UTF-8"))?;
    let body = &request[split + marker.len()..];
    let declared_length = declared_content_length(headers, body.len())?;
    if declared_length > MAX_BODY_BYTES {
        return Err(String::from("proxy request body oversized"));
    }
    let body = body
        .get(..declared_length)
        .ok_or_else(|| String::from("proxy request body was truncated"))?;
    let request_line = headers
        .split("\r\n")
        .next()
        .ok_or_else(|| String::from("proxy request omitted request line"))?;
    let mut output = Vec::with_capacity(request.len());
    output.extend_from_slice(request_line.as_bytes());
    output.extend_from_slice(b"\r\nHost: ");
    output.extend_from_slice(backend.as_bytes());
    output.extend_from_slice(b"\r\n");
    for line in headers.split("\r\n").skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let allowed = matches!(
            name.to_ascii_lowercase().as_str(),
            "authorization"
                | "content-type"
                | "x-mcp-session-id"
                | "x-sts2-instance-id"
                | "x-sts2-caller-id"
                | "x-sts2-session-id"
                | "x-sts2-lease-id"
                | "x-sts2-lease-epoch"
                | "x-sts2-correlation-id"
                | "x-sts2-recovery-capability"
        );
        if allowed {
            output.extend_from_slice(name.to_ascii_lowercase().as_bytes());
            output.extend_from_slice(b":");
            output.extend_from_slice(value.as_bytes());
            output.extend_from_slice(b"\r\n");
        }
    }
    output.extend_from_slice(b"Content-Length: ");
    output.extend_from_slice(body.len().to_string().as_bytes());
    output.extend_from_slice(b"\r\nConnection: close\r\n");
    output.extend_from_slice(b"\r\n");
    output.extend_from_slice(body);
    Ok(output)
}

fn read_request(stream: &mut TcpStream) -> Result<Vec<u8>, String> {
    let marker = b"\r\n\r\n";
    let mut request = Vec::new();
    let header_end = loop {
        let mut chunk = [0_u8; 2048];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| format!("read proxy request: {error}"))?;
        if count == 0 {
            return Err(String::from("proxy request ended before headers"));
        }
        request.extend_from_slice(&chunk[..count]);
        if let Some(index) = request
            .windows(marker.len())
            .position(|window| window == marker)
        {
            if index > MAX_HEADER_BYTES {
                return Err(String::from("proxy request headers oversized"));
            }
            break index + marker.len();
        }
        if request.len() > MAX_HEADER_BYTES {
            return Err(String::from("proxy request headers oversized"));
        }
    };
    let header_text = std::str::from_utf8(&request[..header_end])
        .map_err(|_| String::from("proxy request headers were not UTF-8"))?;
    let content_length = declared_content_length(header_text, 0)?;
    if content_length > MAX_BODY_BYTES {
        return Err(String::from("proxy request body oversized"));
    }
    let request_end = header_end
        .checked_add(content_length)
        .ok_or_else(|| String::from("proxy request size overflowed"))?;
    while request.len() < request_end {
        let mut chunk = [0_u8; 2048];
        let count = stream
            .read(&mut chunk)
            .map_err(|error| format!("read proxy body: {error}"))?;
        if count == 0 {
            return Err(String::from("proxy request ended before body"));
        }
        request.extend_from_slice(&chunk[..count]);
    }
    request.truncate(request_end);
    Ok(request)
}

fn declared_content_length(headers: &str, default: usize) -> Result<usize, String> {
    let mut declared = None;
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.eq_ignore_ascii_case("content-length") {
            continue;
        }
        let length = value
            .trim()
            .parse::<usize>()
            .map_err(|_| String::from("proxy content-length was not numeric"))?;
        if declared.replace(length).is_some() {
            return Err(String::from("proxy request repeated content-length"));
        }
    }
    Ok(declared.unwrap_or(default))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_declared_body_is_rejected() {
        let request = format!(
            "POST /v1/test HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        assert_eq!(
            normalize_request(request.as_bytes(), "127.0.0.1:1234")
                .map_err(|error| error.to_string()),
            Err(String::from("proxy request body oversized"))
        );
    }

    #[test]
    fn malformed_content_length_is_rejected() {
        let request = b"POST /v1/test HTTP/1.1\r\nContent-Length: nope\r\n\r\n";
        assert_eq!(
            normalize_request(request, "127.0.0.1:1234").map_err(|error| error.to_string()),
            Err(String::from("proxy content-length was not numeric"))
        );
    }
}
