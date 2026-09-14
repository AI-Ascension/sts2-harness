// SPDX-License-Identifier: MIT
//! Bounded synthetic model endpoint fixture for the STS2-Exo contract spike (#139).
//!
//! Original fixture: no upstream or proprietary content. It implements the minimum
//! surface the pinned Exo executor calls for one turn and records every request so
//! the spike can prove how many model calls occurred and what was sent.
//!
//! Routes:
//!   POST /chat/completions  -> OpenAI Chat Completions shape (TypeScript Exo harness)
//!   POST /responses         -> OpenAI Responses shape (Rust Basic executor)
//!   GET  /health            -> liveness text
//!
//! This file is intentionally not a Cargo target; the spike driver compiles it with
//! `rustc` so the harness workspace keeps no provider-implementation dependency.
//!
//! Usage: synthetic_model <PORT> <REQUEST_LOG> [ASSISTANT_TEXT]
use std::env;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let port: u16 = args.get(1).and_then(|value| value.parse().ok()).unwrap_or(0);
    let log_path = args
        .get(2)
        .cloned()
        .unwrap_or_else(|| "requests.jsonl".to_string());
    let text = args
        .get(3)
        .cloned()
        .unwrap_or_else(|| "synthetic decision".to_string());

    let listener = TcpListener::bind(("127.0.0.1", port))?;
    let actual = listener.local_addr()?.port();
    println!("{actual}");
    std::io::stdout().flush()?;

    let mut index: u64 = 0;
    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            index += 1;
            let _ = handle(&mut stream, index, &log_path, &text);
        }
    }
    Ok(())
}

fn handle(
    stream: &mut TcpStream,
    index: u64,
    log_path: &str,
    text: &str,
) -> std::io::Result<()> {
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    let mut header_end = None;
    while header_end.is_none() && buffer.len() <= 1_048_576 {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        header_end = find_subslice(&buffer, b"\r\n\r\n").map(|position| position + 4);
    }
    let header_end = header_end.unwrap_or(buffer.len());
    let headers = String::from_utf8_lossy(&buffer[..header_end]).into_owned();
    let content_length = content_length(&headers);

    while buffer.len() < header_end + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
    }
    let body_start = header_end.min(buffer.len());
    let body_end = (header_end + content_length).min(buffer.len());
    let body = String::from_utf8_lossy(&buffer[body_start..body_end]).into_owned();
    record(log_path, index, &body)?;

    let request_line = headers.lines().next().unwrap_or_default();
    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let payload = if path.ends_with("/chat/completions") {
        chat_body(text)
    } else if request_line.starts_with("GET ") {
        "ok\n".to_string()
    } else {
        responses_body(text)
    };
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        payload.len(),
        payload
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn record(log_path: &str, index: u64, body: &str) -> std::io::Result<()> {
    let entry = format!("{{\"index\":{index},\"body\":{}}}\n", json_string(body));
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)?;
    file.write_all(entry.as_bytes())
}

fn content_length(headers: &str) -> usize {
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            return value.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

fn usage_json() -> &'static str {
    "\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":5,\"total_tokens\":16}"
}

fn responses_body(text: &str) -> String {
    format!(
        "{{\"id\":\"resp_synthetic_0001\",\"object\":\"response\",\"status\":\"completed\",\"created_at\":1700000000,\"model\":\"synthetic-model\",\"output\":[{{\"type\":\"message\",\"id\":\"msg_synthetic_0001\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{{\"type\":\"output_text\",\"text\":{},\"annotations\":[]}}]}}],{}}}",
        json_string(text),
        usage_json()
    )
}

fn chat_body(text: &str) -> String {
    format!(
        "{{\"id\":\"chatcmpl_synthetic_0001\",\"object\":\"chat.completion\",\"created\":1700000000,\"model\":\"synthetic-model\",\"choices\":[{{\"index\":0,\"message\":{{\"role\":\"assistant\",\"content\":{}}},\"finish_reason\":\"stop\"}}],{}}}",
        json_string(text),
        usage_json()
    )
}
