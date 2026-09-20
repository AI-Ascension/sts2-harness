// SPDX-License-Identifier: MIT

//! Shared readers for the pinned `watchdog-host-lease-control-v1` targets.
//!
//! Both the vector target and the served-terminal target read the same pin and
//! post the same frames, so the artifact readers, the loopback client, and the
//! exchange-identity normalizer live here instead of being copied.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// The pinned artifact directory and the largest frame it admits.
pub(crate) const ARTIFACT: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-artifact/host-lease-control-v1"
);
pub(crate) const MAX_FRAME_BYTES: usize = 262_144;
/// The published profile classifies its own vectors; do not claim more.
pub(crate) const CLASSIFICATION: &str = "test-vectors-not-live-evidence";

pub(crate) fn artifact_bytes(relative: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    Ok(std::fs::read(format!("{ARTIFACT}/{relative}"))?)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte >> 4)]));
        hex.push(char::from(b"0123456789abcdef"[usize::from(byte & 0x0f)]));
    }
    hex
}

pub(crate) fn vectors() -> Result<Value, Box<dyn std::error::Error>> {
    let value: Value = serde_json::from_slice(&artifact_bytes("proof-vectors.json")?)?;
    if value["profile"] != json!("host-lease-control-proof-v1") {
        return Err(String::from("the pinned proof profile is not v1").into());
    }
    if value["classification"] != json!(CLASSIFICATION) {
        return Err(String::from("the pinned vectors claim live evidence").into());
    }
    Ok(value)
}

pub(crate) fn entries<'a>(
    values: &'a Value,
    name: &str,
) -> Result<Vec<&'a Value>, Box<dyn std::error::Error>> {
    values[name]
        .as_array()
        .map(|entries| entries.iter().collect())
        .ok_or_else(|| format!("the pinned vectors carry no {name}").into())
}

#[derive(Debug)]
pub(crate) struct HttpAnswer {
    pub(crate) status: u16,
    pub(crate) body: Value,
}

pub(crate) fn post(
    address: std::net::SocketAddr,
    path: &str,
    body: &[u8],
) -> Result<HttpAnswer, Box<dyn std::error::Error>> {
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer mod-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes())?;
    stream.write_all(body)?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let text = std::str::from_utf8(&response)?;
    let (head, payload) = text
        .split_once("\r\n\r\n")
        .ok_or("the downstream answered no header block")?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_ascii_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or("the downstream answered no status")?;
    Ok(HttpAnswer {
        status,
        body: serde_json::from_str(payload)?,
    })
}

/// Only the identity of one exchange is fresh: the message id, the envelope
/// time, the proof, and the principal the host signs as. Every other field is
/// copied out of the request, so the pinned acknowledgment fixes it.
pub(crate) fn without_exchange_identity(frame: &Value) -> Value {
    let mut frame = frame.clone();
    for pointer in [
        "/message_id",
        "/sent_at",
        "/auth/proof",
        "/auth/principal_id",
        "/actor/principal_id",
    ] {
        if let Some(slot) = frame.pointer_mut(pointer) {
            *slot = Value::Null;
        }
    }
    frame
}
