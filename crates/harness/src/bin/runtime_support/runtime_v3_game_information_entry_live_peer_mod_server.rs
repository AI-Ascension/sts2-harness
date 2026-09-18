// SPDX-License-Identifier: MIT

use self::responses::downstream_response;
use super::{LoggedRequest, PeerNegative};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub(super) fn spawn_mod_server(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<LoggedRequest>>>,
    negative: PeerNegative,
) -> JoinHandle<Result<(), String>> {
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let (method, path, headers, body) = read_request(&mut stream)?;
                    requests
                        .lock()
                        .map_err(|_| String::from("synthetic mod request log unavailable"))?
                        .push(LoggedRequest {
                            method: method.clone(),
                            path: path.clone(),
                            correlation: headers.get("x-sts2-correlation-id").cloned(),
                            body: body.clone(),
                        });
                    match downstream_response(&method, &path, &headers, &body, negative) {
                        Ok((status, response)) => write_response(&mut stream, status, &response)?,
                        Err(error) => {
                            // Keep the peer alive after a malformed or unexpected request so
                            // the harness can finish cleanup and report the complete request
                            // trace. The 502 remains an explicit downstream refusal.
                            write_response(
                                &mut stream,
                                502,
                                &serde_json::json!({"error":"synthetic_mod_refused","reason":error}),
                            )?;
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return Err(String::from("synthetic mod accept failed")),
            }
        }
        Ok(())
    })
}

fn read_request(
    stream: &mut TcpStream,
) -> Result<(String, String, BTreeMap<String, String>, Value), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| String::from("synthetic mod read timeout unavailable"))?;
    let mut header_bytes = Vec::new();
    let mut byte = [0_u8; 1];
    while !header_bytes.ends_with(b"\r\n\r\n") {
        stream
            .read_exact(&mut byte)
            .map_err(|_| String::from("synthetic mod request headers incomplete"))?;
        header_bytes.push(byte[0]);
        if header_bytes.len() > 16 * 1024 {
            return Err(String::from("synthetic mod request headers exceeded bound"));
        }
    }
    let text = String::from_utf8(header_bytes).map_err(|_| "synthetic mod headers invalid")?;
    let mut lines = text.lines();
    let mut request = lines
        .next()
        .ok_or("synthetic mod request line missing")?
        .split_whitespace();
    let method = request
        .next()
        .ok_or("synthetic mod method missing")?
        .to_owned();
    let path = request
        .next()
        .ok_or("synthetic mod path missing")?
        .to_owned();
    let mut headers = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers
        .get("content-length")
        .ok_or("synthetic mod Content-Length missing")?
        .parse::<usize>()
        .map_err(|_| "synthetic mod Content-Length invalid")?;
    if length > 256 * 1024 {
        return Err(String::from("synthetic mod request body exceeded bound"));
    }
    let mut body = vec![0; length];
    stream
        .read_exact(&mut body)
        .map_err(|_| String::from("synthetic mod request body incomplete"))?;
    let body = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).map_err(|_| "synthetic mod JSON invalid")?
    };
    Ok((method, path, headers, body))
}

fn write_response(stream: &mut TcpStream, status: u16, body: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(body)
        .map_err(|_| String::from("synthetic mod response encode failed"))?;
    let reason = match status {
        200 => "OK",
        503 => "Service Unavailable",
        _ => "Bad Gateway",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        bytes.len()
    )
    .and_then(|()| stream.write_all(&bytes))
    .map_err(|_| String::from("synthetic mod response write failed"))
}

#[path = "runtime_v3_game_information_entry_live_peer_responses.rs"]
mod responses;
