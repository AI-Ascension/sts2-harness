// SPDX-License-Identifier: MIT

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

use super::fixture::Result;

const EXCHANGE_DEADLINE: Duration = Duration::from_secs(5);
const CONNECT_LIMIT: Duration = Duration::from_secs(2);
const IO_POLL: Duration = Duration::from_millis(10);
const HEADER_LIMIT: usize = 8 * 1024;
const REQUEST_BODY_CEILING: usize = 1_048_577;
const RESPONSE_CEILING: usize = 8 * 1024;
const HEADER_COUNT_LIMIT: usize = 64;

pub(crate) struct BoundedResponse {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

enum WriteFailure {
    Deadline,
    Io(ErrorKind),
}

pub(crate) fn exchange(
    address: SocketAddr,
    request_head: &[u8],
    request_body: &[u8],
    request_body_limit: usize,
    response_limit: usize,
    tolerate_rejected_body_close: bool,
) -> Result<BoundedResponse> {
    if !address.ip().is_loopback()
        || request_head.len() > HEADER_LIMIT
        || !request_head.ends_with(b"\r\n\r\n")
        || request_body_limit > REQUEST_BODY_CEILING
        || request_body.len() > request_body_limit
        || response_limit > RESPONSE_CEILING
        || response_limit == 0
    {
        return Err("bounded HTTP exchange inputs refused".into());
    }
    let deadline = Instant::now()
        .checked_add(EXCHANGE_DEADLINE)
        .ok_or("bounded HTTP exchange deadline invalid")?;
    let connect_for = deadline
        .saturating_duration_since(Instant::now())
        .min(CONNECT_LIMIT);
    if connect_for.is_zero() {
        return Err("bounded HTTP exchange deadline elapsed".into());
    }
    let mut stream = TcpStream::connect_timeout(&address, connect_for)
        .map_err(|_| "bounded HTTP exchange connect failed")?;
    stream
        .set_nonblocking(true)
        .map_err(|_| "bounded HTTP exchange socket setup failed")?;
    write_all(&mut stream, request_head, deadline)
        .map_err(|_| "bounded HTTP request headers failed")?;
    match write_all(&mut stream, request_body, deadline) {
        Ok(()) => {}
        Err(WriteFailure::Io(kind))
            if tolerate_rejected_body_close
                && matches!(kind, ErrorKind::BrokenPipe | ErrorKind::ConnectionReset) => {}
        Err(_) => return Err("bounded HTTP request body write failed".into()),
    }
    read_response(&mut stream, deadline, response_limit)
}

fn read_response(
    stream: &mut TcpStream,
    deadline: Instant,
    response_limit: usize,
) -> Result<BoundedResponse> {
    let mut header = Vec::with_capacity(1024);
    let mut byte = [0_u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        read_exact(stream, &mut byte, deadline)?;
        header.push(byte[0]);
        if header.len() > HEADER_LIMIT || header.len() > response_limit {
            return Err("bounded HTTP response headers exceeded their limit".into());
        }
    }
    let text =
        std::str::from_utf8(&header).map_err(|_| "bounded HTTP response headers were not UTF-8")?;
    let mut lines = text.split("\r\n");
    let status_line = lines.next().ok_or("bounded HTTP response status missing")?;
    let mut status_parts = status_line.split_ascii_whitespace();
    if status_parts.next() != Some("HTTP/1.1") {
        return Err("bounded HTTP response protocol refused".into());
    }
    let code = status_parts
        .next()
        .ok_or("bounded HTTP response status missing")?;
    if code.len() != 3 || !code.bytes().all(|part| part.is_ascii_digit()) {
        return Err("bounded HTTP response status invalid".into());
    }
    let status = code
        .parse::<u16>()
        .map_err(|_| "bounded HTTP response status invalid")?;
    let mut names = Vec::new();
    let mut content_length = None;
    for row in lines.filter(|row| !row.is_empty()) {
        if names.len() >= HEADER_COUNT_LIMIT {
            return Err("bounded HTTP response header count exceeded".into());
        }
        let (name, value) = row
            .split_once(':')
            .ok_or("bounded HTTP response header syntax refused")?;
        let name = name.trim().to_ascii_lowercase();
        if name.is_empty()
            || !name
                .bytes()
                .all(|part| part.is_ascii_alphanumeric() || part == b'-')
            || names.iter().any(|seen| seen == &name)
        {
            return Err("bounded HTTP response header name refused".into());
        }
        if name == "transfer-encoding" || name == "expect" || name == "upgrade" {
            return Err("bounded HTTP response framing refused".into());
        }
        if name == "content-length" {
            let value = value.trim();
            if value.is_empty() || !value.bytes().all(|part| part.is_ascii_digit()) {
                return Err("bounded HTTP response length refused".into());
            }
            let parsed = value
                .parse::<usize>()
                .map_err(|_| "bounded HTTP response length refused")?;
            if parsed.to_string() != value {
                return Err("bounded HTTP response length was not canonical".into());
            }
            content_length = Some(parsed);
        }
        names.push(name);
    }
    let length = content_length.ok_or("bounded HTTP response length missing")?;
    if header.len().saturating_add(length) > response_limit {
        return Err("bounded HTTP response exceeded its total byte limit".into());
    }
    let mut body = vec![0_u8; length];
    read_exact(stream, &mut body, deadline)?;
    Ok(BoundedResponse { status, body })
}

fn read_exact(stream: &mut TcpStream, output: &mut [u8], deadline: Instant) -> Result<()> {
    let mut offset = 0;
    while offset < output.len() {
        if Instant::now() >= deadline {
            return Err("bounded HTTP read deadline elapsed".into());
        }
        match stream.read(&mut output[offset..]) {
            Ok(0) => return Err("bounded HTTP response ended early".into()),
            Ok(read) => offset += read,
            Err(error) if error.kind() == ErrorKind::WouldBlock => pause(deadline)?,
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => return Err("bounded HTTP response read failed".into()),
        }
    }
    Ok(())
}

fn write_all(
    stream: &mut TcpStream,
    bytes: &[u8],
    deadline: Instant,
) -> std::result::Result<(), WriteFailure> {
    let mut offset = 0;
    while offset < bytes.len() {
        if Instant::now() >= deadline {
            return Err(WriteFailure::Deadline);
        }
        match stream.write(&bytes[offset..]) {
            Ok(0) => return Err(WriteFailure::Io(ErrorKind::WriteZero)),
            Ok(written) => offset += written,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                pause(deadline).map_err(|_| WriteFailure::Deadline)?;
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(WriteFailure::Io(error.kind())),
        }
    }
    Ok(())
}

fn pause(deadline: Instant) -> Result<()> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("bounded HTTP exchange deadline elapsed".into());
    }
    thread::sleep(IO_POLL.min(remaining));
    Ok(())
}
