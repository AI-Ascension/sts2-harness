// SPDX-License-Identifier: MIT

//! Deterministic compiled peer for the persistent-provider fixture lane.

use serde_json::json;
use std::io::{self, BufRead, Write};
use sts2_harness::provider_session::{NativeFrame, parse_native_request};

fn main() {
    if let Err(error) = run() {
        eprintln!("provider-session-peer: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout());
    let mut initialized = false;
    let mut sequence = 1_u64;
    for line in stdin.lock().lines() {
        let line = line.map_err(|_| "read failed".to_owned())?;
        let mut framed = line.into_bytes();
        framed.push(b'\n');
        let (id, method, params) = match parse_native_request(&framed) {
            Ok(value) => value,
            Err(_) => {
                write_frame(
                    &mut stdout,
                    &NativeFrame::error(None, -32600, "invalid request"),
                )?;
                continue;
            }
        };
        if std::env::var("PROVIDER_SESSION_PEER_SERVER_REQUEST")
            .ok()
            .as_deref()
            == Some("1")
            && method == "thread/read"
        {
            write_frame(
                &mut stdout,
                &NativeFrame::server_request(900, "shell/execute"),
            )?;
        }
        let response = match method.as_str() {
            "initialize" if !initialized => {
                initialized = true;
                json!({"profile_id":"codex-app-server-fixture-v1","tools":false,"ambient_history":false})
            }
            "initialize" => {
                write_frame(
                    &mut stdout,
                    &NativeFrame::error(Some(id), -32002, "initialized twice"),
                )?;
                continue;
            }
            "thread/start" => {
                json!({"thread_id":"native-thread-1","history_epoch":0,"continuity_sha256":sts2_harness::sha256_hex("empty-provider-history")})
            }
            "thread/read" => {
                write_frame(
                    &mut stdout,
                    &NativeFrame::notification(sequence, json!({"kind":"history.read","count":1})),
                )?;
                sequence = sequence.saturating_add(1);
                json!({"history_epoch":1,"watermark":1,"coverage":"partial","items":[]})
            }
            "turn/start" => {
                write_frame(
                    &mut stdout,
                    &NativeFrame::notification(
                        sequence,
                        json!({"kind":"turn.started","turn_ref":"turn-1"}),
                    ),
                )?;
                sequence = sequence.saturating_add(1);
                json!({"turn_ref":"turn-1","status":"acknowledged","usage":{"input_tokens":1,"cached_input_tokens":0,"output_tokens":1}})
            }
            "turn/interrupt" => json!({"status":"cancelled"}),
            "thread/fork" => json!({"thread_id":"native-fork-1","history_epoch":1}),
            "thread/compact/start" => json!({"status":"acknowledged","compaction_epoch":1}),
            _ => {
                let _ = params;
                write_frame(
                    &mut stdout,
                    &NativeFrame::error(Some(id), -32601, "method denied"),
                )?;
                continue;
            }
        };
        write_frame(&mut stdout, &NativeFrame::response(id, response))?;
    }
    Ok(())
}

fn write_frame(stdout: &mut impl Write, frame: &NativeFrame) -> Result<(), String> {
    let bytes = frame
        .encode_line()
        .map_err(|_| "encode failed".to_owned())?;
    stdout
        .write_all(&bytes)
        .map_err(|_| "write failed".to_owned())?;
    stdout.flush().map_err(|_| "flush failed".to_owned())
}
