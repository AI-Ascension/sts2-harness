// SPDX-License-Identifier: MIT

//! Wire-level checks that the synthetic entry peer sends no header it did not ask for.
//!
//! The gateway enforces a CLOSED request-header allow-list. CPython's `http.client`
//! sends `Accept-Encoding: identity` from `putrequest` on every HTTP/1.1 request, so a
//! caller that does not opt out is refused with a 400 `unsupported_header` for a header
//! it never chose. These tests drive the ACTUAL generated script against a loopback
//! listener and assert on the real request bytes.
//!
//! Asserting only "the request succeeded" would be vacuous: it passed before the fix on
//! any listener that ignores unknown headers. The load-bearing assertion is ABSENCE.

use super::peer::mcp_server_script;
use super::support::free_loopback_address;
use std::collections::BTreeSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

/// The gateway's published request-header allow-list, copied from
/// `crates/gateway/src/bin/runtime_support/service_authorization.rs::header_is_allowed`
/// at sts2-gateway `ff4cd1c6c0a0e6c4dd2e599e7e07025bed062f69`.
///
/// This embed is a deliberate maintenance obligation, not a convenience. If the gateway
/// widens or renames its list, this test is EXPECTED to fail until these names are
/// updated here. Do not "fix" the failure by loosening the assertions below.
const GATEWAY_HEADER_ALLOW_LIST: [&str; 18] = [
    "authorization",
    "connection",
    "content-length",
    "content-type",
    "host",
    "x-mcp-request-id",
    "x-mcp-session-id",
    "x-sts2-instance-id",
    "x-sts2-caller-id",
    "x-sts2-session-id",
    "x-sts2-lease-id",
    "x-sts2-lease-epoch",
    "x-sts2-workflow-boot-epoch",
    "x-sts2-correlation-id",
    "x-sts2-capabilities-version",
    "x-sts2-episode-profile",
    "x-sts2-peer-token",
    "x-sts2-recovery-capability",
];

/// Capture the header block of a single HTTP/1.1 request from a loopback listener.
fn capture_request_headers() -> Result<Vec<String>, String> {
    let address = free_loopback_address();
    let listener =
        TcpListener::bind(address).map_err(|error| format!("bind {address}: {error}"))?;
    let accept = std::thread::spawn(move || {
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("accept: {error}"))?;
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .map_err(|error| format!("read timeout: {error}"))?;
        let mut bytes = Vec::new();
        let mut byte = [0_u8; 1];
        while !bytes.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => bytes.push(byte[0]),
                Err(error) => return Err(format!("read: {error}")),
            }
            if bytes.len() > 16 * 1024 {
                return Err(String::from("synthetic request headers exceeded bound"));
            }
        }
        let head = String::from_utf8(bytes).map_err(|error| format!("utf8: {error}"))?;
        let content_length: usize = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().ok())?
            })
            .ok_or("request omitted content length")?;
        let mut body = vec![0_u8; content_length];
        stream
            .read_exact(&mut body)
            .map_err(|error| format!("body read: {error}"))?;
        // Answer 200 so the script completes its read and exits cleanly.
        let response = br#"{"ok":true}"#;
        stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    response.len()
                )
                .as_bytes(),
            )
            .and_then(|_| stream.write_all(response))
            .map_err(|error| format!("write: {error}"))?;
        Ok(head)
    });

    let script = write_script();
    let request = jsonrpc_call(tool_and_arguments());
    let mut child = Command::new("python3")
        .arg(&script)
        .env("STS2_GATEWAY_ADDR", address.to_string())
        .env("STS2_SESSION_ID", "session-1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("spawn python3: {error}"))?;
    child
        .stdin
        .as_mut()
        .ok_or("child stdin unavailable")?
        .write_all(request.as_bytes())
        .map_err(|error| format!("write stdin: {error}"))?;
    drop(child.stdin.take());
    child.wait().map_err(|error| format!("wait: {error}"))?;
    let captured = accept
        .join()
        .map_err(|_| String::from("listener thread panicked"))??;
    Ok(captured
        .lines()
        .skip(1)
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, _)| name.trim().to_ascii_lowercase())
        })
        .collect())
}

fn write_script() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "sts2-entry-peer-{}-{:?}.py",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::write(
        &path,
        mcp_server_script(Path::new("/tmp/sts2-entry-peer.log")),
    )
    .expect("write generated peer script");
    path
}

fn jsonrpc_call((name, arguments): (&str, String)) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}\n"
    )
}

fn tool_and_arguments() -> (&'static str, String) {
    (
        "sts2.game_information_detail",
        serde_json::json!({
            "instance_ref":"instance-1",
            "instance_id":"instance-1",
            "definition_ref":"card-1",
            "namespaced_ids":["ns:1"],
            "definition_refs":["card-1"],
            "instance_ids":["instance-1"],
            "projection":"summary",
            "detail_level":"summary",
            "fields":["display_name"],
            "content_manifest_id":"content-1",
            "locale":"en-US",
            "mcp_session_id":"mcp-session-1",
            "lease_id":"lease-1",
            "lease_epoch":1,
            "page_items":4,
            "item_bytes":4096,
            "page_bytes":65536,
            "text_bytes":4096,
            "cursor":null
        })
        .to_string(),
    )
}

#[test]
fn the_generated_peer_script_is_valid_python() {
    let script = write_script();
    let output = Command::new("python3")
        .args(["-m", "py_compile"])
        .arg(&script)
        .output()
        .expect("run py_compile");
    assert!(
        output.status.success(),
        "generated peer script is not valid python: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_generated_peer_sends_no_accept_encoding_header() {
    let headers = capture_request_headers().expect("capture the peer's request headers");
    assert!(
        !headers.iter().any(|name| name == "accept-encoding"),
        "the peer must not send accept-encoding; CPython injects it unless putrequest is \
         called with skip_accept_encoding=True. Captured: {headers:?}"
    );
}

#[test]
fn every_header_the_peer_sends_is_on_the_gateway_allow_list() {
    let headers = capture_request_headers().expect("capture the peer's request headers");
    let allow: BTreeSet<&str> = GATEWAY_HEADER_ALLOW_LIST.into_iter().collect();
    let unexpected: Vec<&String> = headers
        .iter()
        .filter(|name| !allow.contains(name.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "the peer sent headers outside the gateway's closed allow-list: {unexpected:?} \
         (sent: {headers:?})"
    );
}

#[test]
fn a_header_the_peer_does_not_set_is_still_absent() {
    let headers = capture_request_headers().expect("capture the peer's request headers");
    // A scoping assertion, so a future "just send everything" edit cannot pass.
    for absent in [
        "authorization",
        "x-sts2-peer-token",
        "x-sts2-recovery-capability",
        "x-sts2-caller-id",
    ] {
        assert!(
            !headers.iter().any(|name| name == absent),
            "the peer must not send {absent}; it never asked for it. Sent: {headers:?}"
        );
    }
}
